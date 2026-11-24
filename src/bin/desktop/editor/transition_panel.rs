use eframe::egui::{
    self, Align2, Color32, DragAndDrop, Event, FontId, PointerButton, Pos2, RawInput, Rect,
    RichText, Sense, Stroke, StrokeKind, WidgetInfo, WidgetType, pos2, vec2,
};

use crate::ui_theme::upper_palette;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Category {
    All,
    Basic,
    Dissolve,
    Zoom,
    Slide,
    Spin,
    Glitch,
    Wipe,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TransitionRole {
    Entrance,
    Exit,
    Cut,
}

impl TransitionRole {
    const ALL: [Self; 3] = [Self::Entrance, Self::Exit, Self::Cut];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Entrance => "入场",
            Self::Exit => "出场",
            Self::Cut => "转场",
        }
    }
}

impl Default for Category {
    fn default() -> Self {
        Self::All
    }
}

impl Category {
    const ALL: [Self; 9] = [
        Self::All,
        Self::Basic,
        Self::Dissolve,
        Self::Zoom,
        Self::Slide,
        Self::Spin,
        Self::Glitch,
        Self::Wipe,
        Self::Light,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Basic => "基础",
            Self::Dissolve => "溶解",
            Self::Zoom => "缩放",
            Self::Slide => "滑动",
            Self::Spin => "旋转",
            Self::Glitch => "故障",
            Self::Wipe => "擦除",
            Self::Light => "光效",
        }
    }
}

#[derive(Clone, Copy)]
struct Preset {
    id: &'static str,
    name: &'static str,
    category: Category,
    role: TransitionRole,
    kind: PreviewKind,
}

#[derive(Clone, Copy)]
enum PreviewKind {
    FadeInBlack,
    FadeOutBlack,
    FadeBlack,
    Dissolve,
    Blur,
    Zoom,
    Slide,
    Spin,
    Glitch,
    Wipe,
    Light,
}

const PRESETS: [Preset; 24] = [
    Preset { id: "fade-in-black", name: "黑场淡入", category: Category::Basic, role: TransitionRole::Entrance, kind: PreviewKind::FadeInBlack },
    Preset { id: "fade-out-black", name: "黑场淡出", category: Category::Basic, role: TransitionRole::Exit, kind: PreviewKind::FadeOutBlack },
    Preset { id: "fade-black", name: "黑场淡入淡出", category: Category::Basic, role: TransitionRole::Cut, kind: PreviewKind::FadeBlack },
    Preset { id: "concat.dissolve", name: "溶解", category: Category::Dissolve, role: TransitionRole::Cut, kind: PreviewKind::Dissolve },
    Preset { id: "concat.blur-dissolve", name: "模糊溶解", category: Category::Dissolve, role: TransitionRole::Cut, kind: PreviewKind::Blur },
    Preset { id: "concat.film-burn", name: "胶片燃烧", category: Category::Dissolve, role: TransitionRole::Cut, kind: PreviewKind::Light },
    Preset { id: "concat.zoom-transition", name: "缩放", category: Category::Zoom, role: TransitionRole::Cut, kind: PreviewKind::Zoom },
    Preset { id: "concat.zoom-blur-transition", name: "缩放模糊", category: Category::Zoom, role: TransitionRole::Cut, kind: PreviewKind::Blur },
    Preset { id: "concat.push", name: "推入", category: Category::Slide, role: TransitionRole::Cut, kind: PreviewKind::Slide },
    Preset { id: "concat.slide", name: "滑动", category: Category::Slide, role: TransitionRole::Cut, kind: PreviewKind::Slide },
    Preset { id: "concat.spin", name: "旋转", category: Category::Spin, role: TransitionRole::Cut, kind: PreviewKind::Spin },
    Preset { id: "concat.cube", name: "立方体", category: Category::Spin, role: TransitionRole::Cut, kind: PreviewKind::Spin },
    Preset { id: "concat.swirl", name: "漩涡", category: Category::Spin, role: TransitionRole::Cut, kind: PreviewKind::Spin },
    Preset { id: "concat.glitch-transition", name: "故障", category: Category::Glitch, role: TransitionRole::Cut, kind: PreviewKind::Glitch },
    Preset { id: "concat.glitch-warp", name: "故障扭曲", category: Category::Glitch, role: TransitionRole::Cut, kind: PreviewKind::Glitch },
    Preset { id: "concat.rgb-split-wipe", name: "色彩分离", category: Category::Glitch, role: TransitionRole::Cut, kind: PreviewKind::Glitch },
    Preset { id: "concat.linear-wipe", name: "线性擦除", category: Category::Wipe, role: TransitionRole::Cut, kind: PreviewKind::Wipe },
    Preset { id: "concat.clock-wipe", name: "时钟擦除", category: Category::Wipe, role: TransitionRole::Cut, kind: PreviewKind::Wipe },
    Preset { id: "concat.iris", name: "虹膜", category: Category::Wipe, role: TransitionRole::Cut, kind: PreviewKind::Wipe },
    Preset { id: "concat.shape-wipe", name: "形状擦除", category: Category::Wipe, role: TransitionRole::Cut, kind: PreviewKind::Wipe },
    Preset { id: "concat.whip-pan", name: "甩镜", category: Category::Wipe, role: TransitionRole::Cut, kind: PreviewKind::Spin },
    Preset { id: "concat.light-leak-flash", name: "漏光闪现", category: Category::Light, role: TransitionRole::Cut, kind: PreviewKind::Light },
    Preset { id: "concat.light-leak-transition", name: "漏光", category: Category::Light, role: TransitionRole::Cut, kind: PreviewKind::Light },
    Preset { id: "concat.fade-white", name: "白场淡入淡出", category: Category::Basic, role: TransitionRole::Cut, kind: PreviewKind::FadeBlack },
];

#[derive(Default)]
pub(super) struct State {
    category: Category,
    role: Option<TransitionRole>,
    search: String,
    selected: Option<&'static str>,
    card_hits: Vec<CardHit>,
    press: Option<CardPress>,
    released_drag: bool,
}

struct CardHit {
    rect: Rect,
    id: &'static str,
    name: &'static str,
    role: TransitionRole,
}

struct CardPress {
    origin: Pos2,
    id: &'static str,
    name: &'static str,
    role: TransitionRole,
    started: bool,
}

#[derive(Clone, Copy)]
pub(super) struct DraggedTransition {
    pub(super) id: &'static str,
    pub(super) name: &'static str,
    pub(super) role: TransitionRole,
}

#[derive(Clone, Copy)]
pub(super) struct TransitionChoice {
    pub(super) id: &'static str,
    pub(super) name: &'static str,
    pub(super) role: TransitionRole,
}

impl State {
    pub(super) fn process(&mut self, ctx: &egui::Context, raw: &RawInput) {
        self.released_drag = false;
        if !raw.focused {
            self.cancel_drag(ctx);
            return;
        }
        let threshold = ctx.options(|options| options.input_options.max_click_dist);
        for event in &raw.events {
            match *event {
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: true,
                    ..
                } => {
                    self.cancel_drag(ctx);
                    self.press = self
                        .card_hits
                        .iter()
                        .rev()
                        .find(|card| card.rect.contains(pos))
                        .map(|card| {
                            // Reserve the gesture before the pointer reaches the
                            // timeline so normal clip dragging cannot steal it.
                            DragAndDrop::set_payload(
                                ctx,
                                DraggedTransition {
                                    id: card.id,
                                    name: card.name,
                                    role: card.role,
                                },
                            );
                            CardPress {
                                origin: pos,
                                id: card.id,
                                name: card.name,
                                role: card.role,
                                started: false,
                            }
                        });
                }
                Event::PointerMoved(pos) => self.move_drag(ctx, pos, threshold),
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: false,
                    ..
                } => {
                    self.move_drag(ctx, pos, threshold);
                    self.released_drag |= self.press.take().is_some_and(|press| press.started);
                }
                Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                } => self.cancel_drag(ctx),
                Event::PointerGone if self.press.is_some() => self.cancel_drag(ctx),
                _ => {}
            }
        }
    }

    fn move_drag(&mut self, ctx: &egui::Context, pos: Pos2, threshold: f32) {
        if let Some(press) = &mut self.press
            && !press.started
            && press.origin.distance_sq(pos) > threshold * threshold
        {
            press.started = true;
            DragAndDrop::set_payload(
                ctx,
                DraggedTransition {
                    id: press.id,
                    name: press.name,
                    role: press.role,
                },
            );
        }
    }

    fn cancel_drag(&mut self, ctx: &egui::Context) {
        self.press = None;
        if DragAndDrop::has_payload_of_type::<DraggedTransition>(ctx) {
            DragAndDrop::clear_payload(ctx);
        }
    }

    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        selected_clip: Option<u64>,
    ) -> Option<TransitionChoice> {
        let colors = upper_palette(ui.ctx());
        let mut chosen = None;
        self.card_hits.clear();
        let suppress_click = self.released_drag;
        let panel_height = ui.available_height().max(0.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.vertical(|ui| {
                ui.set_width(84.0);
                ui.set_min_height(panel_height);
                ui.label(RichText::new("转场").size(12.0).strong().color(colors.text));
                ui.add_space(5.0);
                for category in Category::ALL {
                    let selected = self.category == category;
                    if ui
                        .selectable_label(selected, category.label())
                        .clicked()
                    {
                        self.category = category;
                    }
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(panel_height);
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("搜索转场")
                        .desired_width(ui.available_width()),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    for role in TransitionRole::ALL {
                        let selected = self.role == Some(role);
                        if ui.selectable_label(selected, role.label()).clicked() {
                            self.role = Some(role);
                        }
                    }
                });
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .id_salt("transition-gallery")
                    .auto_shrink([false, false])
                    .max_height((panel_height - 38.0).max(80.0))
                    .show(ui, |ui| {
                        let query = self.search.trim().to_lowercase();
                        let list = PRESETS.iter().filter(|preset| {
                            (self.category == Category::All || preset.category == self.category)
                                && self.role.is_none_or(|role| preset.role == role)
                                && (query.is_empty()
                                    || preset.name.to_lowercase().contains(&query))
                        });
                        ui.columns(2, |columns| {
                            for (index, preset) in list.enumerate() {
                                let column = &mut columns[index % 2];
                                if transition_card(
                                    column,
                                    *preset,
                                    self.selected == Some(preset.id),
                                    &mut self.card_hits,
                                    suppress_click,
                                ) {
                                    self.selected = Some(preset.id);
                                    chosen = Some(TransitionChoice {
                                        id: preset.id,
                                        name: preset.name,
                                        role: preset.role,
                                    });
                                }
                            }
                        });
                    });
                if selected_clip.is_none() {
                    ui.label(
                        RichText::new("选择时间线片段后应用转场")
                            .size(10.0)
                            .color(colors.muted),
                    );
                }
            });
        });
        chosen
    }
}

fn transition_card(
    ui: &mut egui::Ui,
    preset: Preset,
    selected: bool,
    card_hits: &mut Vec<CardHit>,
    suppress_click: bool,
) -> bool {
    let colors = upper_palette(ui.ctx());
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), 116.0), Sense::click_and_drag());
    card_hits.push(CardHit {
        rect,
        id: preset.id,
        name: preset.name,
        role: preset.role,
    });
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, preset.name));
    response.dnd_set_drag_payload(DraggedTransition {
        id: preset.id,
        name: preset.name,
        role: preset.role,
    });
    let stroke = if selected {
        Stroke::new(1.5, colors.accent)
    } else if response.hovered() {
        Stroke::new(1.0, colors.text)
    } else {
        Stroke::new(1.0, colors.border)
    };
    ui.painter().rect_filled(rect, 6.0, colors.field);
    ui.painter().rect_stroke(rect, 6.0, stroke, StrokeKind::Inside);
    let preview = rect.shrink2(vec2(6.0, 26.0));
    paint_preview(ui.painter(), preview, preset.kind);
    ui.painter().text(
        pos2(rect.left() + 7.0, rect.bottom() - 13.0),
        Align2::LEFT_CENTER,
        preset.name,
        FontId::proportional(10.0),
        colors.text,
    );
    let clicked = response.clicked() && !suppress_click;
    response.on_hover_cursor(egui::CursorIcon::Grab);
    clicked
}

fn paint_preview(painter: &egui::Painter, rect: egui::Rect, kind: PreviewKind) {
    let left = Color32::from_rgb(44, 92, 140);
    let right = Color32::from_rgb(202, 92, 106);
    painter.rect_filled(rect, 4.0, left);
    match kind {
        PreviewKind::FadeInBlack => {
            painter.rect_filled(rect, 4.0, Color32::from_black_alpha(150));
            painter.rect_filled(rect.shrink2(vec2(rect.width() * 0.28, 0.0)), 4.0, left);
        }
        PreviewKind::FadeOutBlack => {
            painter.rect_filled(rect, 4.0, Color32::from_black_alpha(150));
            painter.rect_filled(rect.shrink2(vec2(rect.width() * 0.28, 0.0)), 4.0, right);
        }
        PreviewKind::FadeBlack => {
            painter.rect_filled(rect, 4.0, Color32::from_black_alpha(170));
        }
        PreviewKind::Dissolve => {
            painter.rect_filled(rect.shrink(6.0), 4.0, right);
            painter.rect_filled(rect.shrink2(vec2(rect.width() * 0.28, 0.0)), 4.0, left);
        }
        PreviewKind::Blur => {
            painter.rect_filled(rect, 4.0, Color32::from_rgb(137, 157, 181));
            painter.rect_filled(rect.shrink2(vec2(16.0, 8.0)), 4.0, right.gamma_multiply(0.7));
        }
        PreviewKind::Zoom => {
            painter.rect_filled(rect, 4.0, right);
            painter.rect_stroke(rect.shrink(10.0), 3.0, Stroke::new(3.0, left), StrokeKind::Inside);
            painter.rect_stroke(rect.shrink(18.0), 3.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
        }
        PreviewKind::Slide => {
            painter.rect_filled(rect, 4.0, right);
            let incoming = rect.translate(vec2(-rect.width() * 0.32, 0.0));
            painter.rect_filled(incoming, 4.0, left);
        }
        PreviewKind::Spin => {
            painter.rect_filled(rect, 4.0, right);
            painter.circle_stroke(rect.center(), rect.height() * 0.28, Stroke::new(3.0, left));
            painter.line_segment([rect.left_center(), rect.right_center()], Stroke::new(2.0, Color32::WHITE));
        }
        PreviewKind::Glitch => {
            painter.rect_filled(rect, 4.0, Color32::from_rgb(42, 66, 95));
            for (index, color) in [left, right, Color32::from_rgb(234, 211, 91)].into_iter().enumerate() {
                let y = rect.top() + 8.0 + index as f32 * 10.0;
                painter.rect_filled(Rect::from_min_max(pos2(rect.left() + 4.0, y), pos2(rect.right() - 12.0 + index as f32 * 4.0, y + 5.0)), 1.0, color);
            }
        }
        PreviewKind::Wipe => {
            painter.rect_filled(rect, 4.0, left);
            painter.rect_filled(Rect::from_min_max(pos2(rect.center().x, rect.top()), rect.max), 4.0, right);
        }
        PreviewKind::Light => {
            painter.rect_filled(rect, 4.0, Color32::from_rgb(228, 156, 73));
            painter.rect_filled(rect.shrink2(vec2(rect.width() * 0.32, 0.0)), 4.0, Color32::from_rgb(250, 224, 140));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, PointerButton, Rect, RawInput};

    #[test]
    fn dragging_a_transition_card_publishes_a_payload() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut state = State::default();
        let mut frame = |events| {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(320.0, 300.0))),
                events,
                ..RawInput::default()
            };
            state.process(&ctx, &input);
            ctx.run_ui(input, |ui| {
                let _ = state.show(ui, Some(1));
            })
        };
        let first = frame(Vec::new());
        let bounds = first
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .iter()
            .find_map(|(_, node)| (node.label() == Some("黑场淡入")).then(|| node.bounds()).flatten())
            .expect("转场卡片未出现在无障碍树");
        let center = egui::pos2(
            ((bounds.x0 + bounds.x1) * 0.5) as f32,
            ((bounds.y0 + bounds.y1) * 0.5) as f32,
        );
        frame(vec![
            Event::PointerMoved(center),
            Event::PointerButton {
                pos: center,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        frame(vec![Event::PointerMoved(center + vec2(24.0, 24.0))]);
        assert!(DragAndDrop::has_payload_of_type::<DraggedTransition>(&ctx));
    }

    #[test]
    fn release_frame_with_movement_publishes_a_payload() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut state = State::default();
        let mut frame = |events| {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(320.0, 300.0))),
                events,
                ..RawInput::default()
            };
            state.process(&ctx, &input);
            let mut payload_present = false;
            let output = ctx.run_ui(input, |ui| {
                let _ = state.show(ui, Some(1));
                payload_present = DragAndDrop::has_payload_of_type::<DraggedTransition>(ui.ctx());
            });
            (output, payload_present)
        };
        let (first, _) = frame(Vec::new());
        let bounds = first
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .iter()
            .find_map(|(_, node)| (node.label() == Some("黑场淡入")).then(|| node.bounds()).flatten())
            .expect("转场卡片未出现在无障碍树");
        let origin = egui::pos2(
            ((bounds.x0 + bounds.x1) * 0.5) as f32,
            ((bounds.y0 + bounds.y1) * 0.5) as f32,
        );
        let drop = origin + vec2(80.0, 80.0);
        let (_output, payload_present) = frame(vec![
            Event::PointerMoved(origin),
            Event::PointerButton {
                pos: origin,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            Event::PointerMoved(drop),
            Event::PointerButton {
                pos: drop,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        assert!(payload_present, "同一帧的拖拽释放没有发布转场载荷");
    }
}
