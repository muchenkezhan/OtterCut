use eframe::egui::{self, RichText, Sense, Stroke, StrokeKind, WidgetInfo, WidgetType, vec2};

use crate::ui_icons::Icon;
use crate::ui_theme::upper_palette;

use super::super::background::{BackdropPreset, BackgroundSource};
use super::super::dynamic_recipe::DynamicRecipe;

pub(super) enum GalleryAction {
    Import,
    Select(BackdropPreset),
}

pub(super) fn show_dynamic(
    ui: &mut egui::Ui,
    current: Option<&DynamicRecipe>,
) -> Option<DynamicRecipe> {
    let recent = dynamic_cards(ui, current, &DynamicRecipe::modern_presets(), "modern");
    let classic = DynamicRecipe::presets();
    let show_classic =
        current.is_some_and(|current| classic.iter().any(|preset| matches_recipe(current, preset)));
    let earlier = egui::CollapsingHeader::new("更多预设")
        .id_salt("classic-dynamic-gallery")
        .default_open(show_classic)
        .show(ui, |ui| dynamic_cards(ui, current, &classic, "classic"));
    recent.or(earlier.body_returned.flatten())
}

fn matches_recipe(current: &DynamicRecipe, preset: &DynamicRecipe) -> bool {
    current.name == preset.name
        && current.pattern == preset.pattern
        && current.colors == preset.colors
}

fn dynamic_cards(
    ui: &mut egui::Ui,
    current: Option<&DynamicRecipe>,
    presets: &[DynamicRecipe],
    set: &'static str,
) -> Option<DynamicRecipe> {
    const GAP: f32 = 6.0;
    let width = ui.available_width();
    if width <= 0.0 {
        return None;
    }
    let key = egui::Id::new(("dynamic-gallery-samples", set));
    let sample_aspect = 16.0 / 9.0;
    let textures = ui
        .data(|data| data.get_temp::<Vec<egui::TextureHandle>>(key))
        .unwrap_or_else(|| {
            let textures: Vec<_> = presets
                .iter()
                .map(|preset| {
                    sample_texture(ui.ctx(), &preset.name, [128, 72], |u, v| {
                        preset.sample(u, v)
                    })
                })
                .collect();
            ui.data_mut(|data| data.insert_temp(key, textures.clone()));
            textures
        });
    let columns = if presets.len() == 6 && width >= 232.0 {
        3
    } else if presets.len() == 4 && width >= 330.0 {
        4
    } else if width >= 98.0 {
        2
    } else {
        1
    };
    let tile_width = ((width - GAP * (columns - 1) as f32) / columns as f32).floor();
    let size = vec2(tile_width, (tile_width / sample_aspect).clamp(30.0, 90.0));
    let colors = upper_palette(ui.ctx());
    let mut selected = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = vec2(GAP, GAP);
        for start in (0..presets.len()).step_by(columns) {
            ui.horizontal_top(|ui| {
                for index in start..(start + columns).min(presets.len()) {
                    let preset = &presets[index];
                    let active = current.is_some_and(|current| matches_recipe(current, preset));
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        if card(ui, size, Some(&textures[index]), &preset.name, active) {
                            selected = Some(preset.clone());
                        }
                        ui.add_sized(
                            [tile_width, 16.0],
                            egui::Label::new(
                                RichText::new(&preset.name).size(11.0).color(if active {
                                    colors.text
                                } else {
                                    colors.muted
                                }),
                            )
                            .truncate()
                            .selectable(false),
                        );
                    });
                }
            });
        }
    });
    selected
}

pub(super) fn show(
    ui: &mut egui::Ui,
    selected: &BackgroundSource,
    patterned: bool,
) -> Option<GalleryAction> {
    const GAP: f32 = 6.0;
    const TARGET_TILE_WIDTH: f32 = 56.0;

    let textures = swatch_textures(ui.ctx(), patterned);
    let first_preset = usize::from(patterned);
    let count = BackdropPreset::ALL.len() + first_preset;
    let width = ui.available_width();
    if width <= 0.0 {
        return None;
    }

    let min_columns = if width >= 80.0 { 2 } else { 1 };
    let columns =
        (((width + GAP) / (TARGET_TILE_WIDTH + GAP)).floor() as usize).clamp(min_columns, count);
    let tile_width = ((width - GAP * (columns - 1) as f32) / columns as f32).floor();
    let size = vec2(tile_width, (tile_width * 0.78).clamp(32.0, 70.0));
    let mut action = None;

    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = vec2(GAP, GAP);
        for start in (0..count).step_by(columns) {
            ui.horizontal(|ui| {
                for index in start..(start + columns).min(count) {
                    let Some(preset_index) = index.checked_sub(first_preset) else {
                        if card(
                            ui,
                            size,
                            None,
                            "导入背景图片",
                            matches!(selected, BackgroundSource::Image(_)),
                        ) {
                            action = Some(GalleryAction::Import);
                        }
                        continue;
                    };
                    let preset = BackdropPreset::ALL[preset_index];
                    let active = match selected {
                        BackgroundSource::Wallpaper(current) => patterned && *current == preset,
                        BackgroundSource::Gradient(current) => !patterned && *current == preset,
                        _ => false,
                    };
                    if card(
                        ui,
                        size,
                        Some(&textures[preset_index]),
                        preset.label(),
                        active,
                    ) {
                        action = Some(GalleryAction::Select(preset));
                    }
                }
            });
        }
    });
    action
}

fn card(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    texture: Option<&egui::TextureHandle>,
    label: &str,
    active: bool,
) -> bool {
    const RADIUS: u8 = 7;

    let colors = upper_palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, active, label));
    if let Some(texture) = texture {
        egui::Image::new((texture.id(), size))
            .corner_radius(RADIUS)
            .paint_at(ui, rect);
    } else {
        let fill = if active {
            colors.selection.lerp_to_gamma(colors.accent, 0.10)
        } else if response.hovered() {
            colors.hover
        } else {
            colors.control
        };
        let ink = if active { colors.accent } else { colors.muted };
        ui.painter().rect_filled(rect, RADIUS, fill);
        Icon::FolderPlus.paint(ui.painter(), rect.center(), ink);
    }

    let border = if active {
        Stroke::new(2.0_f32, colors.accent)
    } else if response.has_focus() || response.hovered() {
        Stroke::new(1.0_f32, colors.muted)
    } else {
        Stroke::new(1.0_f32, colors.border)
    };
    ui.painter()
        .rect_stroke(rect, RADIUS, border, StrokeKind::Inside);
    if active && rect.width() >= 27.0 {
        let center = rect.right_top() + vec2(-10.5, 10.5);
        ui.painter().circle_filled(center, 6.5, colors.accent);
        ui.painter().add(egui::Shape::line(
            vec![
                center + vec2(-2.6, 0.0),
                center + vec2(-0.8, 1.7),
                center + vec2(2.7, -2.0),
            ],
            Stroke::new(1.35_f32, colors.on_accent),
        ));
    }

    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
        .clicked()
}

fn swatch_textures(ctx: &egui::Context, patterned: bool) -> Vec<egui::TextureHandle> {
    let key = egui::Id::new(("background-gallery-samples", patterned));
    if let Some(textures) = ctx.data(|data| data.get_temp(key)) {
        return textures;
    }

    let textures: Vec<_> = BackdropPreset::ALL
        .into_iter()
        .map(|preset| {
            sample_texture(ctx, preset.label(), [96, 72], |u, v| {
                preset.sample(u, v, patterned)
            })
        })
        .collect();
    ctx.data_mut(|data| data.insert_temp(key, textures.clone()));
    textures
}

fn sample_texture(
    ctx: &egui::Context,
    name: &str,
    size: [usize; 2],
    sample: impl Fn(f32, f32) -> [u8; 3],
) -> egui::TextureHandle {
    let mut rgb = Vec::with_capacity(size[0] * size[1] * 3);
    for y in 0..size[1] {
        for x in 0..size[0] {
            rgb.extend(sample(
                x as f32 / (size[0] - 1) as f32,
                y as f32 / (size[1] - 1) as f32,
            ));
        }
    }
    ctx.load_texture(
        name,
        egui::ColorImage::from_rgb(size, &rgb),
        egui::TextureOptions::LINEAR,
    )
}
