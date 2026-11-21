use eframe::egui::{self, Align, Layout, RichText, vec2};

use super::super::canvas_format::{AspectRatio, CanvasFormat};
use super::compact_menu;

#[derive(Clone)]
struct CustomSize {
    width: String,
    height: String,
}

impl CustomSize {
    fn new(size: (u32, u32)) -> Self {
        let even = |value: u32| {
            (value.clamp(
                CanvasFormat::MIN_CUSTOM_DIMENSION,
                CanvasFormat::MAX_CUSTOM_DIMENSION,
            ) & !1)
                .to_string()
        };
        Self {
            width: even(size.0),
            height: even(size.1),
        }
    }

    fn format(&self) -> Option<CanvasFormat> {
        let format = CanvasFormat::Custom {
            width: self.width.trim().parse().ok()?,
            height: self.height.trim().parse().ok()?,
        };
        format.is_valid().then_some(format)
    }
}

pub(super) fn picker(
    ui: &mut egui::Ui,
    width: f32,
    current: CanvasFormat,
    composition_size: Option<(u32, u32)>,
) -> Option<CanvasFormat> {
    let spacing = ui.spacing_mut();
    spacing.interact_size.y = super::QUALITY_HEIGHT;
    spacing.button_padding = vec2(6.0, 3.0);
    spacing.icon_width = 12.0;
    spacing.icon_spacing = 4.0;

    let label = match current {
        CanvasFormat::Original => "原始",
        CanvasFormat::Preset(ratio) => ratio.label(),
        CanvasFormat::Custom { .. } => "自定义",
    };
    let id = egui::Id::new("custom-canvas-size");
    let mut selection = None;
    let response = egui::ComboBox::from_id_salt("preview-canvas-ratio")
        .width(width)
        .height(356.0)
        .truncate()
        .selected_text(RichText::new(label).size(11.0))
        .show_ui(ui, |ui| {
            compact_menu(ui);
            ui.set_min_width(112.0);
            if ui
                .selectable_label(current.is_original(), "适应（原始）")
                .clicked()
            {
                selection = Some(CanvasFormat::Original);
                ui.close();
            }
            if ui
                .selectable_label(matches!(current, CanvasFormat::Custom { .. }), "自定义…")
                .clicked()
            {
                let dimensions = current
                    .dimensions()
                    .or(composition_size)
                    .unwrap_or((1920, 1080));
                ui.ctx()
                    .data_mut(|data| data.insert_temp(id, CustomSize::new(dimensions)));
                ui.close();
            }
            ui.separator();
            for ratio in AspectRatio::ALL {
                if ratio == AspectRatio::Portrait {
                    ui.separator();
                }
                let format = CanvasFormat::Preset(ratio);
                let (width, height) = ratio.dimensions();
                if ui
                    .selectable_label(current == format, ratio.label())
                    .on_hover_text(format!("{width} × {height} 像素"))
                    .clicked()
                {
                    selection = Some(format);
                    ui.close();
                }
            }
        })
        .response;
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_label("画幅比例"));
    if let Some((width, height)) = current.dimensions().or(composition_size) {
        response.on_hover_text(format!("画幅比例 · {width} × {height}"));
    } else {
        response.on_hover_text("画幅比例");
    }

    custom_dialog(ui.ctx(), id)
        .or(selection)
        .filter(|&format| format != current)
}

fn custom_dialog(ctx: &egui::Context, id: egui::Id) -> Option<CanvasFormat> {
    let mut draft = ctx.data(|data| data.get_temp::<CustomSize>(id))?;
    let modal = egui::Modal::new(id).show(ctx, |ui| {
        ui.set_min_width(244.0);
        ui.heading("自定义画幅");
        ui.add_space(8.0);
        egui::Grid::new("custom-canvas-fields")
            .spacing(vec2(8.0, 6.0))
            .show(ui, |ui| {
                for (label, value) in [("宽度", &mut draft.width), ("高度", &mut draft.height)]
                {
                    let name = ui.label(label);
                    ui.add(
                        egui::TextEdit::singleline(value)
                            .desired_width(88.0)
                            .char_limit(6),
                    )
                    .labelled_by(name.id);
                    ui.label("像素");
                    ui.end_row();
                }
            });
        ui.label(RichText::new("宽高需为 2–4096 的偶数").size(11.0).weak());
        ui.add_space(8.0);
        let format = draft.format();
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let apply = ui
                .add_enabled(format.is_some(), egui::Button::new("应用"))
                .clicked();
            let cancel = ui.button("取消").clicked();
            (format.filter(|_| apply), cancel)
        })
        .inner
    });
    let (format, cancel) = modal.inner;
    let close = format.is_some() || cancel || modal.should_close();
    ctx.data_mut(|data| {
        if close {
            data.remove::<CustomSize>(id);
        } else {
            data.insert_temp(id, draft);
        }
    });
    format
}
