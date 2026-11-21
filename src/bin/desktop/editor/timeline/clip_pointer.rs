use eframe::egui::{self, Event, PointerButton, Pos2, Ui};
use takeseam::editor_timeline::Timeline;

use super::canvas::Viewport;
use super::gesture::ClipDrag;
use super::track_rows::Rows;
use super::{ClipHit, TimelineView, gesture::Gesture};

pub(super) struct ClipPress {
    drag: ClipDrag,
    origin: Pos2,
}

impl TimelineView {
    pub(super) fn capture_clip_drag(
        &mut self,
        ui: &Ui,
        timeline: &Timeline,
        rows: Rows,
        view: Viewport,
    ) {
        let (focused, events) = ui.input(|input| {
            (
                input.viewport().focused.unwrap_or(true),
                input
                    .events
                    .iter()
                    .filter_map(Input::from_event)
                    .collect::<Vec<_>>(),
            )
        });
        if !focused {
            self.clip_press = None;
            self.drag = None;
            return;
        }
        let threshold = ui
            .ctx()
            .options(|options| options.input_options.max_click_dist);
        for event in events {
            match event {
                Input::Press(p) => {
                    self.drag = None;
                    self.clip_press = self.clip_drag_at(ui, timeline, rows, view, p);
                }
                Input::Move(p) | Input::Release(p) => {
                    if self.drag.is_none()
                        && let Some(press) = &self.clip_press
                        && press.origin.distance_sq(p) > threshold * threshold
                    {
                        let mut drag = press.drag;
                        let delta = p - press.origin;
                        if matches!(drag.kind, Gesture::Volume { .. })
                            && delta.x.abs() > delta.y.abs()
                        {
                            drag.kind = Gesture::Move;
                        }
                        self.drag = Some(drag);
                    }
                    if matches!(event, Input::Release(_)) {
                        self.clip_press = None;
                    }
                }
                Input::Escape => {
                    self.clip_press = None;
                    self.drag = None;
                }
                Input::Gone if self.clip_press.is_some() => {
                    self.clip_press = None;
                    self.drag = None;
                }
                Input::Gone => {}
            }
        }
    }

    fn clip_drag_at(
        &self,
        ui: &Ui,
        timeline: &Timeline,
        rows: Rows,
        view: Viewport,
        p: Pos2,
    ) -> Option<ClipPress> {
        if !ui.clip_rect().intersect(view.rect).contains(p)
            || ui
                .ctx()
                .layer_id_at(p)
                .is_some_and(|layer| layer != ui.layer_id())
        {
            return None;
        }
        let track = rows.at(timeline, p.y)?;
        let time = view.time_at(p.x);
        let clip = track.clip_at(time)?;
        let index = timeline
            .tracks()
            .iter()
            .position(|item| item.id == track.id)?;
        let row = rows.rect(view, index);
        let rect = view.clip_rect(row, clip.start, clip.end());
        let handle = (rect.width() / 4.0).min(9.0);
        let volume = self.mode.volume_line(rect, handle, track.kind);
        let kind =
            if let Some(line) = volume.filter(|line| line.hit(clip.volume_db_tenths).contains(p)) {
                Gesture::Volume {
                    line,
                    offset: p.y - line.y_at(clip.volume_db_tenths),
                }
            } else {
                self.mode
                    .hit_regions(rect, handle, clip)
                    .into_iter()
                    .flatten()
                    .find_map(|(hit, target)| {
                        if hit.contains(p)
                            && let ClipHit::Drag(kind) = target
                        {
                            Some(kind)
                        } else {
                            None
                        }
                    })?
            };
        Some(ClipPress {
            drag: ClipDrag {
                id: clip.id,
                kind,
                origin: time,
            },
            origin: p,
        })
    }
}

enum Input {
    Press(Pos2),
    Move(Pos2),
    Release(Pos2),
    Escape,
    Gone,
}

impl Input {
    fn from_event(e: &Event) -> Option<Self> {
        match *e {
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                ..
            } => Some(if pressed {
                Self::Press(pos)
            } else {
                Self::Release(pos)
            }),
            Event::PointerMoved(p) => Some(Self::Move(p)),
            Event::PointerGone => Some(Self::Gone),
            Event::Key {
                key: egui::Key::Escape,
                pressed: true,
                ..
            } => Some(Self::Escape),
            _ => None,
        }
    }
}

pub(super) fn release_position(input: &egui::InputState) -> Option<Pos2> {
    input
        .events
        .iter()
        .rev()
        .find_map(|event| match Input::from_event(event) {
            Some(Input::Release(p)) => Some(p),
            _ => None,
        })
}
