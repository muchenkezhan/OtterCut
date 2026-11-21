use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::Result;
use eframe::egui::{self, Color32};
use takeseam::windows_preview::{self, PreviewFrame};
use takeseam::windows_recorder::CaptureSource;

use super::ui_icons::Icon;
use super::ui_theme;

const CACHE_CAPACITY: usize = 28;
const RETRY_DELAY: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct SourcePreview {
    source: Option<CaptureSource>,
    worker: Option<PreviewWorker>,
    texture: Option<egui::TextureHandle>,
    error: Option<String>,
}

struct PreviewWorker {
    stop: Arc<AtomicBool>,
    frames: Receiver<PreviewFrame>,
    thread: JoinHandle<Result<()>>,
}

impl SourcePreview {
    pub fn reset(&mut self) {
        if let Some(worker) = &self.worker {
            worker.stop.store(true, Ordering::Relaxed);
        }
        self.source = None;
        self.texture = None;
        self.error = None;
    }

    pub fn update(&mut self, ctx: &egui::Context, selected: Option<CaptureSource>) {
        if selected != self.source {
            self.reset();
            self.source = selected;
        }

        if let Some(worker) = &self.worker
            && !worker.stop.load(Ordering::Relaxed)
            && let Ok(frame) = worker.frames.try_recv()
        {
            let image =
                egui::ColorImage::from_rgba_unmultiplied([frame.width, frame.height], &frame.rgba);
            if let Some(texture) = &mut self.texture {
                texture.set(image, egui::TextureOptions::LINEAR);
            } else {
                self.texture = Some(ctx.load_texture(
                    "capture-source-preview",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
        }

        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.thread.is_finished())
            && let Some(worker) = self.worker.take()
        {
            let cancelled = worker.stop.load(Ordering::Relaxed);
            let result = worker.thread.join();
            if !cancelled {
                self.texture = None;
                self.error = Some(match result {
                    Ok(Ok(())) => String::from("来源预览已结束"),
                    Ok(Err(error)) => format!("{error:#}"),
                    Err(_) => String::from("来源预览意外停止"),
                });
            }
        }

        if self.worker.is_none()
            && self.error.is_none()
            && let Some(source) = self.source
        {
            match PreviewWorker::start(ctx, source, false) {
                Ok(worker) => self.worker = Some(worker),
                Err(error) => self.error = Some(format!("无法启动来源预览（{error}）")),
            }
        }
        if self.worker.is_some() {
            ctx.request_repaint_after(Duration::from_millis(180));
        }
    }

    pub fn status(&self) -> (&str, Option<&str>) {
        if let Some(error) = &self.error {
            ("暂时无法预览", Some(error.as_str()))
        } else if self.texture.is_some() {
            ("实时预览", None)
        } else if self.source.is_some() {
            ("正在获取画面…", None)
        } else {
            ("预览已暂停", None)
        }
    }

    pub fn texture(&self) -> Option<&egui::TextureHandle> {
        self.texture.as_ref()
    }
}

pub fn paint_thumbnail(
    painter: &egui::Painter,
    rect: egui::Rect,
    texture: Option<&egui::TextureHandle>,
) {
    let colors = ui_theme::palette(painter.ctx());
    painter.rect_filled(rect, 8, colors.monitor);
    if let Some(texture) = texture {
        let bounds = rect.shrink(1.0);
        let pixels = texture.size_vec2();
        let scale = (bounds.width() / pixels.x).min(bounds.height() / pixels.y);
        let image = egui::Rect::from_center_size(bounds.center(), pixels * scale);
        painter.add(
            egui::epaint::RectShape::filled(image, 7, Color32::WHITE).with_texture(
                texture.id(),
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            ),
        );
    } else {
        Icon::Media.paint(painter, rect.center(), colors.muted);
    }
}

impl Drop for SourcePreview {
    fn drop(&mut self) {
        self.reset();
    }
}

impl PreviewWorker {
    fn start(
        ctx: &egui::Context,
        source: CaptureSource,
        first_frame_only: bool,
    ) -> std::io::Result<Self> {
        let (sender, frames) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = Arc::clone(&stop);
        let frame_stop = Arc::clone(&stop);
        let repaint = ctx.clone();
        let thread = thread::Builder::new()
            .name(format!("{} 来源预览", takeseam::PRODUCT_NAME))
            .spawn(move || {
                let repaint_after_finish = repaint.clone();
                let result = windows_preview::capture(source, &cancel, move |frame| {
                    if sender.try_send(frame).is_ok() {
                        repaint.request_repaint();
                    }
                    if first_frame_only {
                        frame_stop.store(true, Ordering::Relaxed);
                    }
                });
                repaint_after_finish.request_repaint();
                result
            })?;
        Ok(Self {
            stop,
            frames,
            thread,
        })
    }
}

#[derive(Default)]
pub struct SourceThumbnails {
    cache: Vec<Thumbnail>,
    worker: Option<ThumbnailWorker>,
}

struct Thumbnail {
    source: CaptureSource,
    content: ThumbnailContent,
}

enum ThumbnailContent {
    Image(egui::TextureHandle),
    Unavailable { message: String, retry_at: Instant },
}

struct ThumbnailWorker {
    source: CaptureSource,
    capture: PreviewWorker,
    cancelled: bool,
}

impl ThumbnailWorker {
    fn cancel(&mut self) {
        self.cancelled = true;
        self.capture.stop.store(true, Ordering::Relaxed);
    }
}

impl SourceThumbnails {
    pub fn reset(&mut self) {
        self.cache.clear();
        if let Some(worker) = &mut self.worker {
            worker.cancel();
        }
    }

    pub fn update(
        &mut self,
        ctx: &egui::Context,
        visible: &[CaptureSource],
        live: Option<CaptureSource>,
    ) {
        let visible = &visible[..visible.len().min(CACHE_CAPACITY)];
        if let Some(worker) = &mut self.worker
            && (Some(worker.source) == live || !visible.contains(&worker.source))
        {
            worker.cancel();
        }

        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.capture.thread.is_finished())
            && let Some(worker) = self.worker.take()
        {
            let result = worker.capture.thread.join();
            if !worker.cancelled {
                let content = match worker.capture.frames.try_recv() {
                    Ok(frame) => ThumbnailContent::Image(ctx.load_texture(
                        "capture-source-thumbnail",
                        egui::ColorImage::from_rgba_unmultiplied(
                            [frame.width, frame.height],
                            &frame.rgba,
                        ),
                        egui::TextureOptions::LINEAR,
                    )),
                    Err(_) => unavailable(match result {
                        Ok(Ok(())) => String::from("没有取得缩略图"),
                        Ok(Err(error)) => format!("{error:#}"),
                        Err(_) => String::from("缩略图采集意外停止"),
                    }),
                };
                self.store(worker.source, content, visible);
            }
        }

        if self.worker.is_none() {
            let now = Instant::now();
            let mut retry_at = None::<Instant>;
            for &source in visible {
                if Some(source) == live {
                    continue;
                }
                match self.content(source) {
                    Some(ThumbnailContent::Image(_)) => continue,
                    Some(ThumbnailContent::Unavailable {
                        retry_at: retry, ..
                    }) if *retry > now => {
                        retry_at = Some(retry_at.map_or(*retry, |time| time.min(*retry)));
                        continue;
                    }
                    _ => {}
                }
                match PreviewWorker::start(ctx, source, true) {
                    Ok(capture) => {
                        self.worker = Some(ThumbnailWorker {
                            source,
                            capture,
                            cancelled: false,
                        });
                        break;
                    }
                    Err(error) => {
                        self.store(
                            source,
                            unavailable(format!("无法启动缩略图采集（{error}）")),
                            visible,
                        );
                        retry_at = Some(
                            retry_at.map_or(now + RETRY_DELAY, |time| time.min(now + RETRY_DELAY)),
                        );
                    }
                }
            }
            if let Some(retry) = retry_at {
                ctx.request_repaint_after(retry.saturating_duration_since(now));
            }
        }
        if self.worker.is_some() {
            ctx.request_repaint_after(Duration::from_millis(180));
        }
    }

    pub fn texture(&self, source: CaptureSource) -> Option<&egui::TextureHandle> {
        match self.content(source) {
            Some(ThumbnailContent::Image(texture)) => Some(texture),
            _ => None,
        }
    }

    pub fn error(&self, source: CaptureSource) -> Option<&str> {
        match self.content(source) {
            Some(ThumbnailContent::Unavailable { message, .. }) => Some(message),
            _ => None,
        }
    }

    fn content(&self, source: CaptureSource) -> Option<&ThumbnailContent> {
        self.cache
            .iter()
            .find(|entry| entry.source == source)
            .map(|entry| &entry.content)
    }

    fn store(
        &mut self,
        source: CaptureSource,
        content: ThumbnailContent,
        visible: &[CaptureSource],
    ) {
        self.cache.retain(|entry| entry.source != source);
        if self.cache.len() == CACHE_CAPACITY {
            let victim = self
                .cache
                .iter()
                .position(|entry| !visible.contains(&entry.source))
                .unwrap_or(0);
            self.cache.remove(victim);
        }
        self.cache.push(Thumbnail { source, content });
    }
}

impl Drop for SourceThumbnails {
    fn drop(&mut self) {
        if let Some(worker) = &mut self.worker {
            worker.cancel();
        }
    }
}

fn unavailable(message: String) -> ThumbnailContent {
    ThumbnailContent::Unavailable {
        message,
        retry_at: Instant::now() + RETRY_DELAY,
    }
}
