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
const QUEUE_LIMIT: usize = 8;

pub(super) struct Thumbnails {
    ctx: egui::Context,
    worker: Option<Worker>,
    pending: HashSet<FrameKey>,
    cache: HashMap<FrameKey, Cached>,
    mode: Mode,
    order: u64,
}

#[derive(Clone, Copy)]
enum Mode {
    Material,
    Timeline,
}

impl Mode {
    fn cache_limit(self) -> usize {
        match self {
            Self::Material => MATERIAL_CACHE_LIMIT,
            Self::Timeline => TIMELINE_CACHE_LIMIT,
        }
    }

    fn texture_name(self) -> &'static str {
        match self {
            Self::Material => "asset-thumbnail",
            Self::Timeline => "timeline-thumbnail",
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
                while let Ok(key) = incoming.recv() {
                    if close.load(Ordering::Acquire) {
                        break;
                    }
                    let image = match mode {
                        Mode::Material => tools.thumbnail(&key.path, &close),
                        Mode::Timeline => {
                            tools.timeline_thumbnail(&key.path, key.timestamp_us, &close)
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

    fn with_mode(tools: Option<MediaTools>, ctx: &egui::Context, mode: Mode) -> Self {
        Self {
            ctx: ctx.clone(),
            worker: tools.and_then(|tools| Worker::start(tools, ctx.clone(), mode)),
            pending: HashSet::new(),
            cache: HashMap::new(),
            mode,
            order: 0,
        }
    }

    pub(super) fn texture(&mut self, path: &Path) -> Option<&egui::TextureHandle> {
        self.frame(path, 0)
    }

    pub(super) fn frame(&mut self, path: &Path, timestamp_us: u64) -> Option<&egui::TextureHandle> {
        let worker = self.worker.as_ref()?;
        for (key, image) in worker.updates.try_iter() {
            self.pending.remove(&key);
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

        let key = FrameKey {
            path: path.to_owned(),
            timestamp_us,
        };
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
}
