use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;

use eframe::egui;

use super::media::MediaTools;

const MATERIAL_CACHE_LIMIT: usize = 96;
const TIMELINE_CACHE_LIMIT: usize = 384;
const PREVIEW_CACHE_LIMIT: usize = 24;
const QUEUE_LIMIT: usize = 8;

pub(super) struct Thumbnails {
    ctx: egui::Context,
    worker: Option<Worker>,
    pending: HashSet<FrameKey>,
    cache: HashMap<FrameKey, Cached>,
    latest_requested: HashMap<PathBuf, u64>,
    mode: Mode,
    order: u64,
}

#[derive(Clone, Copy)]
enum Mode {
    Material,
    Timeline,
    Preview,
}

impl Mode {
    fn cache_limit(self) -> usize {
        match self {
            Self::Material => MATERIAL_CACHE_LIMIT,
            Self::Timeline => TIMELINE_CACHE_LIMIT,
            Self::Preview => PREVIEW_CACHE_LIMIT,
        }
    }

    fn texture_name(self) -> &'static str {
        match self {
            Self::Material => "asset-thumbnail",
            Self::Timeline => "timeline-thumbnail",
            Self::Preview => "preview-thumbnail",
        }
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct FrameKey {
    path: PathBuf,
    timestamp_us: u64,
}

struct Cached {
    texture: Option<egui::TextureHandle>,
    last_used: u64,
}

struct Worker {
    requests: SyncSender<FrameKey>,
    updates: Receiver<(FrameKey, Option<egui::ColorImage>)>,
    stopped: Arc<AtomicBool>,
}

impl Worker {
    fn start(tools: MediaTools, ctx: egui::Context, mode: Mode) -> Option<Self> {
        let (requests, incoming) = mpsc::sync_channel::<FrameKey>(QUEUE_LIMIT);
        let (outgoing, updates) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let close = Arc::clone(&stopped);
        thread::Builder::new()
            .name(format!("{} thumbnails", takeseam::PRODUCT_NAME))
            .spawn(move || {
                while let Ok(mut key) = incoming.recv() {
                    if close.load(Ordering::Acquire) {
                        break;
                    }
                    if matches!(mode, Mode::Preview) {
                        while let Ok(next) = incoming.try_recv() {
                            key = next;
                        }
                    }
                    let image = match mode {
                        Mode::Material => tools.thumbnail(&key.path, &close),
                        Mode::Timeline => {
                            tools.timeline_thumbnail(&key.path, key.timestamp_us, &close)
                        }
                        Mode::Preview => {
                            tools.preview_thumbnail(&key.path, key.timestamp_us, &close)
                        }
                    };
                    if outgoing.send((key, image.ok())).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .ok()?;
        Some(Self {
            requests,
            updates,
            stopped,
        })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
    }
}

impl Thumbnails {
    pub(super) fn new(tools: Option<MediaTools>, ctx: &egui::Context) -> Self {
        Self::with_mode(tools, ctx, Mode::Material)
    }

    pub(super) fn timeline(tools: Option<MediaTools>, ctx: &egui::Context) -> Self {
        Self::with_mode(tools, ctx, Mode::Timeline)
    }

    pub(super) fn preview(tools: Option<MediaTools>, ctx: &egui::Context) -> Self {
        Self::with_mode(tools, ctx, Mode::Preview)
    }

    fn with_mode(tools: Option<MediaTools>, ctx: &egui::Context, mode: Mode) -> Self {
        Self {
            ctx: ctx.clone(),
            worker: tools.and_then(|tools| Worker::start(tools, ctx.clone(), mode)),
            pending: HashSet::new(),
            cache: HashMap::new(),
            latest_requested: HashMap::new(),
            mode,
            order: 0,
        }
    }

    pub(super) fn texture(&mut self, path: &Path) -> Option<&egui::TextureHandle> {
        self.frame(path, 0)
    }

    pub(super) fn frame(&mut self, path: &Path, timestamp_us: u64) -> Option<&egui::TextureHandle> {
        self.poll_updates();
        let worker = self.worker.as_ref()?;
        let key = FrameKey {
            path: path.to_owned(),
            timestamp_us,
        };
        if matches!(self.mode, Mode::Preview) {
            // Preview requests are latest-wins while scrubbing. Do not let old
            // timestamps permanently occupy the pending set after the worker
            // coalesces them from its queue.
            self.pending.retain(|pending| {
                pending.path != key.path || pending.timestamp_us == key.timestamp_us
            });
            self.latest_requested
                .insert(key.path.clone(), key.timestamp_us);
        }
        if !self.cache.contains_key(&key)
            && !self.pending.contains(&key)
            && worker.requests.try_send(key.clone()).is_ok()
        {
            self.pending.insert(key.clone());
        }
        let cached = self.cache.get_mut(&key)?;
        self.order += 1;
        cached.last_used = self.order;
        cached.texture.as_ref()
    }

    pub(super) fn nearest_frame(
        &mut self,
        path: &Path,
        timestamp_us: u64,
        tolerance_us: u64,
    ) -> Option<egui::TextureHandle> {
        self.poll_updates();
        let nearest = self
            .cache
            .keys()
            .filter(|key| key.path == path && key.timestamp_us.abs_diff(timestamp_us) <= tolerance_us)
            .min_by_key(|key| key.timestamp_us.abs_diff(timestamp_us))
            .cloned();
        let nearest_texture = if let Some(key) = nearest {
            self.order += 1;
            self.cache.get_mut(&key).and_then(|cached| {
                cached.last_used = self.order;
                cached.texture.clone()
            })
        } else {
            None
        };
        // Show a nearby cached frame immediately, while always queueing the
        // exact requested timestamp for the next repaint.
        let exact_texture = self.frame(path, timestamp_us).cloned();
        nearest_texture.or(exact_texture)
    }

    fn poll_updates(&mut self) {
        let Some(worker) = &self.worker else {
            return;
        };
        for (key, image) in worker.updates.try_iter() {
            self.pending.remove(&key);
            if matches!(self.mode, Mode::Preview)
                && self
                    .latest_requested
                    .get(&key.path)
                    .is_some_and(|latest| *latest != key.timestamp_us)
            {
                continue;
            }
            if self.cache.len() >= self.mode.cache_limit()
                && let Some(oldest) = self
                    .cache
                    .iter()
                    .min_by_key(|(_, entry)| entry.last_used)
                    .map(|(key, _)| key.clone())
            {
                self.cache.remove(&oldest);
            }
            self.order += 1;
            let texture = image.map(|image| {
                self.ctx.load_texture(
                    self.mode.texture_name(),
                    image,
                    egui::TextureOptions::LINEAR,
                )
            });
            self.cache.insert(
                key,
                Cached {
                    texture,
                    last_used: self.order,
                },
            );
        }
    }
}

