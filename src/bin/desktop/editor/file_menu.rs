use eframe::egui::{self, Align2, FontId, Sense, Stroke, pos2, vec2};

use crate::ui_icons::paint_icon_focus;
use crate::ui_theme::upper_palette;
use takeseam::i18n::{Locale, key};

use super::layout;

pub(super) enum FileAction {
    Import,
    Open,
    Save,
}

pub(super) fn show(ui: &mut egui::Ui, can_open: bool) -> Option<FileAction> {
    let colors = upper_palette(ui.ctx());
    let (bounds, button) = ui.allocate_exact_size(vec2(56.0, 28.0), Sense::click());
    let popup_id = button.id.with("file-menu");
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let locale = Locale::from_context(ui.ctx());
    let file_label = locale.text(key::FILE);
    button.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, file_label));
    let button = button.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    if open || button.hovered() {
        painter.rect_filled(bounds, 4, colors.hover);
    }
    let y = bounds.center().y;
    painter.text(
        pos2(bounds.left() + 10.0, y),
        Align2::LEFT_CENTER,
        file_label,
        FontId::proportional(12.0),
        colors.text,
    );
    let x = bounds.right() - 12.0;
    painter.line(
        vec![
            pos2(x - 3.0, y - 1.5),
            pos2(x, y + 1.5),
            pos2(x + 3.0, y - 1.5),
        ],
        Stroke::new(1.15_f32, colors.muted),
    );
    if button.has_focus() {
        paint_icon_focus(
            painter,
            pos2(bounds.center().x, bounds.bottom() - 3.0),
            colors.accent,
        );
    }

    egui::Popup::menu(&button)
        .id(popup_id)
        .gap(4.0)
        .show(|ui| {
            layout::compact_menu(ui);
            ui.set_min_width(150.0);
            let import = ui.button(locale.text(key::IMPORT_MEDIA));
            let open = ui
                .add_enabled(can_open, egui::Button::new(locale.text(key::OPEN_PROJECT)))
                .on_disabled_hover_text(locale.text(key::NARRATION_SAVE_BEFORE_OPEN));
            let save =
                ui.add(egui::Button::new(locale.text(key::SAVE_PROJECT)).shortcut_text("Ctrl+S"));
            let action = if import.clicked() {
                FileAction::Import
            } else if open.clicked() {
                FileAction::Open
            } else if save.clicked() {
                FileAction::Save
            } else {
                return None;
            };
            ui.close();
            Some(action)
        })
        .and_then(|popup| popup.inner)
}
