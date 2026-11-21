use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Rect, RichText, Sense, UiBuilder, pos2, vec2,
};
use takeseam::PRODUCT_NAME;

use crate::ui_icons::{ButtonStyle, Icon, icon_button, paint_icon_focus};
use crate::ui_theme::{ON_ACCENT_FILL, Palette, palette, upper_palette};

use super::canvas_format::CanvasFormat;
use super::media::PreviewQuality;
use super::preview_viewport::PreviewViewport;
use super::{window_chrome, window_frame};

#[path = "layout/canvas_ratio.rs"]
mod canvas_ratio;

const CONTROL_ROW_HEIGHT: f32 = 32.0;
const CONTROL_GAP: f32 = 6.0;
const SEEK_HEIGHT: f32 = 16.0;
const MIN_SEEK_WIDTH: f32 = 24.0;
const QUALITY_WIDTH: f32 = 58.0;
const QUALITY_HEIGHT: f32 = 24.0;
const RATIO_WIDTH: f32 = 66.0;
const VIEW_ZOOM_WIDTH: f32 = 44.0;
const INLINE_SEEK_WIDTH: f32 = 100.0;
const STACK_SELECTORS_BELOW: f32 = 2.0
    * (VIEW_ZOOM_WIDTH
        + QUALITY_WIDTH
        + RATIO_WIDTH
        + CONTROL_ROW_HEIGHT * 0.5
        + INLINE_SEEK_WIDTH
        + CONTROL_GAP * 4.0);

pub(super) fn compact_menu(ui: &mut egui::Ui) {
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
    let spacing = ui.spacing_mut();
    spacing.interact_size.y = 26.0;
    spacing.item_spacing.y = 3.0;
}

pub(super) fn header(
    ui: &mut egui::Ui,
    logo: &egui::TextureHandle,
    project: &str,
    left_menu: impl FnOnce(&mut egui::Ui),
    actions: impl FnOnce(&mut egui::Ui),
) {
    let colors = upper_palette(ui.ctx());
    egui::Panel::top("editor-header")
        .exact_size(40.0)
        .frame(
            egui::Frame::new()
                .fill(colors.canvas)
                .inner_margin(egui::Margin {
                    left: 12,
                    right: 12,
                    top: 6,
                    bottom: 2,
                }),
        )
        .show_inside(ui, |ui| {
            let row = ui.available_rect_before_wrap();
            ui.set_min_size(row.size());
            let mut action_ui = ui.new_child(
                UiBuilder::new()
                    .id_salt("project-actions")
                    .max_rect(row)
                    .layout(Layout::right_to_left(Align::Center)),
            );
            action_ui.spacing_mut().item_spacing.x = 4.0;
            window_chrome::controls(&mut action_ui);
            action_ui.separator();
            actions(&mut action_ui);

            let mut brand = ui.new_child(
                UiBuilder::new()
                    .id_salt("project-brand")
                    .max_rect(row)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            brand.spacing_mut().item_spacing.x = 8.0;
            brand.image((logo.id(), vec2(22.0, 22.0)));
            brand.add(
                egui::Label::new(
                    RichText::new(PRODUCT_NAME)
                        .size(14.0)
                        .strong()
                        .color(colors.text),
                )
                .selectable(false),
            );
            left_menu(&mut brand);

            let left_end = brand.min_rect().right();
            let right_start = action_ui.min_rect().left();
            window_frame::set_caption(
                Rect::from_x_y_ranges(left_end..=right_start.max(left_end), row.y_range()),
                ui.ctx().pixels_per_point(),
            );
            let safe_left = left_end + 12.0;
            let safe_right = right_start - 12.0;
            if safe_right > safe_left {
                let center = row.center().x.clamp(safe_left, safe_right);
                let half = (center - safe_left).min(safe_right - center);
                let (left, right) = if half >= 64.0 {
                    (center - half, center + half)
                } else {
                    (safe_left, safe_right)
                };
                ui.put(
                    Rect::from_x_y_ranges(left..=right, row.y_range()),
                    egui::Label::new(RichText::new(project).size(12.0).color(colors.text))
                        .halign(Align::Center)
                        .selectable(false)
                        .truncate(),
                );
            }
        });
}

pub(super) fn export_button(ui: &mut egui::Ui, enabled: bool) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| {
        let colors = upper_palette(ui.ctx());
        let (bounds, response) = ui.allocate_exact_size(vec2(68.0, 28.0), Sense::click());
        let label = "导出 MP4";
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), label)
        });
        let (fill, ink) = if !response.enabled() {
            (colors.control, colors.muted)
        } else {
            let fill = if response.hovered() || response.is_pointer_button_down_on() {
                colors.accent_fill.gamma_multiply(0.90)
            } else {
                colors.accent_fill
            };
            (fill, ON_ACCENT_FILL)
        };
        let painter = ui.painter();
        painter.rect_filled(bounds, 5, fill);
        let x = bounds.left() + 16.0;
        Icon::Export.paint_scaled(painter, pos2(x, bounds.center().y), ink, 0.78);
        painter.text(
            pos2(x + 12.0, bounds.center().y),
            Align2::LEFT_CENTER,
            "导出",
            FontId::proportional(12.0),
            ink,
        );
        if response.has_focus() {
            paint_icon_focus(painter, pos2(bounds.center().x, bounds.bottom() - 3.0), ink);
        }
        response
            .on_hover_text(label)
            .on_disabled_hover_text(label)
            .on_hover_cursor(egui::CursorIcon::PointingHand)
    })
    .inner
}

pub(super) fn status(ui: &mut egui::Ui, text: &str, warning: bool) {
    if text.is_empty() {
        return;
    }
    let colors = palette(ui.ctx());
    egui::Panel::bottom("editor-status")
        .exact_size(24.0)
        .frame(
            egui::Frame::new()
                .fill(colors.canvas)
                .inner_margin(egui::Margin::symmetric(12, 2)),
        )
        .show_inside(ui, |ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let tone = if warning { colors.danger } else { colors.muted };
                ui.add(egui::Label::new(RichText::new(text).size(11.0).color(tone)).truncate())
                    .on_hover_text(text);
            });
        });
}

#[derive(Default)]
pub(super) struct TransportInput {
    pub toggle: bool,
    pub seek: Option<u64>,
    pub quality: Option<PreviewQuality>,
    pub canvas_format: Option<CanvasFormat>,
}

#[expect(clippy::too_many_arguments)]
pub(super) fn monitor(
    ui: &mut egui::Ui,
    viewport: &mut PreviewViewport,
    texture: Option<&egui::TextureHandle>,
    composition_size: Option<(u32, u32)>,
    empty: &str,
    position: u64,
    duration: u64,
    playing: bool,
    quality: PreviewQuality,
    canvas_format: CanvasFormat,
    draw_overlay: impl FnOnce(&mut egui::Ui, Rect),
) -> TransportInput {
    let colors = upper_palette(ui.ctx());
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let available = ui.available_size();
        let rows = if available.x < STACK_SELECTORS_BELOW {
            2.0
        } else {
            1.0
        };
        let transport_height = rows * (CONTROL_ROW_HEIGHT + ui.spacing().item_spacing.y);
        let (bounds, _) = ui.allocate_exact_size(
            vec2(available.x, (available.y - transport_height).max(0.0)),
            Sense::hover(),
        );
        let painter = ui.painter().with_clip_rect(bounds);
        painter.rect_filled(bounds, 4, colors.monitor);
        let canvas = bounds.shrink(4.0);
        let display_size = if texture.is_some() {
            composition_size
        } else {
            canvas_format.dimensions()
        };
        let pixels = display_size
            .map(|(width, height)| vec2(width as f32, height as f32))
            .or_else(|| texture.map(egui::TextureHandle::size_vec2));
        let image = pixels
            .filter(|_| canvas.is_positive())
            .map(|pixels| {
                update_preview_viewport(ui, viewport, canvas, pixels);
                viewport.frame(canvas, pixels)
            })
            .filter(egui::Rect::is_positive);
        if let (Some(texture), Some(image)) = (texture, image) {
            painter.with_clip_rect(canvas).image(
                texture.id(),
                image,
                Rect::from_min_max(egui::Pos2::ZERO, pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            let mut overlay =
                ui.new_child(UiBuilder::new().id_salt("preview-overlay").max_rect(canvas));
            overlay.set_clip_rect(ui.clip_rect().intersect(canvas));
            draw_overlay(&mut overlay, image);
        } else {
            let mut ink = colors.muted;
            if let Some(image) = image {
                painter
                    .with_clip_rect(canvas)
                    .rect_filled(image, 0, Color32::BLACK);
                ink = Color32::LIGHT_GRAY;
            }
            if !empty.is_empty() {
                painter.text(
                    bounds.center(),
                    Align2::CENTER_CENTER,
                    empty,
                    FontId::proportional(12.0),
                    ink,
                );
            }
        }
        transport_row(
            ui,
            viewport,
            position,
            duration,
            playing,
            quality,
            canvas_format,
            composition_size,
            colors,
        )
    })
    .inner
}

fn update_preview_viewport(
    ui: &mut egui::Ui,
    viewport: &mut PreviewViewport,
    canvas: Rect,
    pixels: egui::Vec2,
) {
    let pan_id = ui.id().with("preview-middle-pan");
    let hovered = ui.rect_contains_pointer(canvas) && !egui::Popup::is_any_open(ui.ctx());
    let (pointer, middle_start, middle_down, middle_released, primary_down) = ui.input(|input| {
        let start = input.raw.events.iter().find_map(|event| match event {
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Middle,
                pressed: true,
                ..
            } => Some(*pos),
            _ => None,
        });
        (
            input.pointer.interact_pos(),
            start,
            input.pointer.button_down(egui::PointerButton::Middle),
            input.pointer.button_released(egui::PointerButton::Middle),
            input.pointer.primary_down(),
        )
    });
    let previous = ui
        .data(|data| data.get_temp::<egui::Pos2>(pan_id))
        .or(middle_start.filter(|&start| hovered && canvas.contains(start)));
    if !primary_down
        && (middle_down || middle_released)
        && let (Some(pointer), Some(previous)) = (pointer, previous)
    {
        viewport.pan_by(canvas, pixels, pointer - previous);
        if middle_down {
            ui.data_mut(|data| data.insert_temp(pan_id, pointer));
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
    }
    if !middle_down || primary_down || pointer.is_none() {
        ui.data_mut(|data| data.remove::<egui::Pos2>(pan_id));
    }
    if hovered
        && !primary_down
        && !middle_down
        && let Some(pointer) = pointer
    {
        let vertical = ui.input_mut(|input| std::mem::take(&mut input.smooth_scroll_delta.y));
        if vertical != 0.0 {
            viewport.zoom_at(canvas, pixels, pointer, (vertical * 0.0045).exp());
        }
    }
}

fn preview_seek(ui: &mut egui::Ui, position: u64, duration: u64, colors: Palette) -> Option<u64> {
    let width = ui.available_width().floor();
    let enabled = duration > 0;
    let mut value = position.min(duration);
    ui.add_enabled_ui(enabled, |ui| {
        let style = ui.style_mut();
        style.spacing.slider_width = width;
        style.spacing.slider_rail_height = 2.0;
        style.spacing.interact_size.y = SEEK_HEIGHT;
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(11.0));
        style.visuals.selection.bg_fill = colors.muted;
        style.visuals.widgets.inactive.bg_fill = colors.border;
        for widget in [
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
        ] {
            widget.corner_radius = egui::CornerRadius::same(1);
            widget.fg_stroke = egui::Stroke::new(1.0_f32, colors.muted);
        }
        let response = ui.add(
            egui::Slider::new(&mut value, 0..=duration.max(1))
                .show_value(false)
                .trailing_fill(enabled)
                .handle_shape(egui::style::HandleShape::Rect { aspect_ratio: 0.26 }),
        );
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_label("预览进度"));
        response
            .on_hover_text("预览进度")
            .changed()
            .then_some(value)
    })
    .inner
}

fn preview_quality(
    ui: &mut egui::Ui,
    width: f32,
    quality: PreviewQuality,
) -> Option<PreviewQuality> {
    let spacing = ui.spacing_mut();
    spacing.interact_size.y = QUALITY_HEIGHT;
    spacing.button_padding = vec2(6.0, 3.0);
    spacing.icon_width = 12.0;
    spacing.icon_spacing = 4.0;

    let mut selected = quality;
    let response = egui::ComboBox::from_id_salt("preview-quality")
        .width(width)
        .truncate()
        .selected_text(RichText::new(quality.label()).size(11.0))
        .show_ui(ui, |ui| {
            for choice in PreviewQuality::ALL {
                ui.selectable_value(
                    &mut selected,
                    choice,
                    RichText::new(choice.label()).size(11.0),
                );
            }
        })
        .response;
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_label("预览质量"));
    response.on_hover_text("预览质量");
    (selected != quality).then_some(selected)
}

#[expect(clippy::too_many_arguments)]
fn transport_row(
    ui: &mut egui::Ui,
    viewport: &mut PreviewViewport,
    position: u64,
    duration: u64,
    playing: bool,
    quality: PreviewQuality,
    canvas_format: CanvasFormat,
    composition_size: Option<(u32, u32)>,
    colors: Palette,
) -> TransportInput {
    let (row, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), CONTROL_ROW_HEIGHT),
        Sense::hover(),
    );
    let stack_selectors = row.width() < STACK_SELECTORS_BELOW;
    let selector_row = if stack_selectors {
        ui.allocate_exact_size(row.size(), Sense::hover()).0
    } else {
        row
    };
    let total_width = (selector_row.width() - CONTROL_GAP * 2.0 - VIEW_ZOOM_WIDTH).max(0.0);
    let quality_width = QUALITY_WIDTH.min(total_width * 0.5);
    let ratio_width = RATIO_WIDTH.min(total_width - quality_width);
    let y = selector_row.center().y - QUALITY_HEIGHT * 0.5;
    let ratio_bounds = Rect::from_min_size(
        pos2(selector_row.right() - ratio_width, y),
        vec2(ratio_width, QUALITY_HEIGHT),
    );
    let quality_bounds = Rect::from_min_size(
        pos2(ratio_bounds.left() - CONTROL_GAP - quality_width, y),
        vec2(quality_width, QUALITY_HEIGHT),
    );
    let zoom_bounds = Rect::from_min_size(
        pos2(quality_bounds.left() - CONTROL_GAP - VIEW_ZOOM_WIDTH, y),
        vec2(VIEW_ZOOM_WIDTH, QUALITY_HEIGHT),
    );
    let seek_right = if stack_selectors {
        row.right()
    } else {
        zoom_bounds.left() - CONTROL_GAP
    };
    let latest_play_center = seek_right - MIN_SEEK_WIDTH - CONTROL_GAP - CONTROL_ROW_HEIGHT * 0.5;
    let play_center = row
        .center()
        .x
        .min(latest_play_center)
        .max(row.left() + CONTROL_ROW_HEIGHT * 0.5);
    let play = Rect::from_center_size(
        pos2(play_center, row.center().y),
        vec2(CONTROL_ROW_HEIGHT, CONTROL_ROW_HEIGHT),
    );
    let timecode = Rect::from_min_max(
        row.min,
        pos2((play.left() - CONTROL_GAP).max(row.left()), row.bottom()),
    );
    let mut time_ui = ui.new_child(
        UiBuilder::new()
            .id_salt("preview-timecode")
            .max_rect(timecode)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let mut times = egui::text::LayoutJob::default();
    times.append(
        &time(position),
        0.0,
        egui::TextFormat::simple(FontId::monospace(12.0), colors.text),
    );
    times.append(
        &format!(" / {}", time(duration)),
        0.0,
        egui::TextFormat::simple(FontId::monospace(11.0), colors.muted),
    );
    time_ui.set_clip_rect(ui.clip_rect().intersect(timecode));
    time_ui.add(egui::Label::new(times).truncate());
    let mut play_ui = ui.new_child(
        UiBuilder::new()
            .id_salt("preview-play-control")
            .max_rect(play)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let (icon, label) = if playing {
        (Icon::Pause, "暂停")
    } else {
        (Icon::Play, "播放")
    };
    let toggle = icon_button(&mut play_ui, icon, label, duration > 0, ButtonStyle::Quiet).clicked();
    let seek_width = (seek_right - play.right() - CONTROL_GAP).clamp(0.0, 192.0);
    let seeker = Rect::from_min_size(
        pos2(seek_right - seek_width, row.top()),
        vec2(seek_width, row.height()),
    );
    let mut seek_ui = ui.new_child(
        UiBuilder::new()
            .id_salt("preview-seek")
            .max_rect(seeker)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let seek = preview_seek(&mut seek_ui, position, duration, colors);
    let mut quality_ui = ui.new_child(
        UiBuilder::new()
            .id_salt("preview-quality-control")
            .max_rect(quality_bounds)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let quality = preview_quality(&mut quality_ui, quality_width, quality);
    let zoom_label = if viewport.is_fitted() {
        String::from("适应")
    } else {
        format!("{}%", viewport.percent())
    };
    let zoom = ui.put(
        zoom_bounds,
        egui::Button::new(RichText::new(zoom_label).size(11.0)),
    );
    ui.ctx()
        .accesskit_node_builder(zoom.id, |node| node.set_label("恢复预览适应大小"));
    if zoom
        .on_hover_text("恢复适应大小（滚轮缩放，中键平移）")
        .clicked()
    {
        viewport.reset();
        ui.ctx().request_repaint();
    }
    let mut ratio_ui = ui.new_child(
        UiBuilder::new()
            .id_salt("preview-canvas-ratio-control")
            .max_rect(ratio_bounds)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let canvas_format =
        canvas_ratio::picker(&mut ratio_ui, ratio_width, canvas_format, composition_size);
    TransportInput {
        seek,
        toggle,
        quality,
        canvas_format,
    }
}

pub(super) fn time(value: u64) -> String {
    let seconds = value / 1_000_000;
    format!(
        "{:02}:{:02}.{:02}",
        seconds / 60,
        seconds % 60,
        (value / 10_000) % 100
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::dock_model::{DockState, Pane};
    use crate::editor::dock_view;

    #[test]
    fn wheel_zooms_only_the_monitor_content_and_fit_button_resets_it() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let texture = ctx.load_texture(
            "viewport-test",
            egui::ColorImage::filled([1, 1], Color32::WHITE),
            egui::TextureOptions::NEAREST,
        );
        let frame = |view: &mut PreviewViewport, events| {
            let mut rendered = (Rect::NOTHING, Rect::NOTHING);
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(760.0, 410.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    monitor(
                        ui,
                        view,
                        Some(&texture),
                        Some((640, 360)),
                        "",
                        0,
                        3_000_000,
                        false,
                        PreviewQuality::Half,
                        CanvasFormat::Original,
                        |overlay, image| rendered = (overlay.clip_rect(), image),
                    );
                },
            );
            (output, rendered)
        };
        let mut view = PreviewViewport::default();
        let (_, (canvas, fitted)) = frame(&mut view, vec![]);
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, 400.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        };
        let (zoomed, (zoom_clip, zoom_image)) = frame(
            &mut view,
            vec![egui::Event::PointerMoved(fitted.center()), wheel.clone()],
        );
        assert!(view.percent() > 100);
        assert_eq!(zoom_clip, canvas);
        assert!(zoom_image.height() > fitted.height() && !zoom_clip.contains_rect(zoom_image));
        let reset = zoomed
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .iter()
            .find_map(|(_, node)| {
                (node.label() == Some("恢复预览适应大小"))
                    .then(|| node.bounds())
                    .flatten()
            })
            .unwrap();
        let reset = pos2(
            ((reset.x0 + reset.x1) * 0.5) as f32,
            ((reset.y0 + reset.y1) * 0.5) as f32,
        );
        let before = view.percent();
        frame(&mut view, vec![egui::Event::PointerMoved(reset), wheel]);
        assert_eq!(view.percent(), before);
        let click = |pressed| egui::Event::PointerButton {
            pos: reset,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut view, vec![click(true)]);
        frame(&mut view, vec![click(false)]);
        assert!(view.is_fitted());
        let (_, (_, restored)) = frame(&mut view, vec![]);
        assert_eq!(restored, fitted);
    }

    #[test]
    fn file_menu_and_window_controls_share_the_header_at_minimum_width() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let logo = ctx.load_texture(
            "header-test-logo",
            egui::ColorImage::filled([1, 1], Color32::WHITE),
            egui::TextureOptions::NEAREST,
        );
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(760.0, 490.0))),
                ..Default::default()
            },
            |ui| {
                header(
                    ui,
                    &logo,
                    "工程",
                    |ui| {
                        ui.add_sized([56.0, 28.0], egui::Button::new("文件"));
                    },
                    |ui| {
                        export_button(ui, true);
                        icon_button(ui, Icon::Timeline, "工作区", true, ButtonStyle::Quiet);
                    },
                );
            },
        );
        let tree = output.platform_output.accesskit_update.unwrap();
        let mut previous_right = 0.0;
        for label in ["文件", "工作区", "导出 MP4", "最小化", "最大化", "关闭窗口"]
        {
            let bounds = tree
                .nodes
                .iter()
                .find_map(|(_, node)| {
                    (node.label() == Some(label))
                        .then(|| node.bounds())
                        .flatten()
                })
                .unwrap_or_else(|| panic!("缺少按钮 {label}"));
            assert!(bounds.x0 >= previous_right);
            assert!((bounds.y0 + bounds.y1) * 0.5 <= 24.0);
            assert!(bounds.y0 >= 6.0 && bounds.y1 <= 40.0);
            previous_right = bounds.x1;
        }
    }

    #[test]
    fn narrow_monitor_leaves_room_for_seeking_and_exposes_both_selectors() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let window = Rect::from_min_size(egui::Pos2::ZERO, vec2(760.0, 490.0));
        let frame = |events| {
            let mut state = DockState::default();
            let mut preview = Rect::NOTHING;
            let mut transport = TransportInput::default();
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(window),
                    events,
                    ..Default::default()
                },
                |ui| {
                    dock_view::show(ui, &mut state, "预览", |pane, ui| {
                        if pane == Pane::Preview {
                            preview = ui.available_rect_before_wrap();
                            transport = monitor(
                                ui,
                                &mut PreviewViewport::default(),
                                None,
                                None,
                                "",
                                0,
                                12_000_000,
                                false,
                                PreviewQuality::Half,
                                CanvasFormat::Original,
                                |_, _| {},
                            );
                        }
                    });
                },
            );
            (output, transport, preview)
        };

        let (output, _, preview) = frame(vec![]);
        assert!(window.contains_rect(preview));
        let update = output.platform_output.accesskit_update.unwrap();
        let bounds = |update: &egui::accesskit::TreeUpdate, label| {
            let bounds = update
                .nodes
                .iter()
                .find_map(|(_, node)| {
                    (node.label() == Some(label))
                        .then(|| node.bounds())
                        .flatten()
                })
                .unwrap();
            Rect::from_min_max(
                pos2(bounds.x0 as f32, bounds.y0 as f32),
                pos2(bounds.x1 as f32, bounds.y1 as f32),
            )
        };
        let play = bounds(&update, "播放");
        let seek = bounds(&update, "预览进度");
        let quality = bounds(&update, "预览质量");
        let ratio = bounds(&update, "画幅比例");
        assert!(preview.contains_rect(play));
        assert!((play.center().x - preview.center().x).abs() < 1.0);
        assert!(preview.contains_rect(seek), "{seek:?} 不在 {preview:?} 内");
        assert!(seek.width() >= 100.0);
        assert!(seek.left() > play.right());
        assert!((seek.center().y - play.center().y).abs() < 1.0);
        assert!(preview.contains_rect(quality));
        assert!(quality.top() > play.bottom());
        assert!(preview.contains_rect(ratio));
        assert!(ratio.left() > quality.right());
        assert!((ratio.center().y - quality.center().y).abs() < 1.0);

        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let seek_point = pos2(seek.left() + seek.width() * 0.75, seek.center().y);
        let seek_down = frame(vec![
            egui::Event::PointerMoved(seek_point),
            button(seek_point, true),
        ]);
        let seek_up = frame(vec![button(seek_point, false)]);
        assert!(matches!(
            seek_down.1.seek.or(seek_up.1.seek),
            Some(8_500_000..=9_500_000)
        ));
        frame(vec![
            egui::Event::PointerMoved(play.center()),
            button(play.center(), true),
        ]);
        let (_, clicked, _) = frame(vec![button(play.center(), false)]);
        assert!(clicked.toggle);

        frame(vec![
            egui::Event::PointerMoved(ratio.center()),
            button(ratio.center(), true),
        ]);
        frame(vec![button(ratio.center(), false)]);
        let (menu, _, _) = frame(vec![]);
        let portrait = bounds(&menu.platform_output.accesskit_update.unwrap(), "9:16").center();
        frame(vec![
            egui::Event::PointerMoved(portrait),
            button(portrait, true),
        ]);
        let (_, chosen, _) = frame(vec![button(portrait, false)]);
        assert_eq!(
            chosen.canvas_format,
            Some(CanvasFormat::Preset(
                crate::editor::canvas_format::AspectRatio::Portrait
            )),
            "portrait menu item at {portrait:?}"
        );

        frame(vec![
            egui::Event::PointerMoved(ratio.center()),
            button(ratio.center(), true),
        ]);
        frame(vec![button(ratio.center(), false)]);
        let (menu, _, _) = frame(vec![]);
        let custom = bounds(&menu.platform_output.accesskit_update.unwrap(), "自定义…").center();
        frame(vec![
            egui::Event::PointerMoved(custom),
            button(custom, true),
        ]);
        frame(vec![button(custom, false)]);
        let (dialog, _, _) = frame(vec![]);
        let apply = bounds(&dialog.platform_output.accesskit_update.unwrap(), "应用").center();
        frame(vec![egui::Event::PointerMoved(apply), button(apply, true)]);
        let (_, chosen, _) = frame(vec![button(apply, false)]);
        assert_eq!(
            chosen.canvas_format,
            Some(CanvasFormat::Custom {
                width: 1920,
                height: 1080
            })
        );
    }
}
