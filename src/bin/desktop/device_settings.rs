use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Duration;

use anyhow::{Context as _, Result};
use eframe::egui::{self, Align, Layout, RichText, vec2};
use takeseam::camera::{self, CameraDevice, CameraMode, CameraPreview, CameraSelection};
use takeseam::microphone::{InputDevice, InputMeter, input_devices};

use super::editor::recording_ffmpeg_path;
use super::ui_theme;

type DeviceResult<T> = std::result::Result<T, String>;

struct DeviceScan {
    audio: DeviceResult<Vec<InputDevice>>,
    video: DeviceResult<(PathBuf, Vec<CameraDevice>)>,
}

#[derive(Default)]
pub(super) struct DeviceSettings {
    scanned: bool,
    scan: Option<Receiver<DeviceScan>>,
    microphones: Vec<InputDevice>,
    microphone: Option<String>,
    microphone_error: Option<String>,
    meter: Option<InputMeter>,
    meter_level: f32,
    meter_error: Option<String>,
    ffmpeg: Option<PathBuf>,
    cameras: Vec<CameraDevice>,
    camera: Option<CameraDevice>,
    camera_error: Option<String>,
    mode_scan: Option<Receiver<DeviceResult<Vec<CameraMode>>>>,
    modes: Vec<CameraMode>,
    mode: Option<CameraMode>,
    record_camera: bool,
    camera_preview: Option<CameraPreview>,
    camera_texture: Option<egui::TextureHandle>,
    camera_preview_error: Option<String>,
}

impl DeviceSettings {
    pub(super) fn microphone_device(&self) -> Option<&str> {
        self.microphone.as_deref()
    }

    pub(super) fn camera_selection(&self) -> Result<Option<CameraSelection>> {
        if self.record_camera {
            self.selected_camera()
                .map(Some)
                .context("请在设备设置中选择可用的摄像头和画质")
        } else {
            Ok(None)
        }
    }

    pub(super) fn stop_previews(&mut self) {
        self.meter = None;
        self.meter_level = 0.0;
        self.stop_camera_preview();
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui, microphone: &mut bool, busy: bool) {
        if !self.scanned && !busy {
            self.refresh(ui.ctx());
        }
        self.poll(ui.ctx());
        let colors = ui_theme::palette(ui.ctx());
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        ui.spacing_mut().interact_size.y = 28.0;
        ui.horizontal(|ui| {
            ui.label(RichText::new("设备设置").size(13.0).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(!busy && self.scan.is_none(), egui::Button::new("刷新设备"))
                    .clicked()
                {
                    self.refresh(ui.ctx());
                }
            });
        });
        if busy {
            ui.label(
                RichText::new("录制中 · 设备已锁定")
                    .size(11.0)
                    .color(colors.muted),
            );
        } else if self.scan.is_some() {
            ui.label(
                RichText::new("正在读取设备…")
                    .size(11.0)
                    .color(colors.muted),
            );
        }
        ui.add_enabled_ui(!busy && self.scan.is_none(), |ui| {
            self.show_microphone(ui, microphone);
            ui.separator();
            self.show_camera(ui);
        });
    }

    fn refresh(&mut self, ctx: &egui::Context) {
        self.scanned = true;
        self.stop_previews();
        self.mode_scan = None;
        self.microphone_error = None;
        self.camera_error = None;
        self.meter_error = None;
        self.camera_preview_error = None;
        let context = ctx.clone();
        let (send, receive) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("recording-device-scan".into())
            .spawn(move || {
                let audio = input_devices().map_err(|error| format!("{error:#}"));
                let video = recording_ffmpeg_path()
                    .and_then(|ffmpeg| {
                        let devices = camera::devices(&ffmpeg)?;
                        Ok((ffmpeg, devices))
                    })
                    .map_err(|error| format!("{error:#}"));
                let _ = send.send(DeviceScan { audio, video });
                context.request_repaint();
            });
        match worker {
            Ok(_) => self.scan = Some(receive),
            Err(error) => self.camera_error = Some(format!("无法启动设备检测（{error}）")),
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(scan) = take_ready(&mut self.scan) {
            match scan {
                Ok(scan) => self.apply_scan(scan, ctx),
                Err(()) => self.camera_error = Some("设备检测意外停止".into()),
            }
        }
        if let Some(modes) = take_ready(&mut self.mode_scan) {
            match modes {
                Ok(Ok(modes)) => {
                    if self
                        .mode
                        .as_ref()
                        .is_none_or(|current| !modes.contains(current))
                    {
                        self.mode = preferred_mode(&modes).cloned();
                    }
                    self.modes = modes;
                }
                Ok(Err(error)) => self.camera_error = Some(error),
                Err(()) => self.camera_error = Some("画质检测意外停止".into()),
            }
        }
    }

    fn apply_scan(&mut self, scan: DeviceScan, ctx: &egui::Context) {
        match scan.audio {
            Ok(microphones) => self.microphones = microphones,
            Err(error) => {
                self.microphones.clear();
                self.microphone_error = Some(error);
            }
        }
        self.modes.clear();
        match scan.video {
            Ok((ffmpeg, cameras)) => {
                self.ffmpeg = Some(ffmpeg);
                let selected = self.camera.as_ref().and_then(|previous| {
                    cameras
                        .iter()
                        .find(|device| device.selector == previous.selector)
                });
                if selected.is_none() {
                    self.mode = None;
                }
                self.camera = selected.or_else(|| cameras.first()).cloned();
                self.cameras = cameras;
                self.scan_modes(ctx);
            }
            Err(error) => {
                self.ffmpeg = None;
                self.cameras.clear();
                self.camera = None;
                self.mode = None;
                self.camera_error = Some(error);
            }
        }
    }

    fn scan_modes(&mut self, ctx: &egui::Context) {
        let (Some(ffmpeg), Some(device)) = (self.ffmpeg.clone(), self.camera.clone()) else {
            return;
        };
        let context = ctx.clone();
        let (send, receive) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("camera-mode-scan".into())
            .spawn(move || {
                let formats = camera::modes(&ffmpeg, &device).map_err(|error| format!("{error:#}"));
                let _ = send.send(formats);
                context.request_repaint();
            });
        match worker {
            Ok(_) => self.mode_scan = Some(receive),
            Err(error) => self.camera_error = Some(format!("无法启动画质检测（{error}）")),
        }
    }

    fn show_microphone(&mut self, ui: &mut egui::Ui, enabled: &mut bool) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("麦克风").size(12.0).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.checkbox(enabled, "录制麦克风");
            });
        });
        let mut choice = self.microphone.clone();
        let name = self.microphone.as_deref().map_or("系统默认", |id| {
            self.microphones
                .iter()
                .find(|device| device.id == id)
                .map_or("已断开", |device| device.name.as_str())
        });
        ui.horizontal(|ui| {
            let label = ui.label("输入设备");
            egui::ComboBox::from_id_salt("recording-microphone")
                .width(ui.available_width().max(1.0))
                .truncate()
                .selected_text(name)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut choice, None, "系统默认");
                    for device in &self.microphones {
                        ui.selectable_value(&mut choice, Some(device.id.clone()), &device.name);
                    }
                })
                .response
                .labelled_by(label.id);
        });
        if choice != self.microphone {
            self.meter = None;
            self.meter_level = 0.0;
            self.meter_error = None;
            self.microphone = choice;
        }

        ui.horizontal(|ui| {
            let label = if self.meter.is_some() {
                "停止测试"
            } else {
                "测试麦克风"
            };
            if ui.button(label).clicked() {
                if self.meter.take().is_none() {
                    match InputMeter::open(self.microphone_device()) {
                        Ok(meter) => {
                            self.meter = Some(meter);
                            self.meter_error = None;
                        }
                        Err(error) => self.meter_error = Some(format!("{error:#}")),
                    }
                }
                self.meter_level = 0.0;
            }
            if let Some(meter) = &mut self.meter {
                match meter.level() {
                    Ok(level) => {
                        self.meter_level = level.max(self.meter_level * 0.65);
                        ui.ctx().request_repaint_after(Duration::from_millis(40));
                        ui.add(
                            egui::ProgressBar::new(self.meter_level)
                                .desired_width(ui.available_width())
                                .text(format!("输入电平 {:.0}%", self.meter_level * 100.0)),
                        );
                    }
                    Err(error) => {
                        self.meter_error = Some(format!("{error:#}"));
                        self.meter = None;
                    }
                }
            }
        });
        if let Some(error) = self
            .meter_error
            .as_deref()
            .or(self.microphone_error.as_deref())
        {
            ui.colored_label(ui_theme::palette(ui.ctx()).danger, error);
        } else if self.scanned && self.scan.is_none() && self.microphones.is_empty() {
            ui.label(RichText::new("未找到麦克风输入").size(11.0));
        }
    }

    fn show_camera(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("摄像头").size(12.0).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let ready = self.selected_camera().is_some() || self.record_camera;
                ui.add_enabled(
                    ready,
                    egui::Checkbox::new(&mut self.record_camera, "录制摄像头"),
                );
            });
        });
        let mut choice = self.camera.clone();
        let device_label = self
            .camera
            .as_ref()
            .map_or("未检测到摄像头", |camera| camera.name.as_str());
        ui.horizontal(|ui| {
            let label = ui.label("设备");
            egui::ComboBox::from_id_salt("recording-camera")
                .width(ui.available_width().max(1.0))
                .truncate()
                .selected_text(device_label)
                .show_ui(ui, |ui| {
                    for device in &self.cameras {
                        ui.selectable_value(&mut choice, Some(device.clone()), &device.name);
                    }
                })
                .response
                .labelled_by(label.id);
        });
        if choice != self.camera {
            self.stop_camera_preview();
            self.camera = choice;
            self.mode = None;
            self.modes.clear();
            self.camera_error = None;
            self.camera_preview_error = None;
            self.scan_modes(ui.ctx());
        }

        let mut mode = self.mode.clone();
        let mode_label = if self.mode_scan.is_some() {
            "正在检测画质…".to_owned()
        } else {
            self.mode
                .as_ref()
                .map_or_else(|| "无可用画质".to_owned(), CameraMode::label)
        };
        ui.horizontal(|ui| {
            let label = ui.label("画质");
            egui::ComboBox::from_id_salt("recording-camera-mode")
                .width(ui.available_width().max(1.0))
                .truncate()
                .selected_text(mode_label)
                .show_ui(ui, |ui| {
                    for option in &self.modes {
                        ui.selectable_value(&mut mode, Some(option.clone()), option.label());
                    }
                })
                .response
                .labelled_by(label.id);
        });
        if mode != self.mode {
            let previewing = self.camera_preview.is_some();
            self.stop_camera_preview();
            self.mode = mode;
            self.camera_preview_error = None;
            if previewing {
                self.start_camera_preview(ui.ctx());
            }
        }

        let previewing = self.camera_preview.is_some();
        let preview_label = if previewing {
            "关闭摄像头预览"
        } else {
            "打开摄像头预览"
        };
        if ui
            .add_enabled(
                previewing || self.selected_camera().is_some(),
                egui::Button::new(preview_label),
            )
            .clicked()
        {
            if previewing {
                self.stop_camera_preview();
            } else {
                self.start_camera_preview(ui.ctx());
            }
        }
        self.paint_camera_preview(ui);
        if let Some(error) = self
            .camera_preview_error
            .as_deref()
            .or(self.camera_error.as_deref())
        {
            ui.colored_label(ui_theme::palette(ui.ctx()).danger, error);
        }
    }

    fn selected_camera(&self) -> Option<CameraSelection> {
        let mode = self
            .mode
            .as_ref()
            .filter(|mode| self.modes.contains(mode))?
            .clone();
        Some(CameraSelection {
            ffmpeg: self.ffmpeg.clone()?,
            device: self.camera.clone()?,
            mode,
        })
    }

    fn stop_camera_preview(&mut self) {
        self.camera_preview = None;
        self.camera_texture = None;
    }

    fn start_camera_preview(&mut self, ctx: &egui::Context) {
        let Some(selection) = self.selected_camera() else {
            return;
        };
        let context = ctx.clone();
        self.camera_preview_error = None;
        match CameraPreview::start(&selection, move || context.request_repaint()) {
            Ok(preview) => self.camera_preview = Some(preview),
            Err(error) => self.camera_preview_error = Some(format!("{error:#}")),
        }
    }

    fn paint_camera_preview(&mut self, ui: &mut egui::Ui) {
        let Some(preview) = &mut self.camera_preview else {
            return;
        };
        ui.ctx().request_repaint_after(Duration::from_millis(100));
        match preview.poll() {
            Ok(Some(frame)) => {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [frame.width as usize, frame.height as usize],
                    &frame.rgba,
                );
                if let Some(texture) = &mut self.camera_texture {
                    texture.set(image, egui::TextureOptions::LINEAR);
                } else {
                    self.camera_texture = Some(ui.ctx().load_texture(
                        "recording-camera-preview",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
            Ok(None) => {}
            Err(error) => {
                self.stop_camera_preview();
                self.camera_preview_error = Some(format!("{error:#}"));
                return;
            }
        }
        if let Some(texture) = &self.camera_texture {
            let original = texture.size_vec2();
            let scale = (ui.available_width() / original.x).min(160.0 / original.y);
            ui.add(
                egui::Image::new((texture.id(), original * scale))
                    .corner_radius(6)
                    .alt_text("摄像头画面"),
            );
        } else {
            ui.label(RichText::new("正在打开摄像头…").size(11.0));
        }
    }
}

fn take_ready<T>(pending: &mut Option<Receiver<T>>) -> Option<std::result::Result<T, ()>> {
    let result = match pending.as_ref()?.try_recv() {
        Ok(value) => Ok(value),
        Err(TryRecvError::Empty) => return None,
        Err(TryRecvError::Disconnected) => Err(()),
    };
    *pending = None;
    Some(result)
}

fn preferred_mode(modes: &[CameraMode]) -> Option<&CameraMode> {
    modes.iter().min_by_key(|mode| {
        (
            mode.fps_milli.abs_diff(30_000),
            mode.width.abs_diff(1280) + mode.height.abs_diff(720),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_panel_shows_controls_with_no_hardware_and_blocks_invalid_camera_selection() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut settings = DeviceSettings {
            scanned: true,
            ..Default::default()
        };
        let mut microphone = false;
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            settings.show(ui, &mut microphone, false)
        });
        let tree = output.platform_output.accesskit_update.unwrap();
        for label in [
            "设备设置",
            "输入设备",
            "测试麦克风",
            "录制摄像头",
            "打开摄像头预览",
        ] {
            assert!(
                tree.nodes
                    .iter()
                    .any(|(_, node)| node.label() == Some(label) || node.value() == Some(label)),
                "缺少 {label}"
            );
        }
        assert!(settings.camera_selection().unwrap().is_none());
        settings.record_camera = true;
        assert!(settings.camera_selection().is_err());
    }
}
