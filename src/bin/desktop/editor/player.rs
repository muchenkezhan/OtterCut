use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use takeseam::editor_timeline::{Clip, Timeline, Track, TrackKind};

use super::background::BackgroundSettings;
use super::canvas_format::CanvasFormat;
use super::cursor_effect::{CursorEffect, CursorTelemetry};
use super::library::Library;
use super::media::overlays::{Overlays, sequence_size};
use super::media::{MediaInfo, MediaSlice, MediaTools, Preview, PreviewPlan, PreviewQuality};

pub(super) struct Span<'a> {
    key: SpanKey,
    tracks: &'a [Track],
    cursor_effect: Option<CursorEffect>,
}

#[derive(Clone, PartialEq, Eq)]
struct SpanKey {
    start: u64,
    end: u64,
    canvas: Option<PathBuf>,
    canvas_format: CanvasFormat,
    background: BackgroundSettings,
    video: Option<Source>,
    video_decode_path: Option<PathBuf>,
    audio: Vec<Source>,
    overlay_revision: Option<u64>,
    excluded_overlay: Option<u64>,
}

#[derive(Clone, PartialEq, Eq)]
struct Source {
    id: u64,
    path: PathBuf,
    on_timeline: u64,
    source_start: u64,
    source_end: u64,
    volume_db_tenths: i16,
    fade_in_us: u64,
    fade_out_us: u64,
    speed_milli: u16,
    reverse: bool,
}

impl Source {
    fn for_clip(clip: &Clip) -> Self {
        Self {
            id: clip.id,
            path: clip.path.clone(),
            on_timeline: clip.start,
            source_start: clip.source_start,
            source_end: clip.source_end,
            volume_db_tenths: clip.volume_db_tenths,
            fade_in_us: clip.fade_in_us,
            fade_out_us: clip.fade_out_us,
            speed_milli: clip.speed_milli,
            reverse: clip.reverse,
        }
    }

    fn slice(&self, from: u64, to: u64, source_duration: u64, paused: bool) -> Option<MediaSlice> {
        let offset = |time: u64| {
            self.source_start
                .saturating_add(
                    time.saturating_sub(self.on_timeline)
                        .saturating_mul(u64::from(self.speed_milli.max(1)))
                        .checked_div(1000)
                        .unwrap_or(u64::MAX),
                )
        };
        let end = offset(to).min(self.source_end).min(source_duration);
        if end <= self.source_start || (!paused && offset(from) >= end) {
            return None;
        }
        let last = if paused && from >= to {
            end.saturating_sub(33_334).max(self.source_start)
        } else {
            end - 1
        };
        Some(MediaSlice {
            id: self.id,
            path: self.path.clone(),
            range: offset(from).min(last)..end,
            timeline_start: self.on_timeline,
            timeline_duration_us: self
                .source_end
                .saturating_sub(self.source_start)
                .saturating_mul(1000)
                .checked_div(u64::from(self.speed_milli.max(1)))
                .unwrap_or(1),
            volume_db_tenths: self.volume_db_tenths,
            fade_in_us: self.fade_in_us,
            fade_out_us: self.fade_out_us,
            speed_milli: self.speed_milli,
            reverse: self.reverse,
        })
    }
}

impl<'a> Span<'a> {
    pub(super) fn for_asset(path: &Path, info: MediaInfo) -> Self {
        let source = Source {
            id: 0,
            path: path.to_owned(),
            on_timeline: 0,
            source_start: 0,
            source_end: info.duration_us,
            volume_db_tenths: 0,
            fade_in_us: 0,
            fade_out_us: 0,
            speed_milli: 1000,
            reverse: false,
        };
        Self {
            key: SpanKey {
                start: 0,
                end: info.duration_us,
                canvas: None,
                canvas_format: CanvasFormat::Original,
                background: BackgroundSettings::default(),
                video: info.has_video.then(|| source.clone()),
                video_decode_path: None,
                audio: info.has_audio.then_some(source).into_iter().collect(),
                overlay_revision: None,
                excluded_overlay: None,
            },
            tracks: &[],
            cursor_effect: None,
        }
    }

    pub(super) fn at(timeline: &'a Timeline, position: u64) -> Option<Self> {
        let end = timeline.duration();
        if end == 0 {
            return None;
        }
        let position = position.min(end - 1);
        let mut key = SpanKey {
            start: 0,
            end,
            canvas: None,
            canvas_format: CanvasFormat::Original,
            background: BackgroundSettings::default(),
            video: None,
            video_decode_path: None,
            audio: Vec::new(),
            overlay_revision: None,
            excluded_overlay: None,
        };
        for track in timeline.tracks() {
            if track.kind.is_overlay() {
                if !track.clips.is_empty() {
                    key.overlay_revision = Some(timeline.revision());
                }
                continue;
            }
            if !track.enabled {
                continue;
            }
            if track.kind == TrackKind::Video
                && let Some(first) = track.clips.first()
            {
                // A lower video track sets the sequence canvas's aspect ratio.
                key.canvas = Some(first.path.clone());
            }
            let next = track.clips.partition_point(|clip| clip.start <= position);
            if let Some(clip) = track.clips.get(next) {
                key.end = key.end.min(clip.start);
            }
            if next == 0 {
                continue;
            }
            let clip = &track.clips[next - 1];
            if clip.end() <= position {
                key.start = key.start.max(clip.end());
                continue;
            }
            key.start = key.start.max(clip.start);
            key.end = key.end.min(clip.end());
            let source = Source::for_clip(clip);
            if key.video.is_none() && track.kind == TrackKind::Video {
                key.video = Some(source.clone());
            }
            // Audio stream availability comes from the media probe, including the video's original sound.
            key.audio.push(source);
        }
        Some(Self {
            key,
            tracks: timeline.tracks(),
            cursor_effect: None,
        })
    }

    pub(super) fn with_excluded_overlay(mut self, id: Option<u64>) -> Self {
        self.key.excluded_overlay = id;
        self
    }

    pub(super) fn with_video_decode_path(mut self, path: Option<PathBuf>) -> Self {
        self.key.video_decode_path = path;
        self
    }

    pub(super) fn with_volume_override(mut self, id: Option<u64>, volume: Option<i16>) -> Self {
        let (Some(id), Some(volume)) = (id, volume) else {
            return self;
        };
        for source in &mut self.key.audio {
            if source.id == id {
                source.volume_db_tenths = volume;
            }
        }
        self
    }

    pub(super) fn with_canvas_format(mut self, canvas_format: CanvasFormat) -> Self {
        self.key.canvas_format = canvas_format;
        self
    }

    pub(super) fn with_cursor_effect(mut self, effect: &CursorEffect) -> Self {
        self.cursor_effect = Some(effect.clone());
        self
    }

    pub(super) fn for_background(duration: u64) -> Self {
        Self {
            key: SpanKey {
                start: 0,
                end: duration.max(1),
                canvas: None,
                canvas_format: CanvasFormat::Original,
                background: BackgroundSettings::default(),
                video: None,
                video_decode_path: None,
                audio: Vec::new(),
                overlay_revision: None,
                excluded_overlay: None,
            },
            tracks: &[],
            cursor_effect: None,
        }
    }

    pub(super) fn with_background(mut self, background: &BackgroundSettings) -> Self {
        self.key.background.clone_from(background);
        self
    }

    fn plan(
        &self,
        library: &mut Library,
        position: u64,
        playing: bool,
        quality: PreviewQuality,
    ) -> Option<Result<PreviewPlan, String>> {
        let key = &self.key;
        let mut source_info = None;
        let mut video = None;
        if let Some(source) = &key.video {
            let info = match library.request(&source.path)? {
                Ok(info) => info,
                Err(error) => return Some(Err(error.to_owned())),
            };
            if !info.has_video {
                return Some(Err(String::from("视频轨上的素材没有可用画面")));
            }
            source_info = Some(info);
            video = source.slice(position, key.end, info.duration_us, !playing);
            if video.is_none() {
                return Some(Err(String::from("视频片段超出素材时长")));
            }
        }
        let size = match key.canvas_format.dimensions() {
            Some(dimensions) => dimensions,
            None => {
                let canvas = match key.canvas.as_deref() {
                    Some(path) if key.video.as_ref().is_none_or(|source| path != source.path) => {
                        library
                            .request(path)?
                            .ok()
                            .filter(|info| info.has_video)
                            .or(source_info)
                    }
                    _ => source_info,
                };
                if self.tracks.is_empty() {
                    canvas.map_or((1280, 720), |info| (info.width, info.height))
                } else {
                    sequence_size(canvas)
                }
            }
        };
        let mut audio = Vec::new();
        if playing {
            for source in &key.audio {
                let info = match library.request(&source.path)? {
                    Ok(info) => info,
                    Err(error) => return Some(Err(error.to_owned())),
                };
                if info.has_audio
                    && let Some(slice) = source.slice(position, key.end, info.duration_us, false)
                {
                    audio.push(slice);
                }
            }
        }
        let cursor = key.video.as_ref().and_then(|source| {
            let path = PathBuf::from(format!("{}.cursor.json", source.path.display()));
            path.is_file()
                .then(|| CursorTelemetry::load(&path).ok())
                .flatten()
        });
        let from = position.min(key.end.saturating_sub(1));
        let excluded_overlay = if playing { None } else { key.excluded_overlay };
        Some(Ok(PreviewPlan {
            timeline_from: from,
            duration_us: key.end.saturating_sub(from).max(1),
            audio_timeline_start: key.start,
            audio_position: position,
            size,
            quality,
            background: key.background.clone(),
            video,
            video_decode_path: key.video_decode_path.clone(),
            audio,
            black_canvas: key.overlay_revision.is_some(),
            overlays: Overlays::in_range_excluding(self.tracks, from..key.end, excluded_overlay),
            excluded_overlay,
            cursor,
            cursor_effect: self
                .cursor_effect
                .clone()
                .unwrap_or_else(|| CursorEffect::default_for(key.end)),
        }))
    }

    fn audio_plan(
        &self,
        library: &mut Library,
        position: u64,
    ) -> Option<Result<PreviewPlan, String>> {
        let audio_timeline_start = position;
        let duration_us = self.key.end.saturating_sub(position).min(400_000).max(1);
        let mut audio = Vec::new();
        for source in &self.key.audio {
            let info = match library.request(&source.path)? {
                Ok(info) => info,
                Err(error) => return Some(Err(error.to_owned())),
            };
            if info.has_audio
                && let Some(slice) = source.slice(
                    position,
                    position.saturating_add(duration_us),
                    info.duration_us,
                    false,
                )
            {
                audio.push(slice);
            }
        }
        Some(Ok(PreviewPlan {
            timeline_from: audio_timeline_start,
            duration_us,
            audio_timeline_start,
            audio_position: position,
            size: (2, 2),
            quality: PreviewQuality::Sixteenth,
            background: BackgroundSettings::default(),
            video: None,
            video_decode_path: None,
            audio,
            black_canvas: false,
            overlays: Overlays::default(),
            excluded_overlay: None,
            cursor: None,
            cursor_effect: CursorEffect::default_for(duration_us),
        }))
    }

    fn audio_cache_plan(
        &self,
        library: &mut Library,
        position: u64,
    ) -> Option<Result<PreviewPlan, String>> {
        let audio_timeline_start = self.key.start;
        let duration_us = self.key.end.saturating_sub(audio_timeline_start).max(1);
        let mut audio = Vec::new();
        for source in &self.key.audio {
            let info = match library.request(&source.path)? {
                Ok(info) => info,
                Err(error) => return Some(Err(error.to_owned())),
            };
            if info.has_audio
                && let Some(slice) = source.slice(
                    audio_timeline_start,
                    self.key.end,
                    info.duration_us,
                    false,
                )
            {
                audio.push(slice);
            }
        }
        Some(Ok(PreviewPlan {
            timeline_from: audio_timeline_start,
            duration_us,
            audio_timeline_start,
            audio_position: position,
            size: (2, 2),
            quality: PreviewQuality::Sixteenth,
            background: BackgroundSettings::default(),
            video: None,
            video_decode_path: None,
            audio,
            black_canvas: false,
            overlays: Overlays::default(),
            excluded_overlay: None,
            cursor: None,
            cursor_effect: CursorEffect::default_for(duration_us),
        }))
    }
}

impl SpanKey {
    /// Returns true when only the gain of one or more existing audio sources changed.
    ///
    /// Video and audio source placement are deliberately compared here. A change to a
    /// clip's position, source range, or media path must still rebuild the complete preview.
    fn differs_only_in_audio_volume(&self, other: &Self) -> bool {
        self.start == other.start
            && self.end == other.end
            && self.canvas == other.canvas
            && self.canvas_format == other.canvas_format
            && self.background == other.background
            && self.video == other.video
            && self.overlay_revision == other.overlay_revision
            && self.excluded_overlay == other.excluded_overlay
            && self.audio.len() == other.audio.len()
            && self
                .audio
                .iter()
                .zip(&other.audio)
                .all(|(left, right)| left.same_media(right))
            && self
                .audio
                .iter()
                .zip(&other.audio)
                .any(|(left, right)| left.volume_db_tenths != right.volume_db_tenths)
    }

    fn changed_audio_volume(&self, other: &Self) -> Option<(u64, i16)> {
        if !self.differs_only_in_audio_volume(other) {
            return None;
        }
        let mut changed = self
            .audio
            .iter()
            .zip(&other.audio)
            .filter(|(left, right)| left.volume_db_tenths != right.volume_db_tenths);
        let (_, right) = changed.next()?;
        changed.next().is_none().then_some((right.id, right.volume_db_tenths))
    }
}

impl Source {
    fn same_media(&self, other: &Self) -> bool {
        self.id == other.id
            && self.path == other.path
            && self.on_timeline == other.on_timeline
            && self.source_start == other.source_start
            && self.source_end == other.source_end
    }
}

#[derive(Default)]
pub(super) struct Player {
    pub(super) playing: bool,
    quality: PreviewQuality,
    preview: Preview,
    active: Option<Active>,
    scrubbing: bool,
    resume_after_scrub: bool,
    retain_frame_on_sync: bool,
    last_scrub_audio: Option<Instant>,
    scrub_audio_prepared: bool,
    scrub_audio_stop_at: Option<Instant>,
}

struct Active {
    span: SpanKey,
    from: u64,
    playing: bool,
    quality: PreviewQuality,
    audio_base_volumes: Vec<(u64, i16)>,
    requested: bool,
    error: Option<String>,
}

impl Player {
    pub(super) fn reset(&mut self) {
        self.scrubbing = false;
        self.resume_after_scrub = false;
        self.retain_frame_on_sync = false;
        self.last_scrub_audio = None;
        self.scrub_audio_prepared = false;
        self.scrub_audio_stop_at = None;
        self.active = None;
        self.preview.clear();
    }

    pub(super) fn begin_seek(&mut self) {
        if self.scrubbing {
            return;
        }
        self.scrubbing = true;
        self.resume_after_scrub = self.playing;
        self.playing = false;
        self.active = None;
        self.retain_frame_on_sync = true;
        self.last_scrub_audio = None;
        self.scrub_audio_stop_at = None;
        self.preview.stop();
    }

    pub(super) fn end_seek(&mut self) {
        if !self.scrubbing {
            return;
        }
        self.scrubbing = false;
        self.playing = self.resume_after_scrub;
        self.resume_after_scrub = false;
        let had_scrub_audio = self.last_scrub_audio.is_some();
        self.last_scrub_audio = None;
        self.active = None;
        self.scrub_audio_stop_at = had_scrub_audio.then(|| Instant::now() + Duration::from_millis(400));
    }

    pub(super) fn play_scrub_audio(
        &mut self,
        tools: Option<&MediaTools>,
        span: Span<'_>,
        library: &mut Library,
        position: u64,
    ) {
        if !self.scrubbing {
            return;
        }
        let now = Instant::now();
        if self
            .last_scrub_audio
            .is_some_and(|last| now.duration_since(last) < Duration::from_millis(100))
        {
            return;
        }
        self.last_scrub_audio = Some(now);
        let Some(tools) = tools else {
            return;
        };
        let Some(plan) = span.audio_plan(library, position) else {
            return;
        };
        let Ok(plan) = plan else {
            return;
        };
        if plan.audio.is_empty() {
            return;
        }
        self.preview.play_scrub_audio(
            tools,
            plan.audio,
            plan.duration_us,
            plan.audio_timeline_start,
            plan.audio_position,
        );
    }

    pub(super) fn prepare_scrub_audio(
        &mut self,
        tools: Option<&MediaTools>,
        span: Span<'_>,
        library: &mut Library,
        position: u64,
    ) {
        if self.scrub_audio_prepared {
            return;
        }
        let Some(tools) = tools else {
            return;
        };
        let Some(plan) = span.audio_cache_plan(library, position) else {
            return;
        };
        let Ok(plan) = plan else {
            return;
        };
        if plan.audio.is_empty() {
            return;
        }
        self.scrub_audio_prepared = true;
        self.preview.prepare_scrub_audio(
            tools,
            plan.audio,
            plan.duration_us,
            plan.audio_timeline_start,
        );
    }

    pub(super) fn is_scrubbing(&self) -> bool {
        self.scrubbing
    }

    pub(super) fn toggle(&mut self, position: &mut u64, duration: u64) {
        if duration == 0 {
            return;
        }
        self.playing = !self.playing;
        if self.playing && *position >= duration {
            *position = 0;
            self.reset();
            return;
        }
        self.preview.stop();
        if let Some(active) = &mut self.active {
            active.requested = false;
        }
    }

    pub(super) fn quality(&self) -> PreviewQuality {
        self.quality
    }

    pub(super) fn set_quality(&mut self, quality: PreviewQuality) {
        self.quality = quality;
    }

    pub(super) fn texture(&self) -> Option<&egui::TextureHandle> {
        self.preview.texture()
    }

    pub(super) fn texture_composition_size(&self) -> Option<(u32, u32)> {
        self.preview.texture_composition_size()
    }

    pub(super) fn texture_excludes(&self, id: u64) -> bool {
        self.preview.texture_excludes(id)
    }

    pub(super) fn error(&self) -> Option<&str> {
        self.preview.error()
    }

    pub(super) fn audio_error(&self) -> Option<&str> {
        self.preview.audio_error()
    }

    pub(super) fn blank_message(&self) -> Option<&'static str> {
        let span = &self.active.as_ref()?.span;
        (span.video.is_none() && span.overlay_revision.is_none() && !span.background.enabled())
            .then_some(if span.audio.is_empty() {
                "间隙"
            } else {
                "此处只有音频"
            })
    }

    pub(super) fn advance(&mut self, ctx: &egui::Context, position: &mut u64, duration: u64) {
        self.preview.poll(ctx);
        if !self.playing {
            return;
        }
        if self.preview.error().is_some()
            && self
                .active
                .as_ref()
                .is_some_and(|active| active.playing && active.requested)
        {
            self.playing = false;
            self.preview.stop();
            return;
        }
        if let Some(active) = &self.active
            && let Some(elapsed) = self.preview.playback_elapsed()
        {
            let passed = elapsed.as_micros().min(u64::MAX as u128) as u64;
            *position = active
                .from
                .saturating_add(passed)
                .min(active.span.end)
                .min(duration);
        }
        if *position >= duration {
            self.playing = false;
            self.preview.stop();
        } else {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    pub(super) fn sync(
        &mut self,
        ctx: &egui::Context,
        tools: Option<&MediaTools>,
        span: Option<Span<'_>>,
        library: &mut Library,
        position: u64,
    ) -> Option<String> {
        if let Some(stop_at) = self.scrub_audio_stop_at {
            if Instant::now() < stop_at {
                return None;
            }
            self.preview.stop_scrub_audio();
            self.scrub_audio_stop_at = None;
        }
        if self.scrubbing {
            return None;
        }
        let Some(span) = span else {
            if self.active.is_some() {
                self.reset();
            }
            return None;
        };
        let volume_change = self
            .active
            .as_ref()
            .and_then(|active| active.span.changed_audio_volume(&span.key));
        let volume_only = volume_change.is_some();
        let retain_frame = self.retain_frame_on_sync;
        let changed = self.active.as_ref().is_none_or(|active| {
            active.span != span.key
                || active.playing != self.playing
                || active.quality != self.quality
                || (!self.playing && active.from != position)
        });
        let requested = !changed && self.active.as_ref().is_some_and(|active| active.requested);
        if requested {
            return self.active.as_ref()?.error.clone();
        }
        if changed {
            let keep_picture = self.active.as_ref().is_some_and(|active| {
                (span.key.video.is_some()
                    || span.key.overlay_revision.is_some()
                    || span.key.background.enabled())
                    && active.span.canvas == span.key.canvas
                    && active.span.video == span.key.video
                    && active.span.start <= position
                    && position < active.span.end
            });
            if volume_only {
                // Volume automation is handled by the audio path below. Keep the
                // video decoder and its current frame alive while the line moves.
            } else if retain_frame || keep_picture {
                // Keep the displayed frame and its overlay stamp until the replacement arrives.
                self.preview.stop();
            } else {
                self.preview.clear();
            }
        }
        let plan = tools.and_then(|_| span.plan(library, position, self.playing, self.quality));
        let previous_from = self.active.as_ref().map(|active| active.from);
        if changed {
            self.active = Some(Active {
                span: span.key.clone(),
                from: if volume_only {
                    previous_from.unwrap_or(position)
                } else {
                    position
                },
                playing: self.playing,
                quality: self.quality,
                audio_base_volumes: if volume_only {
                    self.active
                        .as_ref()
                        .map(|active| active.audio_base_volumes.clone())
                        .unwrap_or_default()
                } else {
                    span.key
                        .audio
                        .iter()
                        .map(|source| (source.id, source.volume_db_tenths))
                        .collect()
                },
                requested: false,
                error: None,
            });
            self.retain_frame_on_sync = false;
        }
        let active = self.active.as_mut()?;
        let tools = tools?;
        if !active.requested {
            match plan? {
                Ok(plan) => {
                    if volume_only && self.playing {
                        if let Some((source_id, target)) = volume_change {
                            let base = active
                                .audio_base_volumes
                                .iter()
                                .find_map(|(id, volume)| (*id == source_id).then_some(*volume))
                                .unwrap_or(target);
                            self.preview.set_live_volume(source_id, target - base);
                        }
                    } else {
                        self.preview.start(tools, ctx, plan, self.playing);
                    }
                    active.error = None;
                    active.requested = true;
                }
                Err(error) => {
                    self.playing = false;
                    active.error = Some(error);
                    active.requested = true;
                }
            }
        }
        active.error.clone()
    }
}

#[cfg(test)]
#[path = "player/preview_tests.rs"]
mod preview_tests;
