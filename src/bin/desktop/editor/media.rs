use std::io::{BufReader, Read, Seek};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI16, AtomicU64, Ordering};
use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};
use eframe::egui;
use serde_json::Value;
use takeseam::PRODUCT_NAME;

#[path = "media/audio.rs"]
mod audio;
#[path = "media/background.rs"]
pub(super) mod background;
#[path = "media/overlays.rs"]
pub(super) mod overlays;

use audio::{AudioDecode, ScrubAudio};
use overlays::{DEFAULT_CANVAS, Overlays, SCRIPT_NAME, VIDEO_FPS};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PREVIEW_TICK: Duration = Duration::from_millis(15);
const USECS_PER_SEC: u64 = 1_000_000;
const THUMBNAIL_SIZE: [usize; 2] = [320, 180];
const PREVIEW_THUMBNAIL_SIZE: [usize; 2] = [640, 360];
const FILMSTRIP_SIZE: [usize; 2] = [144, 81];
const THUMBNAIL_TIMEOUT: Duration = Duration::from_secs(5);
const PREVIEW_CACHE_CAPACITY: usize = 8;

#[derive(Clone)]
pub struct MediaTools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

#[derive(Clone, Copy, Debug)]
pub struct MediaInfo {
    pub duration_us: u64,
    pub width: u32,
    pub height: u32,
    pub has_video: bool,
    pub has_audio: bool,
}

impl MediaTools {
    pub fn discover() -> Result<Self> {
        Ok(Self {
            ffmpeg: Self::ffmpeg_path()?,
            ffprobe: Self::find_program("ffprobe", "FFprobe")?,
        })
    }

    pub(super) fn ffmpeg_path() -> Result<PathBuf> {
        Self::find_program("ffmpeg", "FFmpeg")
    }

    fn find_program(program: &str, label: &str) -> Result<PathBuf> {
        let program = format!("{program}{}", std::env::consts::EXE_SUFFIX);
        let executable =
            std::env::current_exe().with_context(|| format!("无法确定 {PRODUCT_NAME} 程序位置"))?;
        let folder = executable
            .parent()
            .with_context(|| format!("无法确定 {PRODUCT_NAME} 程序目录"))?;
        let mut search = vec![folder.to_path_buf()];
        search.extend(
            folder
                .ancestors()
                .take(3)
                .map(|base| base.join("tools/ffmpeg/bin")),
        );
        #[cfg(target_os = "macos")]
        if let Some(contents) = folder.parent() {
            search.push(contents.join("Resources/ffmpeg/bin"));
        }
        if let Some(path) = std::env::var_os("PATH") {
            search.extend(std::env::split_paths(&path).filter(|folder| folder.is_absolute()));
        }
        // Finder-launched applications may not inherit the user's shell PATH.
        #[cfg(target_os = "macos")]
        search.extend(["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"].map(PathBuf::from));
        search
            .iter()
            .map(|folder| folder.join(&program))
            .find(|path| is_executable(path))
            .with_context(|| {
                format!("找不到 {label}。请将 {program} 放入 {PRODUCT_NAME} 程序目录或 PATH")
            })
    }

    pub fn probe(&self, path: &Path) -> Result<MediaInfo> {
        let path = local_media(path)?;
        let output = hidden_command(&self.ffprobe)
            .args([
                "-v", "error", "-protocol_whitelist", "file,pipe",
                "-show_entries", "stream=codec_type,width,height,duration,sample_aspect_ratio:stream_disposition=attached_pic:stream_side_data=rotation:format=duration",
                "-of", "json", "-i",
            ])
            .arg(&path)
            .output()
            .context("无法启动 FFprobe")?;
        ensure!(
            output.status.success(),
            "无法读取素材 {}（{}）",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );

        let info: Value = serde_json::from_slice(&output.stdout).context("媒体信息格式无效")?;
        let streams = info["streams"]
            .as_array()
            .context("素材没有可读取的媒体流")?;
        let video = streams.iter().find(|stream| {
            stream["codec_type"].as_str() == Some("video")
                && stream["disposition"]["attached_pic"].as_u64() != Some(1)
        });
        let audio = streams
            .iter()
            .find(|stream| stream["codec_type"].as_str() == Some("audio"));
        ensure!(
            video.is_some() || audio.is_some(),
            "素材中没有可剪辑的视频或音频流"
        );
        let duration_us = duration_us(&info["format"]["duration"])
            .or_else(|| video.and_then(|stream| duration_us(&stream["duration"])))
            .or_else(|| audio.and_then(|stream| duration_us(&stream["duration"])))
            .context("无法确定素材时长")?;
        let (width, height) = video.map(video_dimensions).transpose()?.unwrap_or((0, 0));
        Ok(MediaInfo {
            duration_us,
            width,
            height,
            has_video: video.is_some(),
            has_audio: audio.is_some(),
        })
    }

    pub(super) fn thumbnail(&self, path: &Path, stopped: &AtomicBool) -> Result<egui::ColorImage> {
        thumbnail(&self.ffmpeg, path, 0, THUMBNAIL_SIZE, stopped)
    }

    pub(super) fn timeline_thumbnail(
        &self,
        path: &Path,
        timestamp_us: u64,
        stopped: &AtomicBool,
    ) -> Result<egui::ColorImage> {
        thumbnail(&self.ffmpeg, path, timestamp_us, FILMSTRIP_SIZE, stopped)
    }

    pub(super) fn preview_thumbnail(
        &self,
        path: &Path,
        timestamp_us: u64,
        stopped: &AtomicBool,
    ) -> Result<egui::ColorImage> {
        thumbnail(
            &self.ffmpeg,
            path,
            timestamp_us,
            PREVIEW_THUMBNAIL_SIZE,
            stopped,
        )
    }
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o111 == 0 {
        return false;
    }
    metadata.is_file()
}

fn video_dimensions(video: &Value) -> Result<(u32, u32)> {
    let width = u32::try_from(video["width"].as_u64().unwrap_or(0))?;
    let height = u32::try_from(video["height"].as_u64().unwrap_or(0))?;
    ensure!(width > 0 && height > 0, "视频尺寸无效");
    let ratio = video["sample_aspect_ratio"]
        .as_str()
        .and_then(|ratio| ratio.split_once(':'));
    let width = ratio
        .and_then(|(numerator, denominator)| {
            let numerator = numerator.parse::<u32>().ok()?;
            let denominator = denominator.parse::<u32>().ok()?;
            if numerator == 0 || denominator == 0 {
                return None;
            }
            let display_width = (u64::from(width) * u64::from(numerator)
                + u64::from(denominator) / 2)
                / u64::from(denominator);
            u32::try_from(display_width).ok().filter(|value| *value > 0)
        })
        .unwrap_or(width);
    let rotation = video["side_data_list"]
        .as_array()
        .and_then(|list| list.iter().find_map(|entry| entry["rotation"].as_i64()))
        .unwrap_or(0);
    Ok(if rotation.rem_euclid(180) == 90 {
        (height, width)
    } else {
        (width, height)
    })
}

fn thumbnail(
    executable: &Path,
    path: &Path,
    timestamp_us: u64,
    size: [usize; 2],
    stopped: &AtomicBool,
) -> Result<egui::ColorImage> {
    let path = local_media(path)?;
    let filter = fit_filter(size, "0x111111");
    let mut previous = None;
    let started = Instant::now();
    // The final frame may be earlier than the end of the media, especially at low frame rates.
    for lookback in [0, 50_000, 250_000, 1_000_000] {
        let from_us = timestamp_us.saturating_sub(lookback);
        if previous == Some(from_us) {
            break;
        }
        previous = Some(from_us);
        let mut command = decoding_command(executable, &path, from_us);
        command.args([
            "-map",
            "0:V:0",
            "-an",
            "-sn",
            "-dn",
            "-filter_threads",
            "1",
            "-frames:v",
            "1",
            "-vf",
            &filter,
            "-threads:v",
            "1",
            "-c:v",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-f",
            "rawvideo",
            "pipe:1",
        ]);
        if let Some(image) = read_thumbnail(command, size, started, stopped)? {
            return Ok(image);
        }
    }
    bail!("视频没有缩略图帧")
}

fn read_thumbnail(
    mut command: Command,
    [width, height]: [usize; 2],
    started: Instant,
    stopped: &AtomicBool,
) -> Result<Option<egui::ColorImage>> {
    ensure!(
        !stopped.load(Ordering::Acquire) && started.elapsed() < THUMBNAIL_TIMEOUT,
        "已停止缩略图解码"
    );
    // A single frame can be read after completion so the worker can enforce a deadline.
    let mut output = tempfile::tempfile().context("无法暂存缩略图")?;
    let mut child = command
        .stdout(Stdio::from(output.try_clone()?))
        .spawn()
        .context("无法启动缩略图解码")?;
    let status = loop {
        match child.try_wait() {
            Ok(None)
                if !stopped.load(Ordering::Acquire) && started.elapsed() < THUMBNAIL_TIMEOUT =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            status => break status,
        }
    };
    if !matches!(&status, Ok(Some(_))) {
        let _ = child.kill();
        let _ = child.wait();
    }
    let status = status
        .context("无法查询缩略图解码状态")?
        .context("已停止缩略图解码")?;
    ensure!(status.success(), "无法解码缩略图");
    let mut pixels = vec![0; width * height * 4];
    if output.metadata()?.len() < pixels.len() as u64 {
        return Ok(None);
    }
    output.rewind()?;
    output.read_exact(&mut pixels).context("视频没有缩略图帧")?;
    Ok(Some(egui::ColorImage::from_rgba_unmultiplied(
        [width, height],
        &pixels,
    )))
}

fn fit_filter(size: [usize; 2], background: &str) -> String {
    fit_filter_for_aspect(size, background, "dar")
}

fn fit_filter_for_aspect([width, height]: [usize; 2], background: &str, aspect: &str) -> String {
    format!(
        "scale='max(1,min({width},round({height}*({aspect}))))':'max(1,min({height},round({width}/({aspect}))))':flags=bilinear,setsar=1,format=rgba,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color={background}"
    )
}

pub(super) fn sequence_filter(width: u32, height: u32) -> String {
    format!(
        "scale=w='max(2,2*trunc(min({width},{height}*dar)/2))':h='max(2,2*trunc(min({height},{width}/dar)/2))',setsar=1,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2"
    )
}

#[cfg(test)]
mod thumbnail_tests {
    use super::*;

    #[test]
    #[ignore = "requires FFmpeg in the application media tools path"]
    fn timeline_thumbnails_follow_source_time_and_recover_short_tails() {
        let tools = MediaTools::discover().expect("installed FFmpeg test fixture");
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("two-colors.mp4");
        let output = hidden_command(&tools.ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=96x54:d=1:r=2",
                "-f",
                "lavfi",
                "-i",
                "color=c=lime:s=96x54:d=1:r=2",
                "-filter_complex",
                "[0:v][1:v]concat=n=2:v=1:a=0",
                "-threads",
                "1",
                "-c:v",
                "mpeg4",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stopped = AtomicBool::new(false);
        for (timestamp, green) in [(250_000, false), (1_250_000, true), (2_020_000, true)] {
            let image = tools
                .timeline_thumbnail(&source, timestamp, &stopped)
                .unwrap();
            assert_eq!(image.size, FILMSTRIP_SIZE);
            let center = image.pixels[(image.size[1] / 2) * image.size[0] + image.size[0] / 2];
            if green {
                assert!(center.g() > 210 && center.r() < 40, "{center:?}");
            } else {
                assert!(center.r() > 210 && center.g() < 40, "{center:?}");
            }
        }
        assert!(
            tools
                .timeline_thumbnail(&source, 5_000_000, &stopped)
                .is_err()
        );
    }
}

fn duration_us(value: &Value) -> Option<u64> {
    let seconds = value
        .as_f64()
        .or_else(|| value.as_str()?.parse::<f64>().ok())?;
    let micros = (seconds * USECS_PER_SEC as f64).round();
    (micros >= 1.0 && micros < u64::MAX as f64).then_some(micros as u64)
}

pub(super) fn local_media(path: &Path) -> Result<PathBuf> {
    ensure!(path.is_file(), "素材文件不可用（{}）", path.display());
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

pub(super) fn hidden_command(path: &Path) -> Command {
    let mut command = Command::new(path);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command.stdin(Stdio::null());
    command
}

pub(super) fn decoding_command(executable: &Path, path: &Path, from_us: u64) -> Command {
    let mut command = hidden_command(executable);
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-protocol_whitelist",
            "file,pipe",
            "-ss",
        ])
        .arg(seconds(from_us))
        .args(["-threads", "2", "-i"])
        .arg(path)
        .stderr(Stdio::null())
        .stdout(Stdio::piped());
    command
}

fn seconds(usecs: u64) -> String {
    format!("{}.{:06}", usecs / USECS_PER_SEC, usecs % USECS_PER_SEC)
}

pub(super) fn append_video_fades(
    filter: &mut String,
    timeline_from: u64,
    clip_start: u64,
    clip_duration_us: u64,
    fade_in_us: u64,
    fade_out_us: u64,
) {
    use std::fmt::Write as _;

    let offset = timeline_from.saturating_sub(clip_start);
    if fade_in_us > offset {
        let duration = fade_in_us
            .saturating_sub(offset)
            .max(1) as f64
            / 1_000_000.0;
        let _ = write!(filter, ",fade=t=in:d={duration:.6}:color=black");
    }
    let fade_start = clip_duration_us
        .saturating_sub(fade_out_us)
        .saturating_sub(offset);
    if fade_out_us > 0 && fade_start < clip_duration_us.saturating_sub(offset) {
        let duration = fade_out_us
            .min(clip_duration_us.saturating_sub(offset).saturating_sub(fade_start))
            .max(1) as f64
            / 1_000_000.0;
        let start = fade_start as f64 / 1_000_000.0;
        let _ = write!(filter, ",fade=t=out:st={start:.6}:d={duration:.6}:color=black");
    }
}

pub(super) fn append_clip_audio_filters(
    graph: &mut String,
    volume_db_tenths: i16,
    fade_in_us: u64,
    fade_out_us: u64,
    duration_us: u64,
    speed_milli: u16,
    reverse: bool,
) {
    use std::fmt::Write as _;

    if volume_db_tenths != 0 {
        let _ = write!(graph, ",volume={:.1}dB", f32::from(volume_db_tenths) / 10.0);
    }
    if reverse {
        graph.push_str(",areverse");
    }
    if speed_milli != 1000 {
        let _ = write!(graph, ",atempo={:.3}", f32::from(speed_milli) / 1000.0);
    }
    if fade_in_us > 0 {
        let duration = fade_in_us.min(duration_us).max(1) as f64 / 1_000_000.0;
        let _ = write!(graph, ",afade=t=in:d={duration:.6}");
    }
    if fade_out_us > 0 {
        let fade = fade_out_us.min(duration_us).max(1) as f64 / 1_000_000.0;
        let start = duration_us.saturating_sub(fade_out_us) as f64 / 1_000_000.0;
        let _ = write!(graph, ",afade=t=out:st={start:.6}:d={fade:.6}");
    }
}

pub(super) fn append_segment_audio_filters(
    graph: &mut String,
    volume_db_tenths: i16,
    fade_in_us: u64,
    fade_out_us: u64,
    clip_offset_us: u64,
    clip_duration_us: u64,
    segment_duration_us: u64,
    speed_milli: u16,
    reverse: bool,
) {
    use std::fmt::Write as _;

    if volume_db_tenths != 0 {
        let _ = write!(graph, ",volume={:.1}dB", f32::from(volume_db_tenths) / 10.0);
    }
    if reverse {
        graph.push_str(",areverse");
    }
    if speed_milli != 1000 {
        let _ = write!(graph, ",atempo={:.3}", f32::from(speed_milli) / 1000.0);
    }
    if fade_in_us > clip_offset_us {
        let duration = fade_in_us
            .saturating_sub(clip_offset_us)
            .min(segment_duration_us)
            .max(1) as f64
            / 1_000_000.0;
        let _ = write!(graph, ",afade=t=in:d={duration:.6}");
    }
    let fade_start = clip_duration_us
        .saturating_sub(fade_out_us)
        .saturating_sub(clip_offset_us);
    if fade_out_us > 0 && fade_start < segment_duration_us {
        let duration = fade_out_us
            .min(segment_duration_us.saturating_sub(fade_start))
            .max(1) as f64
            / 1_000_000.0;
        let start = fade_start as f64 / 1_000_000.0;
        let _ = write!(graph, ",afade=t=out:st={start:.6}:d={duration:.6}");
    }
}

#[derive(Default)]
pub struct Preview {
    texture: Option<PreviewTexture>,
    session: Option<PreviewSession>,
    retired_video: Vec<thread::JoinHandle<()>>,
    cached_frames: VecDeque<CachedFrame>,
    scrub_audio: Option<ScrubAudio>,
    error: Option<String>,
    audio_error: Option<String>,
}

struct PreviewTexture {
    handle: egui::TextureHandle,
    composition_size: (u32, u32),
    excluded_overlay: Option<u64>,
}

#[derive(Clone, PartialEq, Eq)]
struct PreviewCacheKey {
    path: PathBuf,
    source_start: u64,
    source_end: u64,
    timeline_from: u64,
    size: [usize; 2],
    composition_size: (u32, u32),
    excluded_overlay: Option<u64>,
}

struct CachedFrame {
    key: PreviewCacheKey,
    pixels: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum PreviewQuality {
    Full = 1,
    #[default]
    Half = 2,
    Quarter = 4,
    Eighth = 8,
    Sixteenth = 16,
}

impl PreviewQuality {
    pub(super) const ALL: [Self; 5] = [
        Self::Full,
        Self::Half,
        Self::Quarter,
        Self::Eighth,
        Self::Sixteenth,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Full => "完整",
            Self::Half => "1/2",
            Self::Quarter => "1/4",
            Self::Eighth => "1/8",
            Self::Sixteenth => "1/16",
        }
    }

    fn raster_size(self, (width, height): (u32, u32)) -> [usize; 2] {
        let width = f64::from(width.max(1));
        let height = f64::from(height.max(1));
        let (limit_width, limit_height) = if width < height {
            (DEFAULT_CANVAS.1, DEFAULT_CANVAS.0)
        } else {
            DEFAULT_CANVAS
        };
        let scale = (f64::from(limit_width) / width)
            .min(f64::from(limit_height) / height)
            .min(1.0)
            / f64::from(self as u32);
        // The preview and subtitle filters use 4:2:0 video, which requires even dimensions.
        let dimension = |value: f64| ((value * scale * 0.5).round().max(1.0) as usize) * 2;
        [dimension(width), dimension(height)]
    }
}

#[cfg(test)]
mod preview_quality_tests {
    use super::*;

    #[test]
    fn preview_raster_follows_the_selected_fraction() {
        assert_eq!(PreviewQuality::default(), PreviewQuality::Half);
        assert_eq!(
            PreviewQuality::ALL.map(|quality| quality.raster_size((1920, 1080))),
            [[1920, 1080], [960, 540], [480, 270], [240, 136], [120, 68]]
        );
    }

    #[test]
    fn oversized_sources_fit_the_native_preview_canvas_before_scaling() {
        assert_eq!(PreviewQuality::Full.raster_size((3840, 2160)), [1920, 1080]);
        assert_eq!(PreviewQuality::Half.raster_size((1080, 1920)), [540, 960]);
    }

    #[test]
    fn tiny_and_odd_sources_still_produce_even_frames() {
        assert_eq!(PreviewQuality::Sixteenth.raster_size((1, 0)), [2, 2]);
        for quality in PreviewQuality::ALL {
            let [width, height] = quality.raster_size((127, 81));
            assert!(width >= 2 && width % 2 == 0);
            assert!(height >= 2 && height % 2 == 0);
        }
    }
}

pub(super) struct MediaSlice {
    /// Stable timeline clip id used to route live volume automation to one source.
    pub id: u64,
    pub path: PathBuf,
    pub range: std::ops::Range<u64>,
    pub timeline_start: u64,
    pub timeline_duration_us: u64,
    pub volume_db_tenths: i16,
    pub fade_in_us: u64,
    pub fade_out_us: u64,
    pub speed_milli: u16,
    pub reverse: bool,
}

pub(super) struct PreviewPlan {
    pub timeline_from: u64,
    pub duration_us: u64,
    pub audio_timeline_start: u64,
    pub audio_position: u64,
    pub size: (u32, u32),
    pub quality: PreviewQuality,
    pub background: super::background::BackgroundSettings,
    pub video: Option<MediaSlice>,
    pub video_decode_path: Option<PathBuf>,
    pub audio: Vec<MediaSlice>,
    pub black_canvas: bool,
    pub overlays: Overlays,
    pub excluded_overlay: Option<u64>,
    pub cursor: Option<super::cursor_effect::CursorTelemetry>,
    pub cursor_effect: super::cursor_effect::CursorEffect,
}

fn preview_cache_key(plan: &PreviewPlan) -> Option<PreviewCacheKey> {
    let video = plan.video.as_ref()?;
    if !plan.overlays.is_empty() || plan.background.enabled() || plan.cursor.is_some() {
        return None;
    }
    Some(PreviewCacheKey {
        path: video.path.clone(),
        source_start: video.range.start,
        source_end: video.range.end,
        timeline_from: plan.timeline_from,
        size: plan.quality.raster_size(plan.size),
        composition_size: plan.size,
        excluded_overlay: plan.excluded_overlay,
    })
}

impl Preview {
    pub(super) fn start(
        &mut self,
        tools: &MediaTools,
        ctx: &egui::Context,
        plan: PreviewPlan,
        playing: bool,
    ) {
        self.stop_scrub_audio();
        self.stop();
        self.error = None;
        self.audio_error = None;
        let cache_key = preview_cache_key(&plan);
        if !playing
            && let Some(key) = cache_key.as_ref()
            && let Some(cached) = self.cached_frames.iter().find(|frame| frame.key == *key)
        {
            self.publish_texture(
                ctx,
                cached.pixels.clone(),
                key.size,
                key.composition_size,
                key.excluded_overlay,
            );
            return;
        }
        let has_audio = playing && !plan.audio.is_empty();
        let has_picture = plan.video.is_some() || plan.black_canvas || plan.background.enabled();
        let mut session =
            PreviewSession::new(plan.size, plan.quality, has_picture, has_audio, playing);
        session.excluded_overlay = plan.excluded_overlay;
        session.cache_key = cache_key;
        if has_picture {
            let path = match plan
                .video
                .as_ref()
                .map(|source| {
                    plan.video_decode_path
                        .as_deref()
                        .unwrap_or(&source.path)
                })
                .map(local_media)
                .transpose()
            {
                Ok(path) => path,
                Err(error) => {
                    self.error = Some(error.to_string());
                    return;
                }
            };
            let (frames, incoming) = mpsc::sync_channel(4);
            session.frames = Some(incoming);
            let (from_us, until_us) = plan.video.as_ref().map_or((0, plan.duration_us), |source| {
                (source.range.start, source.range.end)
            });
            let (speed_milli, reverse) = plan
                .video
                .as_ref()
                .map(|source| (source.speed_milli, source.reverse))
                .unwrap_or((1000, false));
            let (timeline_start, timeline_duration_us, fade_in_us, fade_out_us) = plan
                .video
                .as_ref()
                .map(|source| {
                    (
                        source.timeline_start,
                        source.timeline_duration_us,
                        source.fade_in_us,
                        source.fade_out_us,
                    )
                })
                .unwrap_or((0, plan.duration_us, 0, 0));
            let video = VideoDecode {
                executable: tools.ffmpeg.clone(),
                path,
                span: PreviewSpan {
                    from_us,
                    until_us,
                    playing,
                },
                size: session.size,
                composition_size: session.composition_size,
                timeline_from: plan.timeline_from,
                timeline_duration: plan.duration_us,
                speed_milli,
                reverse,
                timeline_start,
                timeline_duration_us,
                fade_in_us,
                fade_out_us,
                overlays: plan.overlays,
                backdrop: plan.background,
                cursor: plan.cursor,
                cursor_effect: plan.cursor_effect,
                decoder: session.video.clone(),
                frames,
                errors: session.errors_tx.clone(),
                ctx: ctx.clone(),
            };
            match thread::Builder::new()
                .name("preview-video".into())
                .spawn(move || video.run())
            {
                Ok(worker) => session.video_worker = Some(worker),
                Err(error) => {
                    self.error = Some(format!("无法启动视频预览（{error}）"));
                    return;
                }
            }
        }
        if has_audio {
            let audio = AudioDecode {
                executable: tools.ffmpeg.clone(),
                sources: plan.audio,
                duration_us: plan.duration_us,
                shared: session.playback.clone(),
                decoder: session.audio.clone(),
                errors: session.errors_tx.clone(),
                ctx: ctx.clone(),
            };
            match thread::Builder::new()
                .name("preview-audio".into())
                .spawn(move || audio.run())
            {
                Ok(worker) => session.audio_worker = Some(worker),
                Err(error) => {
                    session.playback.audio_ready.store(true, Ordering::Release);
                    self.audio_error = Some(format!("无法启动音频预览（{error}）"));
                }
            }
        }
        self.session = Some(session);
        ctx.request_repaint();
    }

    pub(super) fn play_scrub_audio(
        &mut self,
        tools: &MediaTools,
        sources: Vec<MediaSlice>,
        duration_us: u64,
        timeline_start: u64,
        position: u64,
    ) {
        if sources.is_empty() {
            return;
        }
        if self.scrub_audio.is_none() {
            self.scrub_audio = ScrubAudio::new().ok();
        }
        if let Some(audio) = &self.scrub_audio {
            audio.play(
                &tools.ffmpeg,
                sources,
                duration_us,
                timeline_start,
                position,
            );
        }
    }

    pub(super) fn prepare_scrub_audio(
        &mut self,
        tools: &MediaTools,
        sources: Vec<MediaSlice>,
        duration_us: u64,
        timeline_start: u64,
    ) {
        if sources.is_empty() {
            return;
        }
        if self.scrub_audio.is_none() {
            self.scrub_audio = ScrubAudio::new().ok();
        }
        if let Some(audio) = &self.scrub_audio {
            audio.prepare(&tools.ffmpeg, sources, duration_us, timeline_start);
        }
    }

    /// Changes one source's gain without restarting FFmpeg or the video decoder.
    pub(super) fn set_live_volume(&mut self, source_id: u64, db_tenths: i16) {
        if let Some(session) = &self.session {
            session
                .playback
                .live_volume_source
                .store(source_id, Ordering::Release);
            session
                .playback
                .live_gain_db_tenths
                .store(db_tenths, Ordering::Release);
        }
    }

    pub fn stop(&mut self) {
        self.stop_scrub_audio();
        if let Some(mut session) = self.session.take() {
            self.retired_video.extend(session.video_worker.take());
            // Cancel FFmpeg and drop the receiver before a worker can be joined.
            drop(session);
        }
        self.reap_video_workers();
    }

    pub fn clear(&mut self) {
        self.stop();
        self.texture = None;
        self.error = None;
        self.audio_error = None;
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        self.reap_video_workers();
        let (errors, frame_data, repaint) = {
            let Some(session) = &mut self.session else {
                return;
            };
            let errors = session.errors.try_iter().collect::<Vec<_>>();
            let frame_data = session.latest_frame().map(|frame| {
                (
                    frame.pixels,
                    session.size,
                    session.composition_size,
                    session.excluded_overlay,
                    session.cache_key.clone(),
                )
            });
            let mut repaint = false;
            if session.playing
                && let Some(first_frame_at) = session.first_frame_at
            {
                if session.playback.audio_ready.load(Ordering::Acquire)
                    || first_frame_at.elapsed() > Duration::from_millis(700)
                {
                    session
                        .playback
                        .epoch
                        .get_or_init(|| Instant::now() + Duration::from_millis(15));
                }
                repaint = session.frames.is_some() || session.playback.epoch.get().is_none();
            }
            (errors, frame_data, repaint)
        };
        for error in errors {
            match error {
                PreviewError::Video(error) => self.error = Some(error),
                PreviewError::Audio(error) => self.audio_error = Some(error),
            }
        }
        if let Some((pixels, size, composition_size, excluded_overlay, cache_key)) = frame_data {
            if let Some(key) = cache_key {
                self.cache_frame(key, &pixels);
            }
            self.publish_texture(ctx, pixels, size, composition_size, excluded_overlay);
        }
        if repaint {
            ctx.request_repaint_after(PREVIEW_TICK);
        }
    }

    pub fn texture(&self) -> Option<&egui::TextureHandle> {
        self.texture.as_ref().map(|texture| &texture.handle)
    }

    pub(super) fn texture_composition_size(&self) -> Option<(u32, u32)> {
        self.texture
            .as_ref()
            .map(|texture| texture.composition_size)
    }

    pub(super) fn texture_excludes(&self, id: u64) -> bool {
        self.texture
            .as_ref()
            .is_some_and(|texture| texture.excluded_overlay == Some(id))
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn audio_error(&self) -> Option<&str> {
        self.audio_error.as_deref()
    }

    pub fn playback_elapsed(&self) -> Option<Duration> {
        self.session
            .as_ref()?
            .playback
            .epoch
            .get()
            .map(|epoch| Instant::now().saturating_duration_since(*epoch))
    }

    fn reap_video_workers(&mut self) {
        self.retired_video.retain(|worker| !worker.is_finished());
    }

    pub(super) fn stop_scrub_audio(&mut self) {
        if let Some(audio) = &self.scrub_audio {
            audio.stop();
        }
    }

    fn cache_frame(&mut self, key: PreviewCacheKey, pixels: &[u8]) {
        self.cached_frames.retain(|frame| frame.key != key);
        self.cached_frames.push_front(CachedFrame {
            key,
            pixels: pixels.to_vec(),
        });
        self.cached_frames.truncate(PREVIEW_CACHE_CAPACITY);
    }

    fn publish_texture(
        &mut self,
        ctx: &egui::Context,
        pixels: Vec<u8>,
        size: [usize; 2],
        composition_size: (u32, u32),
        excluded_overlay: Option<u64>,
    ) {
        let image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
        if let Some(texture) = &mut self.texture {
            texture.handle.set(image, egui::TextureOptions::LINEAR);
            texture.composition_size = composition_size;
            texture.excluded_overlay = excluded_overlay;
        } else {
            self.texture = Some(PreviewTexture {
                handle: ctx.load_texture("editor-preview", image, egui::TextureOptions::LINEAR),
                composition_size,
                excluded_overlay,
            });
        }
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.stop();
        // Let cancelled video jobs clean up their subtitle scripts before process exit.
        for worker in self.retired_video.drain(..) {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod preview_shutdown_tests {
    use super::*;

    #[test]
    fn shutdown_cleans_temp_files_from_active_and_replaced_video_workers() -> Result<()> {
        let mut preview = Preview::default();
        let mut scripts = Vec::new();
        for _ in 0..2 {
            preview.stop();
            let stage = tempfile::tempdir()?;
            let script = stage.path().join(SCRIPT_NAME);
            std::fs::write(&script, "字幕预览")?;
            scripts.push(script);

            let (output, frames) = mpsc::sync_channel(0);
            let mut session =
                PreviewSession::new((16, 16), PreviewQuality::Full, true, false, true);
            session.frames = Some(frames);
            session.video_worker = Some(thread::spawn(move || {
                let _ = output.send(VideoFrame {
                    time_us: 0,
                    pixels: Vec::new(),
                });
                // Cleanup can still be running when a seek replaces the session.
                thread::sleep(Duration::from_millis(20));
                drop(stage);
            }));
            preview.session = Some(session);
        }

        drop(preview);
        assert!(scripts.iter().all(|script| !script.exists()));
        Ok(())
    }
}

#[derive(Default)]
struct PlaybackClock {
    epoch: OnceLock<Instant>,
    audio_ready: AtomicBool,
    live_volume_source: AtomicU64,
    live_gain_db_tenths: AtomicI16,
}

#[derive(Clone, Copy)]
struct PreviewSpan {
    from_us: u64,
    until_us: u64,
    playing: bool,
}

impl PreviewSpan {
    fn duration(self) -> u64 {
        self.until_us.saturating_sub(self.from_us).max(1)
    }
}

enum PreviewError {
    Video(String),
    Audio(String),
}

struct PreviewSession {
    playing: bool,
    excluded_overlay: Option<u64>,
    size: [usize; 2],
    composition_size: (u32, u32),
    playback: Arc<PlaybackClock>,
    video: Arc<Decoder>,
    video_worker: Option<thread::JoinHandle<()>>,
    audio: Arc<Decoder>,
    audio_worker: Option<thread::JoinHandle<()>>,
    first_frame_at: Option<Instant>,
    frames: Option<Receiver<VideoFrame>>,
    pending: Option<VideoFrame>,
    cache_key: Option<PreviewCacheKey>,
    errors_tx: mpsc::Sender<PreviewError>,
    errors: Receiver<PreviewError>,
}

impl PreviewSession {
    fn new(
        composition_size: (u32, u32),
        quality: PreviewQuality,
        has_video: bool,
        has_audio: bool,
        playing: bool,
    ) -> Self {
        let (errors_tx, errors) = mpsc::channel();
        Self {
            playing,
            excluded_overlay: None,
            size: quality.raster_size(composition_size),
            composition_size,
            playback: Arc::new(PlaybackClock {
                audio_ready: AtomicBool::new(!has_audio),
                live_volume_source: AtomicU64::new(u64::MAX),
                ..PlaybackClock::default()
            }),
            video: Arc::default(),
            video_worker: None,
            audio: Arc::default(),
            audio_worker: None,
            first_frame_at: (!has_video).then(Instant::now),
            frames: None,
            pending: None,
            cache_key: None,
            errors_tx,
            errors,
        }
    }

    fn latest_frame(&mut self) -> Option<VideoFrame> {
        let elapsed = self
            .playback
            .epoch
            .get()
            .map(|epoch| Instant::now().saturating_duration_since(*epoch).as_micros())
            .unwrap_or(0);
        let mut latest = None;
        // The channel is bounded. Also limit the work per repaint when decoding catches up.
        for _ in 0..8 {
            let frame = match self.pending.take() {
                Some(frame) => frame,
                None => match self.frames.as_ref()?.try_recv() {
                    Ok(frame) => frame,
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.frames = None;
                        break;
                    }
                },
            };
            if self.playing && self.first_frame_at.is_some() && u128::from(frame.time_us) > elapsed
            {
                self.pending = Some(frame);
                break;
            }
            self.first_frame_at.get_or_insert_with(Instant::now);
            latest = Some(frame);
        }
        latest
    }
}

impl Drop for PreviewSession {
    fn drop(&mut self) {
        self.video.cancel();
        self.audio.cancel();
    }
}

#[derive(Default)]
pub(super) struct Decoder {
    child: Mutex<Option<Child>>,
    cancelled: AtomicBool,
}

impl Decoder {
    pub(super) fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.kill();
    }

    fn kill(&self) {
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }

    pub(super) fn decode<T>(
        &self,
        mut command: Command,
        read: impl FnOnce(ChildStdout) -> Result<T>,
    ) -> Result<T> {
        ensure!(!self.cancelled(), "预览已取消");
        let mut child = command.spawn().context("无法启动 FFmpeg")?;
        let stdout = child
            .stdout
            .take()
            .expect("preview decoder has piped stdout");
        {
            let mut slot = self.child.lock().unwrap();
            if self.cancelled() {
                let _ = child.kill();
            }
            *slot = Some(child);
        }
        let decoded = read(stdout);
        if decoded.is_err() {
            self.kill();
        }
        let status = loop {
            {
                let mut slot = self.child.lock().unwrap();
                if let Some(status) = slot
                    .as_mut()
                    .unwrap()
                    .try_wait()
                    .context("无法查询 FFmpeg 状态")?
                {
                    *slot = None;
                    break status;
                }
            }
            thread::sleep(Duration::from_millis(5));
        };
        let data = decoded?;
        ensure!(status.success(), "FFmpeg 解码失败（{status}）");
        Ok(data)
    }
}

struct VideoFrame {
    time_us: u64,
    pixels: Vec<u8>,
}

struct VideoDecode {
    executable: PathBuf,
    path: Option<PathBuf>,
    span: PreviewSpan,
    size: [usize; 2],
    composition_size: (u32, u32),
    timeline_from: u64,
    timeline_duration: u64,
    speed_milli: u16,
    reverse: bool,
    timeline_start: u64,
    timeline_duration_us: u64,
    fade_in_us: u64,
    fade_out_us: u64,
    overlays: Overlays,
    backdrop: super::background::BackgroundSettings,
    cursor: Option<super::cursor_effect::CursorTelemetry>,
    cursor_effect: super::cursor_effect::CursorEffect,
    decoder: Arc<Decoder>,
    frames: SyncSender<VideoFrame>,
    errors: mpsc::Sender<PreviewError>,
    ctx: egui::Context,
}

impl VideoDecode {
    fn run(self) {
        let result = self
            .decode(self.span.from_us, self.span.playing)
            .and_then(|count| {
                if count > 0 || self.decoder.cancelled() {
                    return Ok(());
                }
                // The final display interval can extend beyond the final video timestamp.
                for lookback in [25_000, 60_000, 150_000, 500_000, 1_000_000] {
                    if self.decode(self.span.from_us.saturating_sub(lookback), false)? > 0 {
                        return Ok(());
                    }
                }
                bail!("当前位置没有可显示的视频帧");
            });
        if let Err(error) = result
            && !self.decoder.cancelled()
        {
            let _ = self
                .errors
                .send(PreviewError::Video(format!("无法预览视频（{error}）")));
        }
        self.ctx.request_repaint();
    }

    fn decode(&self, from_us: u64, playing: bool) -> Result<u64> {
        let [width, height] = self.size;
        let mut command = if let Some(path) = self.path.as_ref() {
            decoding_command(&self.executable, path, from_us)
        } else {
            let mut command = hidden_command(&self.executable);
            command
                .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
                .arg(format!("color=c=black:s={width}x{height}:r={VIDEO_FPS}"))
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            command
        };
        let stage = (self.backdrop.enabled() || !self.overlays.is_empty())
            .then(|| tempfile::tempdir().context("无法创建预览合成目录"))
            .transpose()?;
        let mut has_overlays = false;
        if let Some(directory) = &stage {
            if self.backdrop.enabled() {
                background::stage(&self.backdrop, directory.path(), self.composition_size)?;
            }
            has_overlays = if playing {
                self.overlays.write_for_playback(
                    directory.path(),
                    self.composition_size,
                    self.timeline_from..self.timeline_from.saturating_add(self.timeline_duration),
                )?
            } else {
                self.overlays.write_for_frame(
                    directory.path(),
                    self.composition_size,
                    self.timeline_from,
                )?
            };
            command.current_dir(directory.path());
        }
        let (composition_width, composition_height) = self.composition_size;
        let output_filter = if has_overlays {
            format!(
                "setpts=PTS-STARTPTS,format=yuv420p,ass=filename={SCRIPT_NAME}:original_size={composition_width}x{composition_height}"
            )
        } else {
            String::new()
        };
        command.args(["-map", "0:V:0", "-an", "-sn", "-dn", "-filter_threads", "1"]);
        let input_filter = if playing {
            let duration = seconds(self.span.duration());
            command.args(["-t", &duration]);
            let fps = format!("fps={VIDEO_FPS}:start_time=0:eof_action=pass");
            let mut filter = fps;
            if self.reverse {
                filter.push_str(",reverse");
            }
            if self.speed_milli != 1000 {
                filter.push_str(&format!(",setpts=PTS/{:.3}", f32::from(self.speed_milli) / 1000.0));
            }
            append_video_fades(
                &mut filter,
                self.timeline_from,
                self.timeline_start,
                self.timeline_duration_us,
                self.fade_in_us,
                self.fade_out_us,
            );
            if (has_overlays || self.backdrop.enabled() || self.speed_milli != 1000)
                && self.path.is_some()
            {
                filter.push_str(&format!(",tpad=stop_mode=clone:stop_duration={duration}"));
            }
            filter
        } else {
            command.args(["-frames:v", "1"]);
            String::new()
        };
        let filter = if self.backdrop.enabled() {
            background::filter(
                &self.backdrop,
                (width as u32, height as u32),
                self.composition_size,
                self.timeline_from,
                self.path.is_some(),
                &input_filter,
                &output_filter,
            )?
        } else {
            // Map the original display aspect onto the rounded preview raster.
            let aspect = format!("dar*{width}*{composition_height}/({height}*{composition_width})");
            let fit = fit_filter_for_aspect([width, height], "black", &aspect);
            [input_filter, fit, output_filter]
                .into_iter()
                .filter(|filter| !filter.is_empty())
                .collect::<Vec<_>>()
                .join(",")
        };
        command.args([
            "-vf",
            &filter,
            "-fps_mode",
            "passthrough",
            "-threads:v",
            "1",
            "-c:v",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-f",
            "rawvideo",
            "pipe:1",
        ]);
        self.decoder.decode(command, |stdout| {
            let mut reader = BufReader::new(stdout);
            let mut count = 0;
            while !self.decoder.cancelled() {
                let mut pixels = vec![0; width * height * 4];
                match reader.read_exact(&mut pixels) {
                    Ok(()) => (),
                    Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
                    Err(error) => return Err(error.into()),
                }
                let frame_time = self
                    .timeline_from
                    .saturating_add(count * USECS_PER_SEC / VIDEO_FPS);
                if let Some(telemetry) = &self.cursor
                    && let Some(sample) = telemetry.sample_at(
                        frame_time,
                    )
                    && self.cursor_effect.contains(frame_time)
                {
                    render_cursor_overlay(
                        &mut pixels,
                        width as u32,
                        height as u32,
                        sample.x,
                        sample.y,
                        sample.pressed,
                        self.cursor_effect.cursor_scale,
                        self.cursor_effect.click_effect,
                        telemetry.zoom_scale_at(frame_time, self.cursor_effect.zoom_scale),
                    );
                }
                self.frames.send(VideoFrame {
                    time_us: count * USECS_PER_SEC / VIDEO_FPS,
                    pixels,
                })?;
                count += 1;
                self.ctx.request_repaint();
            }
            Ok(count)
        })
    }
}

fn render_cursor_overlay(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    x: f32,
    y: f32,
    pressed: bool,
    scale: f32,
    click_effect: super::cursor_effect::CursorClickEffect,
    zoom_scale: f32,
) {
    if width == 0 || height == 0 {
        return;
    }
    let point = (
        x.clamp(0.0, 1.0) * width as f32,
        y.clamp(0.0, 1.0) * height as f32,
    );
    if zoom_scale > 1.0 {
        zoom_rgba_around_point(pixels, width, height, point, zoom_scale);
    }
    if pressed && click_effect != super::cursor_effect::CursorClickEffect::None {
        let radius = 20.0 * scale;
        let min_x = (point.0 - radius - 3.0).max(0.0) as u32;
        let max_x = (point.0 + radius + 3.0).min(width as f32) as u32;
        let min_y = (point.1 - radius - 3.0).max(0.0) as u32;
        let max_y = (point.1 + radius + 3.0).min(height as f32) as u32;
        for py in min_y..max_y {
            for px in min_x..max_x {
                let distance = ((px as f32 + 0.5 - point.0).powi(2)
                    + (py as f32 + 0.5 - point.1).powi(2))
                .sqrt();
                if click_effect == super::cursor_effect::CursorClickEffect::Ring
                    && (distance - radius).abs() <= 2.5 * scale
                    || click_effect == super::cursor_effect::CursorClickEffect::Dot
                        && distance <= 5.0 * scale
                {
                    blend_rgba(pixels, width, height, px, py, [255, 150, 80, 255], 0.8);
                }
            }
        }
    }
    let size = 22.0 * scale;
    let points = [
        (0.0, 0.0),
        (0.0, size * 1.55),
        (size * 0.42, size * 1.15),
        (size * 0.82, size * 1.72),
        (size * 1.05, size * 1.54),
        (size * 0.64, size * 0.96),
        (size * 1.2, size * 0.96),
    ];
    let min_x = points
        .iter()
        .map(|(point_x, _)| point.0 + point_x)
        .fold(f32::INFINITY, f32::min)
        .max(0.0) as u32;
    let max_x = points
        .iter()
        .map(|(point_x, _)| point.0 + point_x)
        .fold(f32::NEG_INFINITY, f32::max)
        .min(width as f32) as u32;
    let min_y = points
        .iter()
        .map(|(_, point_y)| point.1 + point_y)
        .fold(f32::INFINITY, f32::min)
        .max(0.0) as u32;
    let max_y = points
        .iter()
        .map(|(_, point_y)| point.1 + point_y)
        .fold(f32::NEG_INFINITY, f32::max)
        .min(height as f32) as u32;
    for py in min_y..max_y {
        for px in min_x..max_x {
            let local = (px as f32 + 0.5 - point.0, py as f32 + 0.5 - point.1);
            if polygon_contains(local, &points) {
                blend_rgba(pixels, width, height, px, py, [25, 31, 43, 255], 0.94);
            }
        }
    }
}

fn zoom_rgba_around_point(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    point: (f32, f32),
    scale: f32,
) {
    let original = pixels.to_vec();
    for y in 0..height {
        for x in 0..width {
            let source_x = (point.0 + (x as f32 + 0.5 - point.0) / scale)
                .clamp(0.0, width.saturating_sub(1) as f32) as u32;
            let source_y = (point.1 + (y as f32 + 0.5 - point.1) / scale)
                .clamp(0.0, height.saturating_sub(1) as f32) as u32;
            let source = ((source_y * width + source_x) * 4) as usize;
            let target = ((y * width + x) * 4) as usize;
            pixels[target..target + 4].copy_from_slice(&original[source..source + 4]);
        }
    }
}

fn polygon_contains(point: (f32, f32), polygon: &[(f32, f32)]) -> bool {
    let mut inside = false;
    for (first, second) in polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
    {
        if (first.1 > point.1) != (second.1 > point.1) {
            let at_x = (second.0 - first.0) * (point.1 - first.1)
                / (second.1 - first.1)
                + first.0;
            if point.0 < at_x {
                inside = !inside;
            }
        }
    }
    inside
}

fn blend_rgba(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    color: [u8; 4],
    alpha: f32,
) {
    if x >= width || y >= height {
        return;
    }
    let index = ((y * width + x) * 4) as usize;
    let alpha = alpha.clamp(0.0, 1.0);
    for channel in 0..3 {
        pixels[index + channel] = (f32::from(color[channel]) * alpha
            + f32::from(pixels[index + channel]) * (1.0 - alpha))
            .round() as u8;
    }
    pixels[index + 3] = 0xff;
}
