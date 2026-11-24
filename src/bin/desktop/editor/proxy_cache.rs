use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::UNIX_EPOCH;

use eframe::egui;

use super::media::{MediaInfo, MediaTools};

const PROXY_RECIPE: &str = "intra-2";
const PROXY_DIVISOR: u32 = 4;
const PROXY_MIN_LONG_SIDE: u32 = 960;
const PROXY_QUEUE_LIMIT: usize = 2;

struct ProxyRequest {
    source: PathBuf,
    target: PathBuf,
    size: (u32, u32),
}

struct ProxyResult {
    source: PathBuf,
    target: PathBuf,
    error: Option<String>,
}

pub(super) struct ProxyCache {
    default_root: PathBuf,
    project_root: Option<PathBuf>,
    requests: Option<SyncSender<ProxyRequest>>,
    results: Receiver<ProxyResult>,
    pending: HashSet<PathBuf>,
    ready: HashMap<PathBuf, PathBuf>,
}

impl ProxyCache {
    pub(super) fn new(tools: Option<MediaTools>, ctx: &egui::Context, directory: &Path) -> Self {
        let (requests, incoming) = mpsc::sync_channel(PROXY_QUEUE_LIMIT);
        let (outgoing, results) = mpsc::channel();
        let requests = tools.as_ref().and_then(|tools| {
            let tools = tools.clone();
            let ctx = ctx.clone();
            thread::Builder::new()
                .name("ottercut-proxy".into())
                .spawn(move || {
                    while let Ok(request) = incoming.recv() {
                        let error = write_proxy(&tools, &request).err().map(|error| error.to_string());
                        if outgoing
                            .send(ProxyResult {
                                source: request.source,
                                target: request.target,
                                error,
                            })
                            .is_err()
                        {
                            break;
                        }
                        ctx.request_repaint();
                    }
                })
                .ok()?;
            Some(requests)
        });
        Self {
            default_root: directory.join(".ottercut-cache").join("proxy"),
            project_root: Some(directory.join(".ottercut-cache").join("proxy")),
            requests,
            results,
            pending: HashSet::new(),
            ready: HashMap::new(),
        }
    }

    pub(super) fn set_project(&mut self, project: Option<&Path>) {
        self.project_root = project
            .map(|path| path.with_extension("cache").join("proxy"))
            .or_else(|| Some(self.default_root.clone()));
        self.pending.clear();
        self.ready.clear();
    }

    pub(super) fn poll(&mut self) {
        for result in self.results.try_iter() {
            self.pending.remove(&result.target);
            if result.error.is_none() && result.target.is_file() {
                self.ready.insert(result.source, result.target);
            }
        }
    }

    pub(super) fn ensure(&mut self, path: &Path, info: MediaInfo) -> Option<PathBuf> {
        let root = self.project_root.as_ref()?;
        if !info.has_video || info.width == 0 || info.height == 0 {
            return None;
        }
        let source = path.to_owned();
        let target = proxy_path(root, path, info)?;
        if target.is_file() {
            self.ready.insert(source, target.clone());
            return Some(target);
        }
        if self.pending.insert(target.clone())
            && let Some(requests) = &self.requests
        {
            let request = ProxyRequest {
                source: path.to_owned(),
                target: target.clone(),
                size: proxy_size(info.width, info.height),
            };
            if requests.try_send(request).is_err() {
                self.pending.remove(&target);
            }
        }
        None
    }

}

fn proxy_size(width: u32, height: u32) -> (u32, u32) {
    let width = width.max(2);
    let height = height.max(2);
    let long_side = width.max(height);
    let target = (long_side / PROXY_DIVISOR)
        .max(PROXY_MIN_LONG_SIDE)
        .min(long_side);
    let scale = f64::from(target) / f64::from(long_side);
    let even = |value: u32| (((f64::from(value) * scale).round() as u32) & !1).max(2);
    (even(width), even(height))
}

fn proxy_path(root: &Path, source: &Path, info: MediaInfo) -> Option<PathBuf> {
    let metadata = source.metadata().ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    PROXY_RECIPE.hash(&mut hasher);
    source.hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    info.width.hash(&mut hasher);
    info.height.hash(&mut hasher);
    if let Ok(modified) = metadata.modified()
        && let Ok(seconds) = modified.duration_since(UNIX_EPOCH)
    {
        seconds.as_secs().hash(&mut hasher);
    }
    Some(root.join(format!("{:016x}.mp4", hasher.finish())))
}

fn write_proxy(tools: &MediaTools, request: &ProxyRequest) -> anyhow::Result<()> {
    let Some(parent) = request.target.parent() else {
        anyhow::bail!("代理目录无效");
    };
    std::fs::create_dir_all(parent)?;
    let partial = request.target.with_extension("part.mp4");
    let filter = format!(
        "scale=w={}:h={}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2",
        request.size.0, request.size.1, request.size.0, request.size.1
    );
    let output = super::media::hidden_command(&tools.ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-i",
        ])
        .arg(&request.source)
        .args(["-map", "0:v:0", "-an", "-vf"])
        .arg(filter)
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "28",
            "-g",
            "1",
            "-keyint_min",
            "1",
            "-bf",
            "0",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
        ])
        .arg(&partial)
        .output()?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&partial);
        anyhow::bail!("代理生成失败：{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    std::fs::rename(partial, &request.target)?;
    Ok(())
}

