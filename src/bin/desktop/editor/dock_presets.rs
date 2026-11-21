use eframe::egui;

use crate::ui_icons::{ButtonStyle, Icon, menu_icon_button};

use super::dock_model::{DockState, Preset};
use super::layout;

pub(super) fn show(ui: &mut egui::Ui, state: &mut DockState) -> bool {
    let label = format!("工作区 · {}", state.active.label());
    let button = menu_icon_button(ui, Icon::Timeline, &label, ButtonStyle::Quiet, true);
    let mut changed = false;
    button.popup().gap(5.0).show(|ui| {
        layout::compact_menu(ui);
        ui.set_min_width(108.0);
        for preset in Preset::ALL {
            let selected = state.active == preset;
            let response = ui.add_enabled(
                state.can_select(preset),
                egui::Button::selectable(selected, preset.label()),
            );
            if response.clicked() {
                changed = state.select(preset);
                ui.close();
            }
            if preset == Preset::Custom {
                response.on_disabled_hover_text("拖动面板后会记住自定义布局");
            }
        }
    });
    changed
}
