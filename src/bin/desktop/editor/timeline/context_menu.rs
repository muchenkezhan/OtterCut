use eframe::egui::{self, Event, PointerButton, Pos2, Response, Ui};
use takeseam::editor_timeline::{Timeline, TimelineError, TrackId, TrackKind};

use super::super::layout;
use super::TimelineView;
use super::canvas::Viewport;
use super::commands::{ClipboardClip, Edge, copy_clip, cut_clip, paste_clip, trim};
use super::edge_drop::TrackGap;
use super::track_rows::Rows;

#[derive(Clone, Copy)]
pub(super) struct ContextMenu {
    position: Pos2,
    target: Target,
}

#[derive(Clone, Copy)]
enum Target {
    Clip(u64, u64),
    Header(TrackId),
    Lane(TrackId, u64),
    Ruler(u64),
    Gap(TrackGap, u64),
}

#[derive(Clone, Copy)]
pub(super) enum Action {
    Split(u64, u64),
    Trim(u64, u64, Edge),
    Cut(u64),
    Copy(u64),
    Delete(u64, bool),
    ResetVolume(u64),
    Seek(u64),
    Paste(PasteTarget, u64),
    InsertTrack(TrackKind, Option<TrackId>),
    InsertTrackAfter(TrackKind, TrackId),
    RemoveTrack(TrackId),
    SetOutput(TrackId, bool),
}

#[derive(Clone, Copy)]
pub(super) enum PasteTarget {
    Track(TrackId),
    ActiveTrack,
    NewTrack(Option<TrackId>),
}

fn popup_id(ui: &Ui) -> egui::Id {
    ui.id().with("timeline-context-menu")
}

pub(super) fn is_open(ui: &Ui) -> bool {
    egui::Popup::is_id_open(ui.ctx(), popup_id(ui))
}

pub(super) fn close(ui: &Ui) {
    egui::Popup::close_id(ui.ctx(), popup_id(ui));
}

impl ContextMenu {
    pub(super) fn at_release(
        ui: &Ui,
        view: Viewport,
        rows: Rows,
        timeline: &Timeline,
    ) -> Option<Self> {
        let position = ui.input(|input| {
            if input.pointer.primary_down() || input.pointer.primary_released() || !input.focused {
                return None;
            }
            input.events.iter().rev().find_map(|event| match event {
                Event::PointerButton {
                    pos,
                    button: PointerButton::Secondary,
                    pressed: false,
                    ..
                } => Some(*pos),
                _ => None,
            })
        })?;
        let pointer = ui
            .ctx()
            .layer_transform_from_global(ui.layer_id())
            .map_or(position, |transform| transform * position);
        if !ui.clip_rect().contains(pointer)
            || ui.ctx().layer_id_at(position) != Some(ui.layer_id())
        {
            return None;
        }
        let target = target_at(view, rows, timeline, pointer)?;
        Some(Self { position, target })
    }

    pub(super) fn show(
        self,
        ui: &Ui,
        anchor: &Response,
        opening: bool,
        timeline: &Timeline,
        clipboard: Option<&ClipboardClip>,
        selection: &[u64],
    ) -> Option<Action> {
        let mut popup = egui::Popup::menu(anchor)
            .id(popup_id(ui))
            .at_position(self.position)
            .open_memory(opening.then_some(egui::SetOpenCommand::Bool(true)));
        if opening {
            // A new right click can re-anchor a menu that was already open.
            popup = popup.close_behavior(egui::PopupCloseBehavior::IgnoreClicks);
        }
        popup
            .show(|ui| self.target.menu(ui, timeline, clipboard, selection))
            .and_then(|response| response.inner)
    }
}

fn target_at(view: Viewport, rows: Rows, timeline: &Timeline, pointer: Pos2) -> Option<Target> {
    if pointer.x < rows.area.left() || pointer.x > view.rect.right() {
        return None;
    }
    let at = view.time_at(pointer.x);
    if view.ruler().contains(pointer) {
        return Some(Target::Ruler(at));
    }
    if let Some(track) = rows.at(timeline, pointer.y) {
        if pointer.x < view.rect.left() {
            return Some(Target::Header(track.id));
        }
        if let Some(clip) = track.clip_at(at) {
            let index = timeline
                .tracks()
                .iter()
                .position(|row| row.id == track.id)?;
            if view
                .clip_rect(rows.rect(view, index), clip.start, clip.end())
                .contains(pointer)
            {
                return Some(Target::Clip(clip.id, at));
            }
        }
    }
    if let Some(gap) = TrackGap::at(rows, timeline, pointer) {
        return Some(Target::Gap(gap, at));
    }
    rows.at(timeline, pointer.y)
        .map(|track| Target::Lane(track.id, at))
}

impl Target {
    fn menu(
        self,
        ui: &mut Ui,
        timeline: &Timeline,
        clipboard: Option<&ClipboardClip>,
        selection: &[u64],
    ) -> Option<Action> {
        layout::compact_menu(ui);
        let mut chosen = None;
        let can_paste = clipboard.is_some();
        let mut item = |label, enabled, action| {
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                chosen = Some(action);
                ui.close();
            }
        };

        match self {
            Self::Clip(id, at) => {
                let track = timeline.track_for_clip(id)?;
                let clip = track.clips.iter().find(|clip| clip.id == id)?;
                let interior = clip.start < at && at < clip.end();
                item("在此分割", interior, Action::Split(id, at));
                item(
                    "修剪入点到此处",
                    interior,
                    Action::Trim(id, at, Edge::Start),
                );
                item("修剪出点到此处", interior, Action::Trim(id, at, Edge::End));
                item("剪切片段", true, Action::Cut(id));
                item("复制片段", true, Action::Copy(id));
                item(
                    "在此插入粘贴",
                    can_paste,
                    Action::Paste(PasteTarget::Track(track.id), at),
                );
                let multiple = selection.len() > 1 && selection.contains(&id);
                item(
                    if multiple {
                        "删除选中片段"
                    } else {
                        "删除片段"
                    },
                    true,
                    Action::Delete(id, false),
                );
                item("同轨波纹删除", true, Action::Delete(id, true));
                if track.kind == TrackKind::Audio && clip.volume_db_tenths != 0 {
                    item("重置音量", true, Action::ResetVolume(id));
                }
            }
            Self::Header(id) => {
                let track = timeline.track(id)?;
                item(
                    "在上方新增同类轨道",
                    true,
                    Action::InsertTrack(track.kind, Some(id)),
                );
                item(
                    "在下方新增同类轨道",
                    true,
                    Action::InsertTrackAfter(track.kind, id),
                );
                item("删除轨道", true, Action::RemoveTrack(id));
                let label = match (track.kind, track.enabled) {
                    (TrackKind::Video, true) => "关闭视频轨输出",
                    (TrackKind::Video, false) => "开启视频轨输出",
                    (TrackKind::Audio, true) => "静音",
                    (TrackKind::Audio, false) => "取消静音",
                    (TrackKind::Caption, true) => "隐藏字幕轨",
                    (TrackKind::Caption, false) => "显示字幕轨",
                    (TrackKind::Text, true) => "隐藏文本轨",
                    (TrackKind::Text, false) => "显示文本轨",
                };
                let mut response = ui.button(label);
                if track.kind == TrackKind::Video {
                    response = response.on_hover_text("视频轨输出包含画面和原声");
                }
                if response.clicked() {
                    chosen = Some(Action::SetOutput(id, !track.enabled));
                    ui.close();
                }
            }
            Self::Lane(id, at) => {
                item("在此定位播放头", !timeline.is_empty(), Action::Seek(at));
                item(
                    "在此插入粘贴",
                    can_paste,
                    Action::Paste(PasteTarget::Track(id), at),
                );
            }
            Self::Ruler(at) => {
                item("在此定位播放头", !timeline.is_empty(), Action::Seek(at));
                item(
                    "在此插入粘贴",
                    can_paste,
                    Action::Paste(PasteTarget::ActiveTrack, at),
                );
            }
            Self::Gap(gap, at) => {
                if clipboard.is_some_and(|clip| gap.allows(clip.kind)) {
                    item(
                        "粘贴到新轨道",
                        true,
                        Action::Paste(PasteTarget::NewTrack(gap.before), at),
                    );
                }
                for (kind, label) in [
                    (TrackKind::Text, "在此新增文本轨道"),
                    (TrackKind::Caption, "在此新增字幕轨道"),
                    (TrackKind::Video, "在此新增视频轨道"),
                    (TrackKind::Audio, "在此新增音频轨道"),
                ] {
                    if gap.allows(kind) {
                        item(label, true, Action::InsertTrack(kind, gap.before));
                    }
                }
            }
        }
        chosen
    }
}

impl TimelineView {
    pub(super) fn apply_context_action(
        &mut self,
        action: Action,
        timeline: &mut Timeline,
        playhead: &mut u64,
        selected: &mut Option<u64>,
    ) -> Result<(), TimelineError> {
        match action {
            Action::Split(id, at) => {
                *selected = Some(timeline.split(id, at)?);
                *playhead = at;
            }
            Action::Trim(id, at, edge) => {
                let clip = timeline.clip(id).ok_or(TimelineError::ClipNotFound(id))?;
                if at <= clip.start || at >= clip.end() {
                    return Err(TimelineError::TimeOutOfBounds);
                }
                trim(timeline, id, edge, at, false)?;
                *selected = Some(id);
            }
            Action::Copy(id) => {
                self.clipboard =
                    Some(copy_clip(timeline, id).ok_or(TimelineError::ClipNotFound(id))?);
            }
            Action::Cut(id) => cut_clip(timeline, id, &mut self.clipboard)?,
            Action::Delete(id, false)
                if self.marquee.ids().len() > 1 && self.marquee.ids().contains(&id) =>
            {
                timeline.remove_clips(self.marquee.ids())?;
                self.marquee.clear();
                *selected = None;
            }
            Action::Delete(id, ripple) => timeline.remove(id, ripple)?,
            Action::ResetVolume(id) => timeline.set_clip_volume(id, 0)?,
            Action::Seek(at) => *playhead = at.min(timeline.duration()),
            Action::Paste(target, at) => {
                if let Some(clip) = &self.clipboard {
                    let (track, id) = match target {
                        PasteTarget::Track(id) => paste_clip(timeline, clip, id, at)?,
                        PasteTarget::ActiveTrack => {
                            let track = self.active_track(timeline);
                            paste_clip(timeline, clip, track, at)?
                        }
                        PasteTarget::NewTrack(before) => {
                            timeline.paste_on_new_track_before(clip.kind, before, &clip.clip, at)?
                        }
                    };
                    *selected = Some(id);
                    *playhead = at;
                    self.reveal_added_track(track);
                }
            }
            Action::InsertTrack(kind, before) => {
                let track = timeline.add_track_before(kind, before)?;
                self.reveal_added_track(track);
                *selected = None;
            }
            Action::InsertTrackAfter(kind, after) => {
                let track = timeline.add_track_after(kind, after)?;
                self.reveal_added_track(track);
                *selected = None;
            }
            Action::RemoveTrack(id) => {
                let tracks = timeline.tracks();
                let index = tracks
                    .iter()
                    .position(|track| track.id == id)
                    .ok_or(TimelineError::TrackNotFound(id))?;
                let next = tracks
                    .get(index + 1)
                    .or_else(|| index.checked_sub(1).and_then(|index| tracks.get(index)))
                    .map(|track| track.id);
                timeline.remove_track(id)?;
                if self.reveal_track == Some(id) {
                    self.reveal_track = None;
                }
                if self.track == Some(id) {
                    self.track = next;
                    self.reveal_track = next;
                }
            }
            Action::SetOutput(id, enabled) => timeline.set_track_enabled(id, enabled)?,
        }
        Ok(())
    }
}
