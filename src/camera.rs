use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use tempfile::{Builder, TempPath};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const QUERY_TIMEOUT: Duration = Duration::from_secs(7);
const PREVIEW_TIMEOUT: Duration = Duration::from_secs(6);
const FINISH_TIMEOUT: Duration = Duration::from_secs(8);
const LOG_LIMIT: u64 = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraDevice {
    pub name: String,
    pub selector: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum CaptureFormat {
    Pixel(String),
    Codec(String),
    Automatic,
}

impl CaptureFormat {
    fn priority(&self) -> u8 {
        match self {
            Self::Codec(codec) if codec == "mjpeg" => 0,
            Self::Pixel(_) => 1,
            Self::Codec(_) => 2,
            Self::Automatic => 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraMode {
    pub width: u32,
    pub height: u32,
    pub fps_milli: u32,
    format: CaptureFormat,
}

impl CameraMode {
    pub fn label(&self) -> String {
        format!(
            "{} × {} · {} 帧/秒",
            self.width,
            self.height,
            self.frame_rate()
        )
    }

    fn frame_rate(&self) -> String {
        let value = format!("{}.{:03}", self.fps_milli / 1000, self.fps_milli % 1000);
        value.trim_end_matches('0').trim_end_matches('.').to_owned()
    }

    fn valid(&self) -> bool {
        (1..=16384).contains(&self.width)
            && (1..=16384).contains(&self.height)
            && (1..=1_000_000).contains(&self.fps_milli)
    }
}

#[derive(Clone, Debug)]
pub struct CameraSelection {
    pub ffmpeg: PathBuf,
    pub device: CameraDevice,
    pub mode: CameraMode,
}

pub struct CameraFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn devices(ffmpeg: &Path) -> Result<Vec<CameraDevice>> {
    let log = query(
        command(ffmpeg).args(["-list_devices", "true", "-f", "dshow", "-i", "dummy"]),
        "无法读取摄像头设备",
    )?;
    let cameras = parse_devices(&log);
    if cameras.is_empty()
        && !log.lines().filter_map(dshow_line).any(|line| {
            line.starts_with("DirectShow video devices")
                || line.starts_with("Could not enumerate video only devices")
                || named_device(line).is_some_and(|(_, kind)| kind == "(audio)")
        })
    {
        bail!("无法读取摄像头设备（{}）", diagnostic(&log));
    }
    Ok(cameras)
}

pub fn modes(ffmpeg: &Path, device: &CameraDevice) -> Result<Vec<CameraMode>> {
    let input = device_argument(device)?;
    let log = query(
        command(ffmpeg)
            .args(["-list_options", "true", "-f", "dshow", "-i"])
            .arg(input),
        "无法读取摄像头画质",
    )?;
    let modes = parse_modes(&log);
    ensure!(
        !modes.is_empty(),
        "设备没有报告可用画质（{}）",
        diagnostic(&log)
    );
    Ok(modes)
}

fn dshow_line(line: &str) -> Option<&str> {
    let (_, message) = line
        .trim_start()
        .strip_prefix("[dshow @")?
        .split_once(']')?;
    Some(message.trim())
}

fn named_device(line: &str) -> Option<(&str, &str)> {
    let quoted = line.strip_prefix('"')?;
    let end = quoted.rfind('"')?;
    Some((&quoted[..end], quoted[end + 1..].trim()))
}

fn parse_devices(log: &str) -> Vec<CameraDevice> {
    let mut devices: Vec<CameraDevice> = Vec::new();
    let mut video_section = false;
    let mut latest_video = None;
    for line in log.lines().filter_map(dshow_line) {
        if line.starts_with("DirectShow video devices") {
            video_section = true;
            latest_video = None;
        } else if line.starts_with("DirectShow audio devices") {
            video_section = false;
            latest_video = None;
        } else if let Some((name, kind)) = named_device(line) {
            latest_video = None;
            if !name.is_empty() && (kind == "(video)" || (video_section && kind.is_empty())) {
                latest_video = Some(devices.len());
                devices.push(CameraDevice {
                    name: name.into(),
                    selector: name.into(),
                });
            }
        } else if let Some(alternative) = line.strip_prefix("Alternative name ")
            && let (Some(index), Some((selector, _))) =
                (latest_video.take(), named_device(alternative))
            && !selector.is_empty()
        {
            devices[index].selector = selector.into();
        }
    }
    let mut seen = std::collections::HashSet::new();
    devices.retain(|camera| seen.insert(camera.selector.clone()));
    devices
}

fn parse_modes(log: &str) -> Vec<CameraMode> {
    let mut modes = Vec::new();
    for line in log.lines().filter_map(dshow_line) {
        let words: Vec<_> = line.split_whitespace().collect();
        let format = words
            .first()
            .copied()
            .map_or(CaptureFormat::Automatic, |word| {
                if let Some(pixel) = word.strip_prefix("pixel_format=") {
                    CaptureFormat::Pixel(pixel.into())
                } else if let Some(codec) = word.strip_prefix("vcodec=") {
                    CaptureFormat::Codec(codec.into())
                } else {
                    CaptureFormat::Automatic
                }
            });
        for pair in words.windows(2) {
            let Some((width, height)) = pair[0]
                .strip_prefix("s=")
                .and_then(|size| size.split_once('x'))
            else {
                continue;
            };
            let (Ok(width), Ok(height), Some(Ok(fps))) = (
                width.parse::<u32>(),
                height.parse::<u32>(),
                pair[1].strip_prefix("fps=").map(str::parse::<f64>),
            ) else {
                continue;
            };
            if !fps.is_finite() || !(0.001..=1000.0).contains(&fps) {
                continue;
            }
            let mode = CameraMode {
                width,
                height,
                fps_milli: (fps * 1000.0).round() as u32,
                format: format.clone(),
            };
            if mode.valid() {
                modes.push(mode);
            }
        }
    }
    modes.sort_by_key(|mode| {
        (
            std::cmp::Reverse(u64::from(mode.width) * u64::from(mode.height)),
            std::cmp::Reverse(mode.width),
            std::cmp::Reverse(mode.fps_milli),
            mode.format.priority(),
        )
    });
    modes.dedup_by_key(|mode| (mode.width, mode.height, mode.fps_milli));
    modes
}

fn device_argument(device: &CameraDevice) -> Result<String> {
    // DirectShow treats colons as additional source separators in the input argument.
    let selector = &device.selector;
    ensure!(
        !selector.is_empty() && !selector.contains(':') && !selector.chars().any(char::is_control),
        "摄像头标识无效，请刷新设备列表"
    );
    Ok(format!("video={selector}"))
}

fn command(ffmpeg: &Path) -> Command {
    let mut command = Command::new(ffmpeg);
    command
        .creation_flags(CREATE_NO_WINDOW)
        .env("AV_LOG_FORCE_NOCOLOR", "1")
        .arg("-hide_banner");
    command
}

fn video_input(command: &mut Command, selection: &CameraSelection) -> Result<()> {
    let mode = &selection.mode;
    ensure!(mode.valid(), "摄像头画质无效");
    command
        .args(["-f", "dshow", "-rtbufsize", "96M", "-video_size"])
        .arg(format!("{}x{}", mode.width, mode.height))
        .arg("-framerate")
        .arg(mode.frame_rate());
    match &mode.format {
        CaptureFormat::Pixel(format) => {
            command.arg("-pixel_format").arg(format);
        }
        CaptureFormat::Codec(codec) => {
            command.arg("-c:v").arg(codec);
        }
        CaptureFormat::Automatic => {}
    }
    command.arg("-i").arg(device_argument(&selection.device)?);
    Ok(())
}

struct Process {
    child: Child,
    log: File,
}

impl Process {
    fn spawn(command: &mut Command) -> Result<Self> {
        let log = tempfile::tempfile().context("无法创建摄像头日志")?;
        let child = command
            .stderr(Stdio::from(log.try_clone()?))
            .spawn()
            .context("无法启动 FFmpeg")?;
        Ok(Self { child, log })
    }

    fn wait_until(&mut self, timeout: Duration) -> Result<Option<ExitStatus>> {
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(Some(status));
            }
            if started.elapsed() >= timeout {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn read_log(&mut self) -> Result<String> {
        let size = self.log.metadata()?.len();
        self.log
            .seek(SeekFrom::Start(size.saturating_sub(LOG_LIMIT)))?;
        let mut bytes = Vec::new();
        (&mut self.log).take(LOG_LIMIT).read_to_end(&mut bytes)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn stop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.stop();
    }
}

fn query(command: &mut Command, action: &str) -> Result<String> {
    let mut process = Process::spawn(command.stdin(Stdio::null()).stdout(Stdio::null()))
        .with_context(|| action.to_owned())?;
    // DirectShow lists devices or modes in stderr even though it exits without opening an output.
    ensure!(
        process.wait_until(QUERY_TIMEOUT)?.is_some(),
        "{action}（设备响应超时）"
    );
    process.read_log()
}

fn diagnostic(log: &str) -> String {
    let message = log
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.contains("Immediate exit requested"))
        .unwrap_or("设备未返回错误详情");
    message.chars().take(220).collect()
}

pub struct CameraPreview {
    process: Process,
    reader: Option<JoinHandle<()>>,
    latest: Arc<Mutex<Option<CameraFrame>>>,
    started: Instant,
    received: bool,
}

impl CameraPreview {
    pub fn start(selection: &CameraSelection, notify: impl Fn() + Send + 'static) -> Result<Self> {
        let mut command = command(&selection.ffmpeg);
        command.args(["-nostdin", "-loglevel", "error", "-filter_threads", "1"]);
        video_input(&mut command, selection)?;
        let (width, height) = preview_size(&selection.mode);
        let fps = selection.mode.fps_milli.min(15_000);
        let filter = format!("fps={fps}/1000,scale={width}:{height}:flags=bilinear");
        command
            .args(["-map", "0:v:0", "-an", "-sn", "-dn", "-vf"])
            .arg(filter)
            .args([
                "-c:v",
                "rawvideo",
                "-threads:v",
                "1",
                "-pix_fmt",
                "rgba",
                "-f",
                "rawvideo",
                "pipe:1",
            ]);
        let mut process = Process::spawn(command.stdin(Stdio::null()).stdout(Stdio::piped()))?;
        let stdout = process.child.stdout.take().context("无法读取摄像头预览")?;
        let latest = Arc::new(Mutex::new(None::<CameraFrame>));
        let output = Arc::clone(&latest);
        let reader = thread::Builder::new()
            .name("camera-preview".into())
            .spawn(move || {
                let mut stdout = BufReader::new(stdout);
                let frame_size = width as usize * height as usize * 4;
                let mut bytes = vec![0; frame_size];
                while stdout.read_exact(&mut bytes).is_ok() {
                    let previous = output.lock().unwrap().replace(CameraFrame {
                        width,
                        height,
                        rgba: bytes,
                    });
                    bytes = previous.map_or_else(|| vec![0; frame_size], |frame| frame.rgba);
                    notify();
                }
                notify();
            })
            .context("无法启动摄像头预览读取任务")?;
        Ok(Self {
            process,
            reader: Some(reader),
            latest,
            started: Instant::now(),
            received: false,
        })
    }

    pub fn poll(&mut self) -> Result<Option<CameraFrame>> {
        let frame = self.latest.lock().unwrap().take();
        self.received |= frame.is_some();
        if self.process.child.try_wait()?.is_some()
            || self.reader.as_ref().is_some_and(JoinHandle::is_finished)
        {
            self.process.stop();
            bail!(
                "摄像头预览已停止（{}）",
                diagnostic(&self.process.read_log()?)
            );
        }
        if !self.received && self.started.elapsed() >= PREVIEW_TIMEOUT {
            self.process.stop();
            bail!("摄像头首帧超时，请检查设备是否被占用");
        }
        Ok(frame)
    }
}

impl Drop for CameraPreview {
    fn drop(&mut self) {
        self.process.stop();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn preview_size(mode: &CameraMode) -> (u32, u32) {
    let factor = (640.0 / f64::from(mode.width))
        .min(360.0 / f64::from(mode.height))
        .min(1.0);
    (
        (f64::from(mode.width) * factor).round().max(1.0) as u32,
        (f64::from(mode.height) * factor).round().max(1.0) as u32,
    )
}

pub struct CameraRecording {
    process: Process,
    temporary: TempPath,
    output: PathBuf,
}

impl CameraRecording {
    pub fn start(selection: &CameraSelection, screen_output: &Path) -> Result<Self> {
        let parent = screen_output.parent().context("录像目录无效")?;
        let mut name = screen_output
            .file_stem()
            .context("录像文件名无效")?
            .to_os_string();
        name.push("-摄像头.mp4");
        let output = parent.join(name);
        ensure!(
            !output.try_exists()?,
            "摄像头录像已存在（{}）",
            output.display()
        );
        let temporary = Builder::new()
            .prefix(".ottercut-camera-")
            .suffix(".mp4")
            .tempfile_in(parent)
            .context("无法创建摄像头临时录像")?
            .into_temp_path();
        let mut command = command(&selection.ffmpeg);
        command.args(["-loglevel", "error", "-y", "-filter_threads", "1"]);
        video_input(&mut command, selection)?;
        command
            .args([
                "-map",
                "0:v:0",
                "-an",
                "-sn",
                "-dn",
                "-vf",
                "pad=ceil(iw/2)*2:ceil(ih/2)*2",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "20",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
                "-f",
                "mp4",
            ])
            .arg(temporary.as_os_str());
        let process = Process::spawn(command.stdin(Stdio::piped()).stdout(Stdio::null()))?;
        Ok(Self {
            process,
            temporary,
            output,
        })
    }

    pub fn request_stop(&mut self) {
        if let Some(mut stdin) = self.process.child.stdin.take() {
            let _ = stdin.write_all(b"q\n");
        }
    }

    pub fn finish(mut self) -> Result<PathBuf> {
        self.request_stop();
        let Some(status) = self.process.wait_until(FINISH_TIMEOUT)? else {
            self.process.stop();
            bail!("停止摄像头录制超时");
        };
        ensure!(
            status.success(),
            "摄像头录制失败（{}）",
            diagnostic(&self.process.read_log()?)
        );
        ensure!(has_video_data(&self.temporary)?, "没有取得摄像头视频帧");
        self.temporary
            .persist_noclobber(&self.output)
            .map_err(|error| error.error)
            .with_context(|| format!("无法保存摄像头录像（{}）", self.output.display()))?;
        Ok(self.output)
    }
}

// A successful encoder can write an empty MP4. The camera is video-only, so a
// finalized movie header and nonempty media-data box establish that frames exist.
fn has_video_data(path: &Path) -> io::Result<bool> {
    let mut file = File::open(path)?;
    let end = file.metadata()?.len();
    let mut offset = 0;
    let mut movie = false;
    let mut media = false;
    while end.saturating_sub(offset) >= 8 {
        file.seek(SeekFrom::Start(offset))?;
        let mut header = [0; 8];
        file.read_exact(&mut header)?;
        let nominal_size = u32::from_be_bytes(header[..4].try_into().unwrap());
        let (size, header_size) = match nominal_size {
            0 => (end - offset, 8),
            1 => {
                if end - offset < 16 {
                    return Ok(false);
                }
                let mut extended = [0; 8];
                file.read_exact(&mut extended)?;
                (u64::from_be_bytes(extended), 16)
            }
            size => (u64::from(size), 8),
        };
        if size < header_size || size > end - offset {
            return Ok(false);
        }
        if size > header_size {
            movie |= &header[4..] == b"moov";
            media |= &header[4..] == b"mdat";
        }
        offset += size;
    }
    Ok(movie && media && offset == end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_devices_keep_distinct_directshow_selectors_and_skip_audio() {
        let log = r#"
[dshow @ 123] "Web camera" (video)
[dshow @ 123]   Alternative name "@device_pnp_\\?\usb#first"
[dshow @ 123] "Web camera" (video)
[dshow @ 123]   Alternative name "@device_pnp_\\?\usb#second"
[dshow @ 123] "Microphone" (audio)
[dshow @ 123]   Alternative name "@device_cm_audio"
"#;
        let cameras = parse_devices(log);
        assert_eq!(cameras.len(), 2);
        assert_eq!(cameras[0].name, "Web camera");
        assert!(cameras[0].selector.ends_with("first"));
        assert!(cameras[1].selector.ends_with("second"));
        assert!(
            parse_devices("[dshow @ 123] Could not enumerate video only devices (or none found).")
                .is_empty()
        );
    }

    #[test]
    fn old_directshow_device_headers_are_also_supported() {
        let log = r#"
[dshow @ 123] DirectShow video devices (some may be both video and audio devices)
[dshow @ 123]   "USB Video"
[dshow @ 123] DirectShow audio devices
[dshow @ 123]   "USB Audio"
"#;
        assert_eq!(
            parse_devices(log),
            [CameraDevice {
                name: "USB Video".into(),
                selector: "USB Video".into(),
            }]
        );
    }

    #[test]
    fn modes_use_reported_sizes_and_preserve_fractional_frame_rates() {
        let log = "
[dshow @ 123] pixel_format=yuyv422 min s=640x480 fps=30 max s=640x480 fps=30
[dshow @ 123] pixel_format=nv12 min s=1920x1080 fps=29.9701 max s=1920x1080 fps=29.9701
[dshow @ 123] vcodec=mjpeg min s=1920x1080 fps=10 max s=1920x1080 fps=29.9701
[dshow @ 123] vcodec=mjpeg min s=1920x1080 fps=0 max s=0x0 fps=NaN
";
        let modes = parse_modes(log);
        assert_eq!(modes.len(), 3);
        assert_eq!(
            (modes[0].width, modes[0].height, modes[0].fps_milli),
            (1920, 1080, 29_970)
        );
        assert_eq!(modes[0].label(), "1920 × 1080 · 29.97 帧/秒");
        assert!(matches!(&modes[0].format, CaptureFormat::Codec(name) if name == "mjpeg"));
        assert_eq!(preview_size(&modes[2]), (480, 360));
    }

    #[test]
    fn finalization_requires_nonempty_media_and_a_complete_movie() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            temp.path(),
            b"\x00\x00\x00\x08ftyp\x00\x00\x00\x09moovQ\x00\x00\x00\x09mdatX",
        )
        .unwrap();
        assert!(has_video_data(temp.path()).unwrap());
        std::fs::write(temp.path(), b"\x00\x00\x00\x09moovQ\x00\x00\x00\x08mdat").unwrap();
        assert!(!has_video_data(temp.path()).unwrap());
        std::fs::write(temp.path(), b"\x00\x00\x00\x09moovQ\x00\x00\x00\x0amdatX").unwrap();
        assert!(!has_video_data(temp.path()).unwrap());
    }
}
