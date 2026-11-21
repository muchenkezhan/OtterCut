use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui;
use takeseam::editor_timeline::{Clip, Timeline, Track, TrackKind};

use super::background::BackgroundSettings;
use super::canvas_format::CanvasFormat;
use super::library::Library;
use super::media::overlays::{Overlays, sequence_size};
use super::media::{MediaInfo, MediaSlice, MediaTools, Preview, PreviewPlan, PreviewQuality};

pub(super) struct Span<'a> {
    key: SpanKey,
    tracks: &'a [Track],
}

#[derive(PartialEq, Eq)]
struct SpanKey {
    start: u64,
    end: u64,
    canvas: Option<PathBuf>,
    canvas_format: CanvasFormat,
    background: BackgroundSettings,
    video: Option<Source>,
    audio: Vec<Source>,
    overlay_revision: Option<u64>,
    excluded_overlay: Option<u64>,
}

#[derive(Clone, PartialEq, Eq)]
struct Source {
    path: PathBuf,
    on_timeline: u64,
    source_start: u64,
    source_end: u64,
    volume_db_tenths: i16,
}

impl Source {
    fn for_clip(clip: &Clip) -> Self {
        Self {
            path: clip.path.clone(),
            on_timeline: clip.start,
            source_start: clip.source_start,
            source_end: clip.source_end,
            volume_db_tenths: clip.volume_db_tenths,
        }
    }

    fn slice(&self, from: u64, to: u64, source_duration: u64, paused: bool) -> Option<MediaSlice> {
        let offset = |time: u64| {
            self.source_start
                .saturating_add(time.saturating_sub(self.on_timeline))
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
            path: self.path.clone(),
            range: offset(from).min(last)..end,
            volume_db_tenths: self.volume_db_tenths,
        })
    }
}

impl<'a> Span<'a> {
    pub(super) fn for_asset(path: &Path, info: MediaInfo) -> Self {
        let source = Source {
            path: path.to_owned(),
            on_timeline: 0,
            source_start: 0,
            source_end: info.duration_us,
            volume_db_tenths: 0,
        };
        Self {
            key: SpanKey {
                start: 0,
                end: info.duration_us,
                canvas: None,
                canvas_format: CanvasFormat::Original,
                background: BackgroundSettings::default(),
                video: info.has_video.then(|| source.clone()),
                audio: info.has_audio.then_some(source).into_iter().collect(),
                overlay_revision: None,
                excluded_overlay: None,
            },
            tracks: &[],
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
        })
    }

    pub(super) fn with_excluded_overlay(mut self, id: Option<u64>) -> Self {
        self.key.excluded_overlay = id;
        self
    }

    pub(super) fn with_canvas_format(mut self, canvas_format: CanvasFormat) -> Self {
        self.key.canvas_format = canvas_format;
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
                audio: Vec::new(),
                overlay_revision: None,
                excluded_overlay: None,
            },
            tracks: &[],
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
        let from = position.min(key.end.saturating_sub(1));
        let excluded_overlay = if playing { None } else { key.excluded_overlay };
        Some(Ok(PreviewPlan {
            timeline_from: from,
            duration_us: key.end.saturating_sub(from).max(1),
            size,
            quality,
            background: key.background.clone(),
            video,
            audio,
            black_canvas: key.overlay_revision.is_some(),
            overlays: Overlays::in_range_excluding(self.tracks, from..key.end, excluded_overlay),
            excluded_overlay,
        }))
    }
}

#[derive(Default)]
pub(super) struct Player {
    pub(super) playing: bool,
    quality: PreviewQuality,
    preview: Preview,
    active: Option<Active>,
}

struct Active {
    span: SpanKey,
    from: u64,
    playing: bool,
    quality: PreviewQuality,
    requested: bool,
    error: Option<String>,
}

impl Player {
    pub(super) fn reset(&mut self) {
        self.active = None;
        self.preview.clear();
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
        let Some(span) = span else {
            if self.active.is_some() {
                self.reset();
            }
            return None;
        };
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
                    && (active.from == position
                        || (active.playing && (active.from..=active.span.end).contains(&position)))
            });
            if keep_picture {
                // Keep the displayed frame and its overlay stamp until the replacement arrives.
                self.preview.stop();
            } else {
                self.preview.clear();
            }
        }
        let plan = tools.and_then(|_| span.plan(library, position, self.playing, self.quality));
        if changed {
            self.active = Some(Active {
                span: span.key,
                from: position,
                playing: self.playing,
                quality: self.quality,
                requested: false,
                error: None,
            });
        }
        let active = self.active.as_mut()?;
        let tools = tools?;
        if !active.requested {
            match plan? {
                Ok(plan) => {
                    self.preview.start(tools, ctx, plan, self.playing);
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
