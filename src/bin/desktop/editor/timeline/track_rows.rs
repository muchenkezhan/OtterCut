use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Painter, Rect, Sense, Stroke, StrokeKind, Ui, pos2,
    vec2,
};
use takeseam::editor_timeline::{Timeline, Track, TrackId, TrackKind};

use crate::ui_icons::{Icon, paint_icon_focus};
use crate::ui_theme::{self, Palette};

use super::canvas::{TRACK_HEIGHT, Viewport};
use super::edge_drop::TrackGap;
use super::gesture::Gesture;
use super::{ClipLane, CursorEffect, TimelineView, material_drop};

pub(super) const VERTICAL_SCROLL_WIDTH: f32 = 14.0;
const TEXT_TRACK_HEIGHT: f32 = 32.0;

#[derive(Clone, Copy)]
pub(super) struct Rows<'a> {
    pub(super) area: Rect,
    pub(super) edge_band: f32,
    pub(super) offset: f32,
    pub(super) boundaries: &'a [f32],
}

impl Rows<'_> {
    pub(super) fn measure(timeline: &Timeline) -> Vec<f32> {
        let mut boundaries = Vec::with_capacity(timeline.tracks().len() + 1);
        boundaries.push(0.0);
        for track in timeline.tracks() {
            let height = if track.kind.is_overlay() {
                TEXT_TRACK_HEIGHT
            } else {
                TRACK_HEIGHT
            };
            boundaries.push(boundaries.last().copied().unwrap_or_default() + height);
        }
        boundaries
    }

    pub(super) fn max_offset(self) -> f32 {
        let height = self.boundaries.last().copied().unwrap_or_default() + 2.0 * self.edge_band;
        (height - self.area.height()).max(0.0)
    }

    pub(super) fn boundary(self, index: usize) -> f32 {
        self.area.top() + self.edge_band - self.offset + self.boundaries[index]
    }

    pub(super) fn top_band(self) -> Rect {
        let first_row = self.boundary(0);
        Rect::from_min_max(
            pos2(self.area.left(), first_row - self.edge_band),
            pos2(self.area.right(), first_row),
        )
        .intersect(self.area)
    }

    pub(super) fn bottom_band(self) -> Rect {
        let last_row = self.boundary(self.boundaries.len() - 1);
        Rect::from_min_max(pos2(self.area.left(), last_row), self.area.max).intersect(self.area)
    }

    pub(super) fn index_at(self, y: f32) -> Option<usize> {
        let local = y - self.boundary(0);
        self.boundaries
            .partition_point(|&boundary| boundary <= local)
            .checked_sub(1)
            .filter(|&index| index + 1 < self.boundaries.len())
    }

    pub(super) fn at(self, timeline: &Timeline, y: f32) -> Option<&Track> {
        if !self.area.y_range().contains(y) {
            return None;
        }
        timeline.tracks().get(self.index_at(y)?)
    }

    pub(super) fn rect(self, view: Viewport, index: usize) -> Rect {
        Rect::from_min_max(
            pos2(view.rect.left(), self.boundary(index)),
            pos2(view.rect.right(), self.boundary(index + 1)),
        )
    }

    pub(super) fn reveal(&mut self, track: TrackId, timeline: &Timeline) {
        if let Some(index) = timeline.tracks().iter().position(|item| item.id == track) {
            let start = self.edge_band + self.boundaries[index];
            let end = self.edge_band + self.boundaries[index + 1];
            if start < self.offset {
                self.offset = start;
            } else if end > self.offset + self.area.height() {
                self.offset = end - self.area.height();
            }
        }
    }
}

pub(super) enum TrackAction {
    Select(TrackId),
    SetOutput(TrackId, bool),
}

pub(super) fn paint_header(
    ui: &Ui,
    painter: &Painter,
    area: Rect,
    number: usize,
    track: &Track,
    selected: bool,
    allow_input: bool,
) -> Option<TrackAction> {
    let colors = ui_theme::palette(ui.ctx());
    let output_rect =
        Rect::from_center_size(area.right_center() - vec2(18.0, 0.0), vec2(28.0, 28.0));
    let select_rect = Rect::from_min_max(area.min, pos2(output_rect.left() - 3.0, area.bottom()));
    let name = format!("第{number}条{}", track_title(track.kind));
    let (select, select_hovered) = if allow_input {
        let response = button(
            ui,
            painter,
            select_rect,
            ui.id().with((track.id, "select")),
            &format!("选中{name}"),
            selected,
            &colors,
        );
        (response.clicked(), response.hovered())
    } else {
        (false, false)
    };
    let icon = match track.kind {
        TrackKind::Text => Icon::Text,
        TrackKind::Caption => Icon::Captions,
        TrackKind::Video => Icon::Media,
        TrackKind::Audio => Icon::Audio,
    };
    icon.paint(
        painter,
        select_rect.center(),
        if selected || select_hovered {
            colors.text
        } else {
            colors.muted
        },
    );

    let (toggle, toggle_hovered) = if allow_input {
        let action = match (track.kind, track.enabled) {
            (TrackKind::Audio, true) => "静音",
            (TrackKind::Audio, false) => "取消静音",
            (_, true) => "隐藏",
            (_, false) => "显示",
        };
        let response = button(
            ui,
            painter,
            output_rect,
            ui.id().with((track.id, "output")),
            &format!("{action}{name}"),
            !track.enabled,
            &colors,
        );
        (response.clicked(), response.hovered())
    } else {
        (false, false)
    };
    paint_output_icon(
        painter,
        output_rect,
        track,
        if !track.enabled || toggle_hovered {
            colors.text
        } else {
            colors.muted
        },
    );
    if toggle {
        Some(TrackAction::SetOutput(track.id, !track.enabled))
    } else if select {
        Some(TrackAction::Select(track.id))
    } else {
        None
    }
}

fn button(
    ui: &Ui,
    painter: &Painter,
    area: Rect,
    id: egui::Id,
    label: &str,
    selected: bool,
    colors: &Palette,
) -> egui::Response {
    let rect = area.intersect(painter.clip_rect());
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, label)
    });
    if response.clicked_by(egui::PointerButton::Primary) {
        response.surrender_focus();
    }
    if response.has_focus() {
        paint_icon_focus(painter, area.center() + vec2(0.0, 11.0), colors.text);
    }
    response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(label)
}

fn track_title(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Text => "文本轨",
        TrackKind::Caption => "字幕轨",
        TrackKind::Video => "视频轨",
        TrackKind::Audio => "音频轨",
    }
}

fn paint_output_icon(painter: &Painter, rect: Rect, track: &Track, color: Color32) {
    let center = rect.center();
    let stroke = Stroke::new(1.25_f32, color);
    let outline: &[(f32, f32)] = match track.kind {
        TrackKind::Text | TrackKind::Caption | TrackKind::Video => &[
            (-7.0, 0.0),
            (-3.5, -3.3),
            (0.0, -4.3),
            (3.5, -3.3),
            (7.0, 0.0),
            (3.5, 3.3),
            (0.0, 4.3),
            (-3.5, 3.3),
        ],
        TrackKind::Audio => &[
            (-7.0, -2.1),
            (-4.4, -2.1),
            (-0.6, -5.2),
            (-0.6, 5.2),
            (-4.4, 2.1),
            (-7.0, 2.1),
        ],
    };
    let points = outline.iter().map(|&(x, y)| center + vec2(x, y)).collect();
    painter.add(egui::Shape::closed_line(points, stroke));
    if track.enabled {
        match track.kind {
            TrackKind::Text | TrackKind::Caption | TrackKind::Video => {
                painter.circle_filled(center, 1.7, color);
            }
            TrackKind::Audio => {
                painter.add(egui::Shape::line(
                    vec![
                        center + vec2(2.5, -4.0),
                        center + vec2(4.8, -2.0),
                        center + vec2(4.8, 2.0),
                        center + vec2(2.5, 4.0),
                    ],
                    stroke,
                ));
            }
        }
    } else {
        painter.line_segment([center + vec2(-7.0, 5.8), center + vec2(7.0, -5.8)], stroke);
    }
}

impl TimelineView {
    pub(super) fn draw_rows(
        &mut self,
        ui: &Ui,
        painter: &egui::Painter,
        timeline: &Timeline,
        selected: &mut Option<u64>,
        view: Viewport,
        rows: Rows,
        playhead: &mut u64,
        cursor_effect: &mut CursorEffect,
    ) -> (Option<TrackAction>, Option<(u64, u64)>) {
        let colors = ui_theme::palette(ui.ctx());
        let all = painter.with_clip_rect(rows.area);
        let media = all.with_clip_rect(view.rect);
        let mut video_number = timeline
            .tracks()
            .iter()
            .filter(|track| track.kind == TrackKind::Video)
            .count();
        let mut numbers = [0; 4];
        let mut action = None;
        let mut cut = None;
        let active = self.active_track(timeline);
        let empty = timeline.is_empty();
        let interact = !material_drop::is_dragging(ui.ctx()) && self.drag.is_none();
        let background = if ui.ctx().theme() == egui::Theme::Light {
            colors.surface
        } else {
            colors.track
        };
        if timeline.is_empty() {
            let empty = rows.area.shrink2(vec2(18.0, 18.0));
            all.rect_stroke(
                empty,
                8,
                Stroke::new(1.0, colors.border),
                StrokeKind::Inside,
            );
            all.text(
                empty.center(),
                Align2::CENTER_CENTER,
                "把素材拖进时间线，开始剪辑",
                FontId::proportional(13.0),
                colors.muted,
            );
        }
        let divider = Stroke::new(1.0_f32, background.lerp_to_gamma(colors.border, 0.75));
        for (index, track) in timeline.tracks().iter().enumerate() {
            let number = match track.kind {
                TrackKind::Video => {
                    let number = video_number;
                    video_number -= 1;
                    number
                }
                kind => {
                    let number = &mut numbers[usize::from(kind.stack_order())];
                    *number += 1;
                    *number
                }
            };
            let rect = rows.rect(view, index);
            if !rect.intersect(rows.area).is_positive() {
                continue;
            }
            all.hline(rows.area.x_range(), rect.bottom(), divider);
            let header = Rect::from_min_max(
                pos2(rows.area.left(), rect.top()),
                pos2(rect.left(), rect.bottom()),
            );
            if let Some(input) = paint_header(
                ui,
                &all,
                header,
                number,
                track,
                active == track.id,
                interact,
            ) {
                action = Some(input);
            }
            for clip in &track.clips {
                let mut clip_rect = rect;
                if track.kind == TrackKind::Video
                    && let Some(markers) = self.cursor_track_for(clip.id).map(<[u64]>::to_vec)
                {
                    self.draw_cursor_track_for_clip(
                        ui,
                        &media,
                        view,
                        rect,
                        clip,
                        &markers,
                        timeline.duration(),
                        playhead,
                        selected,
                        cursor_effect,
                        &colors,
                    );
                    clip_rect.min.y += super::CURSOR_TRACK_HEIGHT;
                }
                let lane = ClipLane {
                    view,
                    rect: clip_rect,
                    id: track.id,
                    kind: track.kind,
                    enabled: track.enabled,
                };
                if let Some(at) = self.draw_clip(ui, &media, lane, clip, selected, &colors) {
                    cut = Some((clip.id, at));
                }
            }
            if empty && active == track.id && interact {
                let hint = match track.kind {
                    TrackKind::Text => "在文本页添加文字",
                    TrackKind::Caption => "在字幕页添加或导入字幕",
                    TrackKind::Video | TrackKind::Audio => "拖入素材开始剪辑",
                };
                media.text(
                    rect.left_center() + vec2(12.0, 0.0),
                    Align2::LEFT_CENTER,
                    hint,
                    FontId::proportional(12.0),
                    colors.muted,
                );
            }
        }
        (action, cut)
    }

    pub(super) fn pan_tracks_at_edge(
        &mut self,
        ui: &Ui,
        rows: Rows,
        timeline: &Timeline,
        max: f32,
    ) {
        let area = rows.area;
        if max <= 0.0 || area.height() < 40.0 {
            return;
        }
        let (pointer, held, dt) = ui.input(|input| {
            (
                input.pointer.interact_pos(),
                input.pointer.primary_down(),
                input.stable_dt,
            )
        });
        let Some(pointer) = pointer.filter(|p| area.contains(*p) && held) else {
            return;
        };
        let inserts_tracks = material_drop::is_dragging(ui.ctx())
            || self
                .drag
                .is_some_and(|drag| matches!(drag.kind, Gesture::Move | Gesture::Reorder));
        if inserts_tracks && TrackGap::at(rows, timeline, pointer).is_some() {
            return;
        }
        let up = ((area.top() + 20.0 - pointer.y) / 20.0).clamp(0.0, 1.0);
        let down = ((pointer.y - area.bottom() + 20.0) / 20.0).clamp(0.0, 1.0);
        let next = (self.vertical_scroll + (down - up) * dt.min(0.05) * 250.0).clamp(0.0, max);
        if next != self.vertical_scroll {
            self.vertical_scroll = next;
            ui.ctx().request_repaint();
        }
    }

    pub(super) fn draw_track_scrollbar(
        &mut self,
        ui: &Ui,
        painter: &egui::Painter,
        body: Rect,
        max: f32,
        dragging: bool,
        colors: &Palette,
    ) {
        if max <= 0.0 || body.height() < 24.0 {
            self.vertical_scroll_drag = None;
            return;
        }
        let rail = Rect::from_min_max(
            pos2(body.right() + 5.0, body.top() + 4.0),
            pos2(
                body.right() + VERTICAL_SCROLL_WIDTH - 5.0,
                body.bottom() - 4.0,
            ),
        );
        let thumb_height = (rail.height() * body.height() / (body.height() + max))
            .max(22.0)
            .min(rail.height());
        let range = rail.height() - thumb_height;
        if range <= 0.0 {
            return;
        }
        let top = rail.top() + range * self.vertical_scroll / max;
        let thumb = Rect::from_min_size(pos2(rail.left(), top), vec2(rail.width(), thumb_height));
        if !dragging {
            let response = ui
                .interact(
                    rail.expand2(vec2(4.0, 0.0)),
                    ui.id().with("vertical-track-scroll"),
                    Sense::click_and_drag(),
                )
                .on_hover_text("滚动浏览轨道");
            if response.drag_started()
                && let Some(pointer) = ui.input(|input| input.pointer.press_origin())
            {
                self.vertical_scroll_drag = Some(if thumb.contains(pointer) {
                    pointer.y - thumb.top()
                } else {
                    thumb_height * 0.5
                });
            }
            if (response.clicked() || response.dragged() || response.drag_stopped())
                && let Some(pointer) = response.interact_pointer_pos()
            {
                let offset = self.vertical_scroll_drag.unwrap_or(thumb_height * 0.5);
                self.vertical_scroll =
                    ((pointer.y - rail.top() - offset) / range).clamp(0.0, 1.0) * max;
            }
            if !ui.input(|input| input.pointer.primary_down()) {
                self.vertical_scroll_drag = None;
            }
        }
        painter.rect_filled(rail, 3, colors.control);
        painter.rect_filled(
            thumb,
            2,
            if self.vertical_scroll_drag.is_some() {
                colors.accent
            } else {
                colors.control.lerp_to_gamma(colors.muted, 0.56)
            },
        );
    }
}
