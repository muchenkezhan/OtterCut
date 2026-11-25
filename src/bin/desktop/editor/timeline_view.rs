use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Align2, CursorIcon, DragAndDrop, FontId, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, pos2,
    vec2,
};
use takeseam::editor_timeline::{Clip, PRIMARY_VIDEO, Timeline, TimelineError, TrackId, TrackKind};

use crate::ui_icons::{Icon, paint_razor_cursor};
use crate::ui_theme::{self, Palette};

use super::cursor_effect::CursorEffect;
use super::media::MediaTools;
use super::transition_panel::{DraggedTransition, TransitionRole};

#[path = "timeline/canvas.rs"]
mod canvas;
#[path = "timeline/clip_media.rs"]
mod clip_media;
#[path = "timeline/clip_pointer.rs"]
mod clip_pointer;
#[path = "timeline/commands.rs"]
mod commands;
#[path = "timeline/context_menu.rs"]
mod context_menu;
#[path = "timeline/edge_drop.rs"]
mod edge_drop;
#[path = "timeline/gesture.rs"]
mod gesture;
#[cfg(test)]
#[path = "timeline/interaction_tests.rs"]
mod interaction_tests;
#[path = "timeline/marquee.rs"]
mod marquee;
#[path = "timeline/material_drop.rs"]
mod material_drop;
#[path = "timeline/toolbar.rs"]
mod toolbar;
#[path = "timeline/track_rows.rs"]
mod track_rows;

use canvas::{
    RULER_HEIGHT, SECOND, Viewport, paint_draft, paint_playhead, paint_ruler, timecode,
};
use clip_media::{ClipMedia, VolumeLine};
use clip_pointer::ClipPress;
use commands::{ClipboardClip, Command, Edge, execute, normalize_selection};
use context_menu::ContextMenu;
use edge_drop::TrackGap;
use gesture::{ClipDrag, Gesture, difference};
use marquee::MarqueeSelection;
use track_rows::{Rows, TrackAction, VERTICAL_SCROLL_WIDTH};

const MIN_SCALE: f64 = 0.02;
const MAX_SCALE: f64 = 384.0;
const DEFAULT_SCALE: f64 = 56.0;
const MIN_HORIZON: u64 = 10_000_000;
const TRAILING_SPACE: u64 = 3_000_000;
const TRACK_HEADER: f32 = 72.0;
const SCROLLBAR_HEIGHT: f32 = 16.0;
const CURSOR_TRACK_HEIGHT: f32 = 32.0;
const CURSOR_TRACK_GAP: f32 = 5.0;

#[derive(Default)]
pub struct TimelineView {
    mode: Mode,
    scroll: f64,
    scale: Option<f64>,
    disable_snap: bool,
    clipboard: Option<ClipboardClip>,
    clipboard_feedback: Option<&'static str>,
    marquee: MarqueeSelection,
    drag: Option<ClipDrag>,
    clip_press: Option<ClipPress>,
    scrollbar_drag: Option<f32>,
    track: Option<TrackId>,
    reveal_track: Option<TrackId>,
    vertical_scroll: f32,
    vertical_scroll_drag: Option<f32>,
    media: Option<ClipMedia>,
    context_menu: Option<ContextMenu>,
    volume_changed: bool,
    volume_preview: Option<(u64, i16)>,
    cursor_markers: Vec<u64>,
    cursor_track_available: bool,
    cursor_track_selected: bool,
    cursor_drag: Option<CursorEffectDrag>,
    user_markers: Vec<u64>,
    marker_action: Option<MarkerAction>,
    transition_drop: Option<TransitionDrop>,
    recognize_audio_track: Option<TrackId>,
    playhead_seek: PlayheadSeek,
}

#[derive(Clone, Copy)]
pub(super) enum MarkerAction {
    Add(u64),
    Remove(u64),
}

pub(super) struct TimelineInput {
    pub error: Option<String>,
    pub clipboard_feedback: Option<&'static str>,
    pub add_material: bool,
    pub material_drop: Option<MaterialDrop>,
    pub volume_changed: bool,
    pub volume_preview: Option<(u64, i16)>,
    pub marker_action: Option<MarkerAction>,
    pub transition_drop: Option<TransitionDrop>,
    pub recognize_audio_track: Option<TrackId>,
    pub playhead_seek: bool,
    pub playhead_seek_finished: bool,
}

#[derive(Clone, Copy)]
struct CursorEffectDrag {
    kind: CursorEffectDragKind,
    origin: u64,
    start_us: u64,
    end_us: u64,
}

#[derive(Clone, Copy)]
enum CursorEffectDragKind {
    Move,
    Start,
    End,
}

#[derive(Clone, Copy, Default)]
struct PlayheadSeek {
    active: bool,
    finished: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TransitionDrop {
    pub outgoing: u64,
    pub incoming: u64,
    pub id: &'static str,
    pub name: &'static str,
    pub role: TransitionRole,
    pub cut: u64,
}

pub(super) struct MaterialDrop {
    pub path: PathBuf,
    pub at: u64,
    pub destination: TrackDestination,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TrackDestination {
    Existing(TrackId),
    New {
        kind: TrackKind,
        before: Option<TrackId>,
    },
}

#[derive(Clone, Copy)]
struct ClipLane {
    view: Viewport,
    rect: Rect,
    id: TrackId,
    kind: TrackKind,
    enabled: bool,
}

impl TimelineView {
    pub(super) fn with_media(tools: Option<MediaTools>, ctx: &egui::Context) -> Self {
        Self {
            media: Some(ClipMedia::new(tools, ctx)),
            ..Self::default()
        }
    }

    pub(super) fn active_track(&self, timeline: &Timeline) -> TrackId {
        self.track
            .filter(|&id| timeline.track(id).is_some())
            .or_else(|| timeline.track(PRIMARY_VIDEO).map(|track| track.id))
            .or_else(|| timeline.tracks().first().map(|track| track.id))
            .unwrap_or(PRIMARY_VIDEO)
    }

    pub(super) fn reveal_added_track(&mut self, id: TrackId) {
        self.track = Some(id);
        self.reveal_track = Some(id);
    }

    pub(super) fn set_cursor_markers(&mut self, markers: &[u64], available: bool) {
        if self.cursor_markers.as_slice() != markers {
            self.cursor_markers.clear();
            self.cursor_markers.extend_from_slice(markers);
        }
        self.cursor_track_available = available;
        if !available {
            self.cursor_track_selected = false;
            self.cursor_drag = None;
        }
    }

    pub(super) fn cursor_track_selected(&self) -> bool {
        self.cursor_track_selected
    }

    pub(super) fn set_user_markers(&mut self, markers: Vec<u64>) {
        self.user_markers = markers;
    }

    pub(super) fn video_thumbnail_nearest(
        &mut self,
        path: &Path,
        timestamp_us: u64,
        tolerance_us: u64,
    ) -> Option<egui::TextureHandle> {
        self.media
            .as_mut()?
            .video_nearest_frame(path, timestamp_us, tolerance_us)
    }

    pub(super) fn selected_group(&self, primary: Option<u64>) -> &[u64] {
        let ids = self.marquee.ids();
        if ids.len() > 1 && primary.is_some_and(|id| ids.contains(&id)) {
            ids
        } else {
            &[]
        }
    }

    #[cfg(test)]
    pub fn draw(
        &mut self,
        ui: &mut Ui,
        timeline: &mut Timeline,
        playhead: &mut u64,
        selected: &mut Option<u64>,
        add_enabled: bool,
    ) -> TimelineInput {
        let mut cursor_effect = CursorEffect::default_for(timeline.duration().max(1));
        self.draw_with_cursor_effect(
            ui,
            timeline,
            playhead,
            selected,
            &mut cursor_effect,
            add_enabled,
        )
    }

    pub(super) fn draw_with_cursor_effect(
        &mut self,
        ui: &mut Ui,
        timeline: &mut Timeline,
        playhead: &mut u64,
        selected: &mut Option<u64>,
        cursor_effect: &mut CursorEffect,
        add_enabled: bool,
    ) -> TimelineInput {
        self.volume_changed = false;
        self.volume_preview = None;
        self.marker_action = None;
        self.transition_drop = None;
        self.recognize_audio_track = None;
        self.playhead_seek = PlayheadSeek::default();
        normalize_selection(timeline, playhead, selected);
        self.marquee.prune(timeline, *selected);
        if let Some(track) = selected.and_then(|id| timeline.track_for_clip(id)) {
            self.track = Some(track.id);
        }
        let width = (ui.available_width() - TRACK_HEADER - VERTICAL_SCROLL_WIDTH).max(1.0);
        let input = self.draw_toolbar(ui, timeline, *selected, *playhead, width, add_enabled);
        let transition_drag = DragAndDrop::has_payload_of_type::<DraggedTransition>(ui.ctx());
        let command = if material_drop::is_dragging(ui.ctx()) || transition_drag {
            None
        } else {
            input.command.or_else(|| self.keyboard_command(ui))
        };
        let track_result = input.add_track.map_or(Ok(()), |kind| {
            let before = timeline
                .tracks()
                .get(timeline.tracks().len() / 2)
                .map(|track| track.id);
            let id = timeline.add_track_before(kind, before)?;
            self.reveal_added_track(id);
            *selected = None;
            Ok(())
        });
        let active = self.active_track(timeline);
        let had_selection = selected.is_some();
        let command_result = command.map_or(Ok(None), |command| {
            if matches!(command, Command::Delete(false)) && self.marquee.ids().len() > 1 {
                timeline.remove_clips(self.marquee.ids())?;
                self.marquee.clear();
                *selected = None;
                return Ok(None);
            }
            execute(
                command,
                timeline,
                active,
                playhead,
                selected,
                &mut self.clipboard,
            )
        });
        if let Ok(Some(track)) = &command_result {
            self.reveal_added_track(*track);
            self.marquee.clear();
            self.clipboard_feedback = Some("已粘贴片段");
        } else if command_result.is_ok() && had_selection {
            self.clipboard_feedback = match command {
                Some(Command::Copy) => Some("已复制片段"),
                Some(Command::Cut) => Some("已剪切片段"),
                _ => None,
            };
        }
        let (material_drop, canvas_error) = match self.draw_canvas(
            ui,
            timeline,
            playhead,
            selected,
            cursor_effect,
        ) {
            Ok(drop) => (drop, None),
            Err(error) => (None, Some(error)),
        };
        normalize_selection(timeline, playhead, selected);
        self.marquee.prune(timeline, *selected);
        TimelineInput {
            error: track_result
                .err()
                .or(command_result.err())
                .or(canvas_error)
                .map(|error| error.to_string()),
            clipboard_feedback: self.clipboard_feedback.take(),
            add_material: input.add_material,
            material_drop,
            volume_changed: self.volume_changed,
            volume_preview: self.volume_preview,
            marker_action: self.marker_action,
            transition_drop: self.transition_drop,
            recognize_audio_track: self.recognize_audio_track,
            playhead_seek: self.playhead_seek.active || self.playhead_seek.finished,
            playhead_seek_finished: self.playhead_seek.finished,
        }
    }

    fn handle_transition_drop(
        &mut self,
        ui: &Ui,
        view: Viewport,
        rows: Rows<'_>,
        timeline: &Timeline,
    ) {
        let (pointer, released, held, focused) = ui.input(|input| {
            let release = input.events.iter().rev().find_map(|event| match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    ..
                } => Some(*pos),
                _ => None,
            });
            (
                release.or(input.pointer.interact_pos().or(input.pointer.latest_pos())),
                input.pointer.primary_released(),
                input.pointer.primary_down(),
                input.viewport().focused.unwrap_or(true),
            )
        });
        if !focused || (!held && !released) {
            DragAndDrop::clear_payload(ui.ctx());
            return;
        }
        let Some(pointer) = pointer.filter(|pointer| view.rect.contains(*pointer)) else {
            if released {
                DragAndDrop::clear_payload(ui.ctx());
            }
            return;
        };
        let at = view.time_at(pointer.x);
        let Some(transition) = DragAndDrop::payload::<DraggedTransition>(ui.ctx()) else {
            if released {
                DragAndDrop::clear_payload(ui.ctx());
            }
            return;
        };
        let target = match transition.role {
            TransitionRole::Entrance => {
                Self::transition_entrance_at(rows, timeline, pointer.y, at, view.scale)
                    .map(|incoming| (0, incoming.id, incoming.start))
            }
            TransitionRole::Exit => {
                Self::transition_exit_at(rows, timeline, pointer.y, at, view.scale)
                    .map(|outgoing| (outgoing.id, 0, outgoing.end()))
            }
            TransitionRole::Cut => Self::transition_cut_at(rows, timeline, pointer.y, at, view.scale),
        };
        let Some((outgoing, incoming, cut)) = target else {
            ui.ctx().set_cursor_icon(CursorIcon::NotAllowed);
            if released {
                DragAndDrop::clear_payload(ui.ctx());
            }
            return;
        };
        ui.ctx().set_cursor_icon(CursorIcon::Copy);
        if released {
            let transition = DragAndDrop::take_payload::<DraggedTransition>(ui.ctx())
                .unwrap_or(transition);
            self.transition_drop = Some(TransitionDrop {
                outgoing,
                incoming,
                id: transition.id,
                name: transition.name,
                role: transition.role,
                cut,
            });
        }
    }

fn transition_cut_at(
    rows: Rows<'_>,
    timeline: &Timeline,
    y: f32,
    at: u64,
    scale: f64,
) -> Option<(u64, u64, u64)> {
    let track = rows.at(timeline, y).filter(|track| track.kind == TrackKind::Video)
        .or_else(|| timeline.tracks().iter().find(|track| track.kind == TrackKind::Video));
    let track = track?;
    let tolerance = (12.0 * SECOND / scale.max(0.01)) as u64;
    track
        .clips
        .windows(2)
        .filter_map(|clips| {
            let outgoing = &clips[0];
            let incoming = &clips[1];
            let cut = outgoing.end();
            (cut.abs_diff(incoming.start) <= tolerance && cut.abs_diff(at) <= tolerance)
                .then_some((outgoing.id, incoming.id, incoming.start))
        })
        .min_by_key(|(_, _, cut)| cut.abs_diff(at))
}

fn transition_entrance_at<'a>(
    rows: Rows<'a>,
    timeline: &'a Timeline,
    y: f32,
    at: u64,
    scale: f64,
) -> Option<&'a Clip> {
    let track = rows.at(timeline, y).filter(|track| track.kind == TrackKind::Video)
        .or_else(|| timeline.tracks().iter().find(|track| track.kind == TrackKind::Video))?;
    let tolerance = (12.0 * SECOND / scale.max(0.01)) as u64;
    track.clips.iter().find(|clip| clip.start.abs_diff(at) <= tolerance)
}

fn transition_exit_at<'a>(
    rows: Rows<'a>,
    timeline: &'a Timeline,
    y: f32,
    at: u64,
    scale: f64,
) -> Option<&'a Clip> {
    let track = rows.at(timeline, y).filter(|track| track.kind == TrackKind::Video)
        .or_else(|| timeline.tracks().iter().find(|track| track.kind == TrackKind::Video))?;
    let tolerance = (12.0 * SECOND / scale.max(0.01)) as u64;
    track.clips.iter().find(|clip| clip.end().abs_diff(at) <= tolerance)
}

    fn scale(&self) -> f64 {
        self.scale.unwrap_or(DEFAULT_SCALE)
    }

    fn keyboard_command(&mut self, ui: &Ui) -> Option<Command> {
        let ctx = ui.ctx();
        if self.drag.is_some() || ctx.text_edit_focused() || egui::Popup::is_any_open(ctx) {
            return None;
        }
        let control_focused = ctx.egui_wants_keyboard_input();
        let text_selected = ctx
            .plugin::<egui::text_selection::LabelSelectionState>()
            .lock()
            .has_selection();
        ui.input_mut(|input| {
            // egui-winit delivers native clipboard shortcuts as dedicated events.
            if let Some(command) = input.events.iter().find_map(|event| match event {
                egui::Event::Copy if !text_selected => Some(Command::Copy),
                egui::Event::Cut if !text_selected => Some(Command::Cut),
                egui::Event::Paste(_) => Some(Command::Paste),
                _ => None,
            }) {
                return Some(command);
            }
            for (modifiers, key, command) in [
                (Modifiers::COMMAND | Modifiers::SHIFT, Key::Z, Command::Redo),
                (Modifiers::COMMAND, Key::Y, Command::Redo),
                (Modifiers::COMMAND, Key::Z, Command::Undo),
                (Modifiers::COMMAND, Key::C, Command::Copy),
                (Modifiers::COMMAND, Key::X, Command::Cut),
                (Modifiers::COMMAND, Key::V, Command::Paste),
                (Modifiers::COMMAND, Key::B, Command::Split),
                (Modifiers::SHIFT, Key::Delete, Command::Delete(true)),
            ] {
                if !(text_selected && matches!(command, Command::Copy | Command::Cut))
                    && input.consume_key(modifiers, key)
                {
                    return Some(command);
                }
            }
            if self.marquee.ids().len() > 1 && input.consume_key(Modifiers::NONE, Key::Delete) {
                return Some(Command::Delete(false));
            }
            if control_focused {
                return None;
            }
            for (modifiers, key, command) in [
                (Modifiers::NONE, Key::Delete, Command::Delete(false)),
                (Modifiers::NONE, Key::Q, Command::Trim(Edge::Start, false)),
                (Modifiers::NONE, Key::W, Command::Trim(Edge::End, false)),
                (Modifiers::NONE, Key::Home, Command::SeekStart),
                (Modifiers::NONE, Key::End, Command::SeekEnd),
                (Modifiers::NONE, Key::ArrowLeft, Command::Step(-33_333)),
                (Modifiers::NONE, Key::ArrowRight, Command::Step(33_333)),
            ] {
                if input.consume_key(modifiers, key) {
                    return Some(command);
                }
            }
            for mode in Mode::ALL {
                if input.consume_key(Modifiers::NONE, mode.key()) {
                    self.mode = mode;
                    break;
                }
            }
            None
        })
    }

    fn draw_canvas(
        &mut self,
        ui: &mut Ui,
        timeline: &mut Timeline,
        playhead: &mut u64,
        selected: &mut Option<u64>,
        cursor_effect: &mut CursorEffect,
    ) -> Result<Option<MaterialDrop>, TimelineError> {
        let height = ui.available_height();
        let cursor_band = if self.cursor_track_available {
            CURSOR_TRACK_HEIGHT + CURSOR_TRACK_GAP
        } else {
            0.0
        };
        let track_space = height - RULER_HEIGHT - SCROLLBAR_HEIGHT - cursor_band;
        if track_space <= 20.0 {
            return Ok(None);
        }
        let (rect, response) = ui.allocate_exact_size(
            vec2(ui.available_width().max(TRACK_HEADER + 1.0), height),
            Sense::hover(),
        );
        let content = Rect::from_min_max(
            pos2(rect.left() + TRACK_HEADER, rect.top()),
            pos2(
                rect.right() - VERTICAL_SCROLL_WIDTH,
                rect.bottom() - SCROLLBAR_HEIGHT,
            ),
        );
        let horizon = timeline
            .duration()
            .saturating_add(TRAILING_SPACE)
            .max(MIN_HORIZON);
        let material_drag = material_drop::is_dragging(ui.ctx());
        let transition_drag = DragAndDrop::has_payload_of_type::<DraggedTransition>(ui.ctx());
        let boundaries = Rows::measure(timeline);
        // Keep the complete track group vertically centered while it fits.
        // Once the rows need scrolling, retain a small reachable edge band.
        let track_height = boundaries.last().copied().unwrap_or_default();
        let edge_band = ((track_space - track_height) * 0.5).max(6.0);
        let mut rows = Rows {
            area: Rect::from_min_max(
                pos2(rect.left(), content.top() + RULER_HEIGHT + cursor_band),
                content.max,
            ),
            edge_band,
            offset: self.vertical_scroll,
            boundaries: &boundaries,
        };
        let max_vertical = rows.max_offset();
        if let Some(track) = self.reveal_track.take() {
            rows.reveal(track, timeline);
        }
        self.vertical_scroll = rows.offset.clamp(0.0, max_vertical);
        self.handle_wheel(ui, content, rect, max_vertical);
        rows.offset = self.vertical_scroll;
        if !material_drag && !transition_drag {
            self.capture_clip_drag(
                ui,
                timeline,
                rows,
                Viewport {
                    rect: content,
                    scale: self.scale(),
                    scroll: self.scroll,
                },
            );
            if let Some(drag) = self.drag {
                if !matches!(drag.kind, Gesture::Move) || !self.marquee.ids().contains(&drag.id) {
                    self.marquee.clear();
                }
                *selected = Some(drag.id);
            }
        }
        let dragging = self.drag.is_some()
            || self.cursor_drag.is_some()
            || ((material_drag || transition_drag) && ui.rect_contains_pointer(content));
        if dragging
            && !self
                .drag
                .is_some_and(|drag| matches!(drag.kind, Gesture::Volume { .. }))
        {
            self.pan_at_edge(ui, content, horizon);
            self.pan_tracks_at_edge(ui, rows, timeline, max_vertical);
        }
        self.scroll = self
            .scroll
            .clamp(0.0, self.max_scroll(content.width(), horizon));
        if !dragging {
            self.draw_scrollbar(ui, rect, content, horizon);
        }
        let view = Viewport {
            rect: content,
            scale: self.scale(),
            scroll: self.scroll,
        };
        if transition_drag {
            self.handle_transition_drop(ui, view, rows, timeline);
        }
        let selecting = self.marquee.observe(
            ui,
            timeline,
            view,
            rows,
            selected,
            self.cursor_track_rect(rows),
            !dragging && !material_drag && !transition_drag && self.mode == Mode::Select,
        );
        let colors = ui_theme::palette(ui.ctx());
        let painter = ui.painter_at(rect);
        let cursor_lane = self.cursor_track_rect(rows);
        self.playhead_seek = if !dragging && !selecting && self.cursor_drag.is_none() {
            self.seek_on_canvas(ui, view, timeline.duration(), playhead, cursor_lane)
        } else {
            PlayheadSeek::default()
        };
        let background = if ui.ctx().theme() == egui::Theme::Light {
            colors.surface
        } else {
            colors.track
        };
        painter.rect_filled(rect, 0, background);
        let ruler = Rect::from_min_max(rect.min, pos2(rect.right(), view.ruler().bottom()));
        let rail = Rect::from_min_max(
            pos2(rect.left(), ruler.bottom()),
            pos2(content.left(), content.bottom()),
        );
        painter.rect_filled(ruler, 0, colors.surface);
        painter.rect_filled(rail, 0, background.lerp_to_gamma(colors.track, 0.72));
        painter.hline(
            ruler.x_range(),
            ruler.bottom(),
            Stroke::new(1.0_f32, colors.border),
        );
        painter.vline(
            content.left(),
            content.y_range(),
            Stroke::new(1.0_f32, colors.border),
        );
        let time_label = painter.with_clip_rect(ruler.intersect(Rect::from_min_max(
            rect.min,
            pos2(content.left() - 4.0, ruler.bottom()),
        )));
        time_label.text(
            pos2(rect.left() + 8.0, ruler.center().y),
            Align2::LEFT_CENTER,
            timecode(*playhead),
            FontId::monospace(11.0),
            colors.timecode,
        );
        self.draw_track_scrollbar(
            ui,
            &painter,
            rows.area,
            max_vertical,
            dragging || selecting,
            &colors,
        );
        rows.offset = self.vertical_scroll;
        let time_painter = painter.with_clip_rect(content);
        paint_ruler(&time_painter, view, rows.area, background, &colors);
        self.draw_cursor_track(
            ui,
            &painter,
            view,
            rows,
            timeline.duration(),
            playhead,
            selected,
            cursor_effect,
        );
        let (track_action, cut) = self.draw_rows(ui, &painter, timeline, selected, view, rows);
        self.marquee.paint(&time_painter, &colors);
        edge_drop::describe_edges(ui, rows);
        paint_playhead(&painter, view, *playhead, &colors);
        self.paint_scrollbar(&painter, rect, content, horizon, &colors);
        if material_drag {
            context_menu::close(ui);
            self.context_menu = None;
            let drop = material_drop::interact(
                ui,
                &painter,
                view,
                material_drop::Target {
                    canvas: Rect::from_min_max(rect.min, content.max),
                    rows,
                    active_track: self.active_track(timeline),
                    playhead: *playhead,
                    snap: !self.disable_snap,
                },
                timeline,
            )?;
            if let Some(MaterialDrop {
                destination: TrackDestination::Existing(track),
                ..
            }) = &drop
            {
                self.track = Some(*track);
            }
            return Ok(drop);
        }

        if dragging || selecting {
            context_menu::close(ui);
            self.context_menu = None;
        } else {
            let opening = ContextMenu::at_release(ui, view, rows, timeline, cursor_lane);
            if let Some(menu) = opening {
                self.context_menu = Some(menu);
            }
            if let Some(menu) = self.context_menu
                && let Some(action) = menu.show(
                    ui,
                    &response,
                    opening.is_some(),
                    timeline,
                    self.clipboard.as_ref(),
                    self.marquee.ids(),
                    &self.user_markers,
                )
            {
                self.apply_context_action(action, timeline, playhead, selected)?;
                self.clipboard_feedback = match action {
                    context_menu::Action::Copy(_) => Some("已复制片段"),
                    context_menu::Action::Cut(_) => Some("已剪切片段"),
                    context_menu::Action::Paste(_, _) => Some("已粘贴片段"),
                    _ => None,
                };
                return Ok(None);
            }
            if !context_menu::is_open(ui) {
                self.context_menu = None;
            }
        }
        if self.mode == Mode::Blade
            && self.drag.is_none()
            && !context_menu::is_open(ui)
            && ui.input(|input| input.viewport().focused.unwrap_or(true))
            && ui.rect_contains_pointer(rows.area.intersect(content))
            && let Some(tip) = ui.ctx().pointer_hover_pos()
            && rows.at(timeline, tip.y).is_some()
        {
            let cursor = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                ui.id().with("blade-cursor"),
            ));
            paint_razor_cursor(&cursor, tip);
            ui.ctx().set_cursor_icon(CursorIcon::None);
        }
        match track_action {
            Some(TrackAction::Select(id)) => {
                self.track = Some(id);
                self.cursor_track_selected = false;
                self.marquee.clear();
                if selected.is_some_and(|clip| {
                    timeline
                        .track_for_clip(clip)
                        .is_some_and(|track| track.id != id)
                }) {
                    *selected = None;
                }
            }
            Some(TrackAction::SetOutput(id, enabled)) => timeline.set_track_enabled(id, enabled)?,
            None => {}
        }
        if let Some((id, at)) = cut {
            *selected = Some(timeline.split(id, at)?);
            *playhead = at;
        }
        self.update_drag(ui, &painter, view, rows, timeline, *playhead)?;
        Ok(None)
    }

    fn cursor_track_rect(&self, rows: Rows) -> Option<Rect> {
        if !self.cursor_track_available {
            return None;
        }
        let bottom = rows.area.top() - CURSOR_TRACK_GAP;
        let lane = Rect::from_min_size(
            pos2(rows.area.left() + 6.0, bottom - CURSOR_TRACK_HEIGHT),
            vec2((rows.area.width() - 12.0).max(0.0), CURSOR_TRACK_HEIGHT),
        );
        lane.is_positive().then_some(lane)
    }

    fn draw_cursor_track(
        &mut self,
        ui: &Ui,
        painter: &egui::Painter,
        view: Viewport,
        rows: Rows,
        duration: u64,
        playhead: &mut u64,
        selected: &mut Option<u64>,
        cursor_effect: &mut CursorEffect,
    ) {
        let Some(lane) = self.cursor_track_rect(rows) else {
            return;
        };
        let colors = ui_theme::palette(ui.ctx());
        let header = Rect::from_min_max(lane.min, pos2(view.rect.left(), lane.bottom()));
        let body = Rect::from_min_max(pos2(view.rect.left(), lane.top()), lane.max);
        painter.rect_filled(lane, 5.0, colors.surface.gamma_multiply(0.88));
        painter.rect_stroke(
            lane,
            5.0,
            Stroke::new(1.0_f32, colors.border),
            StrokeKind::Inside,
        );
        painter.rect_filled(header.shrink(2.0), 4.0, colors.control);
        Icon::ClickFollow.paint_scaled(
            painter,
            pos2(header.left() + 15.0, header.center().y),
            colors.accent,
            0.78,
        );
        painter.text(
            pos2(header.left() + 27.0, header.center().y),
            Align2::LEFT_CENTER,
            "鼠标轨道",
            FontId::proportional(10.5),
            if self.cursor_track_selected {
                colors.accent
            } else {
                colors.text
            },
        );
        let header_response = ui.interact(
            header,
            ui.id().with("cursor-track-header"),
            Sense::click(),
        );
        if header_response.clicked() {
            self.cursor_track_selected = true;
            *selected = None;
            self.marquee.clear();
        }

        let body_response = ui.interact(
            body,
            ui.id().with("cursor-track-background"),
            Sense::click(),
        );
        if body_response.clicked()
            && let Some(pointer) = body_response.interact_pointer_pos()
        {
            self.cursor_track_selected = true;
            *selected = None;
            self.marquee.clear();
            *playhead = view.time_at(pointer.x).min(duration);
        }

        let duration = duration.max(1);
        cursor_effect.start_us = cursor_effect.start_us.min(duration - 1);
        cursor_effect.end_us = cursor_effect
            .end_us
            .clamp(cursor_effect.start_us + 1, duration);
        if let Some(drag) = self.cursor_drag
            && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
        {
            let delta = i128::from(view.time_at(pointer.x)) - i128::from(drag.origin);
            match drag.kind {
                CursorEffectDragKind::Move => {
                    let span = drag.end_us.saturating_sub(drag.start_us).max(1);
                    let start = (i128::from(drag.start_us) + delta)
                        .clamp(0, i128::from(duration.saturating_sub(span)))
                        as u64;
                    cursor_effect.start_us = start;
                    cursor_effect.end_us = start.saturating_add(span).min(duration);
                }
                CursorEffectDragKind::Start => {
                    let latest_start = cursor_effect.end_us.saturating_sub(1);
                    cursor_effect.start_us = (i128::from(drag.start_us) + delta)
                        .clamp(0, i128::from(latest_start)) as u64;
                }
                CursorEffectDragKind::End => {
                    let earliest_end = cursor_effect.start_us.saturating_add(1);
                    cursor_effect.end_us = (i128::from(drag.end_us) + delta)
                        .clamp(i128::from(earliest_end), i128::from(duration))
                        as u64;
                }
            }
            self.cursor_track_selected = true;
        }

        let start_x = view.x(cursor_effect.start_us);
        let end_x = view.x(cursor_effect.end_us);
        let visible_range = Rect::from_min_max(
            pos2(start_x.min(end_x - 1.0), body.top() + 4.0),
            pos2(start_x.max(end_x).max(start_x + 1.0), body.bottom() - 4.0),
        )
        .intersect(body);
        if visible_range.is_positive() {
            let range_color = if cursor_effect.enabled {
                colors.accent.gamma_multiply(0.24)
            } else {
                colors.control
            };
            painter.rect_filled(visible_range, 4.0, range_color);
            painter.rect_stroke(
                visible_range,
                4.0,
                Stroke::new(
                    if self.cursor_track_selected { 1.5_f32 } else { 1.0_f32 },
                    if self.cursor_track_selected {
                        colors.accent
                    } else {
                        colors.accent.gamma_multiply(0.55)
                    },
                ),
                StrokeKind::Inside,
            );
            if visible_range.width() >= 75.0 {
                painter.text(
                    pos2(visible_range.left() + 7.0, visible_range.center().y),
                    Align2::LEFT_CENTER,
                    "鼠标效果",
                    FontId::proportional(10.0),
                    colors.text,
                );
            }
            let range_response = ui.interact(
                visible_range,
                ui.id().with("cursor-effect-range"),
                Sense::click_and_drag(),
            );
            if range_response.clicked() {
                self.cursor_track_selected = true;
                *selected = None;
                self.marquee.clear();
            }
            if range_response.drag_started()
                && let Some(pointer) = ui
                    .input(|input| input.pointer.press_origin())
                    .or_else(|| range_response.interact_pointer_pos())
            {
                self.cursor_drag = Some(CursorEffectDrag {
                    kind: CursorEffectDragKind::Move,
                    origin: view.time_at(pointer.x),
                    start_us: cursor_effect.start_us,
                    end_us: cursor_effect.end_us,
                });
                self.cursor_track_selected = true;
                *selected = None;
                self.marquee.clear();
            }
            if range_response.drag_stopped() {
                self.cursor_drag = None;
            }
        }

        for &marker in &self.cursor_markers {
            let x = view.x(marker);
            if !body.x_range().contains(x) {
                continue;
            }
            let marker_area = Rect::from_center_size(
                pos2(x, body.center().y),
                vec2(9.0, body.height()),
            );
            let response = ui
                .interact(
                    marker_area,
                    ui.id().with(("cursor-click", marker)),
                    Sense::click(),
                )
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(format!("点击事件 · {}", timecode(marker)));
            let color = if cursor_effect.contains(marker) {
                colors.accent
            } else {
                colors.muted
            };
            painter.vline(
                x,
                body.top() + 4.0..=body.bottom() - 4.0,
                Stroke::new(1.0_f32, color.gamma_multiply(0.62)),
            );
            painter.circle_filled(pos2(x, body.center().y), 3.0, color);
            if response.clicked() {
                self.cursor_track_selected = true;
                *selected = None;
                self.marquee.clear();
                *playhead = marker.min(duration);
            }
        }

        for (is_start, time) in [
            (true, cursor_effect.start_us),
            (false, cursor_effect.end_us),
        ] {
            let x = view.x(time);
            if !body.x_range().contains(x) {
                continue;
            }
            let handle = Rect::from_min_max(
                pos2(x - 5.0, body.top() + 2.0),
                pos2(x + 5.0, body.bottom() - 2.0),
            )
            .intersect(body);
            if !handle.is_positive() {
                continue;
            }
            let response = ui
                .interact(
                    handle,
                    ui.id().with(("cursor-effect-edge", is_start)),
                    Sense::click_and_drag(),
                )
                .on_hover_cursor(CursorIcon::ResizeHorizontal);
            if response.hovered() || response.dragged() {
                painter.vline(x, handle.y_range(), Stroke::new(2.0_f32, colors.text));
            }
            if response.drag_started()
                && let Some(pointer) = ui
                    .input(|input| input.pointer.press_origin())
                    .or_else(|| response.interact_pointer_pos())
            {
                self.cursor_drag = Some(CursorEffectDrag {
                    kind: if is_start {
                        CursorEffectDragKind::Start
                    } else {
                        CursorEffectDragKind::End
                    },
                    origin: view.time_at(pointer.x),
                    start_us: cursor_effect.start_us,
                    end_us: cursor_effect.end_us,
                });
                self.cursor_track_selected = true;
                *selected = None;
                self.marquee.clear();
            }
            if response.drag_stopped() {
                self.cursor_drag = None;
            }
        }

        for &marker in &self.user_markers {
            let x = view.x(marker);
            if !body.x_range().contains(x) {
                continue;
            }
            let center = pos2(x, body.top() + 7.0);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(center.x, center.y - 4.0),
                    pos2(center.x + 3.5, center.y),
                    pos2(center.x, center.y + 4.0),
                    pos2(center.x - 3.5, center.y),
                ],
                colors.recording,
                Stroke::NONE,
            ));
        }

        if ui.input(|input| input.pointer.primary_released()) {
            self.cursor_drag = None;
        }
    }

    fn seek_on_canvas(
        &self,
        ui: &Ui,
        view: Viewport,
        duration: u64,
        playhead: &mut u64,
        ignored: Option<Rect>,
    ) -> PlayheadSeek {
        let response = ui.interact(view.rect, ui.id().with("seek"), Sense::click_and_drag());
        let ignored_pointer = ignored.is_some_and(|rect| {
            response
                .interact_pointer_pos()
                .is_some_and(|pointer| rect.contains(pointer))
        });
        let active = !ignored_pointer
            && (response.is_pointer_button_down_on() || response.dragged());
        let finished = !ignored_pointer && (response.clicked() || response.drag_stopped());
        if self.drag.is_none()
            && !ignored_pointer
            && (response.clicked() || response.dragged_by(egui::PointerButton::Primary))
            && let Some(pointer) = response.interact_pointer_pos()
        {
            *playhead = view.time_at(pointer.x).min(duration);
        }
        PlayheadSeek { active, finished }
    }

    fn draw_clip(
        &mut self,
        ui: &Ui,
        painter: &egui::Painter,
        lane: ClipLane,
        clip: &Clip,
        selected: &mut Option<u64>,
        colors: &Palette,
    ) -> Option<u64> {
        let rect = lane.view.clip_rect(lane.rect, clip.start, clip.end());
        let visible = rect.intersect(painter.clip_rect());
        if !visible.is_positive() {
            return None;
        }
        let picked = self.marquee.picked(clip.id, *selected);
        let volume_preview = self
            .drag
            .filter(|drag| drag.id == clip.id)
            .and_then(|drag| {
                let pointer = ui.input(|input| {
                    clip_pointer::release_position(input).or(input.pointer.interact_pos())
                })?;
                drag.kind.volume_at(pointer.y)
            });
        let volume = volume_preview.unwrap_or(clip.volume_db_tenths);
        let handle = (rect.width() / 4.0).min(9.0);
        let name = clip.text.as_ref().map_or_else(
            || clip.path.file_name().unwrap_or_default().to_string_lossy(),
            |overlay| {
                overlay
                    .content
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .unwrap_or_default()
                    .into()
            },
        );
        let fill = if !lane.enabled {
            colors.control
        } else {
            match lane.kind {
                TrackKind::Video => colors.clip,
                TrackKind::Audio => colors.audio_clip,
                TrackKind::Caption => colors.clip.lerp_to_gamma(colors.control, 0.45),
                TrackKind::Text => colors.control,
            }
        };
        let border = if picked {
            colors.selected_clip_border
        } else if lane.enabled && lane.kind == TrackKind::Audio {
            fill.lerp_to_gamma(colors.audio_wave, 0.32)
        } else {
            fill.lerp_to_gamma(colors.clip_border, 0.74)
        };
        painter.rect_filled(rect, 0, fill);
        clip_media::paint(
            painter,
            rect,
            lane,
            clip,
            &name,
            colors,
            self.media.as_mut(),
            volume,
        );
        painter.rect_stroke(rect, 0, Stroke::new(1.0_f32, border), StrokeKind::Inside);
        if lane.kind == TrackKind::Video {
            paint_transition_block(painter, lane.view, lane.rect, clip, colors);
            if self.drag.is_none()
                && let Some((transition_start, transition_end)) = transition_time_range(clip)
            {
                for (edge, time) in [
                    (Edge::Start, transition_start),
                    (Edge::End, transition_end),
                ] {
                    let x = lane.view.x(time);
                    let handle = Rect::from_min_max(
                        pos2(x - 6.0, rect.top() - 2.0),
                        pos2(x + 6.0, rect.bottom() + 2.0),
                    )
                    .intersect(ui.clip_rect());
                    if !handle.is_positive() {
                        continue;
                    }
                    let response = ui.interact(
                        handle,
                        ui.id().with((
                            "transition-handle",
                            clip.id,
                            matches!(edge, Edge::Start),
                        )),
                        Sense::click_and_drag(),
                    );
                    let response = response.on_hover_cursor(CursorIcon::ResizeHorizontal);
                    if response.hovered() || response.dragged() {
                        painter.vline(
                            x,
                            rect.top()..=rect.bottom(),
                            Stroke::new(2.0, colors.text),
                        );
                    }
                    if response.drag_started()
                        && let Some(pointer) = response.interact_pointer_pos()
                    {
                        self.drag = Some(ClipDrag {
                            id: clip.id,
                            kind: Gesture::Transition(edge),
                            origin: lane.view.time_at(pointer.x),
                        });
                        self.track = Some(lane.id);
                    }
                }
            }
        }
        let line = self.mode.volume_line(rect, handle, lane.kind);
        if let Some(line) = line {
            let hovering = ui.rect_contains_pointer(line.hit(volume).intersect(visible));
            line.paint(
                painter,
                volume,
                hovering || volume_preview.is_some(),
                colors,
            );
        }
        if picked && rect.width() >= 38.0 && matches!(self.mode, Mode::Select | Mode::Ripple) {
            for x in [rect.left() + 4.0, rect.right() - 4.0] {
                let center = pos2(x, rect.center().y);
                painter.rect_filled(
                    Rect::from_center_size(center, vec2(2.0, 12.0)),
                    1,
                    colors.selected_clip_border,
                );
            }
        }
        if material_drop::is_dragging(ui.ctx()) || self.drag.is_some() {
            return None;
        }
        let mut cut = None;
        for (index, hit) in self
            .mode
            .hit_regions(rect, handle, clip)
            .into_iter()
            .enumerate()
        {
            let Some((hit, target)) = hit else { continue };
            let area = hit.intersect(visible);
            if !area.is_positive() {
                continue;
            }
            let sense = if matches!(target, ClipHit::Drag(_)) {
                Sense::click_and_drag()
            } else {
                Sense::click()
            };
            let response = ui.interact(area, ui.id().with(("clip", clip.id, index)), sense);
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::Button, true, picked, &*name)
            });
            if response.hovered() || response.dragged() {
                if let Some(cursor) = target.cursor() {
                    ui.ctx().set_cursor_icon(cursor);
                }
                if let ClipHit::Drag(Gesture::Trim(edge, _) | Gesture::Roll(edge)) = target {
                    let x = edge.x(rect);
                    painter.vline(
                        x,
                        rect.top() + 7.0..=rect.bottom() - 7.0,
                        Stroke::new(2.0_f32, colors.accent),
                    );
                }
                if matches!(target, ClipHit::Cut)
                    && let Some(pointer) = response.hover_pos()
                {
                    painter.vline(
                        pointer.x,
                        rect.top()..=rect.bottom(),
                        Stroke::new(1.0_f32, colors.accent),
                    );
                }
            }
            if response.clicked() {
                self.cursor_track_selected = false;
                let extend = self.mode == Mode::Select && ui.input(|input| input.modifiers.shift);
                self.marquee.click(clip.id, extend, selected);
                self.track = Some(lane.id);
                response.surrender_focus();
                if matches!(target, ClipHit::Cut)
                    && let Some(pointer) = response.interact_pointer_pos()
                {
                    cut = Some(lane.view.time_at(pointer.x));
                }
            }
            if self.drag.is_none() {
                response.on_hover_text(format!(
                    "{} · {}\n{}",
                    clip.text
                        .as_ref()
                        .map_or(&*name, |overlay| overlay.content.as_str()),
                    timecode(clip.duration()),
                    target.hint()
                ));
            }
        }
        if let Some(line) = line {
            let hit = line.hit(volume).intersect(visible);
            if hit.is_positive() {
                let response = ui.interact(
                    hit,
                    ui.id().with(("clip-volume", clip.id)),
                    Sense::click_and_drag(),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::slider(true, f64::from(volume) / 10.0, "片段音量")
                });
                if response.clicked() {
                    self.cursor_track_selected = false;
                    let extend = ui.input(|input| input.modifiers.shift);
                    self.marquee.click(clip.id, extend, selected);
                    self.track = Some(lane.id);
                    response.surrender_focus();
                }
                response
                    .on_hover_cursor(CursorIcon::ResizeVertical)
                    .on_hover_text("上下拖动调整音量");
            }
        }
        cut
    }

    fn update_drag(
        &mut self,
        ui: &Ui,
        painter: &egui::Painter,
        view: Viewport,
        rows: Rows,
        timeline: &mut Timeline,
        playhead: u64,
    ) -> Result<(), TimelineError> {
        let Some(drag) = self.drag else { return Ok(()) };
        let (pointer, released, down, canceled) = ui.input_mut(|input| {
            let canceled = input.consume_key(Modifiers::NONE, Key::Escape)
                || !input.viewport().focused.unwrap_or(true);
            (
                clip_pointer::release_position(input).or(input.pointer.interact_pos()),
                input.pointer.primary_released(),
                input.pointer.primary_down(),
                canceled,
            )
        });
        if canceled || !down {
            self.drag = None;
        }
        if canceled || (!released && !down) {
            return Ok(());
        }
        let Some(pointer) = pointer else {
            return Ok(());
        };
        if matches!(drag.kind, Gesture::Move)
            && self.marquee.ids().len() > 1
            && self.marquee.ids().contains(&drag.id)
        {
            if !view
                .rect
                .intersect(rows.area)
                .intersect(ui.clip_rect())
                .contains(pointer)
            {
                ui.ctx().set_cursor_icon(CursorIcon::NotAllowed);
                return Ok(());
            }
            let tolerance = if self.disable_snap {
                0
            } else {
                (7.0 / view.scale * SECOND) as u64
            };
            let delta = self.marquee.move_delta(
                timeline,
                difference(view.time_at(pointer.x), drag.origin),
                playhead,
                tolerance,
            );
            if released {
                timeline.move_clips(self.marquee.ids(), delta)?;
            } else {
                let painter = painter.with_clip_rect(view.rect.intersect(rows.area));
                let colors = ui_theme::palette(ui.ctx());
                let valid = self
                    .marquee
                    .paint_move(&painter, timeline, view, rows, delta, &colors);
                ui.ctx().set_cursor_icon(if valid {
                    CursorIcon::Grabbing
                } else {
                    CursorIcon::NotAllowed
                });
            }
            return Ok(());
        }
        let source = timeline
            .track_for_clip(drag.id)
            .ok_or(TimelineError::ClipNotFound(drag.id))?;
        let new_track = if matches!(drag.kind, Gesture::Move | Gesture::Reorder)
            && ui.clip_rect().contains(pointer)
        {
            TrackGap::at(rows, timeline, pointer)
        } else {
            None
        };
        let x = if new_track.is_some() {
            pointer.x.max(view.rect.left())
        } else {
            pointer.x
        };
        let time = view.time_at(x);
        let mut delta = difference(time, drag.origin);
        if !self.disable_snap && delta != 0 {
            delta = drag.snap(
                timeline,
                playhead,
                delta,
                (7.0 / view.scale * SECOND) as u64,
            );
        }
        if let Some(gap) = new_track {
            if !gap.allows(source.kind) {
                ui.ctx().set_cursor_icon(CursorIcon::NotAllowed);
                return if released {
                    Err(TimelineError::IncompatibleTrack)
                } else {
                    Ok(())
                };
            }
            let clip = timeline
                .clip(drag.id)
                .ok_or(TimelineError::ClipNotFound(drag.id))?;
            let at = drag.move_start(clip, delta);
            if released {
                let track = timeline.move_to_new_track_before(drag.id, gap.before, at)?;
                self.reveal_added_track(track);
                ui.ctx().request_repaint();
            } else {
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                gap.paint_target(painter, view, at, rows);
            }
            return Ok(());
        }
        let target = if matches!(drag.kind, Gesture::Move) {
            let Some(target) = rows
                .at(timeline, pointer.y)
                .filter(|_| rows.area.contains(pointer))
            else {
                ui.ctx().set_cursor_icon(CursorIcon::NotAllowed);
                return Ok(());
            };
            target.id
        } else {
            source.id
        };
        ui.ctx().set_cursor_icon(drag.kind.cursor());
        let colors = ui_theme::palette(ui.ctx());
        let painter = painter.with_clip_rect(view.rect.intersect(rows.area));
        match drag.propose(timeline, delta, time, target, pointer.y) {
            Ok(edit) => {
                if let Some(value) = edit.volume_value() {
                    self.volume_preview = Some((drag.id, value));
                }
                if let Some(index) = timeline
                    .tracks()
                    .iter()
                    .position(|track| track.id == edit.track)
                {
                    paint_draft(&edit, &painter, view, rows.rect(view, index), &colors);
                }
                if released {
                    self.volume_changed = matches!(drag.kind, Gesture::Volume { .. });
                    let destination = edit.track;
                    edit.commit(timeline)?;
                    self.track = Some(destination);
                }
            }
            Err(error) if released => return Err(error),
            Err(error) => {
                ui.ctx().set_cursor_icon(CursorIcon::NotAllowed);
                let pos = pos2(
                    view.rect.left() + 10.0,
                    pointer
                        .y
                        .clamp(rows.area.top() + 10.0, rows.area.bottom() - 14.0),
                );
                painter.text(
                    pos,
                    Align2::LEFT_TOP,
                    error.to_string(),
                    FontId::proportional(11.0),
                    colors.danger,
                );
            }
        }
        Ok(())
    }

    fn zoom(&mut self, new_scale: f64, anchor: f32) {
        let new_scale = new_scale.clamp(MIN_SCALE, MAX_SCALE);
        self.scroll += f64::from(anchor) * SECOND * (1.0 / self.scale() - 1.0 / new_scale);
        self.scale = Some(new_scale);
    }

    fn max_scroll(&self, width: f32, horizon: u64) -> f64 {
        (horizon as f64 - f64::from(width) * SECOND / self.scale()).max(0.0)
    }

    fn handle_wheel(&mut self, ui: &Ui, content: Rect, canvas: Rect, max_vertical: f32) {
        if !ui.rect_contains_pointer(canvas) || self.drag.is_some() {
            return;
        }
        ui.input_mut(|input| {
            let zoom = input.zoom_delta();
            if zoom != 1.0
                && let Some(pointer) = input.pointer.hover_pos()
            {
                self.zoom(self.scale() * f64::from(zoom), pointer.x - content.left());
            }
            let delta = input.smooth_scroll_delta();
            if zoom == 1.0 && max_vertical > 0.0 && !input.modifiers.shift {
                self.vertical_scroll = (self.vertical_scroll - delta.y).clamp(0.0, max_vertical);
                self.scroll -= f64::from(delta.x) * SECOND / self.scale();
            } else if zoom == 1.0 {
                self.scroll -= f64::from(delta.x + delta.y) * SECOND / self.scale();
            }
            input.smooth_scroll_delta = egui::Vec2::ZERO;
        });
    }

    fn pan_at_edge(&mut self, ui: &Ui, content: Rect, horizon: u64) {
        if content.width() < 60.0 {
            return;
        }
        let (pointer, held, dt) = ui.input(|input| {
            (
                input.pointer.interact_pos(),
                input.pointer.primary_down(),
                input.stable_dt,
            )
        });
        let Some(pointer) = pointer.filter(|p| content.y_range().contains(p.y) && held) else {
            return;
        };
        let left = ((content.left() + 22.0 - pointer.x) / 22.0).clamp(0.0, 1.0);
        let right = ((pointer.x - content.right() + 22.0) / 22.0).clamp(0.0, 1.0);
        let next =
            self.scroll + f64::from((right - left) * dt.min(0.05) * 460.0) * SECOND / self.scale();
        let next = next.clamp(0.0, self.max_scroll(content.width(), horizon));
        if next != self.scroll {
            self.scroll = next;
            ui.ctx().request_repaint();
        }
    }

    fn scrollbar(&self, rect: Rect, content: Rect, horizon: u64) -> (Rect, Rect) {
        let rail = Rect::from_min_max(
            pos2(content.left() + 4.0, content.bottom() + 4.0),
            pos2(content.right() - 4.0, rect.bottom() - 4.0),
        );
        let visible =
            (f64::from(content.width()) * SECOND / (self.scale() * horizon as f64)).min(1.0);
        let thumb_width = (rail.width() * visible as f32).max(24.0).min(rail.width());
        let max_scroll = self.max_scroll(content.width(), horizon);
        let left =
            rail.left() + (rail.width() - thumb_width) * (self.scroll / max_scroll.max(1.0)) as f32;
        let thumb = Rect::from_min_size(pos2(left, rail.top()), vec2(thumb_width, rail.height()));
        (rail, thumb)
    }

    fn draw_scrollbar(&mut self, ui: &Ui, rect: Rect, content: Rect, horizon: u64) {
        let (rail, thumb) = self.scrollbar(rect, content, horizon);
        let max_scroll = self.max_scroll(content.width(), horizon);
        if max_scroll <= 0.0 || rail.width() <= thumb.width() {
            self.scrollbar_drag = None;
            return;
        }
        let response = ui
            .interact(
                rail.expand2(vec2(0.0, 3.0)),
                ui.id().with("horizontal-scroll"),
                Sense::click_and_drag(),
            )
            .on_hover_text("拖动浏览时间线");
        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(origin) = ui.input(|input| input.pointer.press_origin())
        {
            self.scrollbar_drag = Some(if thumb.contains(origin) {
                origin.x - thumb.left()
            } else {
                thumb.width() / 2.0
            });
        }
        if (response.clicked()
            || response.dragged_by(egui::PointerButton::Primary)
            || response.drag_stopped_by(egui::PointerButton::Primary))
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let offset = self.scrollbar_drag.unwrap_or(thumb.width() / 2.0);
            let fraction = ((pointer.x - rail.left() - offset) / (rail.width() - thumb.width()))
                .clamp(0.0, 1.0);
            self.scroll = max_scroll * f64::from(fraction);
        }
        if response.drag_stopped() || !ui.input(|input| input.pointer.primary_down()) {
            self.scrollbar_drag = None;
        }
    }

    fn paint_scrollbar(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        content: Rect,
        horizon: u64,
        colors: &Palette,
    ) {
        let (rail, thumb) = self.scrollbar(rect, content, horizon);
        if self.max_scroll(content.width(), horizon) > 0.0 {
            painter.rect_filled(rail, 3, colors.control);
            let tint = if self.scrollbar_drag.is_some() {
                colors.accent
            } else {
                colors.control.lerp_to_gamma(colors.muted, 0.56)
            };
            painter.rect_filled(thumb, 3, tint);
        }
    }
}

fn paint_transition_block(
    painter: &egui::Painter,
    view: Viewport,
    lane: Rect,
    clip: &Clip,
    colors: &Palette,
) {
    let Some(transition) = &clip.transition_in else {
        return;
    };
    let Some((start, end)) = transition_time_range(clip) else {
        return;
    };
    let mut block = view.clip_rect(lane, start, end).shrink2(vec2(1.0, 1.0));
    block = block.intersect(painter.clip_rect());
    if !block.is_positive() {
        return;
    }
    let fill = colors.accent.gamma_multiply(0.24);
    let border = colors.text.gamma_multiply(0.92);
    painter.rect_filled(block, 3.0, fill);
    painter.rect_stroke(block, 3.0, Stroke::new(1.5, border), StrokeKind::Inside);
    painter.vline(
        block.left() + 2.0,
        block.top()..=block.bottom(),
        Stroke::new(2.0, border),
    );
    painter.vline(
        block.right() - 2.0,
        block.top()..=block.bottom(),
        Stroke::new(2.0, border),
    );
    let center = block.center();
    painter.line_segment(
        [pos2(center.x - 11.0, center.y), pos2(center.x + 11.0, center.y)],
        Stroke::new(1.5, border),
    );
    painter.line_segment(
        [pos2(center.x - 11.0, center.y), pos2(center.x - 6.0, center.y - 4.0)],
        Stroke::new(1.5, border),
    );
    painter.line_segment(
        [pos2(center.x - 11.0, center.y), pos2(center.x - 6.0, center.y + 4.0)],
        Stroke::new(1.5, border),
    );
    painter.line_segment(
        [pos2(center.x + 11.0, center.y), pos2(center.x + 6.0, center.y - 4.0)],
        Stroke::new(1.5, border),
    );
    painter.line_segment(
        [pos2(center.x + 11.0, center.y), pos2(center.x + 6.0, center.y + 4.0)],
        Stroke::new(1.5, border),
    );
    if block.width() >= 54.0 && !transition.name.is_empty() {
        painter.text(
            pos2(block.left() + 7.0, block.top() + 7.0),
            egui::Align2::LEFT_CENTER,
            &transition.name,
            FontId::proportional(9.0),
            colors.text,
        );
    }
}

pub(super) fn transition_time_range(clip: &Clip) -> Option<(u64, u64)> {
    let transition = clip.transition_in.as_ref()?;
    let duration = transition.duration_us.min(clip.duration().max(1));
    Some(match transition.role.as_str() {
        "入场" => (clip.start, clip.start.saturating_add(duration)),
        "出场" => (clip.end().saturating_sub(duration), clip.end()),
        _ => (
            clip.start.saturating_sub(duration / 2),
            clip.start.saturating_add(duration / 2),
        ),
    })
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Select,
    Blade,
    Reorder,
    Ripple,
    Roll,
    Slip,
    Slide,
}

#[derive(Clone, Copy)]
enum ClipHit {
    Select,
    Cut,
    Drag(Gesture),
}

impl ClipHit {
    fn cursor(self) -> Option<CursorIcon> {
        match self {
            Self::Select => Some(CursorIcon::PointingHand),
            Self::Cut => None,
            Self::Drag(gesture) => Some(gesture.cursor()),
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Select => "点击选择片段",
            Self::Cut => "点击分割片段",
            Self::Drag(gesture) => gesture.hint(),
        }
    }
}

impl Mode {
    const ALL: [Self; 7] = [
        Self::Select,
        Self::Blade,
        Self::Reorder,
        Self::Ripple,
        Self::Roll,
        Self::Slip,
        Self::Slide,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Select => "选择",
            Self::Blade => "刀片",
            Self::Reorder => "重排",
            Self::Ripple => "波纹修剪",
            Self::Roll => "滚动剪辑",
            Self::Slip => "滑移素材",
            Self::Slide => "滑动片段",
        }
    }

    fn key(self) -> Key {
        match self {
            Self::Select => Key::V,
            Self::Blade => Key::C,
            Self::Reorder => Key::M,
            Self::Ripple => Key::B,
            Self::Roll => Key::N,
            Self::Slip => Key::Y,
            Self::Slide => Key::U,
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Select => "拖动片段移动位置，拖动边缘修剪",
            Self::Blade => "点击片段内的位置进行分割",
            Self::Reorder => "拖动片段改变顺序并排紧所在轨道",
            Self::Ripple => "拖动片段边缘修剪，同时移动后续片段",
            Self::Roll => "拖动相连片段的接缝，同时修剪两侧",
            Self::Slip => "拖动以改变取用的素材区间，保持片段位置和时长",
            Self::Slide => "拖动片段并同步修剪相连的两个邻居",
        }
    }

    fn volume_line(self, rect: Rect, handle: f32, kind: TrackKind) -> Option<VolumeLine> {
        if self == Self::Select && kind == TrackKind::Audio {
            VolumeLine::for_clip(rect, handle)
        } else {
            None
        }
    }

    fn hit_regions(self, rect: Rect, width: f32, clip: &Clip) -> [Option<(Rect, ClipHit)>; 3] {
        let region = |rect, kind| Some((rect, kind));
        let whole = |kind| [region(rect, kind), None, None];
        let (left, right, middle) = match self {
            Self::Blade => return whole(ClipHit::Cut),
            Self::Reorder => return whole(ClipHit::Drag(Gesture::Reorder)),
            Self::Slip if clip.text.is_some() => return whole(ClipHit::Select),
            Self::Slip => return whole(ClipHit::Drag(Gesture::Slip)),
            Self::Slide => return whole(ClipHit::Drag(Gesture::Slide)),
            Self::Roll => (
                Gesture::Roll(Edge::Start),
                Gesture::Roll(Edge::End),
                ClipHit::Select,
            ),
            Self::Select | Self::Ripple => (
                Gesture::Trim(Edge::Start, self == Self::Ripple),
                Gesture::Trim(Edge::End, self == Self::Ripple),
                if self == Self::Select {
                    ClipHit::Drag(Gesture::Move)
                } else {
                    ClipHit::Select
                },
            ),
        };
        let left_x = rect.left() + width;
        let right_x = rect.right() - width;
        [
            region(
                Rect::from_x_y_ranges(rect.left()..=left_x, rect.y_range()),
                ClipHit::Drag(left),
            ),
            region(
                Rect::from_x_y_ranges(left_x..=right_x, rect.y_range()),
                middle,
            ),
            region(
                Rect::from_x_y_ranges(right_x..=rect.right(), rect.y_range()),
                ClipHit::Drag(right),
            ),
        ]
    }
}

#[cfg(test)]
mod transition_cut_tests {
    use super::*;
    use std::path::PathBuf;
    use takeseam::editor_timeline::Timeline;

    #[test]
    fn transition_drop_requires_a_cut_between_two_video_clips() {
        let mut timeline = Timeline::default();
        timeline
            .insert_media(PathBuf::from("first.mp4"), 1_000_000, 0)
            .unwrap();
        timeline
            .insert_media(PathBuf::from("second.mp4"), 1_000_000, 1_000_000)
            .unwrap();
        let boundaries = Rows::measure(&timeline);
        let rows = Rows {
            area: Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 240.0)),
            edge_band: 0.0,
            offset: 0.0,
            boundaries: &boundaries,
        };

        assert!(TimelineView::transition_cut_at(
            rows,
            &timeline,
            70.0,
            1_000_000,
            DEFAULT_SCALE,
        )
        .is_some());
        assert!(TimelineView::transition_cut_at(
            rows,
            &timeline,
            70.0,
            1_500_000,
            DEFAULT_SCALE,
        )
        .is_none());
    }
}

#[cfg(test)]
mod cursor_track_interaction_tests {
    use super::*;

    fn frame(
        ctx: &egui::Context,
        view: &mut TimelineView,
        effect: &mut CursorEffect,
        playhead: &mut u64,
        selected: &mut Option<u64>,
        events: Vec<egui::Event>,
    ) {
        let boundaries = [];
        let rows = Rows {
            area: Rect::from_min_size(pos2(0.0, 100.0), vec2(900.0, 100.0)),
            edge_band: 0.0,
            offset: 0.0,
            boundaries: &boundaries,
        };
        let viewport = Viewport {
            rect: Rect::from_min_max(pos2(72.0, 0.0), pos2(900.0, 100.0)),
            scale: 100.0,
            scroll: 0.0,
        };
        let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(1000.0, 250.0));
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            },
            |ui| {
                let painter = ui.painter_at(screen);
                view.draw_cursor_track(
                    ui,
                    &painter,
                    viewport,
                    rows,
                    10_000_000,
                    playhead,
                    selected,
                    effect,
                );
            },
        );
    }

    #[test]
    fn trim_handle_wins_over_a_click_marker_and_tracks_the_pointer_from_press() {
        let ctx = egui::Context::default();
        let mut view = TimelineView::default();
        view.set_cursor_markers(&[1_000_000], true);
        let mut effect = CursorEffect {
            id: 1,
            start_us: 1_000_000,
            end_us: 5_000_000,
            enabled: true,
            cursor_scale: 1.0,
            zoom_scale: 1.6,
            smoothing: 0.7,
            click_effect: super::super::cursor_effect::CursorClickEffect::Ring,
        };
        let mut playhead = 0;
        let mut selected = Some(7);
        let press = pos2(172.0, 79.0);
        let threshold = pos2(182.0, 79.0);
        let destination = pos2(272.0, 79.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };

        frame(&ctx, &mut view, &mut effect, &mut playhead, &mut selected, vec![]);
        frame(
            &ctx,
            &mut view,
            &mut effect,
            &mut playhead,
            &mut selected,
            vec![egui::Event::PointerMoved(press), button(press, true)],
        );
        frame(
            &ctx,
            &mut view,
            &mut effect,
            &mut playhead,
            &mut selected,
            vec![egui::Event::PointerMoved(threshold)],
        );
        frame(
            &ctx,
            &mut view,
            &mut effect,
            &mut playhead,
            &mut selected,
            vec![egui::Event::PointerMoved(destination)],
        );
        frame(
            &ctx,
            &mut view,
            &mut effect,
            &mut playhead,
            &mut selected,
            vec![button(destination, false)],
        );

        assert_eq!(effect.start_us, 2_000_000);
        assert_eq!(effect.end_us, 5_000_000);
        assert_eq!(playhead, 0);
        assert_eq!(selected, None);
    }
}

impl Edge {
    fn x(self, rect: Rect) -> f32 {
        match self {
            Self::Start => rect.left(),
            Self::End => rect.right(),
        }
    }
}

impl Gesture {
    fn cursor(self) -> CursorIcon {
        match self {
            Self::Trim(..) | Self::Roll(_) | Self::Slip | Self::Slide => {
                CursorIcon::ResizeHorizontal
            }
            Self::Move | Self::Reorder => CursorIcon::Grab,
            Self::Volume { .. } => CursorIcon::ResizeVertical,
            Self::Transition(_) => CursorIcon::ResizeHorizontal,
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Move => "拖动片段移动位置",
            Self::Reorder => "拖动片段改变顺序",
            Self::Trim(Edge::Start, false) => "拖动修剪入点",
            Self::Trim(Edge::End, false) => "拖动修剪出点",
            Self::Trim(Edge::Start, true) => "拖动波纹修剪入点",
            Self::Trim(Edge::End, true) => "拖动波纹修剪出点",
            Self::Roll(_) => "拖动相连片段的接缝",
            Self::Slip => "拖动改变片段取用的素材区间",
            Self::Slide => "拖动并同步修剪两个邻居",
            Self::Volume { .. } => "上下拖动调整音量",
            Self::Transition(_) => "拖动转场边界调整时长",
        }
    }
}

