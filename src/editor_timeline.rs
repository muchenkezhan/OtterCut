//! Non-destructive edits and history for media and text tracks.

use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::path::PathBuf;

#[path = "editor_timeline/overlay.rs"]
mod overlay;

pub use overlay::{
    OverlayText, TextAppearance, TextPaint, TextPlacement, TextPosition, TextStyle,
    TextStyleChanges,
};

const HISTORY_LIMIT: usize = 100;

pub type TrackId = u64;
pub const PRIMARY_VIDEO: TrackId = 1;
pub const PRIMARY_AUDIO: TrackId = 2;
const DEFAULT_TRACK_ID_MAX: TrackId = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TrackKind {
    Text,
    Caption,
    Video,
    Audio,
}

impl TrackKind {
    pub fn is_overlay(self) -> bool {
        matches!(self, Self::Text | Self::Caption)
    }

    /// The order in which track groups appear from top to bottom.
    pub fn stack_order(self) -> u8 {
        match self {
            Self::Text => 0,
            Self::Caption => 1,
            Self::Video => 2,
            Self::Audio => 3,
        }
    }
}

/// A track's clips are sorted by time and never overlap one another.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    pub enabled: bool,
    pub clips: Vec<Clip>,
}

impl Track {
    fn new(id: TrackId, kind: TrackKind) -> Self {
        Self {
            id,
            kind,
            enabled: true,
            clips: Vec::new(),
        }
    }

    pub fn duration(&self) -> u64 {
        self.clips.last().map_or(0, Clip::end)
    }

    pub fn clip_at(&self, time: u64) -> Option<&Clip> {
        let index = self.clips.partition_point(|clip| clip.end() <= time);
        self.clips.get(index).filter(|clip| clip.start <= time)
    }
}

/// A contiguous slice of media or generated text. All times are in microseconds.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TransitionSpec {
    pub id: String,
    pub duration_us: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub role: String,
}

/// A contiguous slice of media or generated text. All times are in microseconds.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Clip {
    pub id: u64,
    pub path: PathBuf,
    pub media_duration: u64,
    pub source_start: u64,
    pub source_end: u64,
    pub start: u64,
    #[serde(default, skip_serializing_if = "is_default_volume")]
    pub volume_db_tenths: i16,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fade_in_us: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fade_out_us: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_in: Option<TransitionSpec>,
    #[serde(default = "default_speed_milli", skip_serializing_if = "is_normal_speed")]
    pub speed_milli: u16,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reverse: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub timeline_duration_us: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<OverlayText>,
    /// Sorted source timestamps of click events suppressed for this clip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deleted_cursor_clicks_us: Vec<u64>,
}

fn is_default_volume(volume: &i16) -> bool {
    *volume == 0
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

fn default_speed_milli() -> u16 {
    1000
}

fn is_normal_speed(value: &u16) -> bool {
    *value == 1000
}

impl Clip {
    pub const MIN_VOLUME_DB_TENTHS: i16 = -600;
    pub const MAX_VOLUME_DB_TENTHS: i16 = 120;

    fn full_source(id: u64, path: PathBuf, duration: u64, start: u64) -> Self {
        Self {
            id,
            path,
            media_duration: duration,
            source_start: 0,
            source_end: duration,
            start,
            volume_db_tenths: 0,
            fade_in_us: 0,
            fade_out_us: 0,
            transition_in: None,
            speed_milli: 1000,
            reverse: false,
            timeline_duration_us: 0,
            group_id: None,
            text: None,
            deleted_cursor_clicks_us: Vec::new(),
        }
    }

    fn sort_deleted_cursor_clicks(&mut self) {
        self.deleted_cursor_clicks_us.sort_unstable();
        self.deleted_cursor_clicks_us.dedup();
    }

    pub fn duration(&self) -> u64 {
        if self.timeline_duration_us > 0 {
            return self.timeline_duration_us;
        }
        timeline_duration(
            self.source_end.saturating_sub(self.source_start),
            self.speed_milli.max(1),
        )
    }

    pub fn set_speed(&mut self, speed_milli: u16) {
        self.speed_milli = speed_milli.clamp(500, 2000);
        self.timeline_duration_us = if self.speed_milli == 1000 {
            0
        } else {
            timeline_duration(
                self.source_end.saturating_sub(self.source_start),
                self.speed_milli,
            )
            .max(1)
        };
    }

    /// The end position is exclusive.
    pub fn end(&self) -> u64 {
        self.start.saturating_add(self.duration())
    }

    pub fn source_time_at(&self, timeline_time: u64) -> u64 {
        let offset = timeline_time.saturating_sub(self.start);
        self.source_start.saturating_add(
            offset
                .saturating_mul(u64::from(self.speed_milli.max(1)))
                .checked_div(1000)
                .unwrap_or(u64::MAX),
        )
    }

    /// Maximum amount of time by which the in-point can be extended.
    pub fn trim_headroom(&self) -> u64 {
        if self.text.is_some() {
            self.start
        } else {
            self.source_start
                .saturating_mul(1000)
                .checked_div(u64::from(self.speed_milli.max(1)))
                .unwrap_or(u64::MAX)
        }
    }

    /// Maximum amount of time by which the out-point can be extended.
    pub fn trim_tailroom(&self) -> u64 {
        if self.text.is_some() {
            u64::MAX - self.end()
        } else {
            self.media_duration
                .saturating_sub(self.source_end)
                .saturating_mul(1000)
                .checked_div(u64::from(self.speed_milli.max(1)))
                .unwrap_or(u64::MAX)
        }
    }

    fn shift_in_point(&mut self, delta: i128) -> Result<(), TimelineError> {
        if self.text.is_some() {
            return self.change_text_duration(-delta);
        }
        let source_delta = delta
            .saturating_mul(i128::from(self.speed_milli.max(1)))
            .checked_div(1000)
            .ok_or(TimelineError::InvalidSourceRange)?;
        self.source_start = shift_source_time(self.source_start, source_delta)?;
        if self.timeline_duration_us > 0 {
            self.timeline_duration_us = shift_source_time(self.timeline_duration_us, -delta)?;
        }
        Ok(())
    }

    fn shift_out_point(&mut self, delta: i128) -> Result<(), TimelineError> {
        if self.text.is_some() {
            return self.change_text_duration(delta);
        }
        let source_delta = delta
            .saturating_mul(i128::from(self.speed_milli.max(1)))
            .checked_div(1000)
            .ok_or(TimelineError::InvalidSourceRange)?;
        self.source_end = shift_source_time(self.source_end, source_delta)?;
        if self.timeline_duration_us > 0 {
            self.timeline_duration_us = shift_source_time(self.timeline_duration_us, delta)?;
        }
        Ok(())
    }

    fn change_text_duration(&mut self, delta: i128) -> Result<(), TimelineError> {
        if delta != 0 {
            let duration = shift_source_time(self.duration(), delta)?;
            self.media_duration = duration;
            self.source_start = 0;
            self.source_end = duration;
        }
        Ok(())
    }
}

fn timeline_duration(source_duration: u64, speed_milli: u16) -> u64 {
    let duration = u128::from(source_duration) * 1000 / u128::from(speed_milli);
    u64::try_from(duration).unwrap_or(u64::MAX)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineError {
    ClipNotFound(u64),
    DuplicateClipId(u64),
    TrackNotFound(TrackId),
    DuplicateTrackId(TrackId),
    IncompatibleTrack,
    InvalidText,
    InvalidTextSize,
    InvalidTextPosition,
    InvalidVolume,
    InvalidSpeed,
    InvalidLink,
    InvalidSourceRange,
    InvalidSplitPoint,
    Overlap,
    MissingAdjacentClip,
    TimeOutOfBounds,
    ClipIdExhausted,
    TrackIdExhausted,
}

impl fmt::Display for TimelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClipNotFound(id) => write!(f, "找不到片段 {id}"),
            Self::DuplicateClipId(id) => write!(f, "片段编号 {id} 重复"),
            Self::TrackNotFound(id) => write!(f, "找不到轨道 {id}"),
            Self::DuplicateTrackId(id) => write!(f, "轨道编号 {id} 重复"),
            Self::IncompatibleTrack => f.write_str("无法在目标轨道上执行该编辑"),
            Self::InvalidText => write!(
                f,
                "请输入可显示的文字，最多 {} 个字符",
                OverlayText::MAX_CHARS
            ),
            Self::InvalidTextSize => write!(
                f,
                "字号须在 {} 到 {} 之间",
                TextStyle::MIN_SIZE,
                TextStyle::MAX_SIZE
            ),
            Self::InvalidTextPosition => f.write_str("文字位置超出画面范围"),
            Self::InvalidVolume => write!(
                f,
                "片段音量须在 {} dB 到 {} dB 之间",
                Clip::MIN_VOLUME_DB_TENTHS / 10,
                Clip::MAX_VOLUME_DB_TENTHS / 10
            ),
            Self::InvalidSpeed => f.write_str("片段速度须在 0.5x 到 2x 之间"),
            Self::InvalidLink => f.write_str("至少选择两个片段才能建立链接"),
            Self::InvalidSourceRange => f.write_str("剪辑超出素材范围，或片段时长为零"),
            Self::InvalidSplitPoint => f.write_str("分割点必须位于片段内部"),
            Self::Overlap => f.write_str("剪辑会使两个片段重叠"),
            Self::MissingAdjacentClip => f.write_str("此操作需要无空隙的相邻片段"),
            Self::TimeOutOfBounds => f.write_str("剪辑超出时间线范围"),
            Self::ClipIdExhausted => f.write_str("片段编号已用尽"),
            Self::TrackIdExhausted => f.write_str("轨道编号已用尽"),
        }
    }
}

impl std::error::Error for TimelineError {}

pub struct Timeline {
    tracks: Vec<Track>,
    undo: VecDeque<Vec<Track>>,
    redo: Vec<Vec<Track>>,
    last_id: u64,
    last_track_id: TrackId,
    revision: u64,
}

impl Default for Timeline {
    fn default() -> Self {
        Self {
            tracks: vec![
                Track::new(DEFAULT_TRACK_ID_MAX, TrackKind::Video),
                Track::new(PRIMARY_VIDEO, TrackKind::Video),
                Track::new(PRIMARY_AUDIO, TrackKind::Audio),
            ],
            undo: VecDeque::new(),
            redo: Vec::new(),
            last_id: 0,
            last_track_id: DEFAULT_TRACK_ID_MAX,
            revision: 0,
        }
    }
}

impl Timeline {
    pub fn from_clips(clips: Vec<Clip>) -> Result<Self, TimelineError> {
        let mut timeline = Self::default();
        let index = timeline.track_index(PRIMARY_VIDEO)?;
        timeline.tracks[index].clips = clips;
        Self::from_tracks(timeline.tracks)
    }

    pub fn from_tracks(mut tracks: Vec<Track>) -> Result<Self, TimelineError> {
        for track in &mut tracks {
            track.clips.sort_unstable_by_key(|clip| clip.start);
            for clip in &mut track.clips {
                clip.sort_deleted_cursor_clicks();
            }
        }
        validate(&tracks)?;
        let last_id = tracks
            .iter()
            .flat_map(|track| &track.clips)
            .map(|clip| clip.id)
            .max()
            .unwrap_or_default();
        let last_track_id = tracks
            .iter()
            .fold(DEFAULT_TRACK_ID_MAX, |max_id, track| max_id.max(track.id));
        Ok(Self {
            tracks,
            last_id,
            last_track_id,
            undo: VecDeque::new(),
            redo: Vec::new(),
            revision: 0,
        })
    }

    /// Tracks in on-screen order, starting from the topmost visual layer.
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|track| track.id == id)
    }

    pub fn track_for_clip(&self, id: u64) -> Option<&Track> {
        let (track, _) = self.locate_clip(id).ok()?;
        self.tracks.get(track)
    }

    pub fn clip(&self, id: u64) -> Option<&Clip> {
        let (track, index) = self.locate_clip(id).ok()?;
        self.tracks.get(track)?.clips.get(index)
    }

    pub fn all_clips(&self) -> impl Iterator<Item = &Clip> {
        self.tracks.iter().flat_map(|track| &track.clips)
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.iter().all(|track| track.clips.is_empty())
    }

    /// Main video track. Use [`Self::all_clips`] when operating on the sequence.
    pub fn clips(&self) -> &[Clip] {
        self.track(PRIMARY_VIDEO).map_or(&[], |track| &track.clips)
    }

    pub fn duration(&self) -> u64 {
        self.tracks.iter().map(Track::duration).max().unwrap_or(0)
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn clip_at(&self, time: u64) -> Option<&Clip> {
        self.track(PRIMARY_VIDEO)?.clip_at(time)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn add_track(&mut self, kind: TrackKind) -> Result<TrackId, TimelineError> {
        self.add_track_before(kind, None)
    }

    /// Inserts an empty track before an existing track.
    /// With no anchor, the track is appended after the current tracks.
    pub fn add_track_before(
        &mut self,
        kind: TrackKind,
        before: Option<TrackId>,
    ) -> Result<TrackId, TimelineError> {
        let (id, ()) = self.edit_new_track(kind, before, |_, _, _| Ok(()))?;
        Ok(id)
    }

    /// Inserts an empty track after an existing track.
    pub fn add_track_after(
        &mut self,
        kind: TrackKind,
        after: TrackId,
    ) -> Result<TrackId, TimelineError> {
        let insertion = self.track_index(after)? + 1;
        let (id, ()) = self.edit_new_track_at(kind, insertion, |_, _, _| Ok(()))?;
        Ok(id)
    }

    /// Moves an existing track before an optional anchor while keeping one undo step.
    pub fn move_track_before(
        &mut self,
        id: TrackId,
        before: Option<TrackId>,
    ) -> Result<(), TimelineError> {
        let from = self.track_index(id)?;
        let mut insertion = before
            .map(|anchor| self.track_index(anchor))
            .transpose()?
            .unwrap_or(self.tracks.len());
        if from == insertion || from + 1 == insertion {
            return Ok(());
        }
        self.edit(|tracks, _| {
            let track = tracks.remove(from);
            if from < insertion {
                insertion = insertion.saturating_sub(1);
            }
            tracks.insert(insertion.min(tracks.len()), track);
            Ok(())
        })
    }

    fn edit_new_track<T>(
        &mut self,
        kind: TrackKind,
        before: Option<TrackId>,
        populate: impl FnOnce(&mut Track, &mut [Track], &mut u64) -> Result<T, TimelineError>,
    ) -> Result<(TrackId, T), TimelineError> {
        let insertion = if let Some(before) = before {
            self.track_index(before)?
        } else {
            default_track_insertion(&self.tracks, kind)
        };
        self.edit_new_track_at(kind, insertion, populate)
    }

    fn edit_new_track_at<T>(
        &mut self,
        kind: TrackKind,
        insertion: usize,
        populate: impl FnOnce(&mut Track, &mut [Track], &mut u64) -> Result<T, TimelineError>,
    ) -> Result<(TrackId, T), TimelineError> {
        let mut last_track_id = self.last_track_id;
        let id = allocate_track_id(&mut last_track_id)?;
        let result = self.edit(|tracks, last_id| {
            let mut track = Track::new(id, kind);
            let result = populate(&mut track, tracks, last_id)?;
            tracks.insert(insertion, track);
            Ok(result)
        })?;
        self.last_track_id = id;
        Ok((id, result))
    }

    pub fn set_track_enabled(&mut self, id: TrackId, enabled: bool) -> Result<(), TimelineError> {
        let index = self.track_index(id)?;
        self.edit(|tracks, _| {
            tracks[index].enabled = enabled;
            Ok(())
        })
    }

    /// Removes the track and its clips in one undoable edit.
    pub fn remove_track(&mut self, id: TrackId) -> Result<(), TimelineError> {
        let index = self.track_index(id)?;
        self.edit(|tracks, _| {
            tracks.remove(index);
            Ok(())
        })
    }

    /// Suppresses one click in this video clip without modifying the source recording.
    pub fn delete_cursor_click(&mut self, id: u64, source_us: u64) -> Result<(), TimelineError> {
        let (track, index) = self.locate_clip(id)?;
        let track_data = &self.tracks[track];
        if track_data.kind != TrackKind::Video {
            return Err(TimelineError::IncompatibleTrack);
        }
        let clip = &track_data.clips[index];
        if !(clip.source_start..clip.source_end).contains(&source_us) {
            return Err(TimelineError::InvalidSourceRange);
        }
        let Err(insert_at) = clip.deleted_cursor_clicks_us.binary_search(&source_us) else {
            return Ok(());
        };
        self.edit_track(track, |clips, _| {
            clips[index]
                .deleted_cursor_clicks_us
                .insert(insert_at, source_us);
            Ok(())
        })
    }

    /// Adjusts the gain of a clip on an audio track in tenths of a decibel.
    pub fn set_clip_volume(&mut self, id: u64, db_tenths: i16) -> Result<(), TimelineError> {
        let (track, index) = self.locate_clip(id)?;
        if self.tracks[track].kind != TrackKind::Audio {
            return Err(TimelineError::IncompatibleTrack);
        }
        self.edit_track(track, |clips, _| {
            clips[index].volume_db_tenths = db_tenths;
            Ok(())
        })
    }

    pub fn set_clip_fade_in(&mut self, id: u64, duration: u64) -> Result<(), TimelineError> {
        let (track, index) = self.locate_clip(id)?;
        self.edit_track(track, |clips, _| {
            clips[index].fade_in_us = duration.min(clips[index].duration());
            Ok(())
        })
    }

    pub fn set_clip_fade_out(&mut self, id: u64, duration: u64) -> Result<(), TimelineError> {
        let (track, index) = self.locate_clip(id)?;
        self.edit_track(track, |clips, _| {
            clips[index].fade_out_us = duration.min(clips[index].duration());
            Ok(())
        })
    }

    pub fn set_clip_transition_in(
        &mut self,
        id: u64,
        transition: Option<TransitionSpec>,
    ) -> Result<(), TimelineError> {
        let (track, index) = self.locate_clip(id)?;
        self.edit_track(track, |clips, _| {
            clips[index].transition_in = transition;
            Ok(())
        })
    }

    pub fn set_clip_transition_duration(
        &mut self,
        id: u64,
        duration_us: u64,
    ) -> Result<(), TimelineError> {
        let (track, index) = self.locate_clip(id)?;
        self.edit_track(track, |clips, _| {
            let clip_duration = clips[index].duration();
            let Some(role) = clips[index]
                .transition_in
                .as_ref()
                .map(|transition| transition.role.as_str())
            else {
                return Err(TimelineError::ClipNotFound(id));
            };
            let minimum = 50_000.min(clip_duration);
            let max_duration = if role == "转场" {
                clips
                    .get(index.checked_sub(1).unwrap_or(index))
                    .filter(|_| index > 0)
                    .map(|previous| previous.duration().min(clip_duration).saturating_mul(2))
                    .unwrap_or(clip_duration)
            } else {
                clip_duration
            };
            let clip = &mut clips[index];
            let transition = clip.transition_in.as_mut().expect("transition checked above");
            transition.duration_us = duration_us.clamp(minimum, max_duration.max(minimum));
            Ok(())
        })
    }

    pub fn set_clip_speed(&mut self, id: u64, speed_milli: u16) -> Result<(), TimelineError> {
        let (track, index) = self.locate_clip(id)?;
        self.edit_track(track, |clips, _| {
            let old_end = clips[index].end();
            clips[index].set_speed(speed_milli);
            let delta = i128::from(clips[index].end()) - i128::from(old_end);
            shift_clips(&mut clips[index + 1..], delta)
        })
    }

    pub fn set_clip_reverse(&mut self, id: u64, reverse: bool) -> Result<(), TimelineError> {
        let (track, index) = self.locate_clip(id)?;
        self.edit_track(track, |clips, _| {
            clips[index].reverse = reverse;
            Ok(())
        })
    }

    pub fn link_clips(&mut self, ids: &[u64]) -> Result<(), TimelineError> {
        if ids.len() < 2 {
            return Err(TimelineError::InvalidLink);
        }
        let group_id = *ids.iter().min().ok_or(TimelineError::InvalidLink)?;
        for id in ids {
            if self.clip(*id).is_none() {
                return Err(TimelineError::ClipNotFound(*id));
            }
        }
        self.edit(|tracks, _| {
            for track in tracks {
                for clip in &mut track.clips {
                    if ids.contains(&clip.id) {
                        clip.group_id = Some(group_id);
                    }
                }
            }
            Ok(())
        })
    }

    pub fn unlink_clips(&mut self, ids: &[u64]) -> Result<(), TimelineError> {
        if ids.is_empty() {
            return Err(TimelineError::InvalidLink);
        }
        self.edit(|tracks, _| {
            for track in tracks {
                for clip in &mut track.clips {
                    if ids.contains(&clip.id) {
                        clip.group_id = None;
                    }
                }
            }
            Ok(())
        })
    }

    pub fn append(&mut self, path: PathBuf, duration: u64) -> Result<u64, TimelineError> {
        self.insert_media(
            path,
            duration,
            self.track(PRIMARY_VIDEO).map_or(0, Track::duration),
        )
    }

    /// A new source dropped on an existing clip goes next to it at the nearest edge.
    pub fn material_insertion_point(&self, at: u64) -> u64 {
        self.material_insertion_point_on_track(PRIMARY_VIDEO, at)
    }

    pub fn material_insertion_point_on_track(&self, track: TrackId, at: u64) -> u64 {
        let Some(clip) = self.track(track).and_then(|track| track.clip_at(at)) else {
            return at;
        };
        if at - clip.start <= clip.end() - at {
            clip.start
        } else {
            clip.end()
        }
    }

    pub fn insert_media(
        &mut self,
        path: PathBuf,
        duration: u64,
        at: u64,
    ) -> Result<u64, TimelineError> {
        self.insert_media_on_track(PRIMARY_VIDEO, path, duration, at)
    }

    pub fn insert_media_on_track(
        &mut self,
        track: TrackId,
        path: PathBuf,
        duration: u64,
        at: u64,
    ) -> Result<u64, TimelineError> {
        let index = self.track_index(track)?;
        let start = self.material_insertion_point_on_track(track, at);
        self.insert_clip(index, Clip::full_source(0, path, duration, start))
    }

    /// Creates a track and places the source on it as one timeline edit.
    pub fn insert_media_on_new_track(
        &mut self,
        kind: TrackKind,
        path: PathBuf,
        duration: u64,
        at: u64,
    ) -> Result<(TrackId, u64), TimelineError> {
        self.insert_media_on_new_track_before(kind, None, path, duration, at)
    }

    /// Creates and fills a track immediately before the anchor as one timeline edit.
    /// With no anchor, a video track goes first in its group and an audio track goes last.
    pub fn insert_media_on_new_track_before(
        &mut self,
        kind: TrackKind,
        before: Option<TrackId>,
        path: PathBuf,
        duration: u64,
        at: u64,
    ) -> Result<(TrackId, u64), TimelineError> {
        self.edit_new_track(kind, before, |track, _, last_id| {
            let id = allocate_id(last_id)?;
            track.clips.push(Clip::full_source(id, path, duration, at));
            Ok(id)
        })
    }

    pub fn split(&mut self, id: u64, at: u64) -> Result<u64, TimelineError> {
        let (track, index) = self.locate_clip(id)?;
        self.edit_track(track, |clips, last_id| {
            let right = split_clip(&mut clips[index], at, last_id)?;
            let right_id = right.id;
            clips.insert(index + 1, right);
            Ok(right_id)
        })
    }

    /// Moves a clip to a free position, preserving all other clip positions.
    pub fn move_clip(&mut self, id: u64, start: u64) -> Result<(), TimelineError> {
        let clip = self.clip(id).ok_or(TimelineError::ClipNotFound(id))?;
        let delta = i128::from(start) - i128::from(clip.start);
        let ids = self.linked_clip_ids(id);
        self.move_clips(&ids, i64::try_from(delta).map_err(|_| TimelineError::TimeOutOfBounds)?)
    }

    pub fn linked_clip_ids(&self, id: u64) -> Vec<u64> {
        let Some(clip) = self.clip(id) else {
            return Vec::new();
        };
        let Some(group_id) = clip.group_id else {
            return vec![id];
        };
        self.all_clips()
            .filter(|candidate| candidate.group_id == Some(group_id))
            .map(|candidate| candidate.id)
            .collect()
    }

    /// Moves selected clips together, preserving their tracks and relative positions.
    pub fn move_clips(&mut self, ids: &[u64], delta: i64) -> Result<(), TimelineError> {
        if ids.is_empty() || delta == 0 {
            return Ok(());
        }
        let mut remaining: HashSet<_> = ids.iter().copied().collect();
        self.edit(|tracks, _| {
            for track in tracks {
                let mut changed = false;
                for clip in &mut track.clips {
                    if remaining.remove(&clip.id) {
                        clip.start = shift_time(clip.start, i128::from(delta))
                            .ok_or(TimelineError::TimeOutOfBounds)?;
                        changed = true;
                    }
                }
                if changed {
                    track.clips.sort_unstable_by_key(|clip| clip.start);
                }
            }
            if let Some(id) = ids.iter().find(|id| remaining.contains(id)) {
                return Err(TimelineError::ClipNotFound(*id));
            }
            Ok(())
        })
    }

    /// Moves a clip to a free position on a track of the same kind.
    pub fn move_to_track(
        &mut self,
        id: u64,
        destination: TrackId,
        start: u64,
    ) -> Result<(), TimelineError> {
        let (source, index) = self.locate_clip(id)?;
        let target = self.track_index(destination)?;
        if self.tracks[source].kind != self.tracks[target].kind {
            return Err(TimelineError::IncompatibleTrack);
        }
        self.edit(|tracks, _| {
            let mut clip = tracks[source].clips.remove(index);
            clip.start = start;
            let clips = &mut tracks[target].clips;
            let insertion = clips.partition_point(|other| other.start < start);
            clips.insert(insertion, clip);
            Ok(())
        })
    }

    /// Moves an existing clip onto a new track of the clip's original kind.
    pub fn move_to_new_track(&mut self, id: u64, at: u64) -> Result<TrackId, TimelineError> {
        self.move_to_new_track_before(id, None, at)
    }

    /// Moves a clip onto a new track immediately before the anchor.
    /// With no anchor, a visual track goes first in its group and an audio track goes last.
    pub fn move_to_new_track_before(
        &mut self,
        id: u64,
        before: Option<TrackId>,
        at: u64,
    ) -> Result<TrackId, TimelineError> {
        let (source, index) = self.locate_clip(id)?;
        let kind = self.tracks[source].kind;
        let (track, ()) = self.edit_new_track(kind, before, |track, tracks, _| {
            let mut clip = tracks[source].clips.remove(index);
            clip.start = at;
            track.clips.push(clip);
            Ok(())
        })?;
        Ok(track)
    }

    /// Reorders a clip before another clip, or moves it to the end, and packs the track.
    pub fn reorder(&mut self, id: u64, before: Option<u64>) -> Result<(), TimelineError> {
        let (track, index) = self.locate_clip(id)?;
        let target = match before {
            Some(before_id) => {
                let (target_track, target_clip) = self.locate_clip(before_id)?;
                if track != target_track {
                    return Err(TimelineError::IncompatibleTrack);
                }
                target_clip
            }
            None => self.tracks[track].clips.len(),
        };
        let destination = if target > index { target - 1 } else { target };
        if destination == index {
            return Ok(());
        }

        self.edit_track(track, |clips, _| {
            let clip = clips.remove(index);
            clips.insert(destination, clip);
            let mut start: u64 = 0;
            for clip in clips {
                clip.start = start;
                start = start
                    .checked_add(clip.duration())
                    .ok_or(TimelineError::TimeOutOfBounds)?;
            }
            Ok(())
        })
    }

    /// Trims or extends the source in-point. A ripple trim anchors the clip's start.
    pub fn trim_start(&mut self, id: u64, at: u64, ripple: bool) -> Result<(), TimelineError> {
        let (track, index) = self.locate_clip(id)?;
        self.edit_track(track, |clips, _| {
            let clip = &mut clips[index];
            let delta = i128::from(at) - i128::from(clip.start);
            clip.shift_in_point(delta)?;
            if ripple {
                shift_clips(&mut clips[index + 1..], -delta)?;
            } else {
                clip.start = at;
            }
            Ok(())
        })
    }

    /// Trims or extends the source out-point and optionally moves the following clips.
    pub fn trim_end(&mut self, id: u64, at: u64, ripple: bool) -> Result<(), TimelineError> {
        let (track, index) = self.locate_clip(id)?;
        self.edit_track(track, |clips, _| {
            let clip = &mut clips[index];
            let delta = i128::from(at) - i128::from(clip.end());
            clip.shift_out_point(delta)?;
            if ripple {
                shift_clips(&mut clips[index + 1..], delta)?;
            }
            Ok(())
        })
    }

    /// Deletes a clip, optionally closing the portion of time it occupied.
    pub fn remove(&mut self, id: u64, ripple: bool) -> Result<(), TimelineError> {
        let (track, index) = self.locate_clip(id)?;
        self.edit_track(track, |clips, _| {
            let duration = clips.remove(index).duration();
            if ripple {
                shift_clips(&mut clips[index..], -i128::from(duration))?;
            }
            Ok(())
        })
    }

    /// Removes selected clips without shifting other clips, all in one undoable edit.
    pub fn remove_clips(&mut self, ids: &[u64]) -> Result<(), TimelineError> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut remaining: HashSet<_> = ids.iter().copied().collect();
        self.edit(|tracks, _| {
            for track in tracks {
                track.clips.retain(|clip| !remaining.remove(&clip.id));
            }
            if let Some(id) = ids.iter().find(|id| remaining.contains(id)) {
                return Err(TimelineError::ClipNotFound(*id));
            }
            Ok(())
        })
    }

    /// Changes the source portion without moving or resizing the clip.
    pub fn slip(&mut self, id: u64, delta: i64) -> Result<(), TimelineError> {
        let (track, index) = self.locate_clip(id)?;
        let delta = i128::from(delta);
        self.edit_track(track, |clips, _| {
            let clip = &mut clips[index];
            if clip.text.is_some() {
                return Err(TimelineError::IncompatibleTrack);
            }
            clip.source_start = shift_source_time(clip.source_start, delta)?;
            clip.source_end = shift_source_time(clip.source_end, delta)?;
            Ok(())
        })
    }

    /// Moves the shared cut between two contiguous clips.
    pub fn roll(&mut self, left_id: u64, at: u64) -> Result<(), TimelineError> {
        let (track, index) = self.locate_clip(left_id)?;
        let clips = &self.tracks[track].clips;
        let right = clips
            .get(index + 1)
            .ok_or(TimelineError::MissingAdjacentClip)?;
        if clips[index].end() != right.start {
            return Err(TimelineError::MissingAdjacentClip);
        }
        let delta = i128::from(at) - i128::from(right.start);
        self.edit_track(track, |clips, _| {
            let [left, right] = &mut clips[index..index + 2] else {
                unreachable!("roll requires two adjacent clips");
            };
            left.shift_out_point(delta)?;
            right.shift_in_point(delta)?;
            right.start = at;
            Ok(())
        })
    }

    /// Moves a clip and trims both adjacent clips to keep the track's length fixed.
    pub fn slide(&mut self, id: u64, delta: i64) -> Result<(), TimelineError> {
        let (track, index) = self.locate_clip(id)?;
        let clips = &self.tracks[track].clips;
        let previous = index
            .checked_sub(1)
            .ok_or(TimelineError::MissingAdjacentClip)?;
        let next = clips
            .get(index + 1)
            .ok_or(TimelineError::MissingAdjacentClip)?;
        if clips[previous].end() != clips[index].start || clips[index].end() != next.start {
            return Err(TimelineError::MissingAdjacentClip);
        }
        let delta = i128::from(delta);
        self.edit_track(track, |clips, _| {
            let [left, center, right] = &mut clips[previous..=index + 1] else {
                unreachable!("slide requires three adjacent clips");
            };
            left.shift_out_point(delta)?;
            center.start = shift_time(center.start, delta).ok_or(TimelineError::TimeOutOfBounds)?;
            right.shift_in_point(delta)?;
            right.start = center.end();
            Ok(())
        })
    }

    /// Inserts a copy at the specified time and shifts the downstream clips.
    /// Inserting inside a clip splits it at that position.
    pub fn paste(&mut self, clip: &Clip, at: u64) -> Result<u64, TimelineError> {
        self.paste_on_track(PRIMARY_VIDEO, clip, at)
    }

    pub fn paste_on_track(
        &mut self,
        track: TrackId,
        clip: &Clip,
        at: u64,
    ) -> Result<u64, TimelineError> {
        let index = self.track_index(track)?;
        let mut pasted = clip.clone();
        pasted.start = at;
        pasted.sort_deleted_cursor_clicks();
        self.insert_clip(index, pasted)
    }

    /// Creates a track and pastes the original clip slice as one timeline edit.
    /// With no anchor, the track uses the default position for its kind.
    pub fn paste_on_new_track_before(
        &mut self,
        kind: TrackKind,
        before: Option<TrackId>,
        clip: &Clip,
        at: u64,
    ) -> Result<(TrackId, u64), TimelineError> {
        self.edit_new_track(kind, before, |track, _, last_id| {
            let id = allocate_id(last_id)?;
            let mut pasted = Clip {
                id,
                start: at,
                ..clip.clone()
            };
            pasted.sort_deleted_cursor_clicks();
            track.clips.push(pasted);
            Ok(id)
        })
    }

    fn insert_clip(&mut self, track: usize, mut pasted: Clip) -> Result<u64, TimelineError> {
        self.edit_track(track, |clips, last_id| {
            let at = pasted.start;
            pasted.id = allocate_id(last_id)?;
            let insert_at = clips.partition_point(|clip| clip.start < at);
            if insert_at > 0 && clips[insert_at - 1].end() > at {
                let right = split_clip(&mut clips[insert_at - 1], at, last_id)?;
                clips.insert(insert_at, right);
            }

            // A pre-existing gap can absorb the pasted clip before later clips move.
            let end = at
                .checked_add(pasted.duration())
                .ok_or(TimelineError::TimeOutOfBounds)?;
            if let Some(next) = clips.get(insert_at) {
                let overlap = end.saturating_sub(next.start);
                shift_clips(&mut clips[insert_at..], i128::from(overlap))?;
            }
            let id = pasted.id;
            clips.insert(insert_at, pasted);
            Ok(id)
        })
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop_back() else {
            return false;
        };
        self.redo
            .push(std::mem::replace(&mut self.tracks, previous));
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo
            .push_back(std::mem::replace(&mut self.tracks, next));
        self.revision = self.revision.wrapping_add(1);
        true
    }

    fn track_index(&self, id: TrackId) -> Result<usize, TimelineError> {
        self.tracks
            .iter()
            .position(|track| track.id == id)
            .ok_or(TimelineError::TrackNotFound(id))
    }

    fn locate_clip(&self, id: u64) -> Result<(usize, usize), TimelineError> {
        self.tracks
            .iter()
            .enumerate()
            .find_map(|(track_index, track)| {
                let clip_index = track.clips.iter().position(|clip| clip.id == id)?;
                Some((track_index, clip_index))
            })
            .ok_or(TimelineError::ClipNotFound(id))
    }

    fn edit_track<T>(
        &mut self,
        track: usize,
        change: impl FnOnce(&mut Vec<Clip>, &mut u64) -> Result<T, TimelineError>,
    ) -> Result<T, TimelineError> {
        self.edit(|tracks, last_id| change(&mut tracks[track].clips, last_id))
    }

    fn edit<T>(
        &mut self,
        change: impl FnOnce(&mut Vec<Track>, &mut u64) -> Result<T, TimelineError>,
    ) -> Result<T, TimelineError> {
        let mut tracks = self.tracks.clone();
        let mut last_id = self.last_id;
        let result = change(&mut tracks, &mut last_id)?;
        if tracks == self.tracks {
            return Ok(result);
        }
        validate(&tracks)?;

        if self.undo.len() == HISTORY_LIMIT {
            self.undo.pop_front();
        }
        self.undo
            .push_back(std::mem::replace(&mut self.tracks, tracks));
        self.redo.clear();
        self.last_id = last_id;
        self.revision = self.revision.wrapping_add(1);
        Ok(result)
    }
}

fn validate(tracks: &[Track]) -> Result<(), TimelineError> {
    let mut track_ids = HashSet::with_capacity(tracks.len());
    let mut clip_ids = HashSet::new();
    for track in tracks {
        if !track_ids.insert(track.id) {
            return Err(TimelineError::DuplicateTrackId(track.id));
        }
        let mut previous_end = 0;
        for clip in &track.clips {
            if !clip_ids.insert(clip.id) {
                return Err(TimelineError::DuplicateClipId(clip.id));
            }
            if clip.source_start >= clip.source_end || clip.source_end > clip.media_duration {
                return Err(TimelineError::InvalidSourceRange);
            }
            if !(Clip::MIN_VOLUME_DB_TENTHS..=Clip::MAX_VOLUME_DB_TENTHS)
                .contains(&clip.volume_db_tenths)
            {
                return Err(TimelineError::InvalidVolume);
            }
            if !(500..=2000).contains(&clip.speed_milli) {
                return Err(TimelineError::InvalidSpeed);
            }
            match (track.kind.is_overlay(), &clip.text) {
                (true, Some(text))
                    if clip.path.as_os_str().is_empty() && clip.volume_db_tenths == 0 =>
                {
                    text.validate()?
                }
                (false, None) => {}
                _ => return Err(TimelineError::IncompatibleTrack),
            }
            if clip.start < previous_end {
                return Err(TimelineError::Overlap);
            }
            if clip.fade_in_us > clip.duration() || clip.fade_out_us > clip.duration() {
                return Err(TimelineError::InvalidSourceRange);
            }
            previous_end = clip
                .start
                .checked_add(clip.duration())
                .ok_or(TimelineError::TimeOutOfBounds)?;
        }
    }
    Ok(())
}

fn default_track_insertion(tracks: &[Track], _kind: TrackKind) -> usize {
    tracks.len()
}

fn allocate_track_id(last_id: &mut TrackId) -> Result<TrackId, TimelineError> {
    *last_id = last_id
        .checked_add(1)
        .ok_or(TimelineError::TrackIdExhausted)?;
    Ok(*last_id)
}

fn allocate_id(last_id: &mut u64) -> Result<u64, TimelineError> {
    *last_id = last_id
        .checked_add(1)
        .ok_or(TimelineError::ClipIdExhausted)?;
    Ok(*last_id)
}

fn split_clip(clip: &mut Clip, at: u64, last_id: &mut u64) -> Result<Clip, TimelineError> {
    if at <= clip.start || at >= clip.end() {
        return Err(TimelineError::InvalidSplitPoint);
    }
    let end = clip.end();
    let mut right = Clip {
        id: allocate_id(last_id)?,
        start: at,
        ..clip.clone()
    };
    clip.shift_out_point(-i128::from(end - at))?;
    right.shift_in_point(i128::from(at - clip.start))?;
    Ok(right)
}

fn shift_time(time: u64, delta: i128) -> Option<u64> {
    u64::try_from(i128::from(time) + delta).ok()
}

fn shift_source_time(time: u64, delta: i128) -> Result<u64, TimelineError> {
    shift_time(time, delta).ok_or(TimelineError::InvalidSourceRange)
}

fn shift_clips(clips: &mut [Clip], delta: i128) -> Result<(), TimelineError> {
    for clip in clips {
        clip.start = shift_time(clip.start, delta).ok_or(TimelineError::TimeOutOfBounds)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "editor_timeline/multitrack_tests.rs"]
mod multitrack_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(id: u64, start: u64, source_start: u64, source_end: u64) -> Clip {
        Clip {
            id,
            path: PathBuf::from(format!("clip-{id}.mp4")),
            media_duration: 100,
            source_start,
            source_end,
            start,
            volume_db_tenths: 0,
            fade_in_us: 0,
            fade_out_us: 0,
            transition_in: None,
            speed_milli: 1000,
            reverse: false,
            timeline_duration_us: 0,
            group_id: None,
            text: None,
            deleted_cursor_clicks_us: Vec::new(),
        }
    }

    fn contiguous_timeline() -> Timeline {
        Timeline::from_clips(vec![
            clip(1, 0, 10, 30),
            clip(2, 20, 30, 50),
            clip(3, 40, 40, 60),
        ])
        .unwrap()
    }

    #[test]
    fn deleting_cursor_click_is_undoable_and_local_to_the_clip() {
        let first = clip(1, 0, 10, 30);
        let second = Clip {
            id: 2,
            start: first.end(),
            ..first.clone()
        };
        let mut timeline = Timeline::from_clips(vec![first, second]).unwrap();
        assert_eq!(
            timeline.delete_cursor_click(1, 30),
            Err(TimelineError::InvalidSourceRange)
        );
        assert!(!timeline.can_undo());

        timeline.delete_cursor_click(1, 12).unwrap();
        timeline.delete_cursor_click(1, 12).unwrap();
        assert_eq!(timeline.revision(), 1);
        assert_eq!(timeline.clips()[0].deleted_cursor_clicks_us, [12]);
        assert!(timeline.clips()[1].deleted_cursor_clicks_us.is_empty());
        assert!(timeline.undo());
        assert!(timeline.clips()[0].deleted_cursor_clicks_us.is_empty());
        assert!(timeline.redo());
        assert_eq!(timeline.clips()[0].deleted_cursor_clicks_us, [12]);
        timeline.delete_cursor_click(1, 11).unwrap();
        assert_eq!(timeline.clips()[0].deleted_cursor_clicks_us, [11, 12]);
    }

    #[test]
    fn split_uses_source_offset_and_undo_keeps_clip_ids() {
        let mut timeline = Timeline::from_clips(vec![clip(1, 10, 20, 40)]).unwrap();
        let right = timeline.split(1, 17).unwrap();
        assert_eq!(
            (
                timeline.clips()[0].source_end,
                timeline.clips()[1].source_start
            ),
            (27, 27)
        );
        assert_eq!(timeline.clips()[0].path, timeline.clips()[1].path);
        assert!(timeline.clip_at(9).is_none());
        assert_eq!(timeline.clip_at(16).unwrap().id, 1);
        assert_eq!(timeline.clip_at(17).unwrap().id, right);
        assert!(timeline.clip_at(30).is_none());
        assert!(timeline.undo());
        assert_eq!(timeline.clips(), &[clip(1, 10, 20, 40)]);
        assert!(timeline.redo());
        assert_eq!(timeline.clip_at(17).unwrap().id, right);
    }

    #[test]
    fn ordinary_and_ripple_trims_update_different_timeline_positions() {
        let mut timeline = contiguous_timeline();
        timeline.trim_start(2, 25, false).unwrap();
        assert_eq!(timeline.clips()[1], clip(2, 25, 35, 50));
        assert_eq!(timeline.clips()[2].start, 40);

        timeline.trim_end(2, 36, true).unwrap();
        assert_eq!(timeline.clips()[1], clip(2, 25, 35, 46));
        assert_eq!(timeline.clips()[2].start, 36);

        timeline.trim_start(2, 28, true).unwrap();
        assert_eq!(timeline.clips()[1], clip(2, 25, 38, 46));
        assert_eq!(timeline.clips()[2].start, 33);

        timeline.remove(2, true).unwrap();
        assert_eq!(timeline.clips()[1].start, 25);
        assert!(timeline.clip_at(24).is_none());
    }

    #[test]
    fn move_preserves_gaps_while_reorder_packs_the_track() {
        let mut timeline = contiguous_timeline();
        timeline.move_clip(2, 70).unwrap();
        assert_eq!(timeline.duration(), 90);
        assert!(timeline.clip_at(20).is_none());
        assert_eq!(timeline.clip_at(40).unwrap().id, 3);

        timeline.reorder(2, Some(1)).unwrap();
        assert_eq!(
            timeline.clips(),
            &[clip(2, 0, 30, 50), clip(1, 20, 10, 30), clip(3, 40, 40, 60)]
        );
        assert_eq!(timeline.duration(), 60);

        timeline.remove(1, false).unwrap();
        assert_eq!(timeline.clips()[1].start, 40);
        assert_eq!(timeline.duration(), 60);
    }

    #[test]
    fn slip_roll_and_slide_preserve_their_expected_edges() {
        let mut timeline = contiguous_timeline();
        timeline.slip(2, -7).unwrap();
        assert_eq!(timeline.clips()[1], clip(2, 20, 23, 43));

        timeline.roll(1, 24).unwrap();
        assert_eq!(timeline.clips()[0], clip(1, 0, 10, 34));
        assert_eq!(timeline.clips()[1], clip(2, 24, 27, 43));
        assert_eq!(timeline.clips()[2].start, 40);

        timeline.slide(2, -6).unwrap();
        assert_eq!(timeline.clips()[0], clip(1, 0, 10, 28));
        assert_eq!(timeline.clips()[1], clip(2, 18, 27, 43));
        assert_eq!(timeline.clips()[2], clip(3, 34, 34, 60));
        assert_eq!(timeline.duration(), 60);
    }

    #[test]
    fn speed_changes_timeline_duration_and_ripples_following_clips() {
        let mut timeline = contiguous_timeline();
        timeline.set_clip_speed(2, 2000).unwrap();
        assert_eq!(timeline.clip(2).unwrap().duration(), 10);
        assert_eq!(timeline.clip(2).unwrap().source_start, 30);
        assert_eq!(timeline.clip(2).unwrap().source_end, 50);
        assert_eq!(timeline.clip(3).unwrap().start, 30);
        timeline.set_clip_reverse(2, true).unwrap();
        assert!(timeline.clip(2).unwrap().reverse);
    }

    #[test]
    fn linked_clips_move_together_and_can_be_unlinked() {
        let mut timeline = Timeline::from_tracks(vec![
            Track {
                id: PRIMARY_VIDEO,
                kind: TrackKind::Video,
                enabled: true,
                clips: vec![clip(1, 0, 0, 20)],
            },
            Track {
                id: PRIMARY_AUDIO,
                kind: TrackKind::Audio,
                enabled: true,
                clips: vec![clip(2, 0, 0, 20)],
            },
        ])
        .unwrap();
        timeline.link_clips(&[1, 2]).unwrap();
        timeline.move_clip(1, 100).unwrap();
        assert_eq!(timeline.clip(1).unwrap().start, 100);
        assert_eq!(timeline.clip(2).unwrap().start, 100);
        timeline.unlink_clips(&[1, 2]).unwrap();
        timeline.move_clip(1, 140).unwrap();
        assert_eq!(timeline.clip(2).unwrap().start, 100);
    }

    #[test]
    fn paste_uses_free_space_or_splits_and_inserts() {
        let mut timeline =
            Timeline::from_clips(vec![clip(1, 0, 0, 20), clip(2, 40, 0, 20)]).unwrap();
        let copy = clip(7, 80, 10, 20);
        let pasted = timeline.paste(&copy, 24).unwrap();
        assert_eq!(timeline.clips()[1].id, pasted);
        assert_eq!(timeline.clips()[1].path, copy.path);
        assert_eq!(timeline.clips()[2].start, 40);

        let pasted = timeline.paste(&copy, 11).unwrap();
        assert_eq!(timeline.clips()[0].source_end, 11);
        assert_eq!(timeline.clips()[1].id, pasted);
        assert_eq!(timeline.clips()[2].source_start, 11);
        assert_eq!(timeline.clips()[2].start, 21);
        assert_eq!(timeline.clips()[3].start, 34);
        assert_eq!(timeline.clips()[4].start, 50);
        assert_eq!(timeline.duration(), 70);
        assert!(timeline.undo());
        assert_eq!(timeline.clips().len(), 3);
        assert!(timeline.redo());
        assert_eq!(timeline.clips()[1].id, pasted);
    }

    #[test]
    fn failed_and_no_op_edits_leave_history_intact() {
        let mut timeline = contiguous_timeline();
        let discarded_id = timeline.split(1, 10).unwrap();
        assert!(timeline.undo());
        let revision = timeline.revision();
        let initial = timeline.clips().to_vec();

        assert_eq!(timeline.move_clip(2, 19), Err(TimelineError::Overlap));
        assert_eq!(timeline.split(1, 20), Err(TimelineError::InvalidSplitPoint));
        assert_eq!(
            timeline.slip(1, i64::MIN),
            Err(TimelineError::InvalidSourceRange)
        );
        assert_eq!(
            timeline.slide(2, 20),
            Err(TimelineError::InvalidSourceRange)
        );
        let mut invalid = clip(7, 0, 10, 20);
        invalid.source_end = 101;
        assert_eq!(
            timeline.paste(&invalid, 10),
            Err(TimelineError::InvalidSourceRange)
        );
        timeline.move_clip(2, 20).unwrap();
        timeline.trim_end(2, 40, true).unwrap();
        timeline.reorder(2, Some(3)).unwrap();
        assert_eq!(timeline.clips(), initial);
        assert_eq!(timeline.revision(), revision);
        assert!(!timeline.can_undo());
        assert!(timeline.can_redo());

        let new_id = timeline.split(1, 10).unwrap();
        assert_eq!(new_id, discarded_id + 1);
        assert!(!timeline.can_redo());
    }

    #[test]
    fn invalid_loaded_clips_and_overflow_are_rejected() {
        assert!(matches!(
            Timeline::from_clips(vec![clip(1, 0, 0, 10), clip(1, 10, 0, 10)]),
            Err(TimelineError::DuplicateClipId(1))
        ));
        assert!(matches!(
            Timeline::from_clips(vec![clip(1, 0, 0, 10), clip(2, 9, 0, 10)]),
            Err(TimelineError::Overlap)
        ));
        assert!(matches!(
            Timeline::from_clips(vec![clip(1, u64::MAX, 0, 1)]),
            Err(TimelineError::TimeOutOfBounds)
        ));

        let mut timeline = Timeline::default();
        let path = PathBuf::from("long.mp4");
        assert_eq!(timeline.append(path.clone(), u64::MAX), Ok(1));
        assert_eq!(
            timeline.append(path, 1),
            Err(TimelineError::TimeOutOfBounds)
        );
        assert_eq!(timeline.clips().len(), 1);
        assert_eq!(timeline.duration(), u64::MAX);
    }

    #[test]
    fn history_discards_oldest_edits_at_capacity() {
        let mut timeline = Timeline::default();
        let id = timeline.append(PathBuf::from("clip.mp4"), 1).unwrap();
        for position in 1..=HISTORY_LIMIT as u64 {
            timeline.move_clip(id, position).unwrap();
        }
        for _ in 0..HISTORY_LIMIT {
            assert!(timeline.undo());
        }
        assert!(!timeline.undo());
        assert_eq!(timeline.clips()[0].start, 0);
        for _ in 0..HISTORY_LIMIT {
            assert!(timeline.redo());
        }
        assert!(!timeline.redo());
        assert_eq!(timeline.clips()[0].start, HISTORY_LIMIT as u64);
    }
}

