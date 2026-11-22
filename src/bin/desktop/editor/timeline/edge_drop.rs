use eframe::egui::{self, Painter, Pos2, Sense, Stroke, Ui, pos2};
use takeseam::editor_timeline::{Timeline, TrackId, TrackKind};

use crate::ui_theme;

use super::canvas::Viewport;
use super::track_rows::Rows;

pub(super) const BAND_HEIGHT: f32 = 26.0;
// Each clip is inset by four points from its row edges.
const ROW_SEAM_HALF_HEIGHT: f32 = 4.0;

#[derive(Clone, Copy)]
pub(super) struct TrackGap {
    pub(super) before: Option<TrackId>,
    pub(super) y: f32,
}

impl TrackGap {
    pub(super) fn at(rows: Rows, timeline: &Timeline, pointer: Pos2) -> Option<Self> {
        if !rows.area.contains(pointer) {
            return None;
        }
        let top = rows.top_band();
        if top.is_positive() && pointer.y < top.bottom() {
            return Some(Self {
                before: timeline.tracks().first().map(|track| track.id),
                y: top.y_range().clamp(rows.boundary(0) - 2.0),
            });
        }
        let tracks = timeline.tracks();
        let bottom = rows.bottom_band();
        if bottom.is_positive() && pointer.y >= bottom.top() {
            return Some(Self {
                before: None,
                y: bottom.y_range().clamp(rows.boundary(tracks.len()) + 2.0),
            });
        }
        let index = rows.index_at(pointer.y)?;
        [index, index + 1].into_iter().find_map(|index| {
            let below = tracks.get(index)?;
            let y = rows.boundary(index);
            (rows.area.y_range().contains(y) && (pointer.y - y).abs() <= ROW_SEAM_HALF_HEIGHT)
                .then_some(Self {
                    before: Some(below.id),
                    y,
                })
        })
    }

    pub(super) fn allows(self, kind: TrackKind) -> bool {
        matches!(kind, TrackKind::Video | TrackKind::Audio)
    }

    pub(super) fn paint_target(self, painter: &Painter, view: Viewport, at: u64, rows: Rows) {
        let colors = ui_theme::palette(painter.ctx());
        let time_painter = painter.with_clip_rect(view.rect.intersect(rows.area));
        let y = self.y;
        time_painter.hline(view.rect.x_range(), y, Stroke::new(1.0_f32, colors.accent));
        let x = view.rect.x_range().clamp(view.x(at));
        time_painter.circle_filled(pos2(x, y), 2.5, colors.accent);
    }
}

pub(super) fn describe_edges(ui: &Ui, rows: Rows) {
    if rows.boundaries.len() <= 1 {
        return;
    }
    for (rect, label) in [
        (rows.top_band(), "拖入素材建立视频轨"),
        (rows.bottom_band(), "拖入素材建立音频轨"),
    ] {
        let rect = rect.intersect(ui.clip_rect());
        if rect.is_positive() {
            ui.interact(
                rect,
                ui.id().with(("track-drop-band", label)),
                Sense::hover(),
            )
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label));
        }
    }
}
