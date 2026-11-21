use eframe::egui::{self, Align, Layout, RichText, Ui};
use takeseam::editor_timeline::{Timeline, TrackKind};

use crate::ui_icons::{ButtonStyle, Icon, icon_button, menu_icon_button};
use crate::ui_theme;

use super::super::dock_view::TIMELINE_GRIP_WIDTH;
use super::super::layout;
use super::commands::{Command, Edge, action_button, split_target};
use super::{DEFAULT_SCALE, MAX_SCALE, MIN_SCALE, Mode, SECOND, TimelineView};

const FULL_TOOLBAR_WIDTH: f32 = 528.0;

#[derive(Clone, Copy)]
enum ZoomAction {
    Fit,
    In,
    Out,
}

const ZOOM_ACTIONS: [(ZoomAction, Icon, &str); 3] = [
    (ZoomAction::Fit, Icon::Fit, "显示完整时间线"),
    (ZoomAction::In, Icon::Plus, "放大时间线"),
    (ZoomAction::Out, Icon::Minus, "缩小时间线"),
];

#[derive(Default)]
pub(super) struct ToolbarInput {
    pub(super) command: Option<Command>,
    pub(super) add_material: bool,
    pub(super) add_track: Option<TrackKind>,
}

impl TimelineView {
    pub(super) fn draw_toolbar(
        &mut self,
        ui: &mut Ui,
        timeline: &Timeline,
        selected: Option<u64>,
        playhead: u64,
        width: f32,
        add_enabled: bool,
    ) -> ToolbarInput {
        let mut input = ToolbarInput::default();
        let compact = ui.available_width() < FULL_TOOLBAR_WIDTH + TIMELINE_GRIP_WIDTH;
        let active = self.active_track(timeline);
        let has_clip = selected.is_some();
        let primary = [
            (
                Command::Split,
                split_target(timeline, active, selected, playhead).is_some(),
            ),
            (Command::Delete(false), has_clip),
            (Command::Undo, timeline.can_undo()),
            (Command::Redo, timeline.can_redo()),
        ];
        let secondary = [
            (Command::Trim(Edge::Start, false), has_clip),
            (Command::Trim(Edge::End, false), has_clip),
            (Command::Trim(Edge::Start, true), has_clip),
            (Command::Trim(Edge::End, true), has_clip),
            (Command::Delete(true), has_clip),
            (Command::Cut, has_clip),
            (Command::Copy, has_clip),
            (Command::Paste, self.clipboard.is_some()),
        ];
        let (snap_label, snap_style) = if self.disable_snap {
            ("开启吸附", ButtonStyle::Quiet)
        } else {
            ("关闭吸附", ButtonStyle::Selected)
        };
        let duration = timeline.duration();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if !compact {
                input.add_material = icon_button(
                    ui,
                    Icon::AddMedia,
                    "添加到时间线",
                    add_enabled,
                    ButtonStyle::Quiet,
                )
                .clicked();
                let tracks = menu_icon_button(ui, Icon::Plus, "新增轨道", ButtonStyle::Quiet, true);
                tracks.popup().gap(5.0).show(|ui| {
                    input.add_track = track_menu(ui);
                });
                ui.separator();
            }
            self.mode_menu(ui);
            let blade_active = self.mode == Mode::Blade;
            let blade_style = if blade_active {
                ButtonStyle::Selected
            } else {
                ButtonStyle::Quiet
            };
            let blade = icon_button(ui, Icon::Razor, "刀片 · C", true, blade_style);
            if blade.clicked() {
                self.mode = if blade_active {
                    Mode::Select
                } else {
                    Mode::Blade
                };
                blade.surrender_focus();
                ui.ctx().request_repaint();
            }

            if !compact {
                input.command = action_menu(ui, primary, false);
            }

            let more = menu_icon_button(ui, Icon::More, "更多剪辑操作", ButtonStyle::Quiet, true);
            more.popup().gap(5.0).show(|ui| {
                if compact {
                    layout::compact_menu(ui);
                    ui.set_min_width(142.0);
                    if ui
                        .add_enabled(add_enabled, egui::Button::new("添加到时间线"))
                        .clicked()
                    {
                        input.add_material = true;
                        ui.close();
                    }
                    ui.menu_button("新增轨道", |ui| input.add_track = track_menu(ui));
                    ui.separator();
                    ui.horizontal(|ui| {
                        input.command = action_menu(ui, primary, true);
                    });
                    ui.menu_button("修剪与剪贴板", |ui| {
                        if let Some(action) = action_menu(ui, secondary, true) {
                            input.command = Some(action);
                        }
                    });
                    ui.separator();
                    if ui.button(snap_label).clicked() {
                        self.disable_snap = !self.disable_snap;
                        ui.close();
                    }
                    ui.menu_button(format!("缩放时间线 · {}", self.zoom_caption()), |ui| {
                        for (action, _, label) in ZOOM_ACTIONS {
                            if ui.button(label).clicked() {
                                self.apply_zoom(action, playhead, width, duration);
                                ui.close();
                            }
                        }
                    });
                } else if let Some(action) = action_menu(ui, secondary, true) {
                    input.command = Some(action);
                }
            });

            if !compact {
                ui.separator();
                if icon_button(ui, Icon::Magnet, snap_label, true, snap_style).clicked() {
                    self.disable_snap = !self.disable_snap;
                }
                self.draw_zoom_controls(ui, playhead, width, duration);
            }
        });
        input
    }

    fn mode_menu(&mut self, ui: &mut Ui) {
        let primary = if self.mode == Mode::Blade {
            Mode::Select
        } else {
            self.mode
        };
        let icon = match primary {
            Mode::Select | Mode::Blade => Icon::Cursor,
            Mode::Reorder | Mode::Slip | Mode::Slide => Icon::Move,
            Mode::Ripple | Mode::Roll => Icon::Trim,
        };
        let label = format!("切换剪辑工具（当前：{}）", self.mode.name());
        let menu_style = if self.mode == Mode::Blade {
            ButtonStyle::Quiet
        } else {
            ButtonStyle::Selected
        };
        let tool = menu_icon_button(ui, icon, &label, menu_style, false);
        if tool.main_clicked {
            self.mode = primary;
        }
        tool.popup().gap(5.0).show(|ui| {
            ui.set_min_width(142.0);
            for mode in Mode::ALL {
                if ui
                    .selectable_label(
                        self.mode == mode,
                        format!("{}   {}", mode.name(), mode.key().name()),
                    )
                    .on_hover_text(mode.hint())
                    .clicked()
                {
                    self.mode = mode;
                    ui.close();
                }
            }
        });
    }

    fn zoom_caption(&self) -> String {
        let percent = self.scale() * 100.0 / DEFAULT_SCALE;
        match percent {
            p if p < 0.1 => "<0.1%".into(),
            p if p < 10.0 => format!("{p:.1}%"),
            p => format!("{p:.0}%"),
        }
    }

    fn apply_zoom(&mut self, action: ZoomAction, playhead: u64, width: f32, duration: u64) {
        if matches!(action, ZoomAction::Fit) {
            self.scale = Some(
                (f64::from(width - 20.0) * SECOND / duration.max(SECOND as u64) as f64)
                    .clamp(MIN_SCALE, MAX_SCALE),
            );
            self.scroll = 0.0;
            return;
        }
        let playhead_x = ((playhead as f64 - self.scroll) / SECOND * self.scale()) as f32;
        let anchor = if (0.0..=width).contains(&playhead_x) {
            playhead_x
        } else {
            width * 0.5
        };
        let scale = if matches!(action, ZoomAction::In) {
            self.scale() * 1.35
        } else {
            self.scale() / 1.35
        };
        self.zoom(scale, anchor);
    }

    fn draw_zoom_controls(&mut self, ui: &mut Ui, playhead: u64, width: f32, duration: u64) {
        let caption = self.zoom_caption();
        let colors = ui_theme::palette(ui.ctx());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(TIMELINE_GRIP_WIDTH);
            ui.add_sized(
                [44.0, 24.0],
                egui::Label::new(
                    RichText::new(caption)
                        .size(11.0)
                        .monospace()
                        .color(colors.muted),
                ),
            )
            .on_hover_text("Ctrl + 滚轮缩放时间线");
            for (action, icon, label) in ZOOM_ACTIONS {
                if icon_button(ui, icon, label, true, ButtonStyle::Quiet).clicked() {
                    self.apply_zoom(action, playhead, width, duration);
                }
            }
        });
    }
}

fn track_menu(ui: &mut Ui) -> Option<TrackKind> {
    let mut result = None;
    for (kind, label) in [
        (TrackKind::Text, "新建文本轨"),
        (TrackKind::Caption, "新建字幕轨"),
        (TrackKind::Video, "新建视频轨"),
        (TrackKind::Audio, "新建音频轨"),
    ] {
        if ui.button(label).clicked() {
            result = Some(kind);
            ui.close();
        }
    }
    result
}

fn action_menu(
    ui: &mut Ui,
    actions: impl IntoIterator<Item = (Command, bool)>,
    close_on_click: bool,
) -> Option<Command> {
    let mut result = None;
    for (action, enabled) in actions {
        if action_button(ui, action, enabled) {
            result = Some(action);
            if close_on_click {
                ui.close();
            }
        }
    }
    result
}
