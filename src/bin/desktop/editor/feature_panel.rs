use std::time::Duration;

use eframe::egui::{
    self, Align2, Color32, FontId, Key, Modifiers, Response, RichText, Sense, Stroke, WidgetInfo,
    WidgetType, pos2, vec2,
};

use crate::ui_icons::{Icon, paint_icon_focus};
use crate::ui_theme::upper_palette;
use takeseam::i18n::{Locale, key};

use super::dock_view::{MATERIALS_GRIP_WIDTH, MATERIALS_TAB_HEIGHT};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum FeatureTab {
    #[default]
    Materials,
    Audio,
    Captions,
    Text,
    Background,
    Transitions,
}

impl FeatureTab {
    const ALL: [Self; 6] = [
        Self::Materials,
        Self::Audio,
        Self::Captions,
        Self::Text,
        Self::Background,
        Self::Transitions,
    ];

    fn label(self, locale: Locale) -> &'static str {
        match self {
            Self::Materials => locale.text(key::MATERIALS),
            Self::Audio => locale.text(key::AUDIO),
            Self::Captions => locale.text(key::CAPTIONS),
            Self::Text => locale.text(key::TEXT),
            Self::Background => locale.text(key::BACKGROUND),
            Self::Transitions => locale.text(key::TRANSITIONS),
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Materials => Icon::Media,
            Self::Audio => Icon::Audio,
            Self::Captions => Icon::Captions,
            Self::Text => Icon::Text,
            Self::Background => Icon::Fit,
            Self::Transitions => Icon::Trim,
        }
    }
}

pub(super) fn tabs(ui: &mut egui::Ui, selected: &mut FeatureTab, narration_active: bool) {
    let strip = ui.available_rect_before_wrap();
    let width =
        ((strip.width() - MATERIALS_GRIP_WIDTH) / FeatureTab::ALL.len() as f32).clamp(0.0, 64.0);
    let font = FontId::proportional(11.0);
    let painter = ui.painter();
    let locale = Locale::from_context(ui.ctx());
    let labels = FeatureTab::ALL.map(|tab| {
        painter.layout_no_wrap(tab.label(locale).into(), font.clone(), Color32::PLACEHOLDER)
    });
    let show_labels = labels.iter().all(|label| label.size().x + 24.0 <= width);
    let icon_scale = (width / 18.0).clamp(0.55, 0.82);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let responses = FeatureTab::ALL.map(|_| {
            ui.allocate_exact_size(vec2(width, MATERIALS_TAB_HEIGHT), Sense::click())
                .1
                .on_hover_cursor(egui::CursorIcon::PointingHand)
        });

        if let Some(index) = responses.iter().position(Response::clicked) {
            *selected = FeatureTab::ALL[index];
        } else if let Some(index) = responses.iter().position(Response::has_focus) {
            let next = ui.input_mut(|input| {
                let len = responses.len();
                if input.consume_key(Modifiers::NONE, Key::ArrowLeft) {
                    Some((index + len - 1) % len)
                } else if input.consume_key(Modifiers::NONE, Key::ArrowRight) {
                    Some((index + 1) % len)
                } else {
                    None
                }
            });
            if let Some(next) = next {
                ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
                responses[next].request_focus();
                *selected = FeatureTab::ALL[next];
            }
        }

        let colors = upper_palette(ui.ctx());
        let baseline = responses[0].rect.bottom() - 0.5;
        ui.painter().hline(
            strip.x_range(),
            baseline,
            Stroke::new(1.0_f32, colors.border),
        );
        ui.painter().vline(
            strip.right() - MATERIALS_GRIP_WIDTH,
            (strip.top() + 8.0)..=(strip.top() + MATERIALS_TAB_HEIGHT - 8.0),
            Stroke::new(1.0_f32, colors.border),
        );
        for ((tab, response), label) in FeatureTab::ALL.into_iter().zip(responses).zip(labels) {
            let active = tab == *selected;
            let busy = tab == FeatureTab::Audio && narration_active;
            let accessible_label = if busy {
                locale.text(key::AUDIO)
            } else {
                tab.label(locale)
            };
            response.widget_info(|| {
                WidgetInfo::selected(WidgetType::SelectableLabel, true, active, accessible_label)
            });

            let rect = response.rect;
            let center = rect.center() - vec2(0.0, 0.5);
            let painter = ui.painter().with_clip_rect(rect);
            let text_color = if active || response.hovered() || response.is_pointer_button_down_on()
            {
                colors.text
            } else {
                colors.muted
            };
            let icon_color = if active { colors.accent } else { text_color };
            if show_labels {
                let size = label.size();
                let left = center.x - (size.x + 17.0) * 0.5;
                tab.icon().paint_scaled(
                    &painter,
                    pos2(left + 6.5, center.y),
                    icon_color,
                    icon_scale,
                );
                painter.galley(
                    pos2(left + 17.0, center.y - size.y * 0.5),
                    label,
                    text_color,
                );
            } else {
                tab.icon()
                    .paint_scaled(&painter, center, icon_color, icon_scale);
            }
            if active {
                let half = (rect.width() * 0.33).min(10.0);
                painter.hline(
                    center.x - half..=center.x + half,
                    baseline,
                    Stroke::new(2.0_f32, colors.accent),
                );
            }
            if response.has_focus() {
                paint_icon_focus(&painter, pos2(center.x, rect.top() + 2.0), colors.text);
            }
            if busy {
                painter.circle_filled(
                    pos2(rect.right() - 6.0, rect.top() + 6.0),
                    2.0,
                    colors.recording,
                );
            }
            response.on_hover_text(accessible_label);
        }
    });
}

pub(super) fn action_row(ui: &mut egui::Ui, icon: Icon, label: &str) -> bool {
    let colors = upper_palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let hovered = response.hovered() || response.is_pointer_button_down_on();
    let glyph = pos2(rect.left() + 8.0, rect.center().y);
    let painter = ui.painter().with_clip_rect(rect);
    if hovered {
        painter.rect_filled(rect, 4, colors.hover);
    }
    let icon_color = if hovered { colors.text } else { colors.muted };
    icon.paint_scaled(&painter, glyph, icon_color, 0.88);
    painter.text(
        glyph + vec2(16.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(12.0),
        colors.text,
    );
    if response.has_focus() {
        paint_icon_focus(&painter, pos2(glyph.x, rect.bottom() - 2.0), colors.accent);
    }
    response.clicked()
}

#[derive(Clone, Copy)]
pub(super) enum NarrationView {
    Idle,
    Starting,
    Recording(Duration),
    Saving,
}

pub(super) enum NarrationAction {
    Start,
    Stop,
}

pub(super) fn narration_controls(
    ui: &mut egui::Ui,
    narration: NarrationView,
    error: Option<&str>,
) -> Option<NarrationAction> {
    let colors = upper_palette(ui.ctx());
    let (label, action) = match narration {
        NarrationView::Idle => (
            Locale::from_context(ui.ctx()).text(key::NARRATION_START),
            Some(NarrationAction::Start),
        ),
        NarrationView::Starting => (
            Locale::from_context(ui.ctx()).text(key::NARRATION_STARTING),
            None,
        ),
        NarrationView::Recording(_) => (
            Locale::from_context(ui.ctx()).text(key::RECORD_STOP),
            Some(NarrationAction::Stop),
        ),
        NarrationView::Saving => (
            Locale::from_context(ui.ctx()).text(key::NARRATION_SAVING),
            None,
        ),
    };

    let busy = action.is_none();
    let sense = if busy {
        Sense::focusable_noninteractive()
    } else {
        Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), sense);
    let foreground = if busy {
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, label));
        colors.muted
    } else if response.hovered() || response.is_pointer_button_down_on() {
        colors.accent
    } else {
        colors.text
    };
    if !busy {
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
        response
            .clone()
            .on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    let center = pos2(rect.left() + 8.0, rect.center().y);
    let time = if let NarrationView::Recording(elapsed) = narration {
        Icon::Stop.paint_scaled(ui.painter(), center, colors.recording, 0.82);
        ui.ctx().request_repaint_after(
            Duration::from_secs(1) - Duration::from_nanos(elapsed.subsec_nanos().into()),
        );
        Some(narration_time(elapsed))
    } else {
        if busy {
            ui.painter()
                .circle_stroke(center, 3.0, Stroke::new(1.25_f32, colors.muted));
        } else {
            ui.painter().circle_filled(center, 3.5, colors.recording);
        }
        None
    };
    let painted_label = ui.painter().text(
        pos2(center.x + 16.0, center.y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(12.0),
        foreground,
    );
    if let Some(time) = time {
        let font = FontId::monospace(11.0);
        let timer = ui.painter().layout_no_wrap(time, font, colors.muted);
        let right = rect.right() - 4.0;
        if right - timer.size().x >= painted_label.right() + 4.0 {
            ui.painter().galley(
                pos2(right - timer.size().x, center.y - timer.size().y / 2.0),
                timer,
                colors.muted,
            );
        } else {
            ui.label(timer);
        }
    }
    if response.has_focus() {
        paint_icon_focus(
            ui.painter(),
            pos2(center.x, rect.bottom() - 3.0),
            colors.accent,
        );
    }
    if let Some(error) = error {
        ui.add(egui::Label::new(RichText::new(error).size(11.0).color(colors.danger)).truncate())
            .on_hover_text(error);
    }
    if response.clicked() { action } else { None }
}

fn narration_time(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let hours = seconds / 3600;
    let minutes = (seconds / 60) % 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}
