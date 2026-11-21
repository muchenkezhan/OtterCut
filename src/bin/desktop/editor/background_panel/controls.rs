use std::ops::RangeInclusive;

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Painter, Pos2, Rect, RichText, Sense, Stroke,
    StrokeKind, WidgetInfo, WidgetType, pos2, vec2,
};

use crate::ui_theme::upper_palette;

use super::super::background::BackgroundFrame;

pub(super) fn heading(ui: &mut egui::Ui, label: &str) {
    ui.label(RichText::new(label).strong().size(12.0));
}

pub(super) fn segments<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    group: &'static str,
    selected: &mut T,
    options: &[T],
    label: impl Fn(T) -> &'static str,
    glyph: Option<fn(&Painter, Pos2, T, Color32)>,
) {
    let colors = upper_palette(ui.ctx());
    let width = ui.available_width().max(1.0);
    let min_width = if glyph.is_some() { 39.0 } else { 36.0 };
    let columns = ((width / min_width) as usize).clamp(1, options.len());
    let cell_width = width / columns as f32;
    let height = if glyph.is_some() { 36.0 } else { 29.0 };

    for (row, choices) in options.chunks(columns).enumerate() {
        let (rail, _) = ui.allocate_exact_size(
            vec2(cell_width * choices.len() as f32, height),
            Sense::hover(),
        );
        ui.painter().rect_filled(rail, 7, colors.control);
        for (column, &option) in choices.iter().enumerate() {
            let cell = Rect::from_min_size(
                rail.min + vec2(column as f32 * cell_width, 0.0),
                vec2(cell_width, height),
            )
            .shrink2(vec2(2.0, 3.0));
            let active = *selected == option;
            let title = label(option);
            let response = ui.interact(cell, ui.id().with((group, row, column)), Sense::click());
            response.widget_info(|| {
                WidgetInfo::selected(WidgetType::SelectableLabel, true, active, title)
            });
            let fill = if active {
                colors.selection.lerp_to_gamma(colors.accent, 0.12)
            } else if response.hovered() || response.is_pointer_button_down_on() {
                colors.hover
            } else {
                Color32::TRANSPARENT
            };
            let painter = ui.painter().with_clip_rect(cell);
            painter.rect_filled(cell, 5, fill);
            if active || response.has_focus() {
                painter.rect_stroke(
                    cell,
                    5,
                    Stroke::new(1.0_f32, colors.accent),
                    StrokeKind::Inside,
                );
            }
            let ink = if active { colors.text } else { colors.muted };
            if let Some(paint) = glyph {
                paint(
                    &painter,
                    cell.center(),
                    option,
                    if active { colors.accent } else { ink },
                );
            } else {
                painter.text(
                    cell.center(),
                    Align2::CENTER_CENTER,
                    title,
                    FontId::proportional(11.5),
                    ink,
                );
            }
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(title)
                .clicked()
            {
                *selected = option;
            }
        }
    }
}

pub(super) fn slider<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    title: &str,
    value: &mut T,
    range: RangeInclusive<T>,
    unit: &str,
) {
    let colors = upper_palette(ui.ctx());
    let label = ui
        .horizontal(|ui| {
            let label = ui.label(RichText::new(title).size(11.5)).id;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_sized(
                    [55.0, 20.0],
                    egui::DragValue::new(value)
                        .range(range.clone())
                        .suffix(unit),
                )
                .on_hover_text(title);
            });
            label
        })
        .inner;
    ui.scope(|ui| {
        let width = ui.available_width().floor().max(1.0);
        let style = ui.style_mut();
        style.spacing.slider_width = width;
        style.spacing.slider_rail_height = 4.0;
        style.spacing.interact_size.y = 16.0;
        style.visuals.selection.bg_fill = colors.accent;
        style.visuals.widgets.inactive.bg_fill = colors.border;
        for widget in [
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
        ] {
            widget.bg_stroke = Stroke::new(1.0_f32, colors.accent);
            widget.fg_stroke = Stroke::new(1.0_f32, colors.text);
        }
        ui.add(
            egui::Slider::new(value, range)
                .show_value(false)
                .trailing_fill(true),
        )
        .labelled_by(label);
    });
}

pub(super) fn frame_icon(painter: &Painter, center: Pos2, frame: BackgroundFrame, ink: Color32) {
    let stroke = Stroke::new(1.25_f32, ink);
    let line = |(x, y), (x2, y2)| {
        painter.line_segment([center + vec2(x, y), center + vec2(x2, y2)], stroke);
    };
    let outline = |w, h, y| {
        painter.rect_stroke(
            Rect::from_center_size(center + vec2(0.0, y), vec2(w, h)),
            2,
            stroke,
            StrokeKind::Inside,
        );
    };
    match frame {
        BackgroundFrame::None => {
            for sign in [-1.0, 1.0] {
                for y in [-6.5, 6.5] {
                    line((sign * 4.0, y), (sign * 7.0, y));
                    line((sign * 7.0, y), (sign * 7.0, y * 0.54));
                }
            }
        }
        BackgroundFrame::Window => {
            outline(17.0, 13.0, 0.0);
            line((-7.7, -2.0), (7.7, -2.0));
            painter.circle_filled(pos2(center.x - 4.8, center.y - 4.1), 0.75, ink);
        }
        BackgroundFrame::Laptop => {
            outline(14.0, 10.5, -1.0);
            line((-9.0, 5.2), (9.0, 5.2));
        }
        BackgroundFrame::Phone => {
            outline(9.5, 16.0, 0.0);
            line((-1.6, 5.0), (1.6, 5.0));
        }
        BackgroundFrame::Monitor => {
            outline(17.0, 11.0, -2.0);
            line((0.0, 3.4), (0.0, 6.5));
            line((-3.4, 6.5), (3.4, 6.5));
        }
    }
}
