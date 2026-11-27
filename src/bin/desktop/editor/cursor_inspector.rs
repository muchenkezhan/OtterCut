use std::ops::RangeInclusive;

use eframe::egui::{
    self, Align, Button, Color32, FontId, Frame, Id, Rect, RichText, Sense, Stroke, StrokeKind,
    TextWrapMode, Ui, WidgetInfo, WidgetType, pos2, vec2,
};
use takeseam::i18n::{Locale, MessageKey, key};

use crate::{ui_icons::Icon, ui_theme};

use super::cursor_effect::{
    CursorClickEffect, CursorEffect, CursorStyle, ZOOM_HOLD_RANGE, ZOOM_PAN_RANGE,
    ZOOM_SCALE_RANGE, ZOOM_TRANSITION_RANGE,
};

const ROW_HEIGHT: f32 = 26.0;
const FIELD_WIDTH: f32 = 86.0;
const MICROS_PER_SECOND: f64 = 1_000_000.0;

pub(super) fn show(ui: &mut Ui, effect: &mut CursorEffect, duration: u64) {
    let locale = Locale::from_context(ui.ctx());
    let colors = ui_theme::palette(ui.ctx());
    let tab_id = ui.make_persistent_id("cursor-inspector-tab");
    let mut animation = ui
        .data(|data| data.get_temp::<bool>(tab_id))
        .unwrap_or(true);

    property_row(ui, locale.text(key::CURSOR_EFFECT), 34.0, |ui, label| {
        let (rect, mut response) = ui.allocate_exact_size(vec2(32.0, 18.0), Sense::click());
        if response.clicked() {
            effect.enabled = !effect.enabled;
            response.mark_changed();
        }
        response.widget_info(|| {
            WidgetInfo::selected(
                WidgetType::Checkbox,
                ui.is_enabled(),
                effect.enabled,
                locale.text(key::CURSOR_EFFECT),
            )
        });
        let track = if effect.enabled {
            colors.accent_fill
        } else {
            colors.control
        };
        let painter = ui.painter();
        painter.rect_filled(rect, 9.0, track);
        painter.rect_stroke(
            rect,
            9.0,
            Stroke::new(
                1.0_f32,
                if response.hovered() || response.has_focus() {
                    colors.accent
                } else {
                    colors.border
                },
            ),
            StrokeKind::Inside,
        );
        let x = if effect.enabled {
            rect.right() - 9.0
        } else {
            rect.left() + 9.0
        };
        let thumb = if effect.enabled {
            Color32::WHITE
        } else {
            colors.muted
        };
        painter.circle_filled(pos2(x, rect.center().y), 5.5, thumb);
        response
            .labelled_by(label)
            .on_hover_cursor(egui::CursorIcon::PointingHand);
    });

    if let Some(next) = choices(
        ui,
        &[
            (false, locale.text(key::CURSOR_APPEARANCE)),
            (true, locale.text(key::CURSOR_ANIMATION)),
        ],
        animation,
    ) {
        animation = next;
        ui.data_mut(|data| data.insert_temp(tab_id, animation));
    }
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .id_salt(("cursor-properties", animation))
        .auto_shrink([false, false])
        .scroll_source(
            egui::scroll_area::ScrollSource::SCROLL_BAR
                | egui::scroll_area::ScrollSource::MOUSE_WHEEL,
        )
        .show(ui, |ui| {
            if animation {
                show_animation(ui, effect, locale);
                show_range(ui, effect, duration, locale);
            } else {
                show_appearance(ui, effect, locale);
            }
        });
}

fn section(ui: &mut Ui, title: MessageKey, locale: Locale, body: impl FnOnce(&mut Ui)) {
    let colors = ui_theme::palette(ui.ctx());
    ui.scope(|ui| {
        ui.visuals_mut().collapsing_header_frame = true;
        ui.visuals_mut().widgets.inactive.weak_bg_fill = colors.control;
        ui.spacing_mut().interact_size.y = 29.0;
        egui::CollapsingHeader::new(RichText::new(locale.text(title)).strong())
            .id_salt(title.0)
            .default_open(true)
            .show_unindented(ui, |ui| {
                Frame::new()
                    .inner_margin(egui::Margin::symmetric(6, 4))
                    .show(ui, body);
            });
    });
}

fn choices<T: Copy + PartialEq>(ui: &mut Ui, options: &[(T, &str)], current: T) -> Option<T> {
    let colors = ui_theme::palette(ui.ctx());
    let mut change = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let width =
            (ui.available_width() - (options.len() - 1) as f32 * 3.0) / options.len() as f32;
        for &(value, label) in options {
            let selected = value == current;
            let button = Button::selectable(selected, label)
                .frame_when_inactive(true)
                .fill(if selected {
                    colors.selection
                } else {
                    colors.control
                })
                .stroke(Stroke::new(
                    1.0_f32,
                    if selected {
                        colors.accent
                    } else {
                        colors.border
                    },
                ))
                .wrap_mode(TextWrapMode::Wrap);
            if ui
                .add_sized([width.max(1.0), ROW_HEIGHT], button)
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                change = Some(value);
            }
        }
    });
    change
}

fn property_row(ui: &mut Ui, label: &str, control_width: f32, control: impl FnOnce(&mut Ui, Id)) {
    let colors = ui_theme::palette(ui.ctx());
    let gap = ui.spacing().item_spacing.x;
    let font = egui::TextStyle::Body.resolve(ui.style());
    let label_width = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, colors.text)
        .size()
        .x;
    let width = ui.available_width();
    let text = RichText::new(label).color(colors.muted);
    if width >= label_width + gap + control_width {
        let control_left = ui.available_rect_before_wrap().right() - control_width;
        ui.horizontal(|ui| {
            ui.spacing_mut().interact_size.y = ROW_HEIGHT;
            let label = ui.label(text).id;
            ui.add_space((control_left - ui.next_widget_position().x).max(0.0));
            control(ui, label);
        });
    } else {
        let label = ui.add(egui::Label::new(text).wrap()).id;
        ui.horizontal(|ui| {
            ui.add_space((width - control_width).max(0.0));
            control(ui, label);
        });
    }
}

fn number(
    ui: &mut Ui,
    label: &str,
    value: &mut f64,
    range: RangeInclusive<f64>,
    decimals: usize,
    unit: &str,
    help: &str,
) -> bool {
    let colors = ui_theme::palette(ui.ctx());
    let mut changed = false;
    let width = FIELD_WIDTH.min(ui.available_width());
    property_row(ui, label, width, |ui, label_id| {
        Frame::new()
            .fill(colors.field)
            .corner_radius(5.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let unit_width = if unit.is_empty() { 0.0 } else { 23.0 };
                    let input_width = (width - unit_width).max(1.0);
                    changed = ui
                        .add_sized(
                            [input_width, ROW_HEIGHT],
                            egui::DragValue::new(value)
                                .range(range)
                                .clamp_existing_to_range(false)
                                .speed(10.0_f64.powi(-(decimals as i32)))
                                .fixed_decimals(decimals),
                        )
                        .labelled_by(label_id)
                        .on_hover_text(help)
                        .changed();
                    if !unit.is_empty() {
                        ui.add_sized(
                            [unit_width, ROW_HEIGHT],
                            egui::Label::new(RichText::new(unit).color(colors.muted).size(10.0))
                                .halign(Align::Center),
                        );
                    }
                });
            });
    });
    changed
}

fn seconds(
    ui: &mut Ui,
    label: MessageKey,
    value: &mut u64,
    range: RangeInclusive<u64>,
    decimals: usize,
    locale: Locale,
) {
    let mut seconds = *value as f64 / MICROS_PER_SECOND;
    if number(
        ui,
        locale.text(label),
        &mut seconds,
        *range.start() as f64 / MICROS_PER_SECOND..=*range.end() as f64 / MICROS_PER_SECOND,
        decimals,
        locale.text(key::CURSOR_SECONDS),
        locale.text(key::CURSOR_NUMBER_HELP),
    ) {
        *value = ((seconds * MICROS_PER_SECOND).round() as u64).clamp(*range.start(), *range.end());
    }
}

fn show_animation(ui: &mut Ui, effect: &mut CursorEffect, locale: Locale) {
    section(ui, key::CURSOR_ZOOM, locale, |ui| {
        let mut scale = f64::from(effect.zoom_scale);
        if number(
            ui,
            locale.text(key::CURSOR_SCALE),
            &mut scale,
            f64::from(*ZOOM_SCALE_RANGE.start())..=f64::from(*ZOOM_SCALE_RANGE.end()),
            2,
            "×",
            locale.text(key::CURSOR_NUMBER_HELP),
        ) {
            effect.zoom_scale = scale as f32;
        }
        ui.label(
            RichText::new(locale.text(key::CURSOR_FOLLOW)).color(ui_theme::palette(ui.ctx()).muted),
        );
        if let Some(follow) = choices(
            ui,
            &[
                (true, locale.text(key::CURSOR_FOLLOW_POINTER)),
                (false, locale.text(key::CURSOR_FOLLOW_CLICKS)),
            ],
            effect.zoom_follow_cursor,
        ) {
            effect.zoom_follow_cursor = follow;
        }
        if effect.zoom_follow_cursor {
            let mut smoothing = f64::from(effect.smoothing) * 100.0;
            if number(
                ui,
                locale.text(key::CURSOR_SMOOTHING),
                &mut smoothing,
                0.0..=100.0,
                0,
                "%",
                locale.text(key::CURSOR_NUMBER_HELP),
            ) {
                effect.smoothing = (smoothing / 100.0) as f32;
            }
        } else {
            seconds(
                ui,
                key::CURSOR_PAN,
                &mut effect.zoom_pan_us,
                ZOOM_PAN_RANGE,
                2,
                locale,
            );
        }
        seconds(
            ui,
            key::CURSOR_HOLD,
            &mut effect.zoom_hold_us,
            ZOOM_HOLD_RANGE,
            2,
            locale,
        );
    });

    section(ui, key::CURSOR_TRANSITION, locale, |ui| {
        if let Some(automatic) = choices(
            ui,
            &[
                (true, locale.text(key::CURSOR_AUTO)),
                (false, locale.text(key::CURSOR_CUSTOM)),
            ],
            effect.zoom_transition_us.is_none(),
        ) {
            effect.zoom_transition_us = (!automatic).then(|| effect.zoom_transition_us());
        }
        if let Some(mut duration) = effect.zoom_transition_us {
            seconds(
                ui,
                key::CURSOR_TRANSITION_TIME,
                &mut duration,
                ZOOM_TRANSITION_RANGE,
                2,
                locale,
            );
            effect.zoom_transition_us = Some(duration);
        } else {
            let colors = ui_theme::palette(ui.ctx());
            for (label, duration) in [
                (key::CURSOR_ZOOM_IN, effect.zoom_in_us()),
                (key::CURSOR_ZOOM_OUT, effect.zoom_transition_us()),
            ] {
                property_row(ui, locale.text(label), FIELD_WIDTH, |ui, _| {
                    ui.add_sized(
                        [FIELD_WIDTH.min(ui.available_width()), ROW_HEIGHT],
                        egui::Label::new(
                            RichText::new(format!(
                                "{:.2} {}",
                                duration as f64 / MICROS_PER_SECOND,
                                locale.text(key::CURSOR_SECONDS)
                            ))
                            .color(colors.muted),
                        )
                        .halign(Align::Center),
                    );
                });
            }
        }
    });
}

fn show_range(ui: &mut Ui, effect: &mut CursorEffect, duration: u64, locale: Locale) {
    section(ui, key::CURSOR_RANGE, locale, |ui| {
        let end = effect.end_us.clamp(1, duration);
        seconds(
            ui,
            key::CURSOR_START,
            &mut effect.start_us,
            0..=end - 1,
            3,
            locale,
        );
        let start = effect.start_us.min(duration - 1);
        seconds(
            ui,
            key::CURSOR_END,
            &mut effect.end_us,
            start + 1..=duration,
            3,
            locale,
        );
    });
}

fn show_appearance(ui: &mut Ui, effect: &mut CursorEffect, locale: Locale) {
    section(ui, key::CURSOR_LOOK, locale, |ui| {
        let mut size = f64::from(effect.cursor_scale) * 100.0;
        if number(
            ui,
            locale.text(key::CURSOR_SIZE),
            &mut size,
            50.0..=400.0,
            0,
            "%",
            locale.text(key::CURSOR_NUMBER_HELP),
        ) {
            effect.cursor_scale = (size / 100.0) as f32;
        }
        ui.label(
            RichText::new(locale.text(key::CURSOR_STYLE)).color(ui_theme::palette(ui.ctx()).muted),
        );
        cursor_styles(ui, &mut effect.cursor_style, locale);
    });
    section(ui, key::CURSOR_FEEDBACK, locale, |ui| {
        if let Some(feedback) = choices(
            ui,
            &[
                (CursorClickEffect::Ring, locale.text(key::CURSOR_RING)),
                (CursorClickEffect::Dot, locale.text(key::CURSOR_DOT)),
                (CursorClickEffect::None, locale.text(key::CURSOR_NONE)),
            ],
            effect.click_effect,
        ) {
            effect.click_effect = feedback;
        }
    });
}

fn cursor_styles(ui: &mut Ui, selected: &mut CursorStyle, locale: Locale) {
    let key = Id::new("cursor-style-theme-previews");
    let textures = ui
        .data(|data| data.get_temp::<Vec<(CursorStyle, egui::TextureHandle)>>(key))
        .unwrap_or_else(|| {
            let textures: Vec<_> = CursorStyle::ALL
                .into_iter()
                .enumerate()
                .filter_map(|(index, style)| {
                    let sprite = style.sprite()?;
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [sprite.width as usize, sprite.height as usize],
                        &sprite.rgba,
                    );
                    let filter = if style == CursorStyle::Pixel {
                        egui::TextureOptions::NEAREST
                    } else {
                        egui::TextureOptions::LINEAR
                    };
                    Some((
                        style,
                        ui.ctx()
                            .load_texture(format!("cursor-style-{index}"), image, filter),
                    ))
                })
                .collect();
            ui.data_mut(|data| data.insert_temp(key, textures.clone()));
            textures
        });
    let colors = ui_theme::palette(ui.ctx());
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        let width = ui.available_width().min(61.0);
        for style in CursorStyle::ALL {
            let label = locale.text(match style {
                CursorStyle::Default => key::CURSOR_STYLE_DEFAULT,
                CursorStyle::StudioInk => key::CURSOR_STYLE_STUDIO_INK,
                CursorStyle::Prism => key::CURSOR_STYLE_PRISM,
                CursorStyle::Coral => key::CURSOR_STYLE_CORAL,
                CursorStyle::Pixel => key::CURSOR_STYLE_PIXEL,
                CursorStyle::StarSprout => key::CURSOR_STYLE_STAR_SPROUT,
                CursorStyle::GlassLens => key::CURSOR_STYLE_GLASS_LENS,
            });
            let (rect, response) = ui.allocate_exact_size(vec2(width, 51.0), Sense::click());
            let active = *selected == style;
            let fill = if active {
                colors.selection
            } else if response.hovered() || response.has_focus() {
                colors.hover
            } else {
                colors.control
            };
            let border = if active || response.has_focus() {
                colors.accent
            } else {
                colors.border
            };
            ui.painter().rect(
                rect,
                5.0,
                fill,
                Stroke::new(1.0_f32, border),
                StrokeKind::Inside,
            );
            let preview =
                Rect::from_center_size(rect.center_top() + vec2(0.0, 18.0), vec2(30.0, 30.0));
            if let Some((_, texture)) = textures.iter().find(|(candidate, _)| *candidate == style) {
                ui.painter().image(
                    texture.id(),
                    preview,
                    Rect::from_min_max(egui::Pos2::ZERO, pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                Icon::Cursor.paint_scaled(ui.painter(), preview.center(), colors.text, 1.2);
            }
            let mut job = egui::text::LayoutJob::simple(
                label.to_owned(),
                FontId::proportional(10.0),
                colors.text,
                width - 4.0,
            );
            job.wrap.max_rows = 1;
            let galley = ui.painter().layout_job(job);
            let position = pos2(
                rect.center().x - galley.size().x / 2.0,
                rect.bottom() - 5.0 - galley.size().y,
            );
            ui.painter()
                .with_clip_rect(rect)
                .galley(position, galley, colors.text);
            // Painting uses the full label for accessibility and hover even when the tile is narrow.
            response.widget_info(|| {
                WidgetInfo::selected(WidgetType::Button, ui.is_enabled(), active, label)
            });
            if response
                .on_hover_text(label)
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                *selected = style;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_inspector_keeps_the_zoom_input_visible_and_draggable() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        Locale::set_context(&ctx, takeseam::i18n::Language::EnUs);
        let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(96.0, 420.0));
        let mut effect = CursorEffect::default_for(10_000_000);
        let mut frame = |events| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |ui| show(ui, &mut effect, 10_000_000),
            )
        };
        let bounds = frame(vec![])
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .into_iter()
            .find_map(|(_, node)| {
                (node.role() == egui::accesskit::Role::SpinButton
                    && node.numeric_value() == Some(f64::from(1.8_f32)))
                .then(|| node.bounds())
                .flatten()
            })
            .unwrap();
        let field = Rect::from_min_max(
            pos2(bounds.x0 as f32, bounds.y0 as f32),
            pos2(bounds.x1 as f32, bounds.y1 as f32),
        );
        assert!(screen.contains_rect(field));
        let start = field.center();
        let end = start + vec2(24.0, 0.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(start), button(start, true)]);
        frame(vec![egui::Event::PointerMoved(end)]);
        frame(vec![button(end, false)]);
        assert!(effect.zoom_scale > 1.8);
    }
}
