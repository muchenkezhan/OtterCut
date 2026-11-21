use eframe::egui::{
    self, Align, Color32, CursorIcon, DragAndDrop, FontId, Id, Layout, Pos2, Rect, RichText, Sense,
    Stroke, StrokeKind, Ui, UiBuilder, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};

use crate::ui_theme::{Palette, palette, upper_palette};

use super::dock_model::{Axis, DockNode, DockState, Pane, Placement};

const GUTTER: f32 = 4.0;
const TITLE_HEIGHT: f32 = 21.0;
const CONTENT_PADDING: f32 = 8.0;
pub(super) const MATERIALS_GRIP_WIDTH: f32 = 24.0;
pub(super) const MATERIALS_TAB_HEIGHT: f32 = 32.0;
pub(in crate::editor) const TIMELINE_GRIP_WIDTH: f32 = 24.0;
const TIMELINE_GRIP_HEIGHT: f32 = 32.0;

#[derive(Clone, Copy)]
struct PanelBounds {
    pane: Pane,
    rect: Rect,
}

impl PanelBounds {
    fn content_padding(self) -> f32 {
        CONTENT_PADDING.min(self.rect.width() * 0.5)
    }

    fn drag_handle(self) -> Rect {
        let (width, height) = match self.pane {
            Pane::Materials => (MATERIALS_GRIP_WIDTH, MATERIALS_TAB_HEIGHT),
            Pane::Timeline => (TIMELINE_GRIP_WIDTH, TIMELINE_GRIP_HEIGHT),
            _ => {
                return Rect::from_min_size(
                    self.rect.min,
                    vec2(self.rect.width(), TITLE_HEIGHT.min(self.rect.height())),
                );
            }
        };
        let padding = self.content_padding();
        let right = self.rect.right() - padding;
        let left = (right - width).max(self.rect.left() + padding);
        Rect::from_min_max(
            pos2(left, self.rect.top()),
            pos2(right, self.rect.top() + height.min(self.rect.height())),
        )
    }
}

#[derive(Clone, Copy)]
struct PanelPress {
    source: Pane,
    origin: Pos2,
    pointer: Pos2,
    moved: bool,
}

impl PanelPress {
    fn track(&mut self, pointer: Pos2, threshold_sq: f32) {
        self.pointer = pointer;
        self.moved |= self.origin.distance_sq(pointer) > threshold_sq;
    }
}

struct PanelDrag {
    source: Pane,
    pointer: Pos2,
    released: bool,
}

enum PanelEvent {
    Press(Pos2),
    Move(Pos2),
    Release(Pos2),
    Cancel,
}

#[derive(Clone, Copy)]
struct MovePanel {
    source: Pane,
    target: Pane,
    placement: Placement,
}

pub(super) fn show(
    ui: &mut Ui,
    state: &mut DockState,
    preview_title: &str,
    mut draw: impl FnMut(Pane, &mut Ui),
) -> bool {
    let outer = ui.available_rect_before_wrap();
    if outer.width() <= 2.0 * GUTTER || outer.height() <= 2.0 * GUTTER {
        return false;
    }
    ui.allocate_rect(outer, Sense::hover());
    let mut measure = Measure {
        panels: Vec::with_capacity(Pane::ALL.len()),
        resize_finished: false,
    };
    measure.visit(
        ui,
        &mut state.root,
        ui.id().with("dock"),
        outer.shrink(GUTTER),
    );

    let drag = track_panel_pointer(ui, &measure.panels);
    let mut menu_move = None;
    // A release can add material even when the cards are rebuilt in the same frame.
    for pane in [
        Pane::Timeline,
        Pane::Materials,
        Pane::Inspector,
        Pane::Preview,
    ] {
        let Some(panel) = measure.panels.iter().find(|panel| panel.pane == pane) else {
            continue;
        };
        let (mut body, request) = panel_ui(ui, *panel, preview_title, drag.is_none());
        menu_move = request.or(menu_move);
        draw(pane, &mut body);
    }
    let movement = drag
        .and_then(|drag| panel_drop(ui, &measure.panels, drag))
        .or(menu_move);
    let resized = measure.resize_finished && state.commit_custom();
    resized
        | movement.is_some_and(|movement| {
            state.move_pane(movement.source, movement.target, movement.placement)
        })
}

struct Measure {
    panels: Vec<PanelBounds>,
    resize_finished: bool,
}

impl Measure {
    fn visit(&mut self, ui: &Ui, node: &mut DockNode, id: Id, rect: Rect) {
        match node {
            DockNode::Pane(pane) => self.panels.push(PanelBounds { pane: *pane, rect }),
            DockNode::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let axis = *axis;
                let (range, gap) = SplitRange::new(rect, axis, first, second);
                let mut divider = range.clamp(range.start + range.length * *ratio);
                let grip = splitter_rect(rect, axis, divider, gap);
                let response = ui
                    .interact(grip, id, Sense::DRAG)
                    .on_hover_cursor(match axis {
                        Axis::Horizontal => CursorIcon::ResizeHorizontal,
                        Axis::Vertical => CursorIcon::ResizeVertical,
                    });
                if (response.dragged() || response.drag_stopped())
                    && let Some(pos) = response.interact_pointer_pos()
                    && range.length > 0.0
                {
                    divider = range.clamp(axis_pos(axis, pos) - gap * 0.5);
                    *ratio = ((divider - range.start) / range.length)
                        .clamp(DockNode::MIN_RATIO, DockNode::MAX_RATIO);
                }
                self.resize_finished |= response.drag_stopped();
                if response.hovered() || response.dragged() {
                    let colors = upper_palette(ui.ctx());
                    ui.painter().rect_filled(
                        splitter_rect(rect, axis, divider, gap),
                        1,
                        colors.border,
                    );
                }
                let (first_rect, second_rect) = split_rect(rect, axis, divider, gap);
                self.visit(ui, first, id.with(false), first_rect);
                self.visit(ui, second, id.with(true), second_rect);
            }
        }
    }
}

struct SplitRange {
    start: f32,
    length: f32,
    min: f32,
    max: f32,
}

impl SplitRange {
    fn new(rect: Rect, axis: Axis, first: &DockNode, second: &DockNode) -> (Self, f32) {
        let start = axis_pos(axis, rect.min);
        let extent = (axis_pos(axis, rect.max) - start).max(0.0);
        let gap = GUTTER.min(extent * 0.2);
        let length = extent - gap;
        let span = |node, hard| axis_pos(axis, minimum(node, hard).to_pos2());
        let first_preferred = span(first, false);
        let second_preferred = span(second, false);
        let (first, second) = if length >= first_preferred + second_preferred {
            (first_preferred, second_preferred)
        } else {
            let first_floor = span(first, true);
            let second_floor = span(second, true);
            let floor = first_floor + second_floor;
            let first = if length < floor {
                length * (first_floor / floor)
            } else {
                let share = (length - floor) / (first_preferred + second_preferred - floor);
                first_floor + (first_preferred - first_floor) * share
            };
            (first, length - first)
        };
        let min = start + first;
        let max = (start + length - second).max(min);
        (
            Self {
                start,
                length,
                min,
                max,
            },
            gap,
        )
    }

    fn clamp(&self, divider: f32) -> f32 {
        divider.clamp(self.min, self.max)
    }
}

fn minimum(node: &DockNode, hard: bool) -> Vec2 {
    match node {
        DockNode::Pane(pane) => {
            let (floor, preferred) = match pane {
                Pane::Timeline => (vec2(260.0, 168.0), vec2(348.0, 184.0)),
                Pane::Preview => (vec2(144.0, 100.0), vec2(200.0, 144.0)),
                Pane::Materials => (vec2(116.0, 72.0), vec2(124.0, 92.0)),
                Pane::Inspector => (vec2(112.0, 64.0), vec2(136.0, 92.0)),
            };
            if hard { floor } else { preferred }
        }
        DockNode::Split {
            axis,
            first,
            second,
            ..
        } => {
            let first = minimum(first, hard);
            let second = minimum(second, hard);
            match axis {
                Axis::Horizontal => vec2(first.x + GUTTER + second.x, first.y.max(second.y)),
                Axis::Vertical => vec2(first.x.max(second.x), first.y + GUTTER + second.y),
            }
        }
    }
}

fn axis_pos(axis: Axis, point: Pos2) -> f32 {
    match axis {
        Axis::Horizontal => point.x,
        Axis::Vertical => point.y,
    }
}

fn splitter_rect(rect: Rect, axis: Axis, divider: f32, gap: f32) -> Rect {
    match axis {
        Axis::Horizontal => Rect::from_min_max(
            pos2(divider - 1.0, rect.top()),
            pos2(divider + gap + 1.0, rect.bottom()),
        ),
        Axis::Vertical => Rect::from_min_max(
            pos2(rect.left(), divider - 1.0),
            pos2(rect.right(), divider + gap + 1.0),
        ),
    }
    .intersect(rect)
}

fn split_rect(rect: Rect, axis: Axis, divider: f32, gap: f32) -> (Rect, Rect) {
    let (mut first, mut second) = (rect, rect);
    match axis {
        Axis::Horizontal => {
            first.max.x = divider;
            second.min.x = divider + gap;
        }
        Axis::Vertical => {
            first.max.y = divider;
            second.min.y = divider + gap;
        }
    }
    (first, second)
}

fn panel_ui(
    ui: &mut Ui,
    panel: PanelBounds,
    preview_title: &str,
    allow_menu: bool,
) -> (Ui, Option<MovePanel>) {
    let PanelBounds { pane, rect } = panel;
    let colors = if pane == Pane::Timeline {
        palette(ui.ctx())
    } else {
        upper_palette(ui.ctx())
    };
    let id = ui.id().with(("dock-panel", pane));
    ui.painter().rect_filled(rect, 6, colors.surface);
    let handle = panel.drag_handle();
    let response = ui.interact(handle, id.with("title"), Sense::click_and_drag());
    let drag_label = format!("调整{}位置", pane.label());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &drag_label));
    let request = allow_menu.then(|| panel_menu(&response, pane)).flatten();
    let title = match pane {
        Pane::Materials | Pane::Timeline => None,
        Pane::Preview => Some(preview_title),
        _ => Some(pane.label()),
    };
    paint_title(ui, handle, title, colors, &response);
    response
        .on_hover_cursor(CursorIcon::Grab)
        .on_hover_text("拖动面板，或点击交换位置");

    // Content is registered under its pane ID, so scrollbar and focus state move with it.
    let padding = panel.content_padding();
    let body_top = if matches!(pane, Pane::Materials | Pane::Timeline) {
        rect.top()
    } else {
        handle.bottom()
    };
    let body = Rect::from_min_max(
        pos2(rect.left() + padding, body_top),
        pos2(
            rect.right() - padding,
            (rect.bottom() - GUTTER).max(body_top),
        ),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .id(id.with("content"))
            .max_rect(body)
            .layout(Layout::top_down(Align::Min)),
    );
    child.set_clip_rect(body.intersect(ui.clip_rect()));
    (child, request)
}

fn panel_menu(response: &egui::Response, pane: Pane) -> Option<MovePanel> {
    egui::Popup::menu(response)
        .id(response.id.with("dock-menu"))
        .show(|ui| {
            for target in Pane::ALL.into_iter().filter(|candidate| *candidate != pane) {
                if ui.button(format!("与{}交换位置", target.label())).clicked() {
                    ui.close();
                    return Some(MovePanel {
                        source: pane,
                        target,
                        placement: Placement::Swap,
                    });
                }
            }
            None
        })
        .and_then(|popup| popup.inner)
}

fn paint_title(
    ui: &mut Ui,
    rect: Rect,
    title: Option<&str>,
    colors: Palette,
    response: &egui::Response,
) {
    let painter = ui.painter().with_clip_rect(rect);
    let hot = response.hovered() || response.dragged() || response.has_focus();
    let grip = if hot { colors.text } else { colors.muted };
    let grip_x = title.map_or(rect.center().x, |_| rect.left() + 10.5);
    for column in 0..2 {
        for row in -1..=1 {
            painter.circle_filled(
                pos2(
                    grip_x - 1.5 + 3.0 * column as f32,
                    rect.center().y + 3.0 * row as f32,
                ),
                0.8,
                grip,
            );
        }
    }
    if response.has_focus() {
        painter.hline(
            grip_x - 4.0..=grip_x + 4.0,
            rect.bottom() - 2.0,
            Stroke::new(1.0_f32, colors.accent),
        );
    }
    if let Some(title) = title
        && rect.width() > 25.0
    {
        ui.put(
            Rect::from_min_max(
                pos2(rect.left() + 20.0, rect.top()),
                pos2(rect.right() - 5.0, rect.bottom()),
            ),
            egui::Label::new(
                RichText::new(title)
                    .font(FontId::proportional(11.0))
                    .strong()
                    .color(colors.text),
            )
            .sense(Sense::hover())
            .selectable(false)
            .truncate(),
        );
    }
}

fn panel_drop(ui: &Ui, panels: &[PanelBounds], drag: PanelDrag) -> Option<MovePanel> {
    let PanelDrag {
        source,
        pointer,
        released,
    } = drag;
    if !released {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    }
    let panel = panels
        .iter()
        .find(|panel| panel.pane != source && panel.rect.contains(pointer))?;
    let placement = drop_position(panel.rect, pointer);
    if released {
        return Some(MovePanel {
            source,
            target: panel.pane,
            placement,
        });
    }

    let accent = upper_palette(ui.ctx()).accent;
    let layer = egui::LayerId::new(egui::Order::Foreground, ui.id().with("dock-drop-guide"));
    let painter = ui.ctx().layer_painter(layer).with_clip_rect(ui.clip_rect());
    let guide = drop_guide(panel.rect.shrink(2.0), placement);
    painter.rect_filled(
        guide,
        4,
        Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 31),
    );
    painter.rect_stroke(guide, 4, Stroke::new(1.5_f32, accent), StrokeKind::Inside);
    None
}

fn track_panel_pointer(ui: &Ui, panels: &[PanelBounds]) -> Option<PanelDrag> {
    let ctx = ui.ctx();
    let id = ui.id().with("dock-pointer");
    let mut press = ctx.data_mut(|data| data.remove_temp::<Option<PanelPress>>(id).flatten());
    if DragAndDrop::has_any_payload(ctx) || egui::Popup::is_any_open(ctx) {
        return None;
    }

    let events = ui.input(|input| {
        input.focused.then(|| {
            input
                .raw
                .events
                .iter()
                .filter_map(panel_event)
                .collect::<Vec<_>>()
        })
    })?;
    let threshold = ctx.options(|options| options.input_options.max_click_dist);
    let threshold_sq = threshold * threshold;
    let mut released = None;
    // This also covers motion and release delivered together after a previous press.
    for event in events {
        match event {
            PanelEvent::Press(origin) => {
                press = panels
                    .iter()
                    .find(|panel| panel.drag_handle().contains(origin))
                    .filter(|_| {
                        ctx.layer_id_at(origin)
                            .is_none_or(|layer| layer.order <= ui.layer_id().order)
                    })
                    .map(|panel| PanelPress {
                        source: panel.pane,
                        origin,
                        pointer: origin,
                        moved: false,
                    });
            }
            PanelEvent::Move(pointer) => {
                if let Some(press) = &mut press {
                    press.track(pointer, threshold_sq);
                }
            }
            PanelEvent::Release(pointer) => {
                if let Some(mut press) = press.take() {
                    press.track(pointer, threshold_sq);
                    if press.moved {
                        released = Some(PanelDrag {
                            source: press.source,
                            pointer,
                            released: true,
                        });
                    }
                }
            }
            PanelEvent::Cancel => press = None,
        }
    }

    if press.is_some() {
        ctx.data_mut(|data| data.insert_temp(id, press));
    }
    released.or_else(|| {
        press.filter(|press| press.moved).map(|press| PanelDrag {
            source: press.source,
            pointer: press.pointer,
            released: false,
        })
    })
}

fn panel_event(event: &egui::Event) -> Option<PanelEvent> {
    match *event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            ..
        } => Some(if pressed {
            PanelEvent::Press(pos)
        } else {
            PanelEvent::Release(pos)
        }),
        egui::Event::PointerMoved(pointer) => Some(PanelEvent::Move(pointer)),
        egui::Event::PointerGone
        | egui::Event::Key {
            key: egui::Key::Escape,
            pressed: true,
            ..
        } => Some(PanelEvent::Cancel),
        _ => None,
    }
}

fn drop_position(rect: Rect, cursor: Pos2) -> Placement {
    let rel = (cursor - rect.min) / rect.size();
    let edges = [
        (rel.x, Placement::Left),
        (1.0 - rel.x, Placement::Right),
        (rel.y, Placement::Top),
        (1.0 - rel.y, Placement::Bottom),
    ];
    edges
        .into_iter()
        .filter(|(distance, _)| *distance <= 0.24)
        .min_by(|(left, _), (right, _)| left.total_cmp(right))
        .map_or(Placement::Swap, |(_, placement)| placement)
}

fn drop_guide(mut rect: Rect, placement: Placement) -> Rect {
    match placement {
        Placement::Left => rect.max.x = rect.center().x,
        Placement::Right => rect.min.x = rect.center().x,
        Placement::Top => rect.max.y = rect.center().y,
        Placement::Bottom => rect.min.y = rect.center().y,
        Placement::Swap => {}
    }
    rect
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Context, Event, Key, Modifiers, PointerButton, RawInput};

    fn frame(
        ctx: &Context,
        docking: &mut DockState,
        events: Vec<Event>,
    ) -> (bool, Vec<PanelBounds>) {
        let mut settled = false;
        let mut panels = Vec::new();
        let _ = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
                events,
                ..Default::default()
            },
            |ui| {
                settled = show(ui, docking, "预览", |pane, body| {
                    let content = body.max_rect();
                    let title_height = if matches!(pane, Pane::Materials | Pane::Timeline) {
                        0.0
                    } else {
                        TITLE_HEIGHT
                    };
                    let rect = Rect::from_min_max(
                        content.min - vec2(CONTENT_PADDING, title_height),
                        content.max + vec2(CONTENT_PADDING, GUTTER),
                    );
                    panels.push(PanelBounds { pane, rect });
                });
            },
        );
        (settled, panels)
    }

    fn rect(panels: &[PanelBounds], pane: Pane) -> Rect {
        panels.iter().find(|panel| panel.pane == pane).unwrap().rect
    }

    fn button(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn dock_swaps_on_drop_and_escape_cancels_a_live_drag() {
        for (press_batched, release_batched, escape) in [
            (false, false, false),
            (true, true, false),
            (false, true, false),
            (false, false, true),
        ] {
            let ctx = Context::default();
            let mut dock = DockState::default();
            let original = dock.root.clone();
            let (_, panels) = frame(&ctx, &mut dock, vec![]);
            let source = panels
                .iter()
                .find(|panel| panel.pane == Pane::Materials)
                .unwrap();
            let target = rect(&panels, Pane::Preview);
            let start = source.drag_handle().center();
            let end = target.center();
            frame(&ctx, &mut dock, vec![Event::PointerMoved(start)]);
            let changed = if press_batched {
                frame(
                    &ctx,
                    &mut dock,
                    vec![
                        button(start, true),
                        Event::PointerMoved(end),
                        button(end, false),
                    ],
                )
                .0
            } else if release_batched {
                assert!(!frame(&ctx, &mut dock, vec![button(start, true)]).0);
                frame(
                    &ctx,
                    &mut dock,
                    vec![Event::PointerMoved(end), button(end, false)],
                )
                .0
            } else {
                frame(&ctx, &mut dock, vec![button(start, true)]);
                assert!(!frame(&ctx, &mut dock, vec![Event::PointerMoved(end)]).0);
                if escape {
                    frame(
                        &ctx,
                        &mut dock,
                        vec![Event::Key {
                            key: Key::Escape,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: Modifiers::NONE,
                        }],
                    );
                }
                frame(&ctx, &mut dock, vec![button(end, false)]).0
            };
            assert_eq!(changed, !escape);
            assert_eq!(dock.root == original, escape);
            assert!(!DragAndDrop::has_any_payload(&ctx));
        }
    }

    #[test]
    fn material_tabs_are_outside_the_dock_handle() {
        let ctx = Context::default();
        let mut dock = DockState::default();
        let original = dock.root.clone();
        let (_, panels) = frame(&ctx, &mut dock, vec![]);
        let source = panels
            .iter()
            .find(|panel| panel.pane == Pane::Materials)
            .unwrap();
        let handle = source.drag_handle();
        assert_eq!(handle.width(), MATERIALS_GRIP_WIDTH);
        assert_eq!(handle.height(), MATERIALS_TAB_HEIGHT);
        assert_eq!(handle.right(), source.rect.right() - CONTENT_PADDING);
        let start = pos2(source.rect.center().x, handle.center().y);
        let end = rect(&panels, Pane::Preview).center();
        let (changed, _) = frame(
            &ctx,
            &mut dock,
            vec![
                button(start, true),
                Event::PointerMoved(end),
                button(end, false),
            ],
        );
        assert!(!changed);
        assert_eq!(dock.root, original);
    }

    #[test]
    fn timeline_grip_follows_zoom_and_still_moves_the_panel() {
        use crate::editor::timeline_view::TimelineView;
        use takeseam::editor_timeline::Timeline;

        let ctx = Context::default();
        ctx.enable_accesskit();
        let mut dock = DockState::default();
        let mut timeline = Timeline::default();
        let mut view = TimelineView::default();
        let mut playhead = 0;
        let mut selected = None;
        let output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
                ..Default::default()
            },
            |ui| {
                show(ui, &mut dock, "预览", |pane, body| {
                    if pane == Pane::Timeline {
                        view.draw(body, &mut timeline, &mut playhead, &mut selected, false);
                    }
                });
            },
        );
        let tree = output.platform_output.accesskit_update.unwrap();
        let bounds = |label| {
            tree.nodes.iter().find_map(|(_, node)| {
                (node.label() == Some(label) || node.value() == Some(label))
                    .then(|| node.bounds())
                    .flatten()
            })
        };
        let grip = bounds("调整时间线位置").unwrap();
        let zoom = bounds("100%").unwrap();
        assert!(grip.x0 >= zoom.x1);
        assert!(((grip.y0 + grip.y1 - zoom.y0 - zoom.y1) * 0.5).abs() < 2.0);
        assert!(bounds("时间线").is_none());

        let original = dock.root.clone();
        let (_, panels) = frame(&ctx, &mut dock, vec![]);
        let start = pos2(
            ((grip.x0 + grip.x1) * 0.5) as f32,
            ((grip.y0 + grip.y1) * 0.5) as f32,
        );
        let end = rect(&panels, Pane::Preview).center();
        let (changed, _) = frame(
            &ctx,
            &mut dock,
            vec![
                button(start, true),
                Event::PointerMoved(end),
                button(end, false),
            ],
        );
        assert!(changed);
        assert_ne!(dock.root, original);
    }

    #[test]
    fn splitter_keeps_up_with_a_drag_and_commits_on_mouse_release() {
        let ctx = Context::default();
        let mut dock = DockState::default();
        let (_, panels) = frame(&ctx, &mut dock, vec![]);
        let timeline = rect(&panels, Pane::Timeline);
        let start = pos2(timeline.center().x, timeline.top() - GUTTER * 0.5);
        let end = start - vec2(0.0, 32.0);
        frame(&ctx, &mut dock, vec![Event::PointerMoved(start)]);
        frame(&ctx, &mut dock, vec![button(start, true)]);
        let (changed, moved) = frame(&ctx, &mut dock, vec![Event::PointerMoved(end)]);
        assert!(!changed);
        assert!(rect(&moved, Pane::Timeline).top() <= timeline.top() - 31.0);
        assert!(frame(&ctx, &mut dock, vec![button(end, false)]).0);
    }

    #[test]
    fn four_stacked_panels_keep_the_timeline_usable_at_minimum_height() {
        let ctx = Context::default();
        let mut root =
            Pane::ALL
                .into_iter()
                .skip(1)
                .fold(DockNode::Pane(Pane::Materials), |first, pane| {
                    DockNode::Split {
                        axis: Axis::Vertical,
                        ratio: 0.65,
                        first: Box::new(first),
                        second: Box::new(DockNode::Pane(pane)),
                    }
                });
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(760.0, 426.0));
        let mut measure = Measure {
            panels: Vec::new(),
            resize_finished: false,
        };
        let _ = ctx.run_ui(
            RawInput {
                screen_rect: Some(viewport),
                ..Default::default()
            },
            |ui| measure.visit(ui, &mut root, ui.id(), viewport.shrink(GUTTER)),
        );
        let timeline = rect(&measure.panels, Pane::Timeline);
        assert!(timeline.height() >= 168.0 - 0.01);
        assert!(viewport.contains_rect(timeline));
        assert!(
            measure
                .panels
                .windows(2)
                .all(|pair| pair[0].rect.bottom() < pair[1].rect.top())
        );
    }
}
