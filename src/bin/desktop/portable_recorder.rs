#[cfg(target_os = "macos")]
use std::io::{Read as _, Write as _};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use eframe::egui::{self, RichText, Stroke, vec2};
#[cfg(target_os = "macos")]
use eframe::egui::{Align, Layout};
use takeseam::PRODUCT_NAME;
use takeseam::i18n::{Language, Locale, key};
#[cfg(target_os = "macos")]
use takeseam::microphone::{InputMeter, input_devices};
use takeseam::platform::{self, RecordingRequest, SavedRecording};
#[cfg(target_os = "macos")]
use takeseam::platform::{CameraPreviewCommand, CameraPreviewShape};

#[cfg(target_os = "macos")]
use crate::open;
use crate::ui_icons::{self, ButtonStyle, Icon};
use crate::{APP_LOGO, load_logo, ui_theme};

#[cfg(target_os = "macos")]
const BAR_WIDTH: f32 = 432.0;
#[cfg(target_os = "linux")]
const BAR_WIDTH: f32 = 396.0;
const BAR_HEIGHT: f32 = 58.0;
const STATUS_HEIGHT: f32 = 38.0;
#[cfg(target_os = "macos")]
const PANEL_WIDTH: f32 = 370.0;
const TOOLBAR_LOGO: &[u8] = include_bytes!("../../../assets/ottercut-toolbar-logo.png");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(target_os = "macos")]
enum Panel {
    Sources,
    Devices,
}

pub fn launch() -> eframe::Result {
    let icon = eframe::icon_data::from_png_bytes(APP_LOGO)
        .map_err(|error| eframe::Error::AppCreation(Box::new(error)))?;
    let mark = eframe::icon_data::from_png_bytes(TOOLBAR_LOGO)
        .map_err(|error| eframe::Error::AppCreation(Box::new(error)))?;
    let viewport = egui::ViewportBuilder::default()
        .with_title(PRODUCT_NAME)
        .with_inner_size([BAR_WIDTH, BAR_HEIGHT])
        .with_resizable(false)
        .with_transparent(true)
        .with_decorations(false)
        .with_icon(icon);
    #[cfg(target_os = "macos")]
    let viewport = viewport.with_always_on_top();
    eframe::run_native(
        PRODUCT_NAME,
        eframe::NativeOptions {
            centered: true,
            viewport,
            ..Default::default()
        },
        Box::new(|creation| {
            let ctx = &creation.egui_ctx;
            ui_theme::initialize(ctx);
            Locale::initialize_context(ctx);
            let recorder = Recorder {
                mark: load_logo(ctx, &mark),
                folder: initial_folder(),
                microphone: false,
                #[cfg(target_os = "macos")]
                microphone_device: None,
                #[cfg(target_os = "macos")]
                microphones: Vec::new(),
                #[cfg(target_os = "macos")]
                meter: None,
                #[cfg(target_os = "macos")]
                meter_error: None,
                #[cfg(target_os = "macos")]
                meter_level: 0.0,
                #[cfg(target_os = "macos")]
                camera_device: None,
                #[cfg(target_os = "macos")]
                camera_mode: None,
                #[cfg(target_os = "macos")]
                cameras: Vec::new(),
                #[cfg(target_os = "macos")]
                camera_preview: None,
                #[cfg(target_os = "macos")]
                camera_preview_shape: camera_preview_shape_preference(),
                #[cfg(target_os = "macos")]
                record_camera: false,
                #[cfg(target_os = "macos")]
                click_follow: false,
                #[cfg(target_os = "macos")]
                source_window: false,
                #[cfg(target_os = "macos")]
                source: None,
                #[cfg(target_os = "macos")]
                source_selection: None,
                #[cfg(target_os = "macos")]
                record_after_source: false,
                #[cfg(target_os = "macos")]
                device_scan: None,
                #[cfg(target_os = "macos")]
                panel: None,
                job: None,
                editor: None,
                notice: None,
                notice_visible: false,
                close_after_save: false,
                drag_sent: false,
                window_height: BAR_HEIGHT,
            };
            #[cfg(target_os = "macos")]
            let recorder = {
                let mut recorder = recorder;
                if let Err(error) = recorder.start_device_scan(ctx) {
                    recorder.notice(format!("{error:#}"), true);
                }
                recorder
            };
            Ok(Box::new(recorder))
        }),
    )
}

enum Event {
    Started,
    #[cfg(target_os = "macos")]
    CameraPreviewVisibility(bool),
    #[cfg(target_os = "macos")]
    CameraPreviewClosed,
    #[cfg(target_os = "macos")]
    CameraPreviewUnavailable,
    #[cfg(target_os = "macos")]
    CameraUnavailable,
    Finished(Result<Option<SavedRecording>>),
}

#[cfg(target_os = "macos")]
enum SourceSelectionEvent {
    Finished(Result<Option<String>>),
}

struct Job {
    events: Receiver<Event>,
    stop: Arc<AtomicBool>,
    started: Option<Instant>,
    #[cfg(target_os = "macos")]
    camera_preview: Option<bool>,
    #[cfg(target_os = "macos")]
    camera_preview_commands: mpsc::Sender<CameraPreviewCommand>,
}

struct Notice {
    text: String,
    error: bool,
}

struct Recorder {
    mark: egui::TextureHandle,
    folder: PathBuf,
    microphone: bool,
    #[cfg(target_os = "macos")]
    microphone_device: Option<String>,
    #[cfg(target_os = "macos")]
    microphones: Vec<platform::MicrophoneDevice>,
    #[cfg(target_os = "macos")]
    meter: Option<InputMeter>,
    #[cfg(target_os = "macos")]
    meter_error: Option<String>,
    #[cfg(target_os = "macos")]
    meter_level: f32,
    #[cfg(target_os = "macos")]
    camera_device: Option<String>,
    #[cfg(target_os = "macos")]
    camera_mode: Option<platform::CameraMode>,
    #[cfg(target_os = "macos")]
    cameras: Vec<platform::CameraDevice>,
    #[cfg(target_os = "macos")]
    camera_preview: Option<Child>,
    #[cfg(target_os = "macos")]
    camera_preview_shape: CameraPreviewShape,
    #[cfg(target_os = "macos")]
    record_camera: bool,
    #[cfg(target_os = "macos")]
    click_follow: bool,
    #[cfg(target_os = "macos")]
    source_window: bool,
    #[cfg(target_os = "macos")]
    source: Option<String>,
    #[cfg(target_os = "macos")]
    source_selection: Option<Receiver<SourceSelectionEvent>>,
    #[cfg(target_os = "macos")]
    record_after_source: bool,
    #[cfg(target_os = "macos")]
    device_scan: Option<Receiver<Result<platform::DeviceList>>>,
    #[cfg(target_os = "macos")]
    panel: Option<Panel>,
    job: Option<Job>,
    editor: Option<Child>,
    notice: Option<Notice>,
    notice_visible: bool,
    close_after_save: bool,
    drag_sent: bool,
    window_height: f32,
}

impl Recorder {
    fn start_recording(&mut self, ctx: &egui::Context) -> Result<()> {
        #[cfg(target_os = "macos")]
        if self.source.is_none() {
            return self.choose_source(ctx, true);
        }
        let output = self.next_path()?;
        let request = RecordingRequest {
            output,
            microphone: self.microphone,
            duration: None,
            #[cfg(target_os = "macos")]
            source: self.source.clone(),
            #[cfg(target_os = "macos")]
            microphone_device: self.microphone_device.clone(),
            #[cfg(target_os = "macos")]
            camera_device: if self.record_camera {
                Some(
                    self.camera_device
                        .clone()
                        .context("请在设备设置中选择摄像头")?,
                )
            } else {
                None
            },
            #[cfg(target_os = "macos")]
            camera_mode: if self.record_camera {
                Some(self.camera_mode.clone().context("请为摄像头选择画质")?)
            } else {
                None
            },
            #[cfg(target_os = "macos")]
            click_follow: self.click_follow,
        };
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let (events, receive) = mpsc::channel();
        let repaint = ctx.clone();
        #[cfg(target_os = "macos")]
        let (camera_preview_commands, preview_commands) = mpsc::channel();
        #[cfg(target_os = "macos")]
        let camera_preview = {
            let visible = self.camera_preview.is_some();
            self.stop_camera_preview();
            if self.record_camera {
                let _ = camera_preview_commands
                    .send(CameraPreviewCommand::Shape(self.camera_preview_shape));
                if visible {
                    let _ = camera_preview_commands.send(CameraPreviewCommand::Visibility(true));
                }
            }
            self.record_camera.then_some(visible)
        };
        thread::Builder::new()
            .name("ottercut-capture".into())
            .spawn(move || {
                #[cfg(target_os = "macos")]
                let result = platform::record_with_camera_preview(
                    request,
                    &worker_stop,
                    &preview_commands,
                    |event| {
                        let event = match event {
                            platform::RecordingEvent::Started => Event::Started,
                            platform::RecordingEvent::CameraPreviewVisibility(visible) => {
                                Event::CameraPreviewVisibility(visible)
                            }
                            platform::RecordingEvent::CameraPreviewClosed => {
                                Event::CameraPreviewClosed
                            }
                            platform::RecordingEvent::CameraPreviewUnavailable => {
                                Event::CameraPreviewUnavailable
                            }
                            platform::RecordingEvent::CameraUnavailable => Event::CameraUnavailable,
                        };
                        let _ = events.send(event);
                        repaint.request_repaint();
                    },
                );
                #[cfg(target_os = "linux")]
                let result = platform::record(request, &worker_stop, || {
                    let _ = events.send(Event::Started);
                    repaint.request_repaint();
                });
                let _ = events.send(Event::Finished(result));
                repaint.request_repaint();
            })
            .context("无法创建录制线程")?;
        self.job = Some(Job {
            events: receive,
            stop,
            started: None,
            #[cfg(target_os = "macos")]
            camera_preview,
            #[cfg(target_os = "macos")]
            camera_preview_commands,
        });
        #[cfg(target_os = "macos")]
        {
            self.panel = None;
            self.notice("正在启动录制…", false);
        }
        #[cfg(target_os = "linux")]
        self.notice("请在系统选择器中选择屏幕或窗口", false);
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn choose_source(&mut self, ctx: &egui::Context, record_after: bool) -> Result<()> {
        if self.source_selection.is_some() || self.job.is_some() {
            return Ok(());
        }
        let mode = if self.source_window {
            "window"
        } else {
            "display"
        };
        let context = ctx.clone();
        let (send, receive) = mpsc::channel();
        thread::Builder::new()
            .name("ottercut-source-picker".into())
            .spawn(move || {
                let result = platform::select_source(mode);
                let _ = send.send(SourceSelectionEvent::Finished(result));
                context.request_repaint();
            })
            .context("无法打开来源选择器")?;
        self.source_selection = Some(receive);
        self.record_after_source = record_after;
        self.notice("请在 macOS 系统选择器中选择来源", false);
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn update_source_selection(&mut self, ctx: &egui::Context) {
        let Some(selection) = &self.source_selection else {
            return;
        };
        match selection.try_recv() {
            Ok(SourceSelectionEvent::Finished(Ok(Some(source)))) => {
                self.source = Some(source);
                self.source_selection = None;
                self.notice("录制来源已选择", false);
                if self.record_after_source {
                    self.record_after_source = false;
                    if let Err(error) = self.start_recording(ctx) {
                        self.notice(format!("{error:#}"), true);
                    }
                }
            }
            Ok(SourceSelectionEvent::Finished(Ok(None))) => {
                self.source_selection = None;
                self.record_after_source = false;
                self.notice("来源选择已取消", false);
            }
            Ok(SourceSelectionEvent::Finished(Err(error))) => {
                self.source_selection = None;
                self.record_after_source = false;
                self.notice(format!("{error:#}"), true);
            }
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.source_selection = None;
                self.notice("来源选择器已中断", true);
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn start_device_scan(&mut self, ctx: &egui::Context) -> Result<()> {
        if self.device_scan.is_some() {
            return Ok(());
        }
        let context = ctx.clone();
        let (send, receive) = mpsc::channel();
        thread::Builder::new()
            .name("ottercut-device-scan".into())
            .spawn(move || {
                let result = platform::capture_devices().map(|mut devices| {
                    if let Ok(inputs) = input_devices() {
                        for microphone in &mut devices.microphones {
                            microphone.meter_id = inputs
                                .iter()
                                .find(|device| device.name == microphone.name)
                                .map(|device| device.id.clone());
                        }
                    }
                    devices
                });
                let _ = send.send(result);
                context.request_repaint();
            })
            .context("无法启动设备检测")?;
        self.device_scan = Some(receive);
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn update_device_scan(&mut self, ctx: &egui::Context) {
        let Some(scan) = &self.device_scan else {
            return;
        };
        match scan.try_recv() {
            Ok(Ok(devices)) => {
                let selected_microphone = self.microphone_device.as_ref().and_then(|previous| {
                    devices
                        .microphones
                        .iter()
                        .find(|device| &device.id == previous)
                        .map(|device| device.id.clone())
                });
                let camera = devices
                    .cameras
                    .iter()
                    .find(|device| self.camera_device.as_ref() == Some(&device.id))
                    .or_else(|| {
                        devices
                            .cameras
                            .iter()
                            .find(|device| !device.modes.is_empty())
                    })
                    .or_else(|| devices.cameras.first());
                let camera_device = camera.map(|device| device.id.clone());
                let camera_mode = camera.and_then(|device| {
                    self.camera_mode
                        .as_ref()
                        .filter(|mode| {
                            self.camera_device == camera_device && device.modes.contains(mode)
                        })
                        .cloned()
                        .or_else(|| preferred_camera_mode(&device.modes))
                });
                let camera_changed = self.camera_device != camera_device;
                self.microphones = devices.microphones;
                self.cameras = devices.cameras;
                self.microphone_device = selected_microphone;
                self.camera_device = camera_device;
                self.camera_mode = camera_mode;
                self.device_scan = None;
                if camera_changed || self.camera_mode.is_none() {
                    self.record_camera = false;
                    self.stop_camera_preview();
                }
            }
            Ok(Err(error)) => {
                self.device_scan = None;
                self.notice(format!("无法读取录制设备（{error:#}）"), true);
            }
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.device_scan = None;
                self.notice("设备检测已中断", true);
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn macos_toolbar(&mut self, ui: &mut egui::Ui) {
        let capture_active = self.job.is_some();
        let finishing = self
            .job
            .as_ref()
            .is_some_and(|job| job.stop.load(Ordering::Relaxed));
        let waiting_for_source = self.source_selection.is_some();
        let colors = ui_theme::palette(ui.ctx());
        let locale = Locale::from_context(ui.ctx());
        let source_icon = if self.source_window {
            Icon::Window
        } else {
            Icon::Display
        };
        let source_label = if self.source_window {
            locale.text(key::SOURCE_WINDOW)
        } else {
            locale.text(key::SOURCE_DISPLAY)
        };
        let record_label = if finishing {
            locale.text(key::RECORD_SAVING)
        } else if capture_active {
            locale.text(key::RECORD_STOP)
        } else if waiting_for_source {
            locale.text(key::SOURCE_PICKER)
        } else {
            locale.text(key::RECORD_START)
        };

        ui.spacing_mut().item_spacing.x = 3.0;
        ui.add(egui::Image::new((self.mark.id(), vec2(25.0, 25.0))));
        if ui_icons::icon_button(
            ui,
            source_icon,
            source_label,
            !capture_active && !waiting_for_source,
            if self.source.is_some() {
                ButtonStyle::Selected
            } else {
                ButtonStyle::Quiet
            },
        )
        .clicked()
        {
            self.panel = (self.panel != Some(Panel::Sources)).then_some(Panel::Sources);
        }
        let microphone_label = if self.microphone {
            locale.text(key::MICROPHONE_DISABLE)
        } else {
            locale.text(key::MICROPHONE_ENABLE)
        };
        if ui_icons::icon_button(
            ui,
            Icon::Microphone,
            microphone_label,
            !capture_active,
            if self.microphone {
                ButtonStyle::Selected
            } else {
                ButtonStyle::Quiet
            },
        )
        .clicked()
        {
            self.microphone = !self.microphone;
        }
        let camera_label = if self.record_camera {
            key::CAMERA_DISABLE
        } else if self.camera_mode.is_some() {
            key::CAMERA_ENABLE
        } else if self.device_scan.is_some() {
            key::DEVICE_READING
        } else if self.cameras.is_empty() {
            key::DEVICE_NO_CAMERA
        } else {
            key::DEVICE_NO_QUALITY
        };
        if ui_icons::icon_button(
            ui,
            Icon::Camera,
            locale.text(camera_label),
            !capture_active && (self.record_camera || self.camera_mode.is_some()),
            if self.record_camera {
                ButtonStyle::Selected
            } else {
                ButtonStyle::Quiet
            },
        )
        .clicked()
        {
            self.set_record_camera(!self.record_camera);
        }
        let preview = match &self.job {
            Some(job) => job.camera_preview,
            None => (self.record_camera || self.camera_preview.is_some())
                .then_some(self.camera_preview.is_some()),
        };
        if let Some(visible) = preview
            && ui_icons::icon_button(
                ui,
                if visible { Icon::Eye } else { Icon::EyeOff },
                locale.text(if visible {
                    key::CAMERA_PREVIEW_HIDE
                } else {
                    key::CAMERA_PREVIEW_SHOW
                }),
                !finishing,
                if visible {
                    ButtonStyle::Selected
                } else {
                    ButtonStyle::Quiet
                },
            )
            .clicked()
        {
            self.set_camera_preview(!visible);
        }
        if ui_icons::icon_button(
            ui,
            Icon::ClickFollow,
            if self.click_follow {
                locale.text(key::CLICK_FOLLOW_DISABLE)
            } else {
                locale.text(key::CLICK_FOLLOW_ENABLE)
            },
            !capture_active,
            if self.click_follow {
                ButtonStyle::Selected
            } else {
                ButtonStyle::Quiet
            },
        )
        .clicked()
        {
            self.click_follow = !self.click_follow;
        }
        let elapsed = self
            .job
            .as_ref()
            .and_then(|job| job.started)
            .map_or(0, |start| start.elapsed().as_secs());
        ui.allocate_ui_with_layout(
            vec2(53.0, 32.0),
            Layout::centered_and_justified(egui::Direction::LeftToRight),
            |ui| {
                ui.label(
                    RichText::new(format!("{:02}:{:02}", elapsed / 60, elapsed % 60))
                        .monospace()
                        .color(if capture_active {
                            colors.recording
                        } else {
                            colors.text
                        }),
                );
            },
        );
        if record_button(
            ui,
            capture_active,
            !finishing && !waiting_for_source,
            record_label,
        ) {
            if capture_active {
                self.request_stop();
            } else if let Err(error) = self.start_recording(ui.ctx()) {
                self.notice(format!("{error:#}"), true);
            }
        }
        let editor_label = if self.editor.is_some() {
            locale.text(key::EDITOR_OPEN)
        } else {
            locale.text(key::EDITOR_OPEN)
        };
        if ui_icons::icon_button(
            ui,
            Icon::Editor,
            editor_label,
            self.editor.is_none(),
            if self.editor.is_some() {
                ButtonStyle::Selected
            } else {
                ButtonStyle::Quiet
            },
        )
        .clicked()
            && let Err(error) = self.open_editor()
        {
            self.notice(format!("{error:#}"), true);
        }
        if ui_icons::icon_button(
            ui,
            Icon::Settings,
            locale.text(key::SETTINGS),
            !capture_active,
            if self.panel == Some(Panel::Devices) {
                ButtonStyle::Selected
            } else {
                ButtonStyle::Quiet
            },
        )
        .clicked()
        {
            self.panel = (self.panel != Some(Panel::Devices)).then_some(Panel::Devices);
            if self.panel == Some(Panel::Devices)
                && let Err(error) = self.start_device_scan(ui.ctx())
            {
                self.notice(format!("{error:#}"), true);
            }
        }
        if ui_icons::icon_button(
            ui,
            Icon::Close,
            locale.text(key::CLOSE),
            true,
            ButtonStyle::Quiet,
        )
        .clicked()
        {
            if capture_active {
                self.close_after_save = true;
                self.request_stop();
            } else {
                self.stop_camera_preview();
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn request_stop(&mut self) {
        if let Some(job) = &self.job
            && !job.stop.swap(true, Ordering::Relaxed)
        {
            let text = if job.started.is_some() {
                "正在保存录像…"
            } else {
                "正在取消…"
            };
            self.notice(text, false);
        }
    }

    fn finish(&mut self, result: Result<Option<SavedRecording>>) {
        #[cfg(target_os = "macos")]
        let restore_preview = !self.close_after_save
            && self.record_camera
            && result.is_ok()
            && self
                .job
                .as_ref()
                .is_some_and(|job| job.camera_preview == Some(true));
        self.job = None;
        match result {
            Ok(Some(saved)) => {
                let name = saved
                    .path
                    .file_name()
                    .unwrap_or(saved.path.as_os_str())
                    .to_string_lossy();
                let camera = saved.camera_path.as_ref().map(|path| {
                    path.file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy()
                });
                let mut text = format!("已保存 {name}");
                if let Some(camera) = camera {
                    text.push_str(&format!(" · 摄像头 {camera}"));
                }
                if let Some(warning) = saved.warning {
                    text.push_str(&format!(" · {warning}"));
                }
                self.notice(text, false);
            }
            Ok(None) => self.notice("录制已取消", false),
            Err(error) => {
                self.close_after_save = false;
                self.notice(format!("{error:#}"), true);
            }
        }
        #[cfg(target_os = "macos")]
        if restore_preview {
            self.set_camera_preview(true);
        }
    }

    fn update_recording(&mut self, ctx: &egui::Context) {
        let Some(job) = &mut self.job else {
            return;
        };
        loop {
            match job.events.try_recv() {
                Ok(Event::Started) => {
                    job.started = Some(Instant::now());
                    if !job.stop.load(Ordering::Relaxed) {
                        self.notice = None;
                    }
                }
                #[cfg(target_os = "macos")]
                Ok(Event::CameraPreviewVisibility(visible)) => job.camera_preview = Some(visible),
                #[cfg(target_os = "macos")]
                Ok(Event::CameraPreviewClosed) => job.camera_preview = Some(false),
                #[cfg(target_os = "macos")]
                Ok(Event::CameraPreviewUnavailable) => {
                    job.camera_preview = None;
                    self.notice = Some(Notice {
                        text: Locale::from_context(ctx)
                            .text(key::CAMERA_PREVIEW_UNAVAILABLE)
                            .to_owned(),
                        error: false,
                    });
                }
                #[cfg(target_os = "macos")]
                Ok(Event::CameraUnavailable) => job.camera_preview = None,
                Ok(Event::Finished(result)) => {
                    self.finish(result);
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100));
                    break;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.finish(Err(anyhow::anyhow!("录制线程已中断")));
                    break;
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn stop_camera_preview(&mut self) {
        if let Some(mut preview) = self.camera_preview.take() {
            let _ = preview.kill();
            let _ = preview.wait();
        }
    }

    #[cfg(target_os = "macos")]
    fn set_record_camera(&mut self, enabled: bool) {
        self.record_camera = enabled;
        self.set_camera_preview(enabled);
    }

    #[cfg(target_os = "macos")]
    fn set_camera_preview(&mut self, visible: bool) {
        if let Some(job) = &mut self.job {
            if job.camera_preview.is_some()
                && job
                    .camera_preview_commands
                    .send(CameraPreviewCommand::Visibility(visible))
                    .is_ok()
            {
                job.camera_preview = Some(visible);
            }
        } else if !visible {
            self.stop_camera_preview();
        } else if self.camera_preview.is_none()
            && let Some(device) = self.camera_device.as_deref()
        {
            match platform::start_camera_preview(device, self.camera_preview_shape) {
                Ok(preview) => self.camera_preview = Some(preview),
                Err(error) => {
                    self.record_camera = false;
                    self.notice(format!("{error:#}"), true);
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn set_camera_preview_shape(&mut self, shape: CameraPreviewShape) {
        self.camera_preview_shape = shape;
        if let Some(stdin) = self
            .camera_preview
            .as_mut()
            .and_then(|preview| preview.stdin.as_mut())
            && let Err(error) = CameraPreviewCommand::Shape(shape).write_to(stdin)
        {
            self.notice(format!("无法更新摄像头预览形状（{error}）"), true);
        }
        if let Some(path) = camera_preview_shape_path() {
            let save = || -> std::io::Result<()> {
                let directory = path
                    .parent()
                    .expect("preview shape preference has a parent folder");
                std::fs::create_dir_all(directory)?;
                let mut staged = tempfile::NamedTempFile::new_in(directory)?;
                writeln!(staged, "{}", shape.as_str())?;
                staged.persist(path).map_err(|error| error.error)?;
                Ok(())
            };
            if let Err(error) = save() {
                self.notice(format!("无法保存摄像头预览形状（{error}）"), true);
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn update_camera_preview(&mut self, ctx: &egui::Context) {
        let Some(preview) = self.camera_preview.as_mut() else {
            return;
        };
        match preview.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    self.record_camera = false;
                    let mut details = String::new();
                    if let Some(stderr) = preview.stderr.take() {
                        let _ = stderr.take(4096).read_to_string(&mut details);
                    }
                    self.notice(
                        if details.trim().is_empty() {
                            format!("无法打开摄像头预览（{status}）")
                        } else {
                            details.trim().to_owned()
                        },
                        true,
                    );
                }
                self.camera_preview = None;
            }
            Ok(None) => ctx.request_repaint_after(Duration::from_millis(150)),
            Err(error) => {
                self.stop_camera_preview();
                self.record_camera = false;
                self.notice(format!("无法检查摄像头预览状态（{error}）"), true);
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn show_sources_panel(&mut self, ui: &mut egui::Ui) {
        let colors = ui_theme::palette(ui.ctx());
        let locale = Locale::from_context(ui.ctx());
        ui.set_min_width(PANEL_WIDTH);
        ui.horizontal(|ui| {
            ui.label(RichText::new(locale.text(key::SOURCE_TITLE)).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("关闭").clicked() {
                    self.panel = None;
                }
            });
        });
        let mut source_window = self.source_window;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut source_window, false, locale.text(key::SOURCE_DISPLAY));
            ui.selectable_value(&mut source_window, true, locale.text(key::SOURCE_WINDOW));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(
                        self.job.is_none() && self.source_selection.is_none(),
                        egui::Button::new(locale.text(key::SOURCE_PICKER)),
                    )
                    .clicked()
                    && let Err(error) = self.choose_source(ui.ctx(), false)
                {
                    self.notice(format!("{error:#}"), true);
                }
            });
        });
        if source_window != self.source_window {
            self.source_window = source_window;
            self.source = None;
        }
        let selected = self
            .source
            .as_deref()
            .and_then(|source| serde_json::from_str::<serde_json::Value>(source).ok());
        let label = selected
            .as_ref()
            .map_or(locale.text(key::SOURCE_NOT_SELECTED), |selection| {
                if selection.get("style").and_then(serde_json::Value::as_i64) == Some(1) {
                    locale.text(key::SOURCE_SELECTED_WINDOW)
                } else {
                    locale.text(key::SOURCE_SELECTED_DISPLAY)
                }
            });
        ui.label(RichText::new(label).color(if self.source.is_some() {
            colors.text
        } else {
            colors.muted
        }));
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new(locale.text(key::SOURCE_SAVE_TO)).color(colors.muted));
            let mut folder = self.folder.display().to_string();
            ui.add_sized(
                vec2((ui.available_width() - 105.0).max(20.0), 24.0),
                egui::TextEdit::singleline(&mut folder).interactive(false),
            );
            if ui_icons::icon_button(
                ui,
                Icon::FolderOpen,
                "打开保存目录",
                true,
                ButtonStyle::Quiet,
            )
            .clicked()
                && let Err(error) = open(&self.folder)
            {
                self.notice(format!("{error:#}"), true);
            }
            if ui.button(locale.text(key::SOURCE_CHANGE_FOLDER)).clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_directory(&self.folder)
                    .pick_folder()
            {
                self.folder = path;
            }
        });
    }

    #[cfg(target_os = "macos")]
    fn show_devices_panel(&mut self, ui: &mut egui::Ui) {
        let colors = ui_theme::palette(ui.ctx());
        let locale = Locale::from_context(ui.ctx());
        ui.spacing_mut().item_spacing = vec2(4.0, 3.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(locale.text(key::DEVICE_TITLE)).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(locale.text(key::CLOSE)).clicked() {
                    self.panel = None;
                    self.meter = None;
                }
                if ui
                    .add_enabled(
                        self.device_scan.is_none(),
                        egui::Button::new(locale.text(key::DEVICE_REFRESH)),
                    )
                    .clicked()
                    && let Err(error) = self.start_device_scan(ui.ctx())
                {
                    self.notice(format!("{error:#}"), true);
                }
                let (theme_icon, theme_hint) = if ui.ctx().theme() == egui::Theme::Dark {
                    (Icon::Sun, locale.text(key::SWITCH_LIGHT))
                } else {
                    (Icon::Moon, locale.text(key::SWITCH_DARK))
                };
                if ui_icons::icon_button(ui, theme_icon, theme_hint, true, ButtonStyle::Quiet)
                    .clicked()
                    && let Err(error) = ui_theme::toggle(ui.ctx())
                {
                    self.notice(format!("{}: {error}", theme_hint), true);
                }
                ui.menu_button(locale.text(key::LANGUAGE), |ui| {
                    let locale = Locale::from_context(ui.ctx());
                    for language in [Language::ZhCn, Language::EnUs] {
                        let label = match language {
                            Language::ZhCn => locale.text(key::LANGUAGE_ZH),
                            Language::EnUs => locale.text(key::LANGUAGE_EN),
                        };
                        if ui
                            .selectable_label(locale.language() == language, label)
                            .clicked()
                        {
                            if let Err(error) = Locale::set_context_and_persist(ui.ctx(), language)
                            {
                                self.notice(
                                    format!("{}: {error}", locale.text(key::LANGUAGE)),
                                    true,
                                );
                            }
                            ui.close();
                        }
                    }
                });
            });
        });
        if self.device_scan.is_some() {
            ui.label(RichText::new(locale.text(key::DEVICE_READING)).color(colors.muted));
        }
        egui::ScrollArea::vertical()
            .id_salt("macos-device-settings")
            .max_height(190.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.add_enabled_ui(self.job.is_none(), |ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(
                            &mut self.microphone,
                            locale.text(key::DEVICE_RECORD_MICROPHONE),
                        );
                        let selected = self
                            .microphones
                            .iter()
                            .find(|device| Some(&device.id) == self.microphone_device.as_ref())
                            .map_or(locale.text(key::DEVICE_SYSTEM_DEFAULT), |device| {
                                device.name.as_str()
                            });
                        egui::ComboBox::from_id_salt("macos-microphone-device")
                            .width(130.0)
                            .truncate()
                            .selected_text(selected)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.microphone_device,
                                    None,
                                    locale.text(key::DEVICE_SYSTEM_DEFAULT),
                                );
                                for device in &self.microphones {
                                    ui.selectable_value(
                                        &mut self.microphone_device,
                                        Some(device.id.clone()),
                                        &device.name,
                                    );
                                }
                            });
                    });
                    ui.horizontal(|ui| {
                        if ui
                            .small_button(if self.meter.is_some() {
                                locale.text(key::DEVICE_TEST_STOP)
                            } else {
                                locale.text(key::DEVICE_TEST_MICROPHONE)
                            })
                            .clicked()
                        {
                            if self.meter.take().is_none() {
                                let input_id = self.microphone_device.as_ref().and_then(|id| {
                                    self.microphones
                                        .iter()
                                        .find(|device| &device.id == id)
                                        .and_then(|device| device.meter_id.clone())
                                });
                                match InputMeter::open(input_id.as_deref()) {
                                    Ok(meter) => {
                                        self.meter = Some(meter);
                                        self.meter_error = None;
                                    }
                                    Err(error) => self.meter_error = Some(format!("{error:#}")),
                                }
                            }
                        }
                        if let Some(meter) = &mut self.meter {
                            match meter.level() {
                                Ok(level) => {
                                    self.meter_level = level.max(self.meter_level * 0.65);
                                    ui.ctx().request_repaint_after(Duration::from_millis(40));
                                    ui.add(
                                        egui::ProgressBar::new(self.meter_level)
                                            .desired_width(148.0)
                                            .text(format!("{:.0}%", self.meter_level * 100.0)),
                                    );
                                }
                                Err(error) => {
                                    self.meter_error = Some(format!("{error:#}"));
                                    self.meter = None;
                                }
                            }
                        }
                    });
                    if let Some(error) = &self.meter_error {
                        ui.colored_label(colors.danger, error);
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        let mut record_camera = self.record_camera;
                        if ui
                            .add_enabled(
                                self.record_camera || self.camera_mode.is_some(),
                                egui::Checkbox::new(
                                    &mut record_camera,
                                    locale.text(key::DEVICE_RECORD_CAMERA),
                                ),
                            )
                            .changed()
                        {
                            self.set_record_camera(record_camera);
                        }
                        let mut choice = self.camera_device.clone();
                        let selected = self
                            .cameras
                            .iter()
                            .find(|device| Some(&device.id) == choice.as_ref())
                            .map_or(locale.text(key::DEVICE_NO_CAMERA), |device| {
                                device.name.as_str()
                            });
                        egui::ComboBox::from_id_salt("macos-camera-device")
                            .width(170.0)
                            .truncate()
                            .selected_text(selected)
                            .show_ui(ui, |ui| {
                                for device in &self.cameras {
                                    ui.selectable_value(
                                        &mut choice,
                                        Some(device.id.clone()),
                                        &device.name,
                                    );
                                }
                            });
                        if choice != self.camera_device {
                            let previewing = self.camera_preview.is_some();
                            self.stop_camera_preview();
                            self.camera_device = choice;
                            self.camera_mode = self
                                .camera_device
                                .as_deref()
                                .and_then(|id| self.cameras.iter().find(|device| device.id == id))
                                .and_then(|device| preferred_camera_mode(&device.modes));
                            if self.camera_mode.is_none() {
                                self.record_camera = false;
                            }
                            if previewing {
                                self.set_camera_preview(true);
                            }
                        }
                    });
                    if let Some(camera) = self.camera_device.as_deref()
                        && let Some(device) = self.cameras.iter().find(|device| device.id == camera)
                    {
                        let mut mode = self.camera_mode.clone();
                        let selected = mode.as_ref().map_or_else(
                            || locale.text(key::DEVICE_NO_QUALITY).to_owned(),
                            camera_mode_label,
                        );
                        ui.horizontal(|ui| {
                            ui.label(locale.text(key::DEVICE_QUALITY));
                            egui::ComboBox::from_id_salt("macos-camera-mode")
                                .width(175.0)
                                .truncate()
                                .selected_text(selected)
                                .show_ui(ui, |ui| {
                                    for option in &device.modes {
                                        ui.selectable_value(
                                            &mut mode,
                                            Some(option.clone()),
                                            camera_mode_label(option),
                                        );
                                    }
                                });
                        });
                        self.camera_mode = mode;
                    }
                    ui.horizontal(|ui| {
                        let previewing = self.camera_preview.is_some();
                        if ui
                            .add_enabled(
                                previewing || self.camera_device.is_some(),
                                egui::Button::new(if previewing {
                                    locale.text(key::DEVICE_CLOSE_CAMERA)
                                } else {
                                    locale.text(key::DEVICE_OPEN_CAMERA)
                                }),
                            )
                            .clicked()
                        {
                            self.set_camera_preview(!previewing);
                        }
                        ui.label(locale.text(key::CAMERA_PREVIEW_SHAPE));
                        let mut shape = self.camera_preview_shape;
                        let shape_label = |shape| {
                            locale.text(match shape {
                                CameraPreviewShape::Circle => key::CAMERA_PREVIEW_CIRCLE,
                                CameraPreviewShape::Square => key::CAMERA_PREVIEW_SQUARE,
                            })
                        };
                        egui::ComboBox::from_id_salt("macos-camera-preview-shape")
                            .width(80.0)
                            .selected_text(shape_label(shape))
                            .show_ui(ui, |ui| {
                                for option in
                                    [CameraPreviewShape::Circle, CameraPreviewShape::Square]
                                {
                                    ui.selectable_value(&mut shape, option, shape_label(option));
                                }
                            });
                        if shape != self.camera_preview_shape {
                            self.set_camera_preview_shape(shape);
                        }
                    });
                });
            });
    }

    fn next_path(&self) -> Result<PathBuf> {
        for index in 1..=9999 {
            let candidate = self.folder.join(format!("{PRODUCT_NAME}-{index:03}.mp4"));
            if !candidate.try_exists()? {
                return Ok(candidate);
            }
        }
        bail!("保存目录中的录像编号已用完，请更改目录");
    }

    fn open_editor(&mut self) -> Result<()> {
        self.editor = Some(
            Command::new(std::env::current_exe()?)
                .arg("--editor")
                .arg(&self.folder)
                .spawn()
                .context("无法打开剪辑台")?,
        );
        Ok(())
    }

    fn notice(&mut self, text: impl Into<String>, error: bool) {
        self.notice = Some(Notice {
            text: text.into(),
            error,
        });
    }

    #[cfg(target_os = "linux")]
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let capture_active = self.job.is_some();
        let finishing = self
            .job
            .as_ref()
            .is_some_and(|job| job.stop.load(Ordering::Relaxed));
        let locale = Locale::from_context(ui.ctx());
        let elapsed = self
            .job
            .as_ref()
            .and_then(|job| job.started)
            .map_or(0, |time| time.elapsed().as_secs());
        let colors = ui_theme::palette(ui.ctx());
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.add(egui::Image::new((self.mark.id(), vec2(25.0, 25.0))));
        if ui_icons::icon_button(
            ui,
            Icon::FolderOpen,
            "更改录像保存目录",
            !capture_active,
            ButtonStyle::Quiet,
        )
        .on_hover_text(self.folder.display().to_string())
        .clicked()
            && let Some(folder) = rfd::FileDialog::new()
                .set_directory(&self.folder)
                .pick_folder()
        {
            self.folder = folder;
        }
        let microphone_style = if self.microphone {
            ButtonStyle::Selected
        } else {
            ButtonStyle::Quiet
        };
        let microphone_label = if self.microphone {
            "关闭麦克风"
        } else {
            "开启默认麦克风"
        };
        if ui_icons::icon_button(
            ui,
            Icon::Microphone,
            microphone_label,
            !capture_active,
            microphone_style,
        )
        .clicked()
        {
            self.microphone = !self.microphone;
        }
        ui.allocate_ui_with_layout(
            vec2(59.0, 30.0),
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
            |ui| {
                ui.label(
                    RichText::new(format!("{:02}:{:02}", elapsed / 60, elapsed % 60))
                        .monospace()
                        .color(colors.text),
                );
            },
        );
        let action = if finishing {
            "正在保存…"
        } else if capture_active {
            "停止录制"
        } else {
            "选择来源并录制"
        };
        if record_button(ui, capture_active, !finishing, action) {
            if capture_active {
                self.request_stop();
            } else if let Err(error) = self.start_recording(ui.ctx()) {
                self.notice(format!("{error:#}"), true);
            }
        }
        let editor_style = if self.editor.is_some() {
            ButtonStyle::Selected
        } else {
            ButtonStyle::Quiet
        };
        let editor_label = if self.editor.is_some() {
            "剪辑台已打开"
        } else {
            "打开剪辑台"
        };
        if ui_icons::icon_button(
            ui,
            Icon::Editor,
            editor_label,
            self.editor.is_none(),
            editor_style,
        )
        .clicked()
            && let Err(error) = self.open_editor()
        {
            self.notice(format!("{error:#}"), true);
        }
        let theme_icon = if ui.ctx().theme() == egui::Theme::Dark {
            Icon::Sun
        } else {
            Icon::Moon
        };
        if ui_icons::icon_button(ui, theme_icon, "切换主题", true, ButtonStyle::Quiet).clicked()
            && let Err(error) = ui_theme::toggle(ui.ctx())
        {
            self.notice(format!("无法保存主题（{error}）"), true);
        }
        ui.menu_button(locale.text(key::LANGUAGE), |ui| {
            for language in [Language::ZhCn, Language::EnUs] {
                let selected = locale.language() == language;
                let label = match language {
                    Language::ZhCn => locale.text(key::LANGUAGE_ZH),
                    Language::EnUs => locale.text(key::LANGUAGE_EN),
                };
                if ui.selectable_label(selected, label).clicked() {
                    if let Err(error) = Locale::set_context_and_persist(ui.ctx(), language) {
                        self.notice(format!("{}: {error}", locale.text(key::LANGUAGE)), true);
                    }
                    ui.close();
                }
            }
        });
        if ui_icons::icon_button(ui, Icon::Close, "关闭录制工具条", true, ButtonStyle::Quiet)
            .clicked()
        {
            if capture_active {
                self.close_after_save = true;
                self.request_stop();
            } else {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        self.macos_toolbar(ui);
    }
}

impl eframe::App for Recorder {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ui_theme::sync(ctx);
        #[cfg(target_os = "macos")]
        {
            self.update_source_selection(ctx);
            self.update_device_scan(ctx);
            self.update_camera_preview(ctx);
        }
        if ctx.input(|input| input.viewport().close_requested()) {
            if self.job.is_some() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.close_after_save = true;
                self.request_stop();
            } else {
                #[cfg(target_os = "macos")]
                self.stop_camera_preview();
            }
        }
        self.update_recording(ctx);
        if self.close_after_save && self.job.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if let Some(editor) = &mut self.editor {
            if editor.try_wait().is_ok_and(|status| status.is_some()) {
                self.editor = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }
        let notice_visible = self.notice.is_some();
        self.notice_visible = notice_visible;
        #[cfg(target_os = "macos")]
        let panel_height = match self.panel {
            Some(Panel::Sources) => 165.0,
            Some(Panel::Devices) => 250.0,
            None => 0.0,
        };
        #[cfg(target_os = "linux")]
        let panel_height = 0.0;
        let height = BAR_HEIGHT + panel_height + if notice_visible { STATUS_HEIGHT } else { 0.0 };
        if (self.window_height - height).abs() > 1.0 {
            self.window_height = height;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(BAR_WIDTH, height)));
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let colors = ui_theme::palette(ui.ctx());
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(4))
            .show_inside(ui, |ui| {
                let bar = egui::Frame::new()
                    .fill(colors.surface)
                    .stroke(Stroke::new(1.0_f32, colors.border))
                    .corner_radius(23)
                    .inner_margin(egui::Margin::symmetric(10, 4))
                    .show(ui, |ui| {
                        let layout = egui::Layout::left_to_right(egui::Align::Center);
                        ui.allocate_ui_with_layout(
                            vec2(ui.available_width(), 34.0),
                            layout,
                            |ui| {
                                self.toolbar(ui);
                            },
                        );
                    });
                let (pressed, origin, pointer) = ui.input(|input| {
                    (
                        input.pointer.primary_down(),
                        input.pointer.press_origin(),
                        input.pointer.interact_pos(),
                    )
                });
                if !pressed {
                    self.drag_sent = false;
                } else if !self.drag_sent
                    && let Some((origin, pointer)) = origin.zip(pointer)
                    && bar.response.rect.contains(origin)
                    && (pointer - origin).length_sq() >= 25.0
                {
                    self.drag_sent = true;
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                #[cfg(target_os = "macos")]
                if let Some(panel) = self.panel {
                    egui::Frame::new()
                        .fill(colors.surface)
                        .stroke(Stroke::new(1.0_f32, colors.border))
                        .corner_radius(14)
                        .inner_margin(egui::Margin::symmetric(12, 9))
                        .show(ui, |ui| match panel {
                            Panel::Sources => self.show_sources_panel(ui),
                            Panel::Devices => self.show_devices_panel(ui),
                        });
                }
                if let Some(notice) = &self.notice {
                    let color = if notice.error {
                        colors.danger
                    } else {
                        colors.text
                    };
                    let dismissed = ui
                        .horizontal(|ui| {
                            let label = egui::Label::new(RichText::new(&notice.text).color(color))
                                .truncate();
                            let response = ui.add_sized(
                                vec2((ui.available_width() - 38.0).max(0.0), 25.0),
                                label,
                            );
                            response.on_hover_text(&notice.text);
                            ui_icons::icon_button(
                                ui,
                                Icon::Close,
                                "关闭提示",
                                self.job.is_none(),
                                ButtonStyle::Quiet,
                            )
                            .clicked()
                        })
                        .inner;
                    if dismissed {
                        self.notice = None;
                    }
                }
            });
    }
}

#[cfg(target_os = "macos")]
fn camera_preview_shape_path() -> Option<PathBuf> {
    Some(takeseam::config_directory()?.join("camera-preview-shape"))
}

#[cfg(target_os = "macos")]
fn camera_preview_shape_preference() -> CameraPreviewShape {
    let value = camera_preview_shape_path().and_then(|path| std::fs::read_to_string(path).ok());
    match value.as_deref().map(str::trim) {
        Some("square") => CameraPreviewShape::Square,
        _ => CameraPreviewShape::default(),
    }
}

#[cfg(target_os = "macos")]
fn preferred_camera_mode(modes: &[platform::CameraMode]) -> Option<platform::CameraMode> {
    modes
        .iter()
        .min_by_key(|mode| {
            (
                mode.fps_milli.abs_diff(30_000),
                mode.width.abs_diff(1280) + mode.height.abs_diff(720),
            )
        })
        .cloned()
}

#[cfg(target_os = "macos")]
fn camera_mode_label(mode: &platform::CameraMode) -> String {
    format!(
        "{} × {} · {:.3} fps",
        mode.width,
        mode.height,
        f64::from(mode.fps_milli) / 1000.0
    )
}

fn record_button(ui: &mut egui::Ui, active: bool, enabled: bool, label: &str) -> bool {
    let colors = ui_theme::palette(ui.ctx());
    ui.add_enabled(enabled, |ui: &mut egui::Ui| {
        let (rect, response) = ui.allocate_exact_size(vec2(34.0, 34.0), egui::Sense::click());
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
        let painter = ui.painter();
        let center = rect.center();
        painter.circle_filled(
            center,
            15.0,
            if response.hovered() {
                colors.hover
            } else {
                colors.control
            },
        );
        if active {
            Icon::Stop.paint(painter, center, colors.recording);
        } else {
            painter.circle_filled(center, 5.5, colors.recording);
        }
        if response.has_focus() {
            painter.circle_stroke(center, 15.0, Stroke::new(1.0_f32, colors.accent));
        }
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(label)
    })
    .clicked()
}

fn initial_folder() -> PathBuf {
    let home = std::env::home_dir().unwrap_or_else(std::env::temp_dir);
    let preferred = home.join(if cfg!(target_os = "macos") {
        "Movies"
    } else {
        "Videos"
    });
    if preferred.is_dir() { preferred } else { home }
}
