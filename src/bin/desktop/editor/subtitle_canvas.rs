use std::sync::Arc;

use eframe::egui::{
    self, Align, Color32, CursorIcon, Event, FontId, Galley, Key, PointerButton, Pos2, Rect,
    Stroke, StrokeKind, Ui, pos2, vec2,
};
use takeseam::editor_timeline::{OverlayText, TextPaint, TextPlacement, TextPosition, TextStyle};

use crate::ui_theme::upper_palette;

const ANCHOR_MARGIN: f32 = 56.0 / 1080.0;
const WRAP_WIDTH: f32 = 0.875;
const HANDLE_SIDE: f32 = 6.0;
const HANDLE_HIT: f32 = 12.0;
const DRAG_THRESHOLD: f32 = 2.0;

#[derive(Default)]
pub(super) struct SubtitleCanvas {
    gesture: Option<Gesture>,
}

pub(super) enum CanvasTransform {
    Move(TextPosition),
    Resize(TextStyle),
}

#[derive(Clone, Copy)]
enum Handle {
    Move,
    Corner { corner: Pos2, fixed: Pos2 },
}

struct Gesture {
    id: u64,
    canvas: Rect,
    press: Pos2,
    original: TextStyle,
    draft: TextStyle,
    center: Pos2,
    handle: Handle,
}

impl SubtitleCanvas {
    pub(super) fn excluded_overlay(&self) -> Option<u64> {
        self.gesture
            .as_ref()
            .filter(|gesture| gesture.draft != gesture.original)
            .map(|gesture| gesture.id)
    }

    pub(super) fn cancel(&mut self) {
        self.gesture = None;
    }

    pub(super) fn show(
        &mut self,
        ui: &mut Ui,
        canvas: Rect,
        id: u64,
        text: &OverlayText,
        frame_excludes_text: bool,
    ) -> Option<CanvasTransform> {
        if canvas.width() < 1.0 || canvas.height() < 1.0 {
            self.cancel();
            return None;
        }
        if self.gesture.as_ref().is_some_and(|gesture| {
            gesture.id != id || gesture.original != text.style || gesture.canvas != canvas
        }) {
            self.cancel();
        }
        let painter = ui.painter().with_clip_rect(canvas);
        let original = TextBounds::new(&painter, canvas, &text.content, text.style);
        let can_start = !egui::Popup::is_any_open(ui.ctx());
        let (focused, events) = ui.input(|input| (input.focused, input.raw.events.clone()));
        if !focused {
            self.cancel();
        }
        let mut committed = None;
        for event in events {
            match event {
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: true,
                    ..
                } if focused
                    && can_start
                    && ui.clip_rect().contains(pos)
                    && canvas.contains(pos)
                    && ui
                        .ctx()
                        .layer_id_at(pos)
                        .is_none_or(|layer| layer.order <= ui.layer_id().order) =>
                {
                    if let Some(handle) = hit(original.selection, pos) {
                        self.gesture = Some(Gesture {
                            id,
                            canvas,
                            press: pos,
                            original: text.style,
                            draft: text.style,
                            center: original.content.center(),
                            handle,
                        });
                    }
                }
                Event::PointerMoved(pos) => {
                    if let Some(gesture) = &mut self.gesture {
                        gesture.update(&painter, &text.content, pos);
                    }
                }
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: false,
                    ..
                } => {
                    if let Some(mut gesture) = self.gesture.take() {
                        gesture.update(&painter, &text.content, pos);
                        if gesture.draft != gesture.original {
                            committed = Some((gesture.handle, gesture.draft));
                        }
                    }
                }
                Event::PointerGone
                | Event::Key {
                    key: Key::Escape,
                    pressed: true,
                    ..
                } => self.cancel(),
                _ => {}
            }
        }

        let style = self
            .gesture
            .as_ref()
            .map(|gesture| gesture.draft)
            .or(committed.map(|(_, style)| style))
            .unwrap_or(text.style);
        let bounds = if style == text.style {
            original
        } else {
            TextBounds::new(&painter, canvas, &text.content, style)
        };
        if frame_excludes_text {
            bounds.paint_text(
                &painter,
                style.appearance.paint(),
                style.bold,
                canvas.height() / 1080.0,
            );
        }
        if let Some(handle) = self
            .gesture
            .as_ref()
            .map(|gesture| gesture.handle)
            .or_else(|| {
                ui.rect_contains_pointer(canvas)
                    .then(|| ui.ctx().pointer_hover_pos())
                    .flatten()
                    .and_then(|pointer| hit(bounds.selection, pointer))
            })
        {
            ui.ctx().set_cursor_icon(handle.cursor());
        }
        bounds.paint_selection(&painter, upper_palette(ui.ctx()).accent);
        committed.and_then(|(handle, style)| match handle {
            Handle::Move => style.position.map(CanvasTransform::Move),
            Handle::Corner { .. } => Some(CanvasTransform::Resize(style)),
        })
    }
}

impl Handle {
    fn cursor(self) -> CursorIcon {
        match self {
            Self::Move => CursorIcon::Move,
            Self::Corner { corner, fixed } if (corner.x - fixed.x) * (corner.y - fixed.y) > 0.0 => {
                CursorIcon::ResizeNwSe
            }
            Self::Corner { .. } => CursorIcon::ResizeNeSw,
        }
    }
}

impl Gesture {
    fn update(&mut self, painter: &egui::Painter, text: &str, pointer: Pos2) {
        let delta = pointer - self.press;
        if delta.length() < DRAG_THRESHOLD {
            self.draft = self.original;
            return;
        }
        match self.handle {
            Handle::Move => {
                let bounds = TextBounds::new(painter, self.canvas, text, self.original);
                self.draft.position = Some(position_in(
                    self.canvas,
                    self.center + delta,
                    bounds.content.size(),
                ));
            }
            Handle::Corner { corner, fixed } => {
                let diagonal = corner - fixed;
                let ratio = 1.0 + diagonal.dot(delta) / diagonal.length_sq().max(1.0);
                let size = (f32::from(self.original.size) * ratio).round().clamp(
                    f32::from(TextStyle::MIN_SIZE),
                    f32::from(TextStyle::MAX_SIZE),
                ) as u16;
                if size == self.original.size {
                    self.draft = self.original;
                    return;
                }
                self.draft.size = size;
                let ratio = f32::from(size) / f32::from(self.original.size);
                let center = fixed + (self.center - fixed) * ratio;
                let bounds = TextBounds::new(painter, self.canvas, text, self.draft);
                self.draft.position = Some(position_in(self.canvas, center, bounds.content.size()));
            }
        }
    }
}

fn position_in(canvas: Rect, center: Pos2, size: egui::Vec2) -> TextPosition {
    let clamp = |start: f32, extent: f32, at: f32, half: f32| {
        let margin = (half / extent).min(0.5);
        (((at - start) / extent).clamp(margin, 1.0 - margin) * f32::from(TextPosition::MAX)).round()
            as u16
    };
    TextPosition {
        x: clamp(canvas.left(), canvas.width(), center.x, size.x * 0.5),
        y: clamp(canvas.top(), canvas.height(), center.y, size.y * 0.5),
    }
}

fn hit(rect: Rect, pointer: Pos2) -> Option<Handle> {
    for (corner, fixed) in [
        (rect.left_top(), rect.right_bottom()),
        (rect.right_top(), rect.left_bottom()),
        (rect.left_bottom(), rect.right_top()),
        (rect.right_bottom(), rect.left_top()),
    ] {
        if Rect::from_center_size(corner, egui::Vec2::splat(HANDLE_HIT)).contains(pointer) {
            return Some(Handle::Corner { corner, fixed });
        }
    }
    rect.contains(pointer).then_some(Handle::Move)
}

struct TextBounds {
    galley: Arc<Galley>,
    content: Rect,
    selection: Rect,
}

impl TextBounds {
    fn new(painter: &egui::Painter, canvas: Rect, text: &str, style: TextStyle) -> Self {
        let scale = canvas.height() / 1080.0;
        let format = egui::TextFormat::simple(
            FontId::proportional((f32::from(style.size) * scale).max(1.0)),
            Color32::PLACEHOLDER,
        );
        let mut job = egui::text::LayoutJob::default();
        job.halign = Align::Center;
        job.wrap.max_width = canvas.width() * WRAP_WIDTH;
        job.append(text, 0.0, format);
        let galley = painter.layout_job(job);
        let size = galley.size();
        let center = style.position.map_or_else(
            || {
                let y = match style.placement {
                    TextPlacement::Top => {
                        canvas.top() + canvas.height() * ANCHOR_MARGIN + size.y * 0.5
                    }
                    TextPlacement::Center => canvas.center().y,
                    TextPlacement::Bottom => {
                        canvas.bottom() - canvas.height() * ANCHOR_MARGIN - size.y * 0.5
                    }
                };
                pos2(canvas.center().x, y)
            },
            |position| {
                let fraction = |value| f32::from(value) / f32::from(TextPosition::MAX);
                pos2(
                    canvas.left() + canvas.width() * fraction(position.x),
                    canvas.top() + canvas.height() * fraction(position.y),
                )
            },
        );
        let content = Rect::from_center_size(center, size);
        let selection =
            Rect::from_center_size(center, (size + vec2(14.0, 10.0)).max(vec2(25.0, 19.0)));
        Self {
            galley,
            content,
            selection,
        }
    }

    fn paint_text(&self, painter: &egui::Painter, paint: TextPaint, bold: bool, scale: f32) {
        let color = |[r, g, b, a]: [u8; 4]| Color32::from_rgba_unmultiplied(r, g, b, a);
        let offset = |amount| egui::Vec2::splat(f32::from(amount) * scale);
        let origin = self.content.min - self.galley.rect.min.to_vec2();
        if let Some(background) = paint.background {
            painter.rect_filled(
                self.content
                    .expand(f32::from(paint.background_padding()) * scale),
                0,
                color(background),
            );
        } else if paint.shadow > 0 {
            painter.galley(
                origin + offset(paint.shadow),
                self.galley.clone(),
                color(paint.outline),
            );
        }
        if paint.outline_width > 0 {
            let radius = f32::from(paint.outline_width) * scale;
            for angle in 0..8 {
                let radians = std::f32::consts::TAU * angle as f32 / 8.0;
                painter.galley(
                    origin + vec2(radians.cos(), radians.sin()) * radius,
                    self.galley.clone(),
                    color(paint.outline),
                );
            }
        }
        painter.galley(origin, self.galley.clone(), color(paint.foreground));
        if bold {
            painter.galley(
                origin + vec2((scale * 1.2).max(0.25), 0.0),
                self.galley.clone(),
                color(paint.foreground),
            );
        }
    }

    fn paint_selection(&self, painter: &egui::Painter, color: Color32) {
        let stroke = Stroke::new(1.0_f32, color);
        painter.rect_stroke(self.selection, 3, stroke, StrokeKind::Inside);
        for corner in [
            self.selection.left_top(),
            self.selection.right_top(),
            self.selection.left_bottom(),
            self.selection.right_bottom(),
        ] {
            painter.rect(
                Rect::from_center_size(corner, egui::Vec2::splat(HANDLE_SIDE)),
                1,
                Color32::WHITE,
                stroke,
                StrokeKind::Inside,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dragging_the_caption_updates_only_the_preview_until_release() {
        let ctx = egui::Context::default();
        let canvas = Rect::from_min_size(pos2(40.0, 40.0), vec2(700.0, 380.0));
        let text = OverlayText {
            content: "可拖动的字幕".into(),
            style: TextStyle::default(),
        };
        let mut tool = SubtitleCanvas::default();
        let mut frame = |events| {
            let mut output = None;
            let mut selection = Rect::NOTHING;
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 560.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    selection =
                        TextBounds::new(ui.painter(), canvas, &text.content, text.style).selection;
                    output = tool.show(ui, canvas, 7, &text, true);
                },
            );
            (selection, output, tool.excluded_overlay())
        };
        let (bounds, _, _) = frame(Vec::new());
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let center = bounds.center();
        let end = center + vec2(70.0, -55.0);
        assert!(frame(vec![button(center, true)]).1.is_none());
        let (_, pending, excluded) = frame(vec![Event::PointerMoved(end)]);
        assert!(pending.is_none());
        assert_eq!(excluded, Some(7));
        let (_, change, excluded) = frame(vec![button(end, false)]);
        let Some(CanvasTransform::Move(position)) = change else {
            panic!("dragging the center should commit a move");
        };
        assert_eq!(position.x, 6_000);
        assert!(position.y < 8_500);
        assert_eq!(excluded, None);

        let start = bounds.right_bottom();
        let resized = start + (start - bounds.left_top()) * 0.5;
        let (_, change, _) = frame(vec![
            button(start, true),
            Event::PointerMoved(resized),
            button(resized, false),
        ]);
        let Some(CanvasTransform::Resize(style)) = change else {
            panic!("dragging a corner should commit a resize");
        };
        assert_eq!(style.size, 72);
    }
}
