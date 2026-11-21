use eframe::egui::{self, Ui};
use takeseam::editor_timeline::{Clip, Timeline, TimelineError, TrackId, TrackKind};

use crate::ui_icons::{ButtonStyle, Icon, icon_button};

#[derive(Clone, Copy)]
pub(super) enum Edge {
    Start,
    End,
}

impl Edge {
    pub(super) fn time(self, clip: &Clip) -> u64 {
        match self {
            Self::Start => clip.start,
            Self::End => clip.end(),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Command {
    Split,
    Trim(Edge, bool),
    Delete(bool),
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
    SeekStart,
    SeekEnd,
    Step(i64),
}

pub(super) struct ClipboardClip {
    pub(super) clip: Clip,
    pub(super) kind: TrackKind,
    source_track: TrackId,
}

pub(super) fn action_button(ui: &mut Ui, command: Command, enabled: bool) -> bool {
    let toolbar_action = match command {
        Command::Split => Some((Icon::Split, "分割 · Ctrl+B")),
        Command::Delete(false) => Some((Icon::Delete, "删除片段 · Delete")),
        Command::Undo => Some((Icon::Undo, "撤销 · Ctrl+Z")),
        Command::Redo => Some((Icon::Redo, "重做 · Ctrl+Shift+Z")),
        _ => None,
    };
    if let Some((icon, label)) = toolbar_action {
        return icon_button(ui, icon, label, enabled, ButtonStyle::Quiet).clicked();
    }
    let (label, tooltip) = match command {
        Command::Trim(Edge::Start, false) => ("修剪入点", "将片段入点修剪到播放头 · Q"),
        Command::Trim(Edge::End, false) => ("修剪出点", "将片段出点修剪到播放头 · W"),
        Command::Trim(Edge::Start, true) => ("波纹修剪入点", "修剪到播放头并移动后续片段"),
        Command::Trim(Edge::End, true) => ("波纹修剪出点", "修剪到播放头并移动后续片段"),
        Command::Delete(true) => ("波纹删除", "删除片段并将后续片段向前移动 · Shift+Delete"),
        Command::Copy => ("复制片段", "复制选中的片段 · Ctrl+C"),
        Command::Cut => ("剪切片段", "剪切选中的片段 · Ctrl+X"),
        Command::Paste => ("粘贴到播放头", "在播放头插入已复制片段 · Ctrl+V"),
        _ => return false,
    };
    ui.add_enabled(enabled, egui::Button::new(label))
        .on_hover_text(tooltip)
        .clicked()
}

pub(super) fn execute(
    command: Command,
    timeline: &mut Timeline,
    track: TrackId,
    playhead: &mut u64,
    selected: &mut Option<u64>,
    clipboard: &mut Option<ClipboardClip>,
) -> Result<Option<TrackId>, TimelineError> {
    let mut pasted_track = None;
    match command {
        Command::Split => {
            if let Some(id) = split_target(timeline, track, *selected, *playhead) {
                *selected = Some(timeline.split(id, *playhead)?);
            }
        }
        Command::Trim(edge, ripple) => {
            if let Some(id) = *selected {
                let clip = timeline.clip(id).ok_or(TimelineError::ClipNotFound(id))?;
                if *playhead < clip.start || *playhead > clip.end() {
                    return Err(TimelineError::TimeOutOfBounds);
                }
                trim(timeline, id, edge, *playhead, ripple)?;
            }
        }
        Command::Delete(ripple) => {
            if let Some(id) = *selected {
                timeline.remove(id, ripple)?;
            }
        }
        Command::Copy => {
            if let Some(id) = *selected
                && let Some(clip) = copy_clip(timeline, id)
            {
                *clipboard = Some(clip);
            }
        }
        Command::Cut => {
            if let Some(id) = *selected {
                cut_clip(timeline, id, clipboard)?;
            }
        }
        Command::Paste => {
            if let Some(clip) = clipboard.as_ref() {
                let (destination, id) = paste_clip(timeline, clip, track, *playhead)?;
                *selected = Some(id);
                pasted_track = Some(destination);
            }
        }
        Command::Undo => {
            timeline.undo();
        }
        Command::Redo => {
            timeline.redo();
        }
        Command::SeekStart => *playhead = 0,
        Command::SeekEnd => *playhead = timeline.duration(),
        Command::Step(delta) => {
            *playhead = playhead
                .saturating_add_signed(delta)
                .min(timeline.duration())
        }
    }
    normalize_selection(timeline, playhead, selected);
    Ok(pasted_track)
}

pub(super) fn copy_clip(timeline: &Timeline, id: u64) -> Option<ClipboardClip> {
    let track = timeline.track_for_clip(id)?;
    let clip = track.clips.iter().find(|clip| clip.id == id)?;
    Some(ClipboardClip {
        clip: clip.clone(),
        kind: track.kind,
        source_track: track.id,
    })
}

pub(super) fn cut_clip(
    timeline: &mut Timeline,
    id: u64,
    clipboard: &mut Option<ClipboardClip>,
) -> Result<(), TimelineError> {
    let clip = copy_clip(timeline, id).ok_or(TimelineError::ClipNotFound(id))?;
    timeline.remove(id, false)?;
    *clipboard = Some(clip);
    Ok(())
}

pub(super) fn paste_clip(
    timeline: &mut Timeline,
    clipboard: &ClipboardClip,
    preferred: TrackId,
    at: u64,
) -> Result<(TrackId, u64), TimelineError> {
    if timeline
        .track(preferred)
        .is_some_and(|track| track.kind == clipboard.kind)
    {
        return timeline
            .paste_on_track(preferred, &clipboard.clip, at)
            .map(|id| (preferred, id));
    }
    let end = at
        .checked_add(clipboard.clip.duration())
        .ok_or(TimelineError::TimeOutOfBounds)?;
    // When redirecting to another track, preserve existing clips at the paste point.
    let destination = timeline
        .tracks()
        .iter()
        .filter(|track| {
            track.kind == clipboard.kind
                && track.enabled
                && track
                    .clips
                    .iter()
                    .all(|clip| clip.end() <= at || clip.start >= end)
        })
        .min_by_key(|track| track.id != clipboard.source_track)
        .map(|track| track.id);
    if let Some(track) = destination {
        timeline
            .paste_on_track(track, &clipboard.clip, at)
            .map(|id| (track, id))
    } else {
        timeline.paste_on_new_track_before(clipboard.kind, None, &clipboard.clip, at)
    }
}

pub(super) fn split_target(
    timeline: &Timeline,
    track: TrackId,
    selected: Option<u64>,
    playhead: u64,
) -> Option<u64> {
    selected
        .and_then(|id| timeline.clip(id))
        .filter(|clip| clip.start < playhead && playhead < clip.end())
        .or_else(|| {
            timeline
                .track(track)?
                .clip_at(playhead)
                .filter(|clip| clip.start < playhead)
        })
        .map(|clip| clip.id)
}

pub(super) fn trim(
    timeline: &mut Timeline,
    id: u64,
    edge: Edge,
    at: u64,
    ripple: bool,
) -> Result<(), TimelineError> {
    match edge {
        Edge::Start => timeline.trim_start(id, at, ripple),
        Edge::End => timeline.trim_end(id, at, ripple),
    }
}

pub(super) fn normalize_selection(
    timeline: &Timeline,
    playhead: &mut u64,
    selected: &mut Option<u64>,
) {
    if let Some(id) = *selected
        && timeline.clip(id).is_none()
    {
        *selected = None;
    }
    *playhead = (*playhead).min(timeline.duration());
}
