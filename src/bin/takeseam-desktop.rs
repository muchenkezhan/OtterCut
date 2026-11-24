#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn main() -> eframe::Result {
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() == Some(std::ffi::OsStr::new("--editor")) {
        let directory = arguments
            .next()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        return editor::launch(directory);
    }
    #[cfg(windows)]
    return desktop::launch();
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    return portable_recorder::launch();
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn main() {
    eprintln!(
        "{} 支持 Windows、macOS 和 Linux 桌面。",
        takeseam::PRODUCT_NAME
    );
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
use eframe::egui;
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
use std::path::Path;

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
const APP_LOGO: &[u8] = include_bytes!("../../assets/takeseam-brand.png");

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn load_logo(ctx: &egui::Context, icon: &egui::IconData) -> egui::TextureHandle {
    ctx.load_texture(
        takeseam::PRODUCT_NAME,
        egui::ColorImage::from_rgba_unmultiplied(
            [icon.width as usize, icon.height as usize],
            &icon.rgba,
        ),
        egui::TextureOptions::LINEAR,
    )
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn open(path: &Path) -> anyhow::Result<()> {
    use anyhow::Context as _;
    #[cfg(windows)]
    let program = "explorer.exe";
    #[cfg(target_os = "macos")]
    let program = "/usr/bin/open";
    #[cfg(target_os = "linux")]
    let program = "xdg-open";

    let path = std::path::absolute(path).context("无法解析要打开的路径")?;
    let mut child = std::process::Command::new(program)
        .arg(path)
        .spawn()
        .context("无法打开路径")?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[path = "desktop/portable_recorder.rs"]
mod portable_recorder;

#[cfg(windows)]
#[path = "desktop/device_settings.rs"]
mod device_settings;

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
#[path = "desktop/editor.rs"]
mod editor;

#[cfg(windows)]
#[path = "desktop/monitor_work_area.rs"]
mod monitor_work_area;

#[cfg(windows)]
#[path = "desktop/popup_placement.rs"]
mod popup_placement;

#[cfg(windows)]
#[path = "desktop/source_preview.rs"]
mod source_preview;

#[cfg(windows)]
#[path = "desktop/tray.rs"]
mod tray;

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
#[path = "desktop/ui_icons.rs"]
mod ui_icons;

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
#[path = "desktop/ui_theme.rs"]
mod ui_theme;

#[cfg(windows)]
mod desktop {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    use anyhow::{Context as _, Result, bail};
    use eframe::egui::{self, Align, Color32, Layout, RichText, Stroke, vec2};
    use takeseam::PRODUCT_NAME as APP_NAME;
    use takeseam::i18n::{Language, Locale, key};
    use takeseam::windows_recorder::{CaptureSource, RecordingRequest, SavedRecording, record};
    use windows_capture::monitor::Monitor;
    use windows_capture::window::Window;

    use super::device_settings::DeviceSettings;
    use super::editor::{EditorWindow, EditorWindowState};
    use super::monitor_work_area;
    use super::popup_placement::PopupPlacement;
    use super::source_preview::{SourcePreview, SourceThumbnails, paint_thumbnail};
    use super::tray::{Tray, TrayAction};
    use super::ui_icons::Icon as SharedIcon;
    use super::ui_theme;
    use super::{APP_LOGO, load_logo, open};

    const FRAME_INTERVAL: Duration = Duration::from_millis(50);
    const TOOLBAR_WIDTH: f32 = 396.0;
    const TOOLBAR_HEIGHT: f32 = 58.0;
    const WINDOW_MARGIN: f32 = 5.0;
    const PANEL_GAP: f32 = 7.0;
    const PANEL_VERTICAL_PADDING: f32 = 20.0;
    const TOOLBAR_DRAG_THRESHOLD: f32 = 5.0;
    const HINT_ROW_HEIGHT: f32 = 29.0;
    const SOURCE_CARD_WIDTH: f32 = 100.0;
    const PANEL_BUTTON_WIDTH: f32 = 28.0;
    const LOGO: &[u8] = APP_LOGO;
    const TOOLBAR_LOGO: &[u8] = include_bytes!("../../assets/ottercut-toolbar-logo.png");

    pub fn launch() -> eframe::Result {
        let icon = eframe::icon_data::from_png_bytes(LOGO)
            .map_err(|error| eframe::Error::AppCreation(Box::new(error)))?;
        let toolbar_icon = eframe::icon_data::from_png_bytes(TOOLBAR_LOGO)
            .map_err(|error| eframe::Error::AppCreation(Box::new(error)))?;
        let restart = Arc::new(AtomicBool::new(false));
        let restart_requested = Arc::clone(&restart);
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title(APP_NAME)
                .with_inner_size([TOOLBAR_WIDTH, TOOLBAR_HEIGHT])
                .with_decorations(false)
                .with_transparent(true)
                .with_resizable(false)
                .with_always_on_top()
                .with_icon(icon.clone()),
            centered: true,
            ..Default::default()
        };
        eframe::run_native(
            APP_NAME,
            options,
            Box::new(|creation| {
                Ok(Box::new(RecorderApp::new(
                    &creation.egui_ctx,
                    icon,
                    toolbar_icon,
                    restart_requested,
                )))
            }),
        )?;
        if restart.load(Ordering::Relaxed) {
            let executable = std::env::current_exe()
                .map_err(|error| eframe::Error::AppCreation(Box::new(error)))?;
            Command::new(executable)
                .spawn()
                .map_err(|error| eframe::Error::AppCreation(Box::new(error)))?;
        }
        Ok(())
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum SourceKind {
        Display,
        Window,
    }

    struct SourceOption {
        label: String,
        details: String,
        source: CaptureSource,
    }

    impl SourceKind {
        fn list(self) -> Result<Vec<SourceOption>> {
            match self {
                Self::Display => Ok(Monitor::enumerate()?
                    .into_iter()
                    .enumerate()
                    .map(|(index, monitor)| {
                        let name = monitor.name().unwrap_or_default();
                        let name = name.trim_end_matches('\0');
                        let label = if name.is_empty() {
                            format!("显示器 {}", index + 1)
                        } else {
                            format!("显示器 {} · {name}", index + 1)
                        };
                        let details = match (monitor.width(), monitor.height()) {
                            (Ok(width), Ok(height)) => format!("{width} × {height}"),
                            _ => String::new(),
                        };
                        SourceOption {
                            label,
                            details,
                            source: CaptureSource::Monitor(monitor),
                        }
                    })
                    .collect()),
                Self::Window => Ok(Window::enumerate()?
                    .into_iter()
                    .filter_map(|window| {
                        if window.process_id().ok() == Some(std::process::id()) {
                            return None;
                        }
                        let label = window.title().ok()?;
                        if label.trim().is_empty() {
                            return None;
                        }
                        let details = window.process_name().unwrap_or_default();
                        Some(SourceOption {
                            label,
                            details,
                            source: CaptureSource::Window(window),
                        })
                    })
                    .collect()),
            }
        }
    }

    enum Event {
        Started(Instant),
        Finished(Result<SavedRecording>),
    }

    struct Task {
        stop: Arc<AtomicBool>,
        events: Receiver<Event>,
        worker: JoinHandle<()>,
        started: Option<Instant>,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Shutdown {
        Restart,
        Exit,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    enum PopupPage {
        Sources,
        Devices,
    }

    enum ToolbarCommand {
        TogglePanel(PopupPage),
        ToggleMicrophone,
        ToggleClickFollow,
        Record,
        ToggleEditor,
        Close,
    }

    #[derive(Clone, Copy)]
    struct ToolbarPress {
        anchor: egui::Pos2,
        on_button: bool,
        dragging: bool,
    }

    struct RecorderApp {
        logo: egui::TextureHandle,
        editor: EditorWindow,
        preview: SourcePreview,
        thumbnails: SourceThumbnails,
        kind: SourceKind,
        sources: Vec<SourceOption>,
        selected: usize,
        microphone: bool,
        devices: DeviceSettings,
        click_follow: bool,
        directory: PathBuf,
        task: Option<Task>,
        elapsed: Duration,
        saved: Option<SavedRecording>,
        message: String,
        tray: Option<Tray>,
        tray_actions: Receiver<TrayAction>,
        toolbar_visible: bool,
        shutdown: Option<Shutdown>,
        quitting: bool,
        restart_requested: Arc<AtomicBool>,
        panel: Option<PopupPage>,
        popup_placement: PopupPlacement,
        toolbar_press: Option<ToolbarPress>,
    }

    impl RecorderApp {
        fn new(
            ctx: &egui::Context,
            icon: egui::IconData,
            toolbar_icon: egui::IconData,
            restart_requested: Arc<AtomicBool>,
        ) -> Self {
            ui_theme::initialize(ctx);
            Locale::initialize_context(ctx);
            let logo = load_logo(ctx, &toolbar_icon);
            let (send_action, tray_actions) = mpsc::channel();
            let notify = ctx.clone();
            let tray = Tray::new(&icon, move |action| {
                if send_action.send(action).is_ok() {
                    notify.request_repaint_of(egui::ViewportId::ROOT);
                }
            });
            let (tray, tray_error) = match tray {
                Ok(tray) => (Some(tray), None),
                Err(error) => (None, Some(format!("无法初始化托盘图标（{error:#}）"))),
            };
            let directory = std::env::var_os("USERPROFILE")
                .map(|home| PathBuf::from(home).join("Videos"))
                .filter(|path| path.is_dir())
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_else(std::env::temp_dir);
            let mut app = Self {
                logo,
                editor: EditorWindow::default(),
                preview: SourcePreview::default(),
                thumbnails: SourceThumbnails::default(),
                kind: SourceKind::Display,
                sources: Vec::new(),
                selected: 0,
                microphone: false,
                devices: DeviceSettings::default(),
                click_follow: false,
                directory,
                task: None,
                elapsed: Duration::ZERO,
                saved: None,
                message: String::new(),
                tray,
                tray_actions,
                toolbar_visible: true,
                shutdown: None,
                quitting: false,
                restart_requested,
                panel: None,
                popup_placement: PopupPlacement::default(),
                toolbar_press: None,
            };
            app.refresh_sources();
            if let Some(error) = tray_error {
                app.message = error;
            }
            app
        }

        fn refresh_sources(&mut self) {
            self.preview.reset();
            self.thumbnails.reset();
            let selected = self.sources.get(self.selected).map(|source| source.source);
            match self.kind.list() {
                Ok(sources) => {
                    self.selected = selected
                        .and_then(|selected| {
                            sources.iter().position(|source| source.source == selected)
                        })
                        .unwrap_or_default();
                    self.sources = sources;
                    if self.sources.is_empty() {
                        self.message =
                            String::from("当前没有可录制的来源。打开目标窗口后可刷新列表。");
                    } else {
                        self.message.clear();
                    }
                }
                Err(error) => {
                    self.sources.clear();
                    self.message = format!("无法读取录制来源（{error:#}）");
                }
            }
        }

        fn start(&mut self, ctx: &egui::Context) -> Result<()> {
            let source = self
                .sources
                .get(self.selected)
                .context("请选择录制来源")?
                .source;
            let output = next_recording_path(&self.directory)?;
            self.devices.stop_previews();
            let settings = RecordingRequest {
                source,
                output,
                microphone: self.microphone,
                microphone_device: self.devices.microphone_device().map(str::to_owned),
                camera: self.devices.camera_selection()?,
                click_follow: self.click_follow,
                duration: None,
            };
            self.preview.reset();
            self.thumbnails.reset();
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = Arc::clone(&stop);
            let (sender, events) = mpsc::channel();
            let repaint = ctx.clone();
            let worker = thread::Builder::new()
                .name(format!("{APP_NAME} 录制"))
                .spawn(move || {
                    let result = record(settings, &worker_stop, |at| {
                        let _ = sender.send(Event::Started(at));
                        repaint.request_repaint();
                    });
                    let _ = sender.send(Event::Finished(result));
                    repaint.request_repaint();
                })
                .context("无法启动录制任务")?;

            self.message.clear();
            self.elapsed = Duration::ZERO;
            self.task = Some(Task {
                stop,
                events,
                worker,
                started: None,
            });
            Ok(())
        }

        fn update_task(&mut self, ctx: &egui::Context) {
            if let Some(task) = &mut self.task {
                while let Ok(event) = task.events.try_recv() {
                    match event {
                        Event::Started(at) => task.started = Some(at),
                        Event::Finished(result) => {
                            self.finish(ctx, result);
                            return;
                        }
                    }
                }
                if task.worker.is_finished() {
                    // A finished worker queues its result before exiting. A disconnected sender
                    // is the only path where there is no result to apply.
                    match task.events.try_recv() {
                        Ok(Event::Finished(result)) => self.finish(ctx, result),
                        Ok(Event::Started(at)) => {
                            task.started = Some(at);
                            ctx.request_repaint();
                        }
                        Err(_) => self.finish(ctx, Err(anyhow::anyhow!("录制任务意外结束"))),
                    }
                } else {
                    if let Some(at) = task.started
                        && !task.stop.load(Ordering::Relaxed)
                    {
                        self.elapsed = at.elapsed();
                    }
                    let interval = if self.toolbar_visible {
                        FRAME_INTERVAL
                    } else {
                        Duration::from_millis(250)
                    };
                    ctx.request_repaint_after(interval);
                }
            }
        }

        fn finish(&mut self, ctx: &egui::Context, result: Result<SavedRecording>) {
            let started = if let Some(task) = self.task.take() {
                let started = task.started.is_some();
                let _ = task.worker.join();
                started
            } else {
                false
            };
            match result {
                Ok(saved) => {
                    self.message = saved.warning.clone().unwrap_or_default();
                    self.saved = Some(saved);
                    if self.shutdown.is_none() && self.toolbar_visible && !self.message.is_empty() {
                        self.set_panel(ctx, Some(PopupPage::Sources));
                    }
                }
                Err(error) => {
                    if self.shutdown.is_some() && !started {
                        return;
                    }
                    self.shutdown = None;
                    let description = format!("{error:#}");
                    self.message = if description.contains("0x80070005") && self.microphone {
                        String::from("Windows 拒绝了麦克风访问。关闭麦克风即可录制画面。")
                    } else {
                        description
                    };
                    self.show_toolbar(ctx);
                    self.set_panel(ctx, Some(PopupPage::Sources));
                }
            }
        }

        fn open_editor(&mut self, ctx: &egui::Context, toggle: bool) {
            self.set_panel(ctx, None);
            let result = if toggle {
                self.editor.toggle(ctx, &self.directory)
            } else {
                self.editor.open(ctx, &self.directory)
            };
            if let Err(error) = result {
                self.message = format!("{error:#}");
                self.show_toolbar(ctx);
                self.set_panel(ctx, Some(PopupPage::Sources));
            }
        }

        fn show_toolbar(&mut self, ctx: &egui::Context) {
            self.toolbar_visible = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                egui::WindowLevel::AlwaysOnTop,
            ));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        fn close_toolbar(&mut self, ctx: &egui::Context) {
            if self.tray.is_some() {
                self.set_panel(ctx, None);
                self.toolbar_visible = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            } else {
                self.request_shutdown(ctx, Shutdown::Exit);
            }
        }

        fn request_shutdown(&mut self, ctx: &egui::Context, action: Shutdown) {
            self.shutdown = Some(action);
            self.set_panel(ctx, None);
            if let Some(task) = &self.task {
                task.stop.store(true, Ordering::Relaxed);
            }
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }

        fn set_panel(&mut self, ctx: &egui::Context, panel: Option<PopupPage>) {
            if self.panel != panel {
                egui::Popup::close_all(ctx);
            }
            self.panel = panel;
            if panel != Some(PopupPage::Sources) {
                self.preview.reset();
                self.thumbnails.reset();
            }
            if panel != Some(PopupPage::Devices) {
                self.devices.stop_previews();
            }
        }

        fn toolbar(&mut self, ui: &mut egui::Ui) -> Option<egui::Pos2> {
            let colors = ui_theme::palette(ui.ctx());
            let locale = Locale::from_context(ui.ctx());
            let (action, record_icon, can_record) = match &self.task {
                Some(task) if task.stop.load(Ordering::Relaxed) => {
                    (locale.text(key::RECORD_SAVING), Icon::Saving, false)
                }
                Some(task) if task.started.is_some() => (
                    locale.text(key::RECORD_STOP),
                    Icon::Shared(SharedIcon::Stop),
                    true,
                ),
                Some(_) => (
                    locale.text(key::RECORD_STOP),
                    Icon::Shared(SharedIcon::Stop),
                    true,
                ),
                None => (
                    locale.text(key::RECORD_START),
                    Icon::Record,
                    !self.sources.is_empty(),
                ),
            };
            let (source_name, source_icon) = match self.kind {
                SourceKind::Display => (locale.text(key::SOURCE_DISPLAY), Icon::Display),
                SourceKind::Window => (locale.text(key::SOURCE_WINDOW), Icon::Window),
            };
            let (editor_icon, editor_label, editor_active, editor_enabled) =
                match self.editor.state() {
                    EditorWindowState::Closed => {
                        (Icon::Editor, locale.text(key::EDITOR_OPEN), false, true)
                    }
                    EditorWindowState::Opening => {
                        (Icon::Editor, locale.text(key::EDITOR_OPENING), false, false)
                    }
                    EditorWindowState::Visible => (
                        Icon::EditorMinimize,
                        locale.text(key::EDITOR_MINIMIZE),
                        true,
                        true,
                    ),
                    EditorWindowState::Minimized => (
                        Icon::EditorRestore,
                        locale.text(key::EDITOR_RESTORE),
                        false,
                        true,
                    ),
                };
            let mut command = None;
            let mut buttons = Vec::with_capacity(7);
            let mut toolbar_button = |ui: &mut egui::Ui, icon, label, selected| {
                let response = icon_button(ui, icon, label, selected);
                buttons.push(response.rect);
                response
            };

            let bar = egui::Frame::new()
                .fill(colors.surface)
                .stroke(Stroke::new(1.0_f32, colors.border))
                .corner_radius(24)
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.horizontal(|ui| {
                        let (logo_slot, logo) =
                            ui.allocate_exact_size(vec2(32.0, 32.0), egui::Sense::hover());
                        let logo_size = vec2(24.0, 24.0);
                        egui::Image::new((self.logo.id(), logo_size)).paint_at(
                            ui,
                            egui::Rect::from_center_size(logo_slot.center(), logo_size),
                        );
                        logo.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Image, true, APP_NAME)
                        });
                        queue_icon_hint(ui, &logo, locale.text(key::TOOLBAR_DRAG));
                        if toolbar_button(
                            ui,
                            source_icon,
                            source_name,
                            Some(self.panel == Some(PopupPage::Sources)),
                        )
                        .clicked()
                        {
                            command = Some(ToolbarCommand::TogglePanel(PopupPage::Sources));
                        }

                        ui.add_enabled_ui(self.task.is_none(), |ui| {
                            let label = if self.microphone {
                                locale.text(key::MICROPHONE_DISABLE)
                            } else {
                                locale.text(key::MICROPHONE_ENABLE)
                            };
                            if toolbar_button(
                                ui,
                                Icon::Microphone(self.microphone),
                                label,
                                Some(self.microphone),
                            )
                            .clicked()
                            {
                                command = Some(ToolbarCommand::ToggleMicrophone);
                            }
                            let label = if self.click_follow {
                                locale.text(key::CLICK_FOLLOW_DISABLE)
                            } else {
                                locale.text(key::CLICK_FOLLOW_ENABLE)
                            };
                            if toolbar_button(ui, Icon::ClickFollow, label, Some(self.click_follow))
                                .clicked()
                            {
                                command = Some(ToolbarCommand::ToggleClickFollow);
                            }
                        });
                        self.timer(ui);

                        if ui
                            .add_enabled_ui(can_record, |ui| {
                                toolbar_button(ui, record_icon, action, None)
                            })
                            .inner
                            .clicked()
                        {
                            command = Some(ToolbarCommand::Record);
                        }
                        if ui
                            .add_enabled_ui(editor_enabled, |ui| {
                                toolbar_button(ui, editor_icon, editor_label, Some(editor_active))
                            })
                            .inner
                            .clicked()
                        {
                            command = Some(ToolbarCommand::ToggleEditor);
                        }
                        if toolbar_button(
                            ui,
                            Icon::Settings,
                            locale.text(key::SETTINGS),
                            Some(self.panel == Some(PopupPage::Devices)),
                        )
                        .clicked()
                        {
                            command = Some(ToolbarCommand::TogglePanel(PopupPage::Devices));
                        }
                        let close = if self.tray.is_some() {
                            locale.text(key::HIDE_TOOLBAR)
                        } else {
                            locale.text(key::CLOSE)
                        };
                        if toolbar_button(ui, Icon::Shared(SharedIcon::Close), close, None)
                            .clicked()
                        {
                            command = Some(ToolbarCommand::Close);
                        }
                    });
                });

            let (dragged, destination) = self.drag_toolbar(ui, bar.response.rect, &buttons);
            if dragged {
                return destination;
            }
            let ctx = ui.ctx();
            match command {
                Some(ToolbarCommand::TogglePanel(page)) => {
                    self.set_panel(ctx, (self.panel != Some(page)).then_some(page));
                }
                Some(ToolbarCommand::ToggleMicrophone) => {
                    self.microphone = !self.microphone;
                    self.message.clear();
                }
                Some(ToolbarCommand::ToggleClickFollow) => {
                    self.click_follow = !self.click_follow;
                    self.message.clear();
                }
                Some(ToolbarCommand::Record) => {
                    if let Some(task) = &self.task {
                        task.stop.store(true, Ordering::Relaxed);
                    } else {
                        match self.start(ctx) {
                            Ok(()) => self.set_panel(ctx, None),
                            Err(error) => {
                                self.message = format!("{error:#}");
                                self.set_panel(ctx, self.panel.or(Some(PopupPage::Sources)));
                            }
                        }
                    }
                }
                Some(ToolbarCommand::ToggleEditor) => self.open_editor(ctx, true),
                Some(ToolbarCommand::Close) => self.close_toolbar(ctx),
                None => {}
            }
            destination
        }

        fn drag_toolbar(
            &mut self,
            ui: &egui::Ui,
            toolbar: egui::Rect,
            buttons: &[egui::Rect],
        ) -> (bool, Option<egui::Pos2>) {
            let on_button = |point| buttons.iter().any(|button| button.contains(point));
            let mut press = self.toolbar_press;
            let mut dragged = press.is_some_and(|press| press.dragging);
            let (destination, hovering) = ui.input(|input| {
                let origin = input.viewport().outer_rect.map(|rect| rect.min);
                let mut position = None;
                for event in &input.events {
                    match event {
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            ..
                        } => {
                            press = toolbar.contains(*pos).then_some(ToolbarPress {
                                anchor: *pos,
                                on_button: on_button(*pos),
                                dragging: false,
                            });
                        }
                        egui::Event::PointerMoved(pos)
                        | egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            ..
                        } => {
                            if let Some(press) = &mut press {
                                let offset = *pos - press.anchor;
                                let distance = offset.length_sq();
                                if distance >= TOOLBAR_DRAG_THRESHOLD * TOOLBAR_DRAG_THRESHOLD {
                                    press.dragging = true;
                                }
                                if press.dragging {
                                    dragged = true;
                                    position = origin
                                        .filter(|_| distance >= 1.0)
                                        .map(|origin| origin + offset);
                                }
                            }
                            if matches!(event, egui::Event::PointerButton { .. }) {
                                press = None;
                            }
                        }
                        _ => {}
                    }
                }
                if !input.pointer.primary_down() {
                    press = None;
                }
                let hovering = input
                    .pointer
                    .hover_pos()
                    .filter(|point| toolbar.contains(*point));
                (position, hovering)
            });
            self.toolbar_press = press;
            if hovering.is_some_and(on_button)
                || self
                    .toolbar_press
                    .is_some_and(|press| press.dragging && press.on_button)
            {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            } else if self.toolbar_press.is_some_and(|press| press.dragging) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if hovering.is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            (dragged, destination)
        }

        fn timer(&self, ui: &mut egui::Ui) {
            let colors = ui_theme::palette(ui.ctx());
            ui.allocate_ui_with_layout(
                vec2(61.0, 34.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    let color = if self.task.is_some() {
                        colors.recording
                    } else {
                        colors.muted
                    };
                    ui.label(
                        RichText::new(clock(self.elapsed))
                            .monospace()
                            .size(13.0)
                            .color(color),
                    );
                    let status = if self.task.is_some() || self.elapsed.as_secs() >= 3600 {
                        None
                    } else if !self.message.is_empty() {
                        Some((Icon::Warning, colors.danger, "录制遇到问题"))
                    } else {
                        self.saved
                            .as_ref()
                            .map(|_| (Icon::Saved, colors.muted, "已保存"))
                    };
                    if let Some((icon, color, label)) = status {
                        let (rect, response) =
                            ui.allocate_exact_size(vec2(12.0, 16.0), egui::Sense::hover());
                        icon.paint(ui.painter(), rect.center(), color, colors.surface);
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label)
                        });
                        queue_icon_hint(ui, &response, label);
                    }
                },
            );
        }

        fn sources_panel(&mut self, ui: &mut egui::Ui) {
            let locale = Locale::from_context(ui.ctx());
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            ui.spacing_mut().interact_size.y = 28.0;

            let header = ui
                .horizontal(|ui| {
                    ui.label(
                        RichText::new(locale.text(key::SOURCE_TITLE))
                            .size(13.0)
                            .strong(),
                    );
                    let detail = ui.allocate_response(vec2(94.0, 16.0), egui::Sense::hover());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_enabled_ui(self.task.is_none(), |ui| {
                            if panel_icon_button(
                                ui,
                                Icon::Refresh,
                                locale.text(key::SOURCE_REFRESH),
                            )
                            .clicked()
                            {
                                self.refresh_sources();
                            }
                            self.source_mode_buttons(ui);
                        });
                    });
                    detail
                })
                .inner;

            let remaining = if self.sources.is_empty() {
                0
            } else {
                ui.add_enabled_ui(self.task.is_none(), |ui| self.source_cards(ui))
                    .inner
            };
            let detail = if remaining > 0 {
                Some((
                    locale.format(key::SOURCE_REMAINING, &[("count", &remaining.to_string())]),
                    egui::FontId::proportional(11.0),
                ))
            } else if self.kind == SourceKind::Display {
                self.sources
                    .get(self.selected)
                    .filter(|source| !source.details.is_empty())
                    .map(|source| (source.details.clone(), egui::FontId::monospace(11.0)))
            } else {
                None
            };
            if let Some((text, font)) = detail {
                header.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text.as_str())
                });
                ui.painter().text(
                    header.rect.left_center(),
                    egui::Align2::LEFT_CENTER,
                    text,
                    font,
                    colors.muted,
                );
            }

            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new("保存至").size(12.0).color(colors.muted));
                let path = self.directory.to_string_lossy();
                let remaining =
                    ui.available_width() - 2.0 * (PANEL_BUTTON_WIDTH + ui.spacing().item_spacing.x);
                panel_value(ui, remaining, &path, &path);
                if panel_icon_button(ui, Icon::Shared(SharedIcon::FolderOpen), "打开保存目录")
                    .clicked()
                    && let Err(error) = open(&self.directory)
                {
                    self.message = format!("{error:#}");
                }
                let change = ui.add_enabled_ui(self.task.is_none(), |ui| {
                    panel_icon_button(ui, Icon::Shared(SharedIcon::More), "更改目录")
                });
                if change.inner.clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .set_title("选择录像保存目录")
                        .set_directory(&self.directory)
                        .pick_folder()
                {
                    self.directory = path;
                    self.message.clear();
                }
            });

            if let Some(saved) = &self.saved {
                ui.separator();
                let recordings = std::iter::once(("最近录像", saved.path.as_path())).chain(
                    saved
                        .camera_path
                        .as_deref()
                        .map(|path| ("摄像头录像", path)),
                );
                for (label, path) in recordings {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(label).size(12.0).color(colors.muted));
                        let name = path.file_name().unwrap_or_default().to_string_lossy();
                        let remaining =
                            ui.available_width() - PANEL_BUTTON_WIDTH - ui.spacing().item_spacing.x;
                        panel_value(ui, remaining, &name, &path.to_string_lossy());
                        if panel_icon_button(ui, Icon::Shared(SharedIcon::Play), "播放录像")
                            .clicked()
                            && let Err(error) = open(path)
                        {
                            self.message = format!("{error:#}");
                        }
                    });
                }
            }
            if !self.message.is_empty() {
                ui.label(
                    RichText::new(shorten(&self.message, 155))
                        .size(12.0)
                        .color(colors.danger),
                )
                .on_hover_text(&self.message);
            }
        }

        fn source_mode_buttons(&mut self, ui: &mut egui::Ui) {
            let colors = ui_theme::palette(ui.ctx());
            egui::Frame::new()
                .fill(colors.control)
                .corner_radius(10)
                .inner_margin(egui::Margin::symmetric(3, 2))
                .show(ui, |ui| {
                    ui.allocate_ui_with_layout(
                        vec2(62.0, 26.0),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            for (kind, icon, label) in [
                                (SourceKind::Display, Icon::Display, "录制屏幕"),
                                (SourceKind::Window, Icon::Window, "录制窗口"),
                            ] {
                                let selected = self.kind == kind;
                                let choice =
                                    draw_icon_button(ui, icon, label, Some(selected), [30.0, 26.0])
                                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                                        .on_hover_text(label)
                                        .on_disabled_hover_text(label);
                                if choice.clicked() && !selected {
                                    self.kind = kind;
                                    self.refresh_sources();
                                }
                            }
                        },
                    );
                });
        }

        fn source_cards(&mut self, ui: &mut egui::Ui) -> usize {
            let selected = self.selected;
            let live = if self.task.is_none() {
                self.sources.get(selected).map(|source| source.source)
            } else {
                None
            };
            self.preview.update(ui.ctx(), live);
            let width = match (self.kind, self.sources.len()) {
                (_, 1) => ui.available_width(),
                (SourceKind::Window, _) => {
                    ((ui.available_width() - ui.spacing().item_spacing.x) / 2.0).floor()
                }
                (SourceKind::Display, _) => SOURCE_CARD_WIDTH,
            };
            let scroll_key = match self.kind {
                SourceKind::Display => "display-cards",
                SourceKind::Window => "window-cards",
            };
            let mut visible = Vec::new();
            let mut choice = None;
            let mut card_ends = Vec::with_capacity(self.sources.len());

            let scroll = ui
                .scope(|ui| {
                    let style = ui.style_mut();
                    style.always_scroll_the_only_direction = true;
                    style.spacing.scroll.floating = false;
                    style.spacing.scroll.bar_width = 6.0;
                    egui::ScrollArea::horizontal()
                        .id_salt(scroll_key)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.horizontal_top(|ui| {
                                for (index, source) in self.sources.iter().enumerate() {
                                    let active = selected == index;
                                    let live_texture =
                                        if active { self.preview.texture() } else { None };
                                    let texture = live_texture
                                        .or_else(|| self.thumbnails.texture(source.source));
                                    let (status, reason) = if live_texture.is_some() {
                                        ("实时画面", None)
                                    } else if texture.is_some() {
                                        ("画面缩略图", None)
                                    } else if active {
                                        self.preview.status()
                                    } else if let Some(error) = self.thumbnails.error(source.source)
                                    {
                                        ("暂时无法预览", Some(error))
                                    } else {
                                        ("正在获取画面…", None)
                                    };
                                    let response = source_card(
                                        ui, width, source, active, texture, status, reason,
                                    );
                                    card_ends.push(response.rect.right());
                                    if ui.is_rect_visible(response.rect) {
                                        visible.push(source.source);
                                    }
                                    if response.clicked() {
                                        choice = Some(index);
                                    }
                                }
                            });
                        })
                })
                .inner;
            if let Some(choice) = choice {
                self.selected = choice;
            }
            let live = if self.task.is_none() {
                self.sources.get(self.selected).map(|source| source.source)
            } else {
                None
            };
            if self.selected != selected {
                self.preview.update(ui.ctx(), live);
            }
            if self.task.is_some() {
                visible.clear();
            }
            self.thumbnails.update(ui.ctx(), &visible, live);
            let visible_right = scroll.inner_rect.right() + 0.5;
            card_ends
                .into_iter()
                .filter(|right| *right > visible_right)
                .count()
        }

        fn appearance_settings(&mut self, ui: &mut egui::Ui) {
            let locale = Locale::from_context(ui.ctx());
            let (theme_icon, theme_hint) = if ui.ctx().theme() == egui::Theme::Dark {
                (SharedIcon::Sun, locale.text(key::SWITCH_LIGHT))
            } else {
                (SharedIcon::Moon, locale.text(key::SWITCH_DARK))
            };
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(locale.text(key::SETTINGS))
                        .size(12.0)
                        .strong(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.menu_button(locale.text(key::LANGUAGE), |ui| {
                        let locale = Locale::from_context(ui.ctx());
                        for language in [Language::ZhCn, Language::EnUs] {
                            let selected = locale.language() == language;
                            let label = match language {
                                Language::ZhCn => locale.text(key::LANGUAGE_ZH),
                                Language::EnUs => locale.text(key::LANGUAGE_EN),
                            };
                            if ui.selectable_label(selected, label).clicked() {
                                if let Err(error) =
                                    Locale::set_context_and_persist(ui.ctx(), language)
                                {
                                    self.message =
                                        format!("{}: {error}", locale.text(key::LANGUAGE));
                                }
                                ui.close();
                            }
                        }
                    });
                    if panel_icon_button(ui, Icon::Shared(theme_icon), theme_hint).clicked()
                        && let Err(error) = ui_theme::toggle(ui.ctx())
                    {
                        self.message = format!("{}（{error}）", theme_hint);
                    }
                });
            });
        }
    }

    impl RecorderApp {
        fn popup_settings(
            &mut self,
            ui: &mut egui::Ui,
            page: PopupPage,
            origin: egui::Pos2,
            width: f32,
        ) -> f32 {
            let colors = ui_theme::palette(ui.ctx());
            let extra = self.popup_placement.extra_height;
            let clearance = PANEL_GAP + WINDOW_MARGIN;
            let height = if extra > 0.0 {
                (extra - clearance).max(1.0)
            } else {
                768.0
            };
            let mut panel_ui = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("toolbar-popup", page))
                    .max_rect(egui::Rect::from_min_size(origin, vec2(width, height)))
                    .layout(Layout::top_down(Align::Min)),
            );
            let panel = egui::Frame::new()
                .fill(colors.surface)
                .stroke(Stroke::new(1.0_f32, colors.border))
                .corner_radius(16)
                .inner_margin(egui::Margin::symmetric(11, 9))
                .show(&mut panel_ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("settings-height")
                        .max_height((height - PANEL_VERTICAL_PADDING).max(1.0))
                        .auto_shrink([false, true])
                        .show(ui, |ui| match page {
                            PopupPage::Sources => self.sources_panel(ui),
                            PopupPage::Devices => {
                                self.devices
                                    .show(ui, &mut self.microphone, self.task.is_some());
                                self.appearance_settings(ui);
                                if !self.message.is_empty() {
                                    ui.colored_label(colors.danger, shorten(&self.message, 155));
                                }
                            }
                        })
                        .content_size
                        .y
                });
            panel.inner + PANEL_VERTICAL_PADDING + clearance
        }

        fn place_popup(&mut self, ctx: &egui::Context, motion: Option<egui::Pos2>, requested: f32) {
            let placement = self.popup_placement;
            let (observed, pointer_down) = ctx.input(|input| {
                (
                    input.viewport().outer_rect.map(|rect| rect.min),
                    input.pointer.any_down(),
                )
            });
            let pixels_per_point = ctx.pixels_per_point();
            let anchor = motion
                .or(observed)
                .map(|point| point + vec2(0.0, placement.toolbar_offset()));
            let next = if pointer_down {
                placement
            } else if self.panel.is_some()
                && let Some(anchor) = anchor
            {
                let toolbar =
                    egui::Rect::from_min_size(anchor, vec2(TOOLBAR_WIDTH, TOOLBAR_HEIGHT));
                let work_area = monitor_work_area::work_area(toolbar, pixels_per_point);
                PopupPlacement::fit(toolbar, work_area, requested)
            } else {
                PopupPlacement {
                    above: false,
                    extra_height: requested.ceil(),
                }
            };

            if placement.extra_height != next.extra_height {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(
                    TOOLBAR_WIDTH,
                    TOOLBAR_HEIGHT + next.extra_height,
                )));
            }
            if let Some(anchor) = anchor
                && (motion.is_some() || placement.toolbar_offset() != next.toolbar_offset())
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(
                    anchor - vec2(0.0, next.toolbar_offset()),
                ));
            }
            if next != placement {
                self.popup_placement = next;
                ctx.request_repaint();
            }
        }
    }

    impl eframe::App for RecorderApp {
        fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            if self.quitting {
                return;
            }
            ui_theme::sync(ctx);
            if ctx.input(|input| input.viewport().close_requested()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.close_toolbar(ctx);
            }
            while let Ok(action) = self.tray_actions.try_recv() {
                match action {
                    TrayAction::OpenEditor => self.open_editor(ctx, false),
                    TrayAction::ShowToolbar => self.show_toolbar(ctx),
                    TrayAction::Restart => self.request_shutdown(ctx, Shutdown::Restart),
                    TrayAction::Exit => self.request_shutdown(ctx, Shutdown::Exit),
                }
            }
            self.update_task(ctx);
            if !self.toolbar_visible
                || self.panel != Some(PopupPage::Sources)
                || self.task.is_some()
            {
                self.preview.update(ctx, None);
                self.thumbnails.update(ctx, &[], None);
            }
            if let Some(shutdown) = self.shutdown {
                if self.task.is_none() && self.editor.close_for_shutdown(ctx) {
                    self.restart_requested
                        .store(shutdown == Shutdown::Restart, Ordering::Relaxed);
                    self.quitting = true;
                    drop(self.tray.take());
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            } else {
                self.editor.poll(ctx);
            }
        }

        fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
            [0.0; 4]
        }

        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            let ctx = ui.ctx().clone();
            let hint_id = egui::Id::new("toolbar-icon-hint");
            ctx.data_mut(|data| data.remove::<(String, f32)>(hint_id));
            let placement = self.popup_placement;

            let (motion, panel_height) = egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(Color32::TRANSPARENT)
                        .inner_margin(WINDOW_MARGIN as i8),
                )
                .show_inside(ui, |ui| {
                    let root = ui.max_rect();
                    let toolbar = egui::Rect::from_min_size(
                        root.min + vec2(0.0, placement.toolbar_offset()),
                        vec2(root.width(), TOOLBAR_HEIGHT - 2.0 * WINDOW_MARGIN),
                    );
                    let mut bar_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt("toolbar-layout")
                            .max_rect(toolbar)
                            .layout(Layout::top_down(Align::Min)),
                    );
                    let motion = self.toolbar(&mut bar_ui);
                    let panel = self.panel.map(|page| {
                        let top = if placement.above {
                            root.top()
                        } else {
                            TOOLBAR_HEIGHT + PANEL_GAP
                        };
                        self.popup_settings(ui, page, egui::pos2(root.left(), top), root.width())
                    });
                    (motion, panel)
                })
                .inner;

            let hint = if self.panel.is_some() {
                None
            } else {
                ctx.data(|data| data.get_temp::<(String, f32)>(hint_id))
            };
            let requested =
                panel_height.unwrap_or(if hint.is_some() { HINT_ROW_HEIGHT } else { 0.0 });
            if let Some((label, anchor)) = hint {
                let colors = ui_theme::palette(&ctx);
                let font = egui::FontId::proportional(12.0);
                let padding = vec2(10.0, 4.0);
                let text = ui.painter().layout_no_wrap(label, font, colors.text);
                let size = text.size() + 2.0 * padding;
                let right_limit = (TOOLBAR_WIDTH - WINDOW_MARGIN - size.x).max(WINDOW_MARGIN);
                let x = (anchor - size.x / 2.0).clamp(WINDOW_MARGIN, right_limit);
                let y = if placement.above {
                    (placement.toolbar_offset() - size.y - 2.0).max(0.0)
                } else {
                    TOOLBAR_HEIGHT + 2.0
                };
                let rect = egui::Rect::from_min_size(egui::pos2(x, y), size);
                let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, hint_id));
                painter.rect(
                    rect,
                    8,
                    colors.surface,
                    Stroke::new(1.0_f32, colors.border),
                    egui::StrokeKind::Inside,
                );
                painter.galley(rect.min + padding, text, colors.text);
            }
            self.place_popup(&ctx, motion, requested);
        }
    }

    fn panel_value(ui: &mut egui::Ui, width: f32, value: &str, hint: &str) {
        ui.allocate_ui_with_layout(
            vec2(width, 28.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_width(width);
                ui.add(egui::Label::new(RichText::new(value).size(12.0)).truncate())
                    .on_hover_text(hint);
            },
        );
    }

    fn source_card(
        ui: &mut egui::Ui,
        width: f32,
        source: &SourceOption,
        selected: bool,
        texture: Option<&egui::TextureHandle>,
        status: &str,
        reason: Option<&str>,
    ) -> egui::Response {
        let window = matches!(source.source, CaptureSource::Window(_));
        let wide_display = !window && width >= 160.0;
        let image_size = if wide_display {
            vec2(121.0, 74.0)
        } else if window {
            let image_width = width - 8.0;
            vec2(image_width, (image_width * 9.0 / 16.0).clamp(84.0, 160.0))
        } else {
            vec2(width - 8.0, 54.0)
        };
        let height = if window { image_size.y + 28.0 } else { 82.0 };
        let (rect, response) = ui.allocate_exact_size(vec2(width, height), egui::Sense::click());
        let label = format!("{}（{status}）", source.label);
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                response.enabled(),
                selected,
                label.as_str(),
            )
        });
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let colors = ui_theme::palette(ui.ctx());
            let fill = if selected {
                colors.selection
            } else if response.hovered() {
                colors.hover
            } else {
                colors.canvas
            };
            let edge = if selected || response.has_focus() {
                colors.accent
            } else {
                colors.border
            };
            painter.rect(
                rect,
                11,
                fill,
                Stroke::new(1.0_f32, edge),
                egui::StrokeKind::Inside,
            );
            let image = egui::Rect::from_min_size(rect.min + vec2(4.0, 4.0), image_size);
            paint_thumbnail(painter, image, texture);
            let offset = if wide_display {
                vec2(135.0, 23.0)
            } else {
                vec2(7.0, image_size.y + 7.0)
            };
            let label_size = if wide_display {
                13.0
            } else if window {
                12.0
            } else {
                11.0
            };
            let mut name = egui::text::LayoutJob::simple(
                source.label.clone(),
                egui::FontId::proportional(label_size),
                colors.text,
                (width - offset.x - 8.0).max(1.0),
            );
            name.wrap.max_rows = 1;
            painter.galley(rect.min + offset, painter.layout_job(name), colors.text);
            if wide_display {
                painter.text(
                    rect.min + vec2(135.0, 47.0),
                    egui::Align2::LEFT_TOP,
                    status,
                    egui::FontId::proportional(11.0),
                    colors.muted,
                );
            }
            if selected {
                let center = image.right_top() + vec2(-10.0, 10.0);
                painter.circle_filled(center, 7.0, colors.accent);
                painter.add(egui::Shape::line(
                    vec![
                        center + vec2(-3.1, 0.1),
                        center + vec2(-0.8, 2.4),
                        center + vec2(3.5, -2.3),
                    ],
                    Stroke::new(1.45_f32, colors.on_accent),
                ));
            }
        }
        let tip = source_tooltip(source);
        let tip = if let Some(reason) = reason {
            format!("{tip}\n{reason}")
        } else {
            tip
        };
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tip)
    }

    fn source_tooltip(source: &SourceOption) -> String {
        if source.details.is_empty() {
            source.label.clone()
        } else {
            format!("{}\n{}", source.label, source.details)
        }
    }

    enum Icon {
        Display,
        Window,
        Microphone(bool),
        ClickFollow,
        Record,
        Saving,
        Editor,
        EditorMinimize,
        EditorRestore,
        Settings,
        Saved,
        Warning,
        Refresh,
        Shared(SharedIcon),
    }

    impl Icon {
        fn is_record_control(&self) -> bool {
            matches!(
                self,
                Self::Record | Self::Saving | Self::Shared(SharedIcon::Stop)
            )
        }

        fn paint(
            &self,
            painter: &egui::Painter,
            center: egui::Pos2,
            color: Color32,
            background: Color32,
        ) {
            let stroke = Stroke::new(1.5_f32, color);
            let point = |x, y| center + vec2(x, y);
            let rectangle = |x, y, w, h, radius: u8| {
                painter.rect_stroke(
                    egui::Rect::from_min_size(point(x, y), vec2(w, h)),
                    radius,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            };
            let line = |x1, y1, x2, y2| {
                painter.line_segment([point(x1, y1), point(x2, y2)], stroke);
            };
            match self {
                Self::Display => {
                    rectangle(-8.0, -7.0, 16.0, 11.0, 2);
                    line(0.0, 4.0, 0.0, 7.0);
                    line(-4.0, 7.0, 4.0, 7.0);
                }
                Self::Window => {
                    rectangle(-8.0, -7.0, 16.0, 14.0, 2);
                    line(-7.0, -2.0, 7.0, -2.0);
                }
                Self::Microphone(active) => {
                    rectangle(-3.0, -8.0, 6.0, 11.0, 3);
                    painter.add(egui::Shape::line(
                        vec![
                            point(-5.5, -1.0),
                            point(-5.5, 2.0),
                            point(-4.3, 4.3),
                            point(-2.3, 5.5),
                            point(0.0, 6.0),
                            point(2.3, 5.5),
                            point(4.3, 4.3),
                            point(5.5, 2.0),
                            point(5.5, -1.0),
                        ],
                        stroke,
                    ));
                    line(0.0, 6.0, 0.0, 9.0);
                    if !active {
                        let slash = [point(-8.0, -8.0), point(8.0, 8.0)];
                        painter.line_segment(slash, Stroke::new(3.8_f32, background));
                        painter.line_segment(slash, stroke);
                    }
                }
                Self::ClickFollow => {
                    SharedIcon::ClickFollow.paint_scaled(painter, center, color, 1.0);
                }
                Self::Record => {
                    painter.circle_filled(center, 6.0, color);
                }
                Self::Saving => {
                    for x in [-5.0, 0.0, 5.0] {
                        painter.circle_filled(point(x, 0.0), 1.3, color);
                    }
                }
                Self::Editor => {
                    SharedIcon::Editor.paint_scaled(painter, center, color, 1.12);
                }
                Self::EditorMinimize => {
                    rectangle(-8.0, -6.0, 16.0, 12.0, 2);
                    line(-3.5, 0.0, 3.5, 0.0);
                }
                Self::EditorRestore => {
                    rectangle(-8.0, -6.0, 16.0, 12.0, 2);
                    line(0.0, 3.0, 0.0, -3.0);
                    line(-3.0, 0.0, 0.0, -3.0);
                    line(0.0, -3.0, 3.0, 0.0);
                }
                Self::Settings => {
                    SharedIcon::Settings.paint_scaled(painter, center, color, 1.08);
                }
                Self::Saved => {
                    line(-4.0, 0.0, -1.0, 3.0);
                    line(-1.0, 3.0, 4.0, -3.0);
                }
                Self::Warning => {
                    line(0.0, -4.5, 0.0, 0.0);
                    painter.circle_filled(point(0.0, 3.8), 0.9, color);
                }
                Self::Refresh => {
                    let arc = (0..=14)
                        .map(|step| {
                            let angle = (-140.0_f32 - step as f32 * 20.0).to_radians();
                            point(angle.cos() * 6.0, angle.sin() * 6.0)
                        })
                        .collect();
                    painter.add(egui::Shape::line(arc, stroke));
                    line(4.6, -3.9, 4.8, -7.2);
                    line(4.6, -3.9, 1.3, -4.0);
                }
                Self::Shared(icon) => icon.paint(painter, center, color),
            }
        }
    }

    fn icon_button(
        ui: &mut egui::Ui,
        icon: Icon,
        label: &str,
        selected: Option<bool>,
    ) -> egui::Response {
        let width = if matches!(icon, Icon::Editor) {
            36.0
        } else if icon.is_record_control() {
            40.0
        } else {
            32.0
        };
        let response = draw_icon_button(ui, icon, label, selected, [width, 34.0]);
        queue_icon_hint(ui, &response, label);
        response
    }

    fn panel_icon_button(ui: &mut egui::Ui, icon: Icon, label: &str) -> egui::Response {
        draw_icon_button(ui, icon, label, None, [PANEL_BUTTON_WIDTH, 28.0])
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(label)
            .on_disabled_hover_text(label)
    }

    fn draw_icon_button(
        ui: &mut egui::Ui,
        icon: Icon,
        label: &str,
        selected: Option<bool>,
        size: [f32; 2],
    ) -> egui::Response {
        let colors = ui_theme::palette(ui.ctx());
        let primary = icon.is_record_control();
        let (rect, response) = ui.allocate_exact_size(size.into(), egui::Sense::click());
        response.widget_info(|| {
            if let Some(selected) = selected {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Button,
                    response.enabled(),
                    selected,
                    label,
                )
            } else {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), label)
            }
        });
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let is_selected = selected.unwrap_or(false);
        let active = response.is_pointer_button_down_on();
        let fill = if active || primary || is_selected {
            colors.selection
        } else if response.hovered() {
            colors.hover
        } else {
            Color32::TRANSPARENT
        };
        let stroke = if response.has_focus() {
            Stroke::new(1.5_f32, colors.accent)
        } else {
            Stroke::NONE
        };
        let color = if !response.enabled() {
            colors.muted.gamma_multiply(0.55)
        } else if primary {
            colors.recording
        } else if is_selected {
            colors.accent
        } else {
            colors.text
        };
        let painter = ui.painter();
        if primary {
            let radius = (rect.width().min(rect.height()) - stroke.width) * 0.5;
            painter.circle(rect.center(), radius, fill, stroke);
        } else {
            painter.rect(rect, 9, fill, stroke, egui::StrokeKind::Inside);
        }
        let background = if fill == Color32::TRANSPARENT {
            colors.surface
        } else {
            fill
        };
        icon.paint(painter, rect.center(), color, background);
        response
    }

    fn queue_icon_hint(ui: &egui::Ui, response: &egui::Response, label: &str) {
        if !response.contains_pointer() || response.clicked() {
            return;
        }
        let (down, remaining) = ui.input(|input| {
            (
                input.pointer.primary_down(),
                0.45 - input.pointer.time_since_last_movement(),
            )
        });
        if down {
            return;
        }
        if remaining > 0.0 {
            ui.ctx().request_repaint_after_secs(remaining);
        } else {
            ui.data_mut(|data| {
                data.insert_temp(
                    egui::Id::new("toolbar-icon-hint"),
                    (label.to_owned(), response.rect.center().x),
                )
            });
        }
    }

    fn clock(duration: Duration) -> String {
        let total = duration.as_secs();
        if total < 3600 {
            format!("{:02}:{:02}", total / 60, total % 60)
        } else {
            format!(
                "{:02}:{:02}:{:02}",
                total / 3600,
                (total / 60) % 60,
                total % 60
            )
        }
    }

    fn shorten(text: &str, count: usize) -> String {
        let mut chars = text.chars();
        let mut result: String = chars.by_ref().take(count).collect();
        if chars.next().is_some() {
            result.push('…');
        }
        result
    }

    fn next_recording_path(directory: &Path) -> Result<PathBuf> {
        for number in 1..=9999 {
            let path = directory.join(format!("{APP_NAME}-{number:03}.mp4"));
            if !path.try_exists()? {
                return Ok(path);
            }
        }
        bail!("保存目录中的录像编号已用完，请更改目录")
    }

    #[cfg(test)]
    mod popup_tests {
        use super::*;
        use eframe::egui::pos2;

        #[test]
        fn fits_below_on_a_monitor_with_negative_coordinates() {
            let area = egui::Rect::from_min_max(pos2(-1280.0, -120.0), pos2(0.0, 840.0));
            let toolbar =
                egui::Rect::from_min_size(pos2(-900.0, 300.0), vec2(356.0, TOOLBAR_HEIGHT));
            let popup = PopupPlacement::fit(toolbar, Some(area), 225.0);
            assert_eq!(
                popup,
                PopupPlacement {
                    above: false,
                    extra_height: 225.0
                }
            );
        }

        #[test]
        fn flips_above_without_moving_the_toolbar_anchor() {
            let area = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1920.0, 1032.0));
            let toolbar =
                egui::Rect::from_min_size(pos2(700.0, 954.0), vec2(356.0, TOOLBAR_HEIGHT));
            let popup = PopupPlacement::fit(toolbar, Some(area), 225.0);
            assert!(popup.above);
            assert_eq!(popup.extra_height, 225.0);
            let window_top = toolbar.top() - popup.toolbar_offset();
            let collapsed = PopupPlacement::fit(toolbar, Some(area), 0.0);
            assert_eq!(
                window_top + popup.toolbar_offset() - collapsed.toolbar_offset(),
                toolbar.top(),
            );
        }

        #[test]
        fn limits_the_panel_to_the_larger_side_when_neither_side_fits() {
            let area = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(640.0, 240.0));
            let toolbar = egui::Rect::from_min_size(pos2(80.0, 120.0), vec2(356.0, TOOLBAR_HEIGHT));
            let popup = PopupPlacement::fit(toolbar, Some(area), 225.0);
            assert_eq!(
                popup,
                PopupPlacement {
                    above: true,
                    extra_height: 120.0
                }
            );
        }
    }
}
