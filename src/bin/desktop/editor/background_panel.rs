use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, FontId, Layout, RichText};

use crate::ui_theme::upper_palette;

use super::background::{
    BackdropPreset, BackgroundFrame, BackgroundMotion, BackgroundSettings, BackgroundShadow,
    BackgroundSource,
};
use super::dynamic_recipe::DynamicRecipe;
use super::media::local_media;

#[path = "background_panel/controls.rs"]
mod controls;
#[path = "background_panel/gallery.rs"]
mod gallery;
#[cfg(test)]
#[path = "background_panel/panel_tests.rs"]
mod panel_tests;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceKind {
    Off,
    Dynamic,
    Picture,
    Gradient,
    Solid,
}

impl SourceKind {
    const ALL: [Self; 5] = [
        Self::Off,
        Self::Dynamic,
        Self::Picture,
        Self::Gradient,
        Self::Solid,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Off => "无",
            Self::Dynamic => "动效",
            Self::Picture => "图片",
            Self::Gradient => "渐变",
            Self::Solid => "纯色",
        }
    }

    fn of(source: &BackgroundSource) -> Self {
        match source {
            BackgroundSource::Off => Self::Off,
            BackgroundSource::Dynamic(_) => Self::Dynamic,
            BackgroundSource::Wallpaper(_) | BackgroundSource::Image(_) => Self::Picture,
            BackgroundSource::Gradient(_) => Self::Gradient,
            BackgroundSource::Solid(_) => Self::Solid,
        }
    }

    fn initial_source(self) -> BackgroundSource {
        match self {
            Self::Off => BackgroundSource::Off,
            Self::Dynamic => BackgroundSource::Dynamic(DynamicRecipe::modern_presets()[0].clone()),
            Self::Picture => BackgroundSource::Wallpaper(BackdropPreset::ALL[0]),
            Self::Gradient => BackgroundSource::Gradient(BackdropPreset::ALL[0]),
            Self::Solid => BackgroundSource::Solid([33, 44, 62]),
        }
    }
}

pub(super) fn show(
    ui: &mut egui::Ui,
    settings: &BackgroundSettings,
    initial_directory: &Path,
) -> Option<BackgroundSettings> {
    let mut draft = settings.clone();
    let file_error = ui.id().with("background-file-error");
    egui::ScrollArea::vertical()
        .id_salt("background-settings-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(5, 9))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 5.0;
                    let current = SourceKind::of(&draft.source);
                    let mut mode = current;
                    controls::segments(
                        ui,
                        "source",
                        &mut mode,
                        &SourceKind::ALL,
                        SourceKind::label,
                        None,
                    );
                    if mode != current {
                        draft.source = mode.initial_source();
                        if let BackgroundSource::Dynamic(recipe) = &draft.source {
                            draft.motion = recipe.motion;
                        }
                        ui.data_mut(|data| data.remove::<String>(file_error));
                    }
                    if matches!(mode, SourceKind::Off | SourceKind::Dynamic) {
                        ui.add_space(3.0);
                        recipes(ui, &mut draft, initial_directory, file_error);
                    }
                    if !draft.enabled() {
                        return;
                    }

                    if let BackgroundSource::Solid(color) = &mut draft.source {
                        ui.add_space(3.0);
                        solid_color(ui, color);
                    } else if matches!(mode, SourceKind::Picture | SourceKind::Gradient) {
                        ui.add_space(3.0);
                        let patterned = mode == SourceKind::Picture;
                        match gallery::show(ui, &draft.source, patterned) {
                            Some(gallery::GalleryAction::Import) => {
                                match choose_image(&draft.source, initial_directory) {
                                    Ok(Some(path)) => {
                                        draft.source = BackgroundSource::Image(path);
                                        ui.data_mut(|data| data.remove::<String>(file_error));
                                    }
                                    Ok(None) => {}
                                    Err(error) => {
                                        ui.data_mut(|data| data.insert_temp(file_error, error));
                                    }
                                }
                            }
                            Some(gallery::GalleryAction::Select(preset)) => {
                                draft.source = if patterned {
                                    BackgroundSource::Wallpaper(preset)
                                } else {
                                    BackgroundSource::Gradient(preset)
                                };
                                ui.data_mut(|data| data.remove::<String>(file_error));
                            }
                            None => {}
                        }
                        if let BackgroundSource::Image(path) = &draft.source {
                            let colors = upper_palette(ui.ctx());
                            let name = path
                                .file_name()
                                .unwrap_or(path.as_os_str())
                                .to_string_lossy();
                            ui.add(
                                egui::Label::new(
                                    RichText::new(name).color(colors.muted).size(11.0),
                                )
                                .truncate(),
                            )
                            .on_hover_text(path.display().to_string());
                        }
                        if let Some(error) = ui.data(|data| data.get_temp::<String>(file_error)) {
                            ui.colored_label(upper_palette(ui.ctx()).danger, error);
                        }
                    }

                    section(ui, "动画");
                    controls::segments(
                        ui,
                        "motion",
                        &mut draft.motion,
                        &BackgroundMotion::ALL,
                        BackgroundMotion::label,
                        None,
                    );
                    if let BackgroundSource::Dynamic(recipe) = &mut draft.source {
                        recipe.motion = draft.motion;
                        controls::slider(ui, "速度", &mut recipe.speed, 10..=300, "%");
                        controls::slider(ui, "强度", &mut recipe.intensity, 0..=100, "%");
                        controls::slider(ui, "方向", &mut recipe.angle, 0..=359, "°");
                    }
                    controls::slider(ui, "模糊背景", &mut draft.blur, 0..=100, "%");

                    section(ui, "画框");
                    controls::segments(
                        ui,
                        "frame",
                        &mut draft.frame,
                        &BackgroundFrame::ALL,
                        BackgroundFrame::label,
                        Some(controls::frame_icon),
                    );
                    ui.label(RichText::new("阴影").size(11.5));
                    controls::segments(
                        ui,
                        "shadow",
                        &mut draft.shadow,
                        &BackgroundShadow::ALL,
                        BackgroundShadow::label,
                        None,
                    );
                    controls::slider(ui, "内边距", &mut draft.inset, 0..=40, "%");
                    controls::slider(ui, "圆角", &mut draft.radius, 0..=80, " px");
                });
        });
    (draft != *settings).then_some(draft)
}

fn recipes(
    ui: &mut egui::Ui,
    settings: &mut BackgroundSettings,
    directory: &Path,
    error_id: egui::Id,
) {
    controls::heading(ui, "动态预设");
    let current = match &settings.source {
        BackgroundSource::Dynamic(recipe) => Some(recipe),
        _ => None,
    };
    if let Some(recipe) = gallery::show_dynamic(ui, current) {
        settings.motion = recipe.motion;
        settings.source = BackgroundSource::Dynamic(recipe);
        ui.data_mut(|data| data.remove::<String>(error_id));
    }

    ui.horizontal_wrapped(|ui| {
        if ui
            .button("导入配方")
            .on_hover_text("打开 JSON 动态配方")
            .clicked()
            && let Some(path) = rfd::FileDialog::new()
                .set_title("导入动态配方")
                .set_directory(directory)
                .add_filter("动态配方", &["json"])
                .pick_file()
        {
            match local_media(&path).and_then(|path| DynamicRecipe::read(&path)) {
                Ok(recipe) => {
                    settings.motion = recipe.motion;
                    settings.source = BackgroundSource::Dynamic(recipe);
                    ui.data_mut(|data| data.remove::<String>(error_id));
                }
                Err(error) => {
                    ui.data_mut(|data| data.insert_temp(error_id, error.to_string()));
                }
            }
        }
        if let BackgroundSource::Dynamic(recipe) = &settings.source
            && ui
                .button("复制配方")
                .on_hover_text("复制当前配方的 JSON，可交给 AI 修改")
                .clicked()
        {
            let mut recipe = recipe.clone();
            recipe.motion = settings.motion;
            if let Ok(json) = serde_json::to_string_pretty(&recipe) {
                ui.ctx().copy_text(json);
            }
        }
    });
    if let BackgroundSource::Dynamic(recipe) = &settings.source {
        let colors = upper_palette(ui.ctx());
        ui.add(
            egui::Label::new(RichText::new(&recipe.name).color(colors.muted).size(11.0)).truncate(),
        );
    }
    if let Some(error) = ui.data(|data| data.get_temp::<String>(error_id)) {
        ui.colored_label(upper_palette(ui.ctx()).danger, error);
    }
}

fn section(ui: &mut egui::Ui, name: &str) {
    ui.add_space(5.0);
    ui.separator();
    ui.add_space(2.0);
    controls::heading(ui, name);
}

fn solid_color(ui: &mut egui::Ui, color: &mut [u8; 3]) {
    let colors = upper_palette(ui.ctx());
    egui::Frame::new()
        .fill(colors.control)
        .corner_radius(7)
        .inner_margin(egui::Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let label = ui.label(RichText::new("颜色").size(11.5));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.color_edit_button_srgb(color).labelled_by(label.id);
                    let [r, g, b] = *color;
                    ui.label(
                        RichText::new(format!("#{r:02X}{g:02X}{b:02X}"))
                            .font(FontId::monospace(11.0))
                            .color(colors.muted),
                    );
                });
            });
        });
}

fn choose_image(
    source: &BackgroundSource,
    initial_directory: &Path,
) -> Result<Option<PathBuf>, String> {
    let directory = match source {
        BackgroundSource::Image(path) => path.parent().unwrap_or(initial_directory),
        _ => initial_directory,
    };
    let Some(path) = rfd::FileDialog::new()
        .set_title("选择背景图片")
        .set_directory(directory)
        .add_filter("背景图片", &["png", "jpg", "jpeg", "webp"])
        .pick_file()
    else {
        return Ok(None);
    };
    if !path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|extension| {
            ["png", "jpg", "jpeg", "webp"]
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
    {
        return Err("请选择 PNG、JPG 或 WebP 图片".to_owned());
    }
    local_media(&path)
        .map(Some)
        .map_err(|error| format!("无法打开背景图片（{error}）"))
}
