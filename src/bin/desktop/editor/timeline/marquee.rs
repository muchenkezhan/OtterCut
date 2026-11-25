use eframe::egui::{
    self, Color32, Event, Key, Painter, PointerButton, Pos2, Rect, Stroke, StrokeKind,
};
use takeseam::editor_timeline::Timeline;

use crate::ui_theme::Palette;

use super::canvas::Viewport;
use super::gesture::difference;
use super::track_rows::Rows;

#[derive(Default)]
pub(super) struct MarqueeSelection {
    ids: Vec<u64>,
    press: Option<MarqueePress>,
    preview: Option<MarqueePreview>,
}

struct MarqueePress {
    origin: Pos2,
    pointer: Pos2,
    base: Vec<u64>,
    extend: bool,
    moved: bool,
}

struct MarqueePreview {
    rect: Rect,
    ids: Vec<u64>,
}

impl MarqueeSelection {
    pub(super) fn ids(&self) -> &[u64] {
        &self.ids
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(super) fn prune(&mut self, timeline: &Timeline, primary: Option<u64>) {
        if self.ids.is_empty() {
            return;
        }
        if primary.is_none_or(|id| !self.ids.contains(&id)) {
            self.ids.clear();
        } else {
            self.ids.retain(|&id| timeline.clip(id).is_some());
        }
    }

    pub(super) fn picked(&self, id: u64, primary: Option<u64>) -> bool {
        self.preview.as_ref().map_or_else(
            || primary == Some(id) || self.ids.contains(&id),
            |preview| preview.ids.contains(&id),
        )
    }

    pub(super) fn click(&mut self, id: u64, extend: bool, primary: &mut Option<u64>) {
        if extend {
            if self.ids.is_empty()
                && let Some(previous) = *primary
            {
                self.ids.push(previous);
            }
            if !self.ids.contains(&id) {
                self.ids.push(id);
            }
        } else {
            self.ids.clear();
        }
        *primary = Some(id);
    }

    pub(super) fn observe(
        &mut self,
        ui: &egui::Ui,
        timeline: &Timeline,
        view: Viewport,
        rows: Rows,
        primary: &mut Option<u64>,
        ignored: Option<Rect>,
        enabled: bool,
    ) -> bool {
        self.preview = None;
        if !enabled || egui::Popup::is_any_open(ui.ctx()) {
            self.press = None;
            return false;
        }
        let (focused, events) = ui.input(|input| (input.focused, input.events.clone()));
        if !focused {
            self.press = None;
            return false;
        }
        let area = view.rect.intersect(rows.area).intersect(ui.clip_rect());
        let threshold = ui
            .ctx()
            .options(|options| options.input_options.max_click_dist);
        let threshold = threshold * threshold;
        let mut finished = false;
        for event in events {
            match event {
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers,
                } => {
                    self.press = None;
                    if area.contains(pos)
                        && !ignored.is_some_and(|rect| rect.contains(pos))
                        && ui
                            .ctx()
                            .layer_id_at(pos)
                            .is_none_or(|layer| layer == ui.layer_id())
                        && !clip_hit(timeline, view, rows, pos)
                    {
                        self.press = Some(MarqueePress {
                            origin: pos,
                            pointer: pos,
                            base: if modifiers.shift {
                                if self.ids.is_empty() {
                                    primary.iter().copied().collect()
                                } else {
                                    self.ids.clone()
                                }
                            } else {
                                Vec::new()
                            },
                            extend: modifiers.shift,
                            moved: false,
                        });
                    }
                }
                Event::PointerMoved(pos) => {
                    if let Some(press) = &mut self.press {
                        press.update(pos, threshold);
                    }
                }
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: false,
                    ..
                } => {
                    if let Some(mut press) = self.press.take() {
                        press.update(pos, threshold);
                        if press.moved {
                            self.ids = covered(timeline, view, rows, area, &press).ids;
                            if primary.is_none_or(|id| !self.ids.contains(&id)) {
                                *primary = self.ids.first().copied();
                            }
                            finished = true;
                        } else if !press.extend {
                            self.ids.clear();
                            *primary = None;
                        }
                    }
                }
                Event::PointerGone
                | Event::Key {
                    key: Key::Escape,
                    pressed: true,
                    ..
                } => {
                    self.press = None;
                }
                _ => {}
            }
        }
        if let Some(press) = self.press.as_ref().filter(|press| press.moved) {
            self.preview = Some(covered(timeline, view, rows, area, press));
        }
        finished || self.preview.is_some()
    }

    pub(super) fn paint(&self, painter: &Painter, colors: &Palette) {
        if let Some(preview) = &self.preview {
            let [r, g, b, _] = colors.accent.to_array();
            painter.rect_filled(
                preview.rect,
                0,
                Color32::from_rgba_unmultiplied(r, g, b, 28),
            );
            painter.rect_stroke(
                preview.rect,
                0,
                Stroke::new(1.0_f32, colors.accent),
                StrokeKind::Inside,
            );
        }
    }

    pub(super) fn move_delta(
        &self,
        timeline: &Timeline,
        delta: i64,
        playhead: u64,
        tolerance: u64,
    ) -> i64 {
        let mut start = u64::MAX;
        let mut end = 0;
        for clip in timeline
            .all_clips()
            .filter(|clip| self.ids.contains(&clip.id))
        {
            start = start.min(clip.start);
            end = end.max(clip.end());
        }
        let mut correction = 0;
        if tolerance > 0 && delta != 0 {
            let mut distance = tolerance.saturating_add(1);
            let other_edges = timeline
                .all_clips()
                .filter(|clip| !self.ids.contains(&clip.id))
                .flat_map(|clip| [clip.start, clip.end()]);
            for edge in [0, playhead].into_iter().chain(other_edges) {
                for boundary in [start, end] {
                    let shifted = boundary.saturating_add_signed(delta);
                    let gap = edge.abs_diff(shifted);
                    if edge != boundary && gap < distance {
                        distance = gap;
                        correction = difference(edge, shifted);
                    }
                }
            }
        }
        delta
            .saturating_add(correction)
            .clamp(difference(0, start), difference(u64::MAX, end))
    }

    pub(super) fn paint_move(
        &self,
        painter: &Painter,
        timeline: &Timeline,
        view: Viewport,
        rows: Rows,
        delta: i64,
        colors: &Palette,
    ) -> bool {
        let mut valid = true;
        for (index, track) in timeline.tracks().iter().enumerate() {
            for clip in track
                .clips
                .iter()
                .filter(|clip| self.ids.contains(&clip.id))
            {
                let start = clip.start.saturating_add_signed(delta);
                let end = clip.end().saturating_add_signed(delta);
                let overlaps = track.clips.iter().any(|other| {
                    !self.ids.contains(&other.id) && other.start < end && start < other.end()
                });
                valid &= !overlaps;
                let bounds = view.clip_rect(rows.rect(view, index), start, end);
                let stroke = if overlaps {
                    colors.muted
                } else {
                    colors.accent
                };
                if bounds.intersects(painter.clip_rect()) {
                    painter.rect_stroke(
                        bounds,
                        0,
                        Stroke::new(1.25_f32, stroke),
                        StrokeKind::Inside,
                    );
                }
            }
        }
        valid
    }
}

impl MarqueePress {
    fn update(&mut self, pointer: Pos2, threshold: f32) {
        self.pointer = pointer;
        self.moved |= self.origin.distance_sq(pointer) > threshold;
    }
}

fn clip_hit(timeline: &Timeline, view: Viewport, rows: Rows, point: Pos2) -> bool {
    if !rows.area.contains(point) {
        return false;
    }
    rows.index_at(point.y)
        .and_then(|index| {
            let track = timeline.tracks().get(index)?;
            let clip = track.clip_at(view.time_at(point.x))?;
            Some(
                view.clip_rect(rows.rect(view, index), clip.start, clip.end())
                    .contains(point),
            )
        })
        .unwrap_or(false)
}

fn covered(
    timeline: &Timeline,
    view: Viewport,
    rows: Rows,
    area: Rect,
    press: &MarqueePress,
) -> MarqueePreview {
    let rect = Rect::from_two_pos(press.origin, press.pointer)
        .expand(1.0)
        .intersect(area);
    let visible = rect.intersect(rows.area);
    let mut ids = press.base.clone();
    for (index, track) in timeline.tracks().iter().enumerate() {
        let row = rows.rect(view, index);
        if !row.intersects(visible) {
            continue;
        }
        for clip in &track.clips {
            let bounds = view.clip_rect(row, clip.start, clip.end());
            if bounds.intersect(visible).is_positive() && !ids.contains(&clip.id) {
                ids.push(clip.id);
            }
        }
    }
    MarqueePreview { rect, ids }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Modifiers, pos2, vec2};
    use takeseam::editor_timeline::{PRIMARY_AUDIO, PRIMARY_VIDEO};

    use super::super::TimelineView;

    #[test]
    fn marquee_in_the_insert_margin_cannot_select_a_scrolled_out_clip() {
        use takeseam::editor_timeline::TrackKind;

        let mut timeline = Timeline::default();
        let first = timeline.add_track(TrackKind::Video).unwrap();
        timeline
            .insert_media_on_track(first, "hidden.mp4".into(), 2_000_000, 0)
            .unwrap();
        let boundaries = Rows::measure(&timeline);
        let rows = Rows {
            area: Rect::from_min_size(pos2(10.0, 100.0), vec2(900.0, 125.0)),
            edge_band: 26.0,
            offset: 75.0,
            boundaries: &boundaries,
        };
        let view = Viewport {
            rect: Rect::from_min_size(pos2(80.0, 50.0), vec2(830.0, 210.0)),
            scale: 60.0,
            scroll: 0.0,
        };
        let press = MarqueePress {
            origin: pos2(100.0, 77.0),
            pointer: pos2(160.0, 95.0),
            base: Vec::new(),
            extend: false,
            moved: true,
        };
        assert!(
            covered(&timeline, view, rows, view.rect, &press)
                .ids
                .is_empty()
        );
    }

    #[test]
    fn box_selects_across_tracks_then_moves_and_deletes_as_a_group() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut timeline = Timeline::default();
        let video = timeline
            .insert_media_on_track(PRIMARY_VIDEO, "group.mp4".into(), 1_000_000, 2_000_000)
            .unwrap();
        let audio = timeline
            .insert_media_on_track(PRIMARY_AUDIO, "voice.wav".into(), 1_000_000, 3_000_000)
            .unwrap();
        let original = timeline.tracks().to_vec();
        let mut selected = None;
        let mut playhead = 0;
        let mut view = TimelineView {
            disable_snap: true,
            ..TimelineView::default()
        };
        let mut frame = |events| {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 490.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let input = view.draw(ui, &mut timeline, &mut playhead, &mut selected, false);
                    assert!(input.error.is_none(), "{:?}", input.error);
                },
            );
            (
                output,
                view.marquee.ids().to_vec(),
                timeline.clip(video).map(|clip| clip.start),
                timeline.clip(audio).map(|clip| clip.start),
            )
        };
        let (output, _, _, _) = frame(Vec::new());
        let tree = output.platform_output.accesskit_update.unwrap();
        let clip_rect = |name| {
            let bounds = tree
                .nodes
                .iter()
                .filter_map(|(_, node)| {
                    (node.label() == Some(name))
                        .then(|| node.bounds())
                        .flatten()
                })
                .max_by(|left, right| (left.x1 - left.x0).total_cmp(&(right.x1 - right.x0)))
                .unwrap();
            Rect::from_min_max(
                pos2(bounds.x0 as f32, bounds.y0 as f32),
                pos2(bounds.x1 as f32, bounds.y1 as f32),
            )
        };
        let video_rect = clip_rect("group.mp4");
        let audio_rect = clip_rect("voice.wav");
        let start = pos2(video_rect.left() - 28.0, video_rect.top() + 4.0);
        let end = pos2(audio_rect.right() + 16.0, audio_rect.center().y);
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let (_, group, _, _) = frame(vec![
            button(start, true),
            Event::PointerMoved(end),
            button(end, false),
        ]);
        assert_eq!(group, vec![video, audio]);
        let from = video_rect.center();
        let to = from + vec2(112.0, 0.0);
        frame(vec![Event::PointerMoved(from)]);
        frame(vec![button(from, true)]);
        let outside = pos2(to.x, 10.0);
        let (_, group, old_video, old_audio) =
            frame(vec![Event::PointerMoved(outside), button(outside, false)]);
        assert_eq!(group, vec![video, audio]);
        assert_eq!(old_video, Some(2_000_000));
        assert_eq!(old_audio, Some(3_000_000));

        frame(vec![Event::PointerMoved(from)]);
        frame(vec![button(from, true)]);
        let (_, group, new_video, new_audio) =
            frame(vec![Event::PointerMoved(to), button(to, false)]);
        assert_eq!(group, vec![video, audio]);
        assert!(new_video.unwrap().abs_diff(4_000_000) <= 2);
        assert!(new_audio.unwrap().abs_diff(5_000_000) <= 2);

        let (_, _, remaining_video, remaining_audio) = frame(vec![Event::Key {
            key: Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        assert_eq!((remaining_video, remaining_audio), (None, None));
        assert!(timeline.undo());
        assert!(timeline.undo());
        assert_eq!(timeline.tracks(), original);
    }
}
