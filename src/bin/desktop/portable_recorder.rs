use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use eframe::egui::{self, RichText, Stroke, vec2};
use takeseam::PRODUCT_NAME;
use takeseam::i18n::{Language, Locale, key};
use takeseam::platform::{self, RecordingRequest, SavedRecording};

use crate::ui_icons::{self, ButtonStyle, Icon};
use crate::{APP_LOGO, load_logo, ui_theme};

const BAR_WIDTH: f32 = 336.0;
const BAR_HEIGHT: f32 = 54.0;
const STATUS_HEIGHT: f32 = 38.0;
const TOOLBAR_LOGO: &[u8] = include_bytes!("../../../assets/ottercut-toolbar-logo.png");

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
            Ok(Box::new(Recorder {
                mark: load_logo(ctx, &mark),
                folder: initial_folder(),
                microphone: false,
                job: None,
                editor: None,
                notice: None,
                notice_visible: false,
                close_after_save: false,
                drag_sent: false,
            }))
        }),
    )
}

enum Event {
    Started,
    Finished(Result<Option<SavedRecording>>),
}

struct Job {
    events: Receiver<Event>,
    stop: Arc<AtomicBool>,
    started: Option<Instant>,
}

struct Notice {
    text: String,
    error: bool,
}

struct Recorder {
    mark: egui::TextureHandle,
    folder: PathBuf,
    microphone: bool,
    job: Option<Job>,
    editor: Option<Child>,
    notice: Option<Notice>,
    notice_visible: bool,
    close_after_save: bool,
    drag_sent: bool,
}

impl Recorder {
    fn start_recording(&mut self, ctx: &egui::Context) -> Result<()> {
        let output = self.next_path()?;
        let request = RecordingRequest {
            output,
            microphone: self.microphone,
            duration: None,
        };
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let (events, receive) = mpsc::channel();
        let repaint = ctx.clone();
        thread::Builder::new()
            .name("ottercut-capture".into())
            .spawn(move || {
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
        });
        self.notice("选择要录制的屏幕或窗口", false);
        Ok(())
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
        self.job = None;
        match result {
            Ok(Some(saved)) => {
                let name = saved
                    .path
                    .file_name()
                    .unwrap_or(saved.path.as_os_str())
                    .to_string_lossy();
                let text = match saved.warning {
                    Some(warning) => format!("已保存 {name} · {warning}"),
                    None => format!("已保存 {name}"),
                };
                self.notice(text, false);
            }
            Ok(None) => self.notice("录制已取消", false),
            Err(error) => {
                self.close_after_save = false;
                self.notice(format!("{error:#}"), true);
            }
        }
    }

    fn update_recording(&mut self, ctx: &egui::Context) {
        let Some(job) = &mut self.job else {
            return;
        };
        let mut recording_started = false;
        loop {
            match job.events.try_recv() {
                Ok(Event::Started) => {
                    job.started = Some(Instant::now());
                    recording_started = !job.stop.load(Ordering::Relaxed);
                }
                Ok(Event::Finished(result)) => {
                    self.finish(result);
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    if recording_started {
                        self.notice = None;
                    }
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
            Icon::Media,
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
}

impl eframe::App for Recorder {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ui_theme::sync(ctx);
        if self.job.is_some() && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_after_save = true;
            self.request_stop();
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
        if self.notice_visible != notice_visible {
            self.notice_visible = notice_visible;
            let height = BAR_HEIGHT + if notice_visible { STATUS_HEIGHT } else { 0.0 };
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
                    .stroke(Stroke::new(1.0, colors.border))
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
            painter.circle_stroke(center, 15.0, Stroke::new(1.0, colors.accent));
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
