use std::f32::consts::FRAC_PI_4;
use std::time::Duration;

use eframe::egui::{
    self, Color32, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, WidgetInfo,
    WidgetType, vec2,
};

use crate::ui_theme::palette;

const RAZOR_CORNERS: [(f32, f32); 4] = [(-7.7, 1.8), (3.0, -7.0), (7.8, -1.4), (-3.0, 7.2)];
const RAZOR_EDGE: [(f32, f32); 2] = [(-3.7, 2.0), (3.7, -4.0)];

pub(crate) fn paint_razor_cursor(painter: &Painter, tip: Pos2) {
    let center = tip - vec2(RAZOR_CORNERS[0].0, RAZOR_CORNERS[0].1);
    let outline = Color32::from_gray(24);
    painter.add(Shape::convex_polygon(
        RAZOR_CORNERS.map(|(x, y)| center + vec2(x, y)).to_vec(),
        Color32::from_gray(250),
        Stroke::new(1.25_f32, outline),
    ));
    painter.line_segment(
        RAZOR_EDGE.map(|(x, y)| center + vec2(x, y)),
        Stroke::new(1.1_f32, outline),
    );
}

#[derive(Clone, Copy)]
pub(crate) enum Icon {
    FolderPlus,
    FolderOpen,
    Export,
    Sun,
    Moon,
    Play,
    Pause,
    Cursor,
    Razor,
    Split,
    Trim,
    Move,
    Delete,
    Undo,
    Redo,
    More,
    Magnet,
    Minus,
    Plus,
    Fit,
    AddMedia,
    Media,
    Editor,
    Audio,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    Microphone,
    #[cfg(target_os = "macos")]
    Camera,
    #[cfg(target_os = "macos")]
    Eye,
    #[cfg(target_os = "macos")]
    EyeOff,
    #[cfg(target_os = "macos")]
    Display,
    #[cfg(target_os = "macos")]
    Window,
    ClickFollow,
    Settings,
    Captions,
    Text,
    Timeline,
    Stop,
    Close,
}

impl Icon {
    pub(crate) fn paint(self, painter: &Painter, center: Pos2, color: Color32) {
        self.paint_scaled(painter, center, color, 1.0);
    }

    pub(crate) fn paint_scaled(self, painter: &Painter, center: Pos2, color: Color32, scale: f32) {
        let stroke = Stroke::new(1.5 * scale, color);
        let point = |x, y| center + vec2(x, y) * scale;
        let radius = |value: u8| (f32::from(value) * scale).round() as u8;
        let line = |(x1, y1), (x2, y2)| {
            painter.line_segment([point(x1, y1), point(x2, y2)], stroke);
        };
        let path = |points: &[(f32, f32)]| {
            painter.add(Shape::line(
                points.iter().map(|&(x, y)| point(x, y)).collect(),
                stroke,
            ));
        };
        let rect = |(x1, y1), (x2, y2), corner| {
            painter.rect_stroke(
                Rect::from_min_max(point(x1, y1), point(x2, y2)),
                radius(corner),
                stroke,
                StrokeKind::Inside,
            );
        };

        match self {
            Self::FolderPlus => {
                path(&[
                    (8.0, 1.0),
                    (8.0, -3.8),
                    (0.2, -3.8),
                    (-1.9, -6.0),
                    (-8.0, -6.0),
                    (-8.0, 6.0),
                    (-0.5, 6.0),
                ]);
                line((2.0, 4.6), (8.0, 4.6));
                line((5.0, 1.6), (5.0, 7.6));
            }
            Self::FolderOpen => {
                path(&[
                    (-8.0, 5.5),
                    (-8.0, -6.0),
                    (-2.4, -6.0),
                    (-0.3, -3.8),
                    (6.8, -3.8),
                    (6.8, -1.3),
                ]);
                path(&[
                    (-8.0, 5.5),
                    (-4.8, -1.3),
                    (9.0, -1.3),
                    (6.0, 5.5),
                    (-8.0, 5.5),
                ]);
            }
            Self::Export => {
                path(&[(-7.5, 2.0), (-7.5, 6.8), (7.5, 6.8), (7.5, 2.0)]);
                line((0.0, 2.4), (0.0, -7.5));
                path(&[(-3.7, -3.8), (0.0, -7.5), (3.7, -3.8)]);
            }
            Self::Sun => {
                painter.circle_stroke(center, 3.5 * scale, stroke);
                for step in 0..8 {
                    let angle = step as f32 * FRAC_PI_4;
                    let ray = vec2(angle.cos(), angle.sin());
                    painter.line_segment(
                        [center + ray * (5.8 * scale), center + ray * (8.2 * scale)],
                        stroke,
                    );
                }
            }
            Self::Moon => {
                let crescent = egui::epaint::CubicBezierShape::from_points_stroke(
                    [
                        point(6.9, 2.5),
                        point(2.7, 11.4),
                        point(-10.1, 7.0),
                        point(-7.0, -2.9),
                    ],
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                );
                painter.add(crescent);
                let tip = egui::epaint::CubicBezierShape::from_points_stroke(
                    [
                        point(-7.0, -2.9),
                        point(-6.2, -5.4),
                        point(-4.3, -6.8),
                        point(-1.7, -7.6),
                    ],
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                );
                painter.add(tip);
                let inner = egui::epaint::CubicBezierShape::from_points_stroke(
                    [
                        point(-1.7, -7.6),
                        point(-4.4, -1.7),
                        point(0.5, 3.8),
                        point(6.9, 2.5),
                    ],
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                );
                painter.add(inner);
            }
            Self::Play => {
                painter.add(Shape::convex_polygon(
                    vec![point(-3.7, -6.4), point(6.1, 0.0), point(-3.7, 6.4)],
                    color,
                    Stroke::NONE,
                ));
            }
            Self::Pause => {
                for x in [-2.7, 2.7] {
                    painter.rect_filled(
                        Rect::from_center_size(point(x, 0.0), vec2(2.2, 12.0) * scale),
                        radius(1),
                        color,
                    );
                }
            }
            Self::Cursor => path(&[
                (-5.0, -8.0),
                (7.0, 1.0),
                (1.5, 2.0),
                (3.8, 7.2),
                (1.0, 8.4),
                (-1.3, 3.2),
                (-5.0, 7.0),
                (-5.0, -8.0),
            ]),
            Self::Razor => {
                painter.add(Shape::closed_line(
                    RAZOR_CORNERS.map(|(x, y)| point(x, y)).to_vec(),
                    stroke,
                ));
                line(RAZOR_EDGE[0], RAZOR_EDGE[1]);
            }
            Self::Split => {
                path(&[(-3.5, -4.4), (-8.0, -4.4), (-8.0, 4.4), (-3.5, 4.4)]);
                path(&[(3.5, -4.4), (8.0, -4.4), (8.0, 4.4), (3.5, 4.4)]);
                line((0.0, -7.5), (0.0, 7.5));
            }
            Self::Trim => {
                path(&[(-8.0, -6.0), (-5.0, -6.0), (-5.0, 6.0), (-8.0, 6.0)]);
                path(&[(8.0, -6.0), (5.0, -6.0), (5.0, 6.0), (8.0, 6.0)]);
                line((-5.0, 0.0), (5.0, 0.0));
            }
            Self::Move => {
                line((-8.0, 0.0), (8.0, 0.0));
                path(&[(-4.5, -3.5), (-8.0, 0.0), (-4.5, 3.5)]);
                path(&[(4.5, -3.5), (8.0, 0.0), (4.5, 3.5)]);
            }
            Self::Delete => {
                line((-7.6, -4.4), (7.6, -4.4));
                path(&[(-2.8, -4.4), (-2.8, -7.0), (2.8, -7.0), (2.8, -4.4)]);
                path(&[(-5.8, -4.0), (-4.7, 6.8), (4.7, 6.8), (5.8, -4.0)]);
                line((-1.8, -1.5), (-1.5, 3.6));
                line((1.8, -1.5), (1.5, 3.6));
            }
            Self::Undo | Self::Redo => {
                let side = if matches!(self, Self::Undo) {
                    1.0
                } else {
                    -1.0
                };
                path(&[(-3.5 * side, -7.0), (-8.0 * side, -2.5), (-3.5 * side, 2.0)]);
                painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                    [
                        point(-8.0 * side, -2.5),
                        point(1.0 * side, -2.5),
                        point(8.0 * side, -4.0),
                        point(8.0 * side, 6.9),
                    ],
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ));
            }
            Self::More => {
                for x in [-5.8, 0.0, 5.8] {
                    painter.circle_filled(point(x, 0.0), 1.25 * scale, color);
                }
            }
            Self::Magnet => {
                path(&[
                    (-7.0, -5.8),
                    (-7.0, 0.8),
                    (-6.3, 3.6),
                    (-3.7, 5.9),
                    (0.0, 6.8),
                    (3.7, 5.9),
                    (6.3, 3.6),
                    (7.0, 0.8),
                    (7.0, -5.8),
                    (3.4, -5.8),
                    (3.4, 0.8),
                    (2.4, 2.6),
                    (0.0, 3.2),
                    (-2.4, 2.6),
                    (-3.4, 0.8),
                    (-3.4, -5.8),
                    (-7.0, -5.8),
                ]);
                line((-7.0, -1.7), (-3.4, -1.7));
                line((3.4, -1.7), (7.0, -1.7));
            }
            Self::Minus | Self::Plus => {
                line((-5.7, 0.0), (5.7, 0.0));
                if matches!(self, Self::Plus) {
                    line((0.0, -5.7), (0.0, 5.7));
                }
            }
            Self::Fit => {
                for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                    path(&[(x * 3.5, y * 7.5), (x * 7.5, y * 7.5), (x * 7.5, y * 3.5)]);
                }
                rect((-3.5, -2.5), (3.5, 2.5), 1_u8);
            }
            Self::AddMedia => {
                rect((-8.0, -6.2), (8.0, 6.2), 2_u8);
                path(&[(-3.5, -6.0), (-3.5, 6.0)]);
                line((-0.4, 0.0), (5.2, 0.0));
                line((2.4, -2.8), (2.4, 2.8));
            }
            Self::Media => {
                rect((-7.6, -6.2), (7.6, 6.2), 2_u8);
                path(&[(-2.0, -3.3), (3.3, 0.0), (-2.0, 3.3), (-2.0, -3.3)]);
            }
            Self::Editor => {
                painter.circle_stroke(point(-6.3, -3.8), 1.8 * scale, stroke);
                painter.circle_stroke(point(-6.3, 3.8), 1.8 * scale, stroke);
                line((-4.7, -2.8), (-0.5, 0.0));
                line((-4.7, 2.8), (-0.5, 0.0));
                line((-0.5, 0.0), (3.0, -5.5));
                line((-0.5, 0.0), (3.0, 5.5));
                for y in [-5.5, 0.0, 5.5] {
                    line((4.2, y), (8.5, y));
                    line((5.8, y - 1.1), (5.8, y + 1.1));
                }
            }
            Self::Audio => {
                path(&[(7.0, 3.0), (7.0, -7.0), (-2.0, -4.8), (-2.0, 5.6)]);
                line((-2.0, -1.7), (7.0, -3.8));
                painter.circle_filled(point(-4.3, 5.6), 2.4 * scale, color);
                painter.circle_filled(point(4.7, 3.0), 2.4 * scale, color);
            }
            Self::Captions => {
                rect((-7.6, -5.6), (7.6, 5.6), 2_u8);
                line((-4.5, -1.1), (2.0, -1.1));
                line((-4.5, 2.3), (-1.1, 2.3));
                line((1.0, 2.3), (4.5, 2.3));
            }
            Self::Text => {
                line((-6.0, -6.0), (6.0, -6.0));
                line((0.0, -6.0), (0.0, 6.0));
                line((-3.0, 6.0), (3.0, 6.0));
            }
            Self::Timeline => {
                rect((-8.0, -5.5), (0.6, -1.5), 1_u8);
                rect((1.7, -5.5), (8.0, -1.5), 1_u8);
                rect((-8.0, 1.0), (-3.3, 5.1), 1_u8);
                rect((-2.2, 1.0), (8.0, 5.1), 1_u8);
            }
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            Self::Microphone => {
                rect((-2.7, -7.0), (2.7, 0.8), 3);
                path(&[
                    (-5.0, -1.0),
                    (-5.0, 0.5),
                    (-4.0, 2.5),
                    (-2.0, 4.0),
                    (2.0, 4.0),
                    (4.0, 2.5),
                    (5.0, 0.5),
                    (5.0, -1.0),
                ]);
                line((0.0, 4.0), (0.0, 7.0));
                line((-2.5, 7.0), (2.5, 7.0));
            }
            #[cfg(target_os = "macos")]
            Self::Camera => {
                rect((-8.0, -5.3), (2.8, 5.3), 2_u8);
                path(&[(2.1, -1.5), (7.5, -4.5), (7.5, 4.5), (2.1, 1.5)]);
            }
            #[cfg(target_os = "macos")]
            Self::Eye | Self::EyeOff => {
                for y in [-7.0, 7.0] {
                    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                        [
                            point(-8.0, 0.0),
                            point(-3.5, y),
                            point(3.5, y),
                            point(8.0, 0.0),
                        ],
                        false,
                        Color32::TRANSPARENT,
                        stroke,
                    ));
                }
                painter.circle_stroke(center, 2.2 * scale, stroke);
                if matches!(self, Self::EyeOff) {
                    line((-7.2, -7.2), (7.2, 7.2));
                }
            }
            #[cfg(target_os = "macos")]
            Self::Display => {
                rect((-8.0, -5.4), (8.0, 4.2), 1_u8);
                line((0.0, 4.2), (0.0, 7.2));
                line((-4.0, 7.2), (4.0, 7.2));
            }
            #[cfg(target_os = "macos")]
            Self::Window => {
                rect((-7.4, -6.2), (7.4, 6.2), 2_u8);
                line((-7.0, -2.3), (7.0, -2.3));
                for x in [-4.5, -1.6, 1.3] {
                    painter.circle_filled(point(x, -4.2), 0.7 * scale, color);
                }
            }
            Self::ClickFollow => {
                Self::Cursor.paint_scaled(painter, point(2.0, 2.0), color, 0.85 * scale);
                line((-8.5, -3.2), (-6.1, -3.2));
                line((-7.2, -7.6), (-5.5, -5.9));
                line((-2.3, -10.0), (-2.3, -7.9));
            }
            Self::Settings => {
                for (y, knob) in [(-4.0, 2.0), (0.0, -2.0), (4.0, 1.0)] {
                    line((-6.5, y), (knob - 2.1, y));
                    line((knob + 2.1, y), (6.5, y));
                    painter.circle_stroke(point(knob, y), 1.8 * scale, stroke);
                }
            }
            Self::Stop => {
                painter.rect_filled(
                    Rect::from_center_size(center, vec2(10.7, 10.7) * scale),
                    radius(2),
                    color,
                );
            }
            Self::Close => {
                line((-4.6, -4.6), (4.6, 4.6));
                line((4.6, -4.6), (-4.6, 4.6));
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ButtonStyle {
    Quiet,
    Selected,
}

pub(crate) fn icon_button(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    enabled: bool,
    style: ButtonStyle,
) -> Response {
    ui.add_enabled(enabled, |ui: &mut Ui| {
        let size = if matches!(icon, Icon::Editor) {
            vec2(36.0, 34.0)
        } else {
            vec2(32.0, 32.0)
        };
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        describe_button(&response, label, style);
        if ui.is_rect_visible(rect) {
            paint_button(ui, icon, rect, &response, style);
        }
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(label)
            .on_disabled_hover_text(label)
    })
}

fn describe_button(response: &Response, label: &str, style: ButtonStyle) {
    response.widget_info(|| match style {
        ButtonStyle::Selected => {
            WidgetInfo::selected(WidgetType::Button, response.enabled(), true, label)
        }
        _ => WidgetInfo::labeled(WidgetType::Button, response.enabled(), label),
    });
}

fn paint_button(ui: &Ui, icon: Icon, rect: Rect, response: &Response, style: ButtonStyle) {
    let colors = palette(ui.ctx());
    let pressed = response.is_pointer_button_down_on();
    let hovered = response.hovered();
    let tint = if !response.enabled() {
        colors.muted
    } else if matches!(style, ButtonStyle::Selected) {
        colors.accent
    } else if pressed || hovered {
        colors.text
    } else {
        colors.muted
    };
    let painter = ui.painter();
    if response.enabled() && response.has_focus() {
        paint_icon_focus(painter, rect.center() + vec2(0.0, 12.0), colors.accent);
    }
    match icon {
        Icon::Editor => icon.paint_scaled(painter, rect.center(), tint, 1.12),
        Icon::Settings => icon.paint_scaled(painter, rect.center(), tint, 1.08),
        _ => icon.paint(painter, rect.center(), tint),
    }
}

pub(crate) fn paint_icon_focus(painter: &Painter, center: Pos2, color: Color32) {
    painter.hline(
        center.x - 4.0..=center.x + 4.0,
        center.y,
        Stroke::new(1.25_f32, color),
    );
}

const MENU_HOLD_SECONDS: f64 = 0.38;
const MENU_HOLD_SLOP: f32 = 6.0;
const MENU_CORNER_SIZE: f32 = 13.0;

pub(crate) struct MenuIconButton {
    anchor: Response,
    open: Option<egui::SetOpenCommand>,
    hold_release: bool,
    pub(crate) main_clicked: bool,
}

impl MenuIconButton {
    pub(crate) fn popup(&self) -> egui::Popup<'_> {
        let popup = egui::Popup::menu(&self.anchor).open_memory(self.open);
        if self.hold_release {
            // The release belongs to the press that just opened this menu.
            popup.close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
        } else {
            popup
        }
    }
}

pub(crate) fn menu_icon_button(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    style: ButtonStyle,
    main_opens_menu: bool,
) -> MenuIconButton {
    let (rect, main) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
    let corner_rect = Rect::from_min_max(
        rect.max - vec2(MENU_CORNER_SIZE, MENU_CORNER_SIZE),
        rect.max,
    );
    let corner = ui.interact(corner_rect, main.id.with("menu-corner"), Sense::click());
    describe_button(&main, label, style);
    describe_button(&corner, &format!("展开{label}"), ButtonStyle::Quiet);

    let (held, suppress_main, hold_release) = menu_hold(&main, &corner);
    let click = main.clicked() && !suppress_main;
    let main_clicked = click && !main_opens_menu;
    if main_clicked && main.clicked_by(egui::PointerButton::Primary) {
        main.surrender_focus();
    }
    let open = if held {
        Some(egui::SetOpenCommand::Bool(true))
    } else if corner.clicked() || (click && main_opens_menu) {
        Some(egui::SetOpenCommand::Toggle)
    } else {
        None
    };

    if ui.is_rect_visible(rect) {
        let visual = main.union(corner.clone());
        paint_button(ui, icon, rect, &visual, style);
        let colors = palette(ui.ctx());
        let tint = if corner.has_focus() || corner.hovered() {
            colors.text
        } else {
            colors.muted
        };
        let tip = rect.right_bottom() - vec2(3.0, 3.0);
        ui.painter().add(Shape::convex_polygon(
            vec![tip - vec2(0.0, 6.0), tip, tip - vec2(6.0, 0.0)],
            tint,
            Stroke::NONE,
        ));
        if corner.has_focus() {
            paint_icon_focus(ui.painter(), tip + vec2(-3.0, 2.0), colors.accent);
        }
    }
    let _ = corner
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("展开选项");
    let anchor = main
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label);

    MenuIconButton {
        anchor,
        open,
        hold_release,
        main_clicked,
    }
}

#[derive(Clone, Copy)]
enum MenuPress {
    Pending { time: f64, origin: Pos2 },
    Opened,
    Cancelled,
}

fn menu_hold(main: &Response, corner: &Response) -> (bool, bool, bool) {
    let ctx = &main.ctx;
    let id = main.id.with("menu-hold");
    let (now, pressed, down, released, position, origin, cancelled) = ctx.input(|input| {
        (
            input.time,
            input.pointer.primary_pressed(),
            input.pointer.primary_down(),
            input.pointer.primary_released(),
            input.pointer.latest_pos(),
            input.pointer.press_origin(),
            input.key_pressed(egui::Key::Escape) || !input.focused || !main.enabled(),
        )
    });
    let local = |point: Pos2| {
        ctx.layer_transform_from_global(main.layer_id)
            .map_or(point, |transform| transform * point)
    };
    let on_main =
        |point: Pos2| main.interact_rect.contains(point) && !corner.interact_rect.contains(point);
    let mut press = ctx.data(|data| data.get_temp::<MenuPress>(id));
    if pressed {
        press = origin
            .map(local)
            .filter(|origin| {
                down && !cancelled && main.is_pointer_button_down_on() && on_main(*origin)
            })
            .map(|origin| MenuPress::Pending { time: now, origin });
    }

    let mut held = false;
    if cancelled && press.is_some() {
        press = Some(MenuPress::Cancelled);
    } else if let Some(MenuPress::Pending { time, origin }) = press {
        if !position.map(local).is_some_and(|point| {
            on_main(point) && origin.distance_sq(point) <= MENU_HOLD_SLOP * MENU_HOLD_SLOP
        }) {
            press = Some(MenuPress::Cancelled);
        } else if down || released {
            let remaining = MENU_HOLD_SECONDS - (now - time);
            if remaining <= 0.0 {
                press = Some(MenuPress::Opened);
                held = true;
            } else if down {
                ctx.request_repaint_after(Duration::from_secs_f64(remaining));
            }
        }
    }

    if !cancelled && main.long_touched() && !suppress_pending_press(press) {
        press = Some(MenuPress::Opened);
        held = true;
    }

    let suppress_main = suppress_pending_press(press);
    let hold_release = released && matches!(press, Some(MenuPress::Opened));
    ctx.data_mut(|data| match press.filter(|_| down) {
        Some(press) => {
            data.insert_temp(id, press);
        }
        None => data.remove::<MenuPress>(id),
    });
    (held, suppress_main, hold_release)
}

fn suppress_pending_press(press: Option<MenuPress>) -> bool {
    matches!(press, Some(MenuPress::Opened | MenuPress::Cancelled))
}

#[cfg(test)]
mod menu_tests {
    use super::*;
    use egui::{Event, Modifiers, PointerButton, pos2};

    struct Frame {
        button: Rect,
        primary: bool,
        popup: bool,
    }

    fn frame(ctx: &egui::Context, time: f64, events: Vec<Event>, menu_only: bool) -> Frame {
        let mut result = None;
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                let button =
                    menu_icon_button(ui, Icon::Cursor, "工具", ButtonStyle::Quiet, menu_only);
                result = Some(Frame {
                    button: button.anchor.rect,
                    primary: button.main_clicked,
                    popup: button.popup().show(|ui| ui.label("选项")).is_some(),
                });
            },
        );
        result.unwrap()
    }

    fn pointer(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn menu_corner_and_primary_click_are_distinct() {
        let ctx = egui::Context::default();
        let button = frame(&ctx, 0.0, vec![], false).button;
        let main = button.center();
        frame(
            &ctx,
            0.1,
            vec![Event::PointerMoved(main), pointer(main, true)],
            false,
        );
        let short = frame(&ctx, 0.2, vec![pointer(main, false)], false);
        assert!(short.primary && !short.popup);

        let corner = button.max - vec2(6.0, 6.0);
        frame(
            &ctx,
            0.3,
            vec![Event::PointerMoved(corner), pointer(corner, true)],
            false,
        );
        let opened = frame(&ctx, 0.4, vec![pointer(corner, false)], false);
        assert!(!opened.primary && opened.popup);

        let ctx = egui::Context::default();
        let main = frame(&ctx, 0.0, vec![], true).button.center();
        frame(
            &ctx,
            0.1,
            vec![Event::PointerMoved(main), pointer(main, true)],
            true,
        );
        let menu_only = frame(&ctx, 0.2, vec![pointer(main, false)], true);
        assert!(!menu_only.primary && menu_only.popup);
    }

    #[test]
    fn menu_hold_opens_once_and_ignores_its_release() {
        let ctx = egui::Context::default();
        let main = frame(&ctx, 0.0, vec![], false).button.center();
        let started = frame(
            &ctx,
            0.1,
            vec![Event::PointerMoved(main), pointer(main, true)],
            false,
        );
        assert!(!started.primary && !started.popup);
        assert!(!frame(&ctx, 0.34, vec![], false).popup);
        assert!(frame(&ctx, 0.49, vec![], false).popup);
        assert!(frame(&ctx, 0.51, vec![], false).popup);
        let released = frame(&ctx, 0.53, vec![pointer(main, false)], false);
        assert!(!released.primary && released.popup);

        let outside = pos2(480.0, 380.0);
        frame(
            &ctx,
            0.6,
            vec![Event::PointerMoved(outside), pointer(outside, true)],
            false,
        );
        frame(&ctx, 0.7, vec![pointer(outside, false)], false);
        assert!(!frame(&ctx, 0.8, vec![], false).popup);
    }

    #[test]
    fn leaving_the_button_cancels_the_pending_hold() {
        let ctx = egui::Context::default();
        let main = frame(&ctx, 0.0, vec![], false).button.center();
        frame(
            &ctx,
            0.1,
            vec![Event::PointerMoved(main), pointer(main, true)],
            false,
        );
        frame(
            &ctx,
            0.2,
            vec![Event::PointerMoved(main + vec2(30.0, 0.0))],
            false,
        );
        frame(&ctx, 0.3, vec![Event::PointerMoved(main)], false);
        assert!(!frame(&ctx, 0.6, vec![], false).popup);
        let released = frame(&ctx, 0.7, vec![pointer(main, false)], false);
        assert!(!released.primary && !released.popup);
    }
}
