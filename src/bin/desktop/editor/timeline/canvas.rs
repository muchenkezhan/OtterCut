use eframe::egui::{self, Align2, Color32, FontId, Rect, Stroke, StrokeKind, pos2, vec2};

use crate::ui_theme::Palette;

use super::gesture::Draft;

pub(super) const SECOND: f64 = 1_000_000.0;
pub(super) const RULER_HEIGHT: f32 = 24.0;
pub(super) const TRACK_HEIGHT: f32 = 60.0;

#[derive(Clone, Copy)]
pub(super) struct Viewport {
    pub(super) rect: Rect,
    pub(super) scale: f64,
    pub(super) scroll: f64,
}

impl Viewport {
    pub(super) fn ruler(self) -> Rect {
        Rect::from_min_size(self.rect.min, vec2(self.rect.width(), RULER_HEIGHT))
    }

    pub(super) fn x(self, time: u64) -> f32 {
        let x = (time as f64 - self.scroll) * self.scale / SECOND;
        self.rect.left() + x.clamp(-16_384.0, f64::from(self.rect.width()) + 16_384.0) as f32
    }

    pub(super) fn time_at(self, x: f32) -> u64 {
        (self.scroll + f64::from(x - self.rect.left()) * SECOND / self.scale).round() as u64
    }

    pub(super) fn clip_rect(self, row: Rect, start: u64, end: u64) -> Rect {
        let left = self.x(start);
        Rect::from_min_max(
            pos2(left, row.top() + 4.0),
            pos2(self.x(end).max(left + 1.0), row.bottom() - 4.0),
        )
    }
}

pub(super) fn paint_ruler(
    painter: &egui::Painter,
    view: Viewport,
    rows: Rect,
    background: Color32,
    colors: &Palette,
) {
    let ruler = view.ruler();
    let major = [
        100_000,
        200_000,
        500_000,
        1_000_000,
        2_000_000,
        5_000_000,
        10_000_000,
        15_000_000,
        30_000_000,
        60_000_000,
        120_000_000,
        300_000_000,
        600_000_000,
        1_800_000_000,
        3_600_000_000,
    ]
    .into_iter()
    .find(|step| *step as f64 * view.scale / SECOND >= 64.0)
    .unwrap_or(3_600_000_000);
    let step = major / 5;
    let first = view.time_at(ruler.left()) / step;
    let last = view.time_at(ruler.right()) / step;
    let grid = Stroke::new(1.0_f32, background.lerp_to_gamma(colors.border, 0.42));
    let grid_painter = painter.with_clip_rect(rows);
    for tick in first..=last {
        let time = tick * step;
        let x = painter.round_to_pixel_center(view.x(time));
        let labeled = tick % 5 == 0;
        let (height, color) = if labeled {
            (7.0, colors.muted)
        } else {
            (3.0, colors.border)
        };
        painter.vline(
            x,
            ruler.bottom() - height..=ruler.bottom(),
            Stroke::new(1.0_f32, color),
        );
        if labeled {
            grid_painter.vline(x, rows.y_range(), grid);
            let mut label = timecode(time);
            if major >= SECOND as u64 {
                label.truncate(label.len() - 4);
            }
            painter.text(
                pos2(x + 4.0, ruler.top() + 3.0),
                Align2::LEFT_TOP,
                label,
                FontId::monospace(10.0),
                colors.muted,
            );
        }
    }
}

pub(super) fn paint_playhead(
    painter: &egui::Painter,
    view: Viewport,
    playhead: u64,
    colors: &Palette,
) {
    let x = view.x(playhead);
    if !view.rect.x_range().contains(x) {
        return;
    }
    let x = painter.round_to_pixel_center(x);
    let y = view.rect.top();
    painter.with_clip_rect(view.rect).vline(
        x,
        y + 9.0..=view.rect.bottom(),
        Stroke::new(1.0_f32, colors.playhead),
    );
    // The head can overhang the ruler edge while the cut still lands on its real coordinate.
    painter
        .with_clip_rect(view.ruler().expand2(vec2(4.0, 0.0)))
        .add(egui::Shape::convex_polygon(
            vec![
                pos2(x - 4.0, y + 1.0),
                pos2(x + 4.0, y + 1.0),
                pos2(x + 4.0, y + 6.0),
                pos2(x, y + 10.0),
                pos2(x - 4.0, y + 6.0),
            ],
            colors.playhead,
            Stroke::NONE,
        ));
}

pub(super) fn paint_drop_guide(
    painter: &egui::Painter,
    view: Viewport,
    at: u64,
    bottom: f32,
    colors: &Palette,
) {
    let x = view.rect.x_range().clamp(view.x(at));
    let ruler = view.ruler();
    painter.vline(
        x,
        ruler.bottom()..=bottom,
        Stroke::new(2.0_f32, colors.accent),
    );
    let size = vec2(74.0, 18.0);
    let left = (x - size.x / 2.0).clamp(ruler.left(), (ruler.right() - size.x).max(ruler.left()));
    let label = Rect::from_min_size(pos2(left, ruler.top() + 3.0), size);
    painter.rect_filled(label, 4, colors.accent);
    painter.text(
        label.center(),
        Align2::CENTER_CENTER,
        timecode(at),
        FontId::monospace(10.0),
        colors.on_accent,
    );
}

pub(super) fn timecode(time: u64) -> String {
    let millis = time / 1_000;
    let seconds = millis / 1_000;
    format!(
        "{:02}:{:02}.{:03}",
        seconds / 60,
        seconds % 60,
        millis % 1_000
    )
}

pub(super) fn paint_draft(
    draft: &Draft,
    painter: &egui::Painter,
    view: Viewport,
    row: Rect,
    colors: &Palette,
) {
    if let Some((start, end)) = draft.ghost {
        let rect = view.clip_rect(row, start, end);
        painter.rect_filled(rect, 0, colors.accent.gamma_multiply(0.18));
        painter.rect_stroke(
            rect,
            0,
            Stroke::new(1.0_f32, colors.accent),
            StrokeKind::Inside,
        );
    }
    if let Some(at) = draft.guide {
        painter.vline(
            view.x(at),
            row.top()..=row.bottom(),
            Stroke::new(2.0_f32, colors.accent),
        );
    }
    if let Some(source_in) = draft.source_in {
        painter.text(
            row.left_top() + vec2(8.0, 8.0),
            Align2::LEFT_TOP,
            format!("素材入点 {}", timecode(source_in)),
            FontId::proportional(11.0),
            colors.accent,
        );
    }
}
