use eframe::egui::{Pos2, Rect, Vec2};

const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 8.0;

/// View-only zoom and pan, relative to the fitted picture.
pub(super) struct PreviewViewport {
    zoom: f32,
    // Position the picture by a fraction of its zoomed dimensions so the view
    // survives changes in preview quality and in the size of the panel.
    pan_fraction: Vec2,
}

impl Default for PreviewViewport {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan_fraction: Vec2::ZERO,
        }
    }
}

impl PreviewViewport {
    pub(super) fn frame(&self, canvas: Rect, pixels: Vec2) -> Rect {
        let center = canvas.center();
        let Some(size) = media_size(canvas, pixels, self.zoom) else {
            let center = if center.is_finite() {
                center
            } else {
                Pos2::ZERO
            };
            return Rect::from_center_size(center, Vec2::ZERO);
        };
        let offset = clamped_offset(canvas.size(), size, self.pan_fraction * size);
        Rect::from_center_size(center + offset, size)
    }

    pub(super) fn zoom_at(&mut self, canvas: Rect, pixels: Vec2, target: Pos2, factor: f32) {
        if !factor.is_finite() || factor <= 0.0 || !target.is_finite() {
            return;
        }
        let mut zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        if (zoom - 1.0).abs() < 0.0001 {
            zoom = 1.0;
        }
        let Some(size) = media_size(canvas, pixels, zoom) else {
            return;
        };
        if zoom != self.zoom {
            let center = self.frame(canvas, pixels).center();
            let offset = (target - canvas.center()) + (center - target) * (zoom / self.zoom);
            self.pan_fraction = clamped_offset(canvas.size(), size, offset) / size;
            self.zoom = zoom;
        }
    }

    pub(super) fn pan_by(&mut self, canvas: Rect, pixels: Vec2, delta: Vec2) {
        if !delta.is_finite() {
            return;
        }
        if let Some(size) = media_size(canvas, pixels, self.zoom) {
            let current = clamped_offset(canvas.size(), size, self.pan_fraction * size);
            self.pan_fraction = clamped_offset(canvas.size(), size, current + delta) / size;
        }
    }

    pub(super) fn percent(&self) -> u16 {
        (self.zoom * 100.0).round() as u16
    }

    pub(super) fn is_fitted(&self) -> bool {
        self.zoom == 1.0
    }

    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }
}

fn media_size(canvas: Rect, pixels: Vec2, zoom: f32) -> Option<Vec2> {
    if !canvas.is_positive()
        || !canvas.is_finite()
        || !pixels.is_finite()
        || pixels.x <= 0.0
        || pixels.y <= 0.0
    {
        return None;
    }
    let fit_scale = (canvas.width() / pixels.x).min(canvas.height() / pixels.y);
    let size = (pixels * fit_scale) * zoom;
    (size.is_finite() && size.x > 0.0 && size.y > 0.0).then_some(size)
}

fn clamped_offset(canvas_size: Vec2, media_size: Vec2, offset: Vec2) -> Vec2 {
    let limit = ((media_size - canvas_size) * 0.5).max(Vec2::ZERO);
    offset.clamp(-limit, limit)
}

#[cfg(test)]
mod tests {
    use eframe::egui::{pos2, vec2};

    use super::*;

    #[test]
    fn zoom_preserves_the_picture_point_under_the_cursor() {
        let canvas = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0));
        let pixels = vec2(800.0, 800.0);
        let cursor = pos2(25.0, 75.0);
        let mut viewport = PreviewViewport::default();
        viewport.zoom_at(canvas, pixels, cursor, 2.0);

        let frame = viewport.frame(canvas, pixels);
        assert_eq!(frame.size(), vec2(200.0, 200.0));
        assert_eq!((cursor - frame.min) / frame.size(), vec2(0.25, 0.75));
        assert_eq!(viewport.percent(), 200);
        assert!(!viewport.is_fitted());
    }

    #[test]
    fn pan_clamps_overflow_and_centers_axes_smaller_than_the_canvas() {
        let canvas = Rect::from_min_size(pos2(4.0, 4.0), vec2(160.0, 160.0));
        let pixels = vec2(640.0, 160.0);
        let mut viewport = PreviewViewport::default();
        viewport.zoom_at(canvas, pixels, canvas.center(), 2.0);
        viewport.pan_by(canvas, pixels, vec2(1_000.0, 1_000.0));
        let frame = viewport.frame(canvas, pixels);
        assert_eq!(frame.left(), canvas.left());
        assert_eq!(frame.center().y, canvas.center().y);

        viewport.pan_by(canvas, pixels, vec2(-1_000.0, -1_000.0));
        assert_eq!(viewport.frame(canvas, pixels).right(), canvas.right());
    }

    #[test]
    fn normalized_pan_scales_with_the_panel_and_survives_quality_changes() {
        let canvas = Rect::from_center_size(Pos2::ZERO, vec2(200.0, 100.0));
        let pixels = vec2(640.0, 320.0);
        let mut viewport = PreviewViewport::default();
        viewport.zoom_at(canvas, pixels, canvas.center(), 2.0);
        viewport.pan_by(canvas, pixels, vec2(40.0, -25.0));
        assert_eq!(
            viewport.frame(canvas, pixels * 0.5),
            viewport.frame(canvas, pixels)
        );

        let bigger = Rect::from_center_size(Pos2::ZERO, canvas.size() * 2.0);
        assert_eq!(viewport.frame(bigger, pixels).center(), pos2(80.0, -50.0));
    }

    #[test]
    fn zoom_has_limits_and_can_reset_after_invalid_geometry() {
        let canvas = Rect::from_center_size(Pos2::ZERO, vec2(160.0, 100.0));
        let pixels = vec2(640.0, 400.0);
        let mut viewport = PreviewViewport::default();
        viewport.zoom_at(canvas, pixels, Pos2::ZERO, 100.0);
        assert_eq!(viewport.percent(), 800);
        viewport.zoom_at(canvas, pixels, Pos2::ZERO, 0.001);
        assert_eq!(viewport.percent(), 25);
        assert_eq!(viewport.frame(canvas, pixels).center(), canvas.center());

        viewport.zoom_at(canvas, Vec2::ZERO, Pos2::ZERO, 2.0);
        viewport.pan_by(Rect::NOTHING, pixels, vec2(70.0, 70.0));
        assert_eq!(viewport.percent(), 25);
        assert_eq!(viewport.frame(canvas, Vec2::ZERO).size(), Vec2::ZERO);

        viewport.reset();
        assert!(viewport.is_fitted());
        assert_eq!(viewport.frame(canvas, pixels), canvas);
    }
}
