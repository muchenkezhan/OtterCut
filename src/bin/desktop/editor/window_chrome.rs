use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, vec2};

use crate::ui_icons::{Icon, paint_icon_focus};
use crate::ui_theme::upper_palette;

pub(super) fn controls(ui: &mut egui::Ui) {
    let maximized = is_maximized(ui);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for control in [
            CaptionButton::Close,
            if maximized {
                CaptionButton::Restore
            } else {
                CaptionButton::Maximize
            },
            CaptionButton::Minimize,
        ] {
            if control.show(ui).clicked() {
                ui.ctx().send_viewport_cmd(control.command());
            }
        }
    });
}

fn is_maximized(ui: &egui::Ui) -> bool {
    ui.input(|input| input.viewport().maximized.unwrap_or(false))
}

#[derive(Clone, Copy)]
enum CaptionButton {
    Minimize,
    Maximize,
    Restore,
    Close,
}

impl CaptionButton {
    fn label(self) -> &'static str {
        match self {
            Self::Minimize => "最小化",
            Self::Maximize => "最大化",
            Self::Restore => "还原窗口",
            Self::Close => "关闭窗口",
        }
    }

    fn command(self) -> egui::ViewportCommand {
        match self {
            Self::Minimize => egui::ViewportCommand::Minimized(true),
            Self::Maximize => egui::ViewportCommand::Maximized(true),
            Self::Restore => egui::ViewportCommand::Maximized(false),
            Self::Close => egui::ViewportCommand::Close,
        }
    }

    fn show(self, ui: &mut egui::Ui) -> egui::Response {
        let colors = upper_palette(ui.ctx());
        let label = self.label();
        let (bounds, response) = ui.allocate_exact_size(vec2(36.0, 32.0), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        let hot = response.hovered() || response.is_pointer_button_down_on();
        let color = if hot && matches!(self, Self::Close) {
            colors.danger
        } else if hot {
            colors.text
        } else {
            colors.muted
        };
        let center = bounds.center();
        if hot {
            ui.painter()
                .rect_filled(bounds.shrink(2.0), 4, colors.hover);
        }
        self.paint(ui.painter(), center, color);
        if response.has_focus() {
            paint_icon_focus(ui.painter(), center + vec2(0.0, 12.0), colors.accent);
        }
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(label)
    }

    fn paint(self, painter: &egui::Painter, center: Pos2, color: Color32) {
        let stroke = Stroke::new(1.15_f32, color);
        let outline = |rect| painter.rect_stroke(rect, 0, stroke, StrokeKind::Inside);
        match self {
            Self::Minimize => {
                painter.hline(center.x - 4.5..=center.x + 4.5, center.y + 2.0, stroke);
            }
            Self::Maximize => {
                outline(Rect::from_center_size(center, vec2(10.0, 10.0)));
            }
            Self::Restore => {
                let front = Rect::from_center_size(center + vec2(-1.5, 1.5), vec2(8.0, 8.0));
                let back = front.translate(vec2(3.0, -3.0));
                painter.line(
                    vec![
                        Pos2::new(back.left(), front.top()),
                        back.left_top(),
                        back.right_top(),
                        back.right_bottom(),
                        Pos2::new(front.right(), back.bottom()),
                    ],
                    stroke,
                );
                outline(front);
            }
            Self::Close => Icon::Close.paint(painter, center, color),
        }
    }
}
