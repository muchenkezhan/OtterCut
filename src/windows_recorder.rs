use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};
use clap::{Args, Parser, Subcommand};
use tempfile::Builder;
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoEncoderError,
    VideoSettingsBuilder, VideoSettingsSubType,
};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    GraphicsCaptureItemType, MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

use crate::camera::{CameraRecording, CameraSelection};
use crate::click_follow::ClickTracker;
use crate::microphone::Microphone;
use crate::zoom_frames::ZoomFrames;

const FRAME_RATE: u32 = 30;
const FRAME_TICKS_PER_SECOND: i64 = 10_000_000;
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Parser)]
#[command(version, about = "OtterCut Windows 录制原型")]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 列出可录制的显示器。
    Sources {
        /// 同时列出可录制窗口的标题。
        #[arg(long)]
        windows: bool,
    },
    /// 录制成 MP4，按 Ctrl+C 正常停止。
    Record(RecordArgs),
}

#[derive(Args)]
struct RecordArgs {
    /// 显示器编号，从 1 开始。未指定来源时录制主显示器。
    #[arg(long, conflicts_with = "window", value_parser = clap::value_parser!(u32).range(1..))]
    display: Option<u32>,
    /// 要录制的窗口完整标题。
    #[arg(long, value_parser = clap::builder::NonEmptyStringValueParser::new())]
    window: Option<String>,
    /// 输出 MP4 路径。
    #[arg(short, long)]
    output: PathBuf,
    /// 指定录制时长，到时自动保存。
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    seconds: Option<u64>,
    /// 同时录制系统默认麦克风。
    #[arg(long)]
    mic: bool,
    /// 点击后让录制画面平滑放大并跟随鼠标。
    #[arg(long)]
    click_follow: bool,
}

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Sources { windows } => list_sources(windows),
        Command::Record(args) => record_command(args),
    }
}

fn list_sources(list_windows: bool) -> Result<()> {
    for (index, monitor) in Monitor::enumerate()?.into_iter().enumerate() {
        let name = monitor.name().or_else(|_| monitor.device_name())?;
        println!(
            "显示器 {}  {}  {} × {}",
            index + 1,
            name.trim_end_matches('\0'),
            monitor.width()?,
            monitor.height()?
        );
    }
    if list_windows {
        for title in Window::enumerate()?
            .into_iter()
            .filter_map(|window| window.title().ok())
            .filter(|title| !title.trim().is_empty())
        {
            println!("窗口  {title}");
        }
    }
    Ok(())
}

fn record_command(args: RecordArgs) -> Result<()> {
    let stopped = Arc::new(AtomicBool::new(false));
    let interrupt = Arc::clone(&stopped);
    ctrlc::set_handler(move || interrupt.store(true, Ordering::Relaxed))
        .context("无法注册 Ctrl+C 停止命令")?;

    let source = if let Some(title) = args.window {
        CaptureSource::Window(find_window(&title)?)
    } else {
        CaptureSource::Monitor(match args.display {
            Some(display) => Monitor::from_index(display as usize)
                .with_context(|| format!("找不到显示器 {display}"))?,
            None => Monitor::primary().context("找不到主显示器")?,
        })
    };
    let request = RecordingRequest {
        source,
        output: args.output,
        microphone: args.mic,
        microphone_device: None,
        camera: None,
        click_follow: args.click_follow,
        duration: args.seconds.map(Duration::from_secs),
    };
    let saved = record(request, &stopped, |_| {
        println!("正在录制，按 Ctrl+C 停止。")
    })?;
    println!("已保存 {}（{} 帧）", saved.path.display(), saved.frames);
    if let Some(warning) = saved.warning {
        bail!("{warning}");
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CaptureSource {
    Monitor(Monitor),
    Window(Window),
}

pub struct RecordingRequest {
    pub source: CaptureSource,
    pub output: PathBuf,
    pub microphone: bool,
    pub microphone_device: Option<String>,
    pub camera: Option<CameraSelection>,
    pub click_follow: bool,
    pub duration: Option<Duration>,
}

pub struct SavedRecording {
    pub path: PathBuf,
    pub frames: u64,
    pub camera_path: Option<PathBuf>,
    pub warning: Option<String>,
}

pub fn record(
    request: RecordingRequest,
    stopped: &AtomicBool,
    mut on_started: impl FnMut(Instant),
) -> Result<SavedRecording> {
    let output = std::path::absolute(&request.output).context("无法解析输出路径")?;
    ensure!(
        output
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4")),
        "输出文件扩展名必须是 .mp4"
    );
    ensure!(
        !output.try_exists()?,
        "输出文件已存在（{}）",
        output.display()
    );
    let parent = output.parent().context("输出路径缺少父目录")?;
    ensure!(parent.is_dir(), "输出目录不存在（{}）", parent.display());
    let staged_file = Builder::new()
        .prefix(".takeseam-")
        .suffix(".mp4")
        .tempfile_in(parent)
        .context("无法在输出目录中创建录制文件")?
        .into_temp_path();

    let mic = request
        .microphone
        .then(|| Microphone::open(request.microphone_device.as_deref()))
        .transpose()?;
    let audio = mic.as_ref().map(|mic| (mic.channels, mic.sample_rate));
    let mut click_tracker = request
        .click_follow
        .then(|| ClickTracker::new(request.source));
    let flags = RecordingFlags {
        path: staged_file.to_path_buf(),
        audio,
        zoom: click_tracker
            .as_ref()
            .map(|tracker| ZoomFrames::new(tracker.sample_handle())),
    };
    let control = match request.source {
        CaptureSource::Window(window) => start_capture(window, flags)?,
        CaptureSource::Monitor(monitor) => start_capture(monitor, flags)?,
    };

    let callback = control.callback();
    let mut started = None;
    let mut camera = None;
    let mut camera_warning = None;
    let session = wait_for_stop(
        &control,
        mic.as_ref(),
        click_tracker.as_mut(),
        stopped,
        request.duration,
        &mut started,
        &mut |at| {
            on_started(at);
            if let Some(selection) = &request.camera {
                match CameraRecording::start(selection, &output) {
                    Ok(recording) => camera = Some(recording),
                    Err(error) => camera_warning = Some(format!("无法启动摄像头录制（{error:#}）")),
                }
            }
        },
    );
    if let Some(camera) = &mut camera {
        camera.request_stop();
    }
    let stop_result = if control.is_finished() {
        control.wait()
    } else {
        control.stop()
    };

    let mut recording = callback.lock();
    let mic_result = match mic.as_ref().filter(|_| started.is_some()) {
        Some(mic) => mic
            .pause()
            .and_then(|()| mic.receive(|packet| recording.send_audio(packet))),
        None => Ok(()),
    };
    let encode_result = recording.finish();
    stop_result.context("停止屏幕采集失败")?;
    encode_result.context("无法完成 MP4 封装")?;
    if recording.frames == 0 {
        session?;
        bail!("没有取得视频帧，请确保录制来源可见并重试");
    }
    ensure!(
        fs::metadata(&staged_file)?.len() > 0,
        "编码器未写入视频文件"
    );

    staged_file.persist_noclobber(&output).map_err(|error| {
        let rescued = error.path.keep().map_err(|err| err.error);
        match rescued {
            Ok(path) => anyhow::anyhow!(
                "无法发布到 {}，录制保存在 {}（{}）",
                output.display(),
                path.display(),
                error.error
            ),
            Err(rescue_error) => anyhow::anyhow!(
                "无法发布录制文件（{}），也无法保留临时文件（{rescue_error}）",
                error.error
            ),
        }
    })?;
    let camera_path = camera.and_then(|camera| match camera.finish() {
        Ok(path) => Some(path),
        Err(error) => {
            camera_warning = Some(format!("无法保存摄像头录像（{error:#}）"));
            None
        }
    });
    let mut warning = session
        .and(mic_result)
        .err()
        .map(|error| format!("{error:#}"))
        .or_else(|| recording.zoom_warning.take());
    if let Some(camera_warning) = camera_warning {
        warning = Some(match warning {
            Some(screen_warning) => format!("{screen_warning}。{camera_warning}"),
            None => camera_warning,
        });
    }
    Ok(SavedRecording {
        path: output,
        frames: recording.frames,
        camera_path,
        warning,
    })
}

fn find_window(title: &str) -> Result<Window> {
    let titles = Window::enumerate().context("无法列出可录制窗口")?;
    let mut matching = titles
        .into_iter()
        .filter(|window| window.title().is_ok_and(|candidate| candidate == title));
    let window = matching
        .next()
        .with_context(|| format!("找不到窗口「{title}」"))?;
    ensure!(matching.next().is_none(), "有多个同名窗口「{title}」");
    Ok(window)
}

fn start_capture<T>(item: T, flags: RecordingFlags) -> Result<RecordingControl>
where
    T: TryInto<GraphicsCaptureItemType> + Send + 'static,
{
    let settings = Settings::new(
        item,
        CursorCaptureSettings::Default,
        DrawBorderSettings::Default,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        flags,
    );
    Recording::start_free_threaded(settings).context("无法启动 Windows Graphics Capture")
}

fn wait_for_stop(
    control: &RecordingControl,
    mic: Option<&Microphone>,
    mut click_tracker: Option<&mut ClickTracker>,
    stopped: &AtomicBool,
    limit: Option<Duration>,
    started: &mut Option<Instant>,
    on_started: &mut impl FnMut(Instant),
) -> Result<()> {
    let waiting_since = Instant::now();
    let callback = control.callback();
    while !control.is_finished() && !stopped.load(Ordering::Relaxed) {
        if started.is_none() {
            let first_frame_at = callback.lock().first_frame_at;
            if let Some(first_frame_at) = first_frame_at {
                if let Some(mic) = mic {
                    mic.start()?;
                    *started = Some(first_frame_at);
                    let delay = first_frame_at.elapsed();
                    let frames = (delay.as_nanos() * u128::from(mic.sample_rate)) / 1_000_000_000;
                    let bytes =
                        usize::try_from(frames)? * usize::from(mic.channels) * size_of::<i16>();
                    callback.lock().send_audio(&vec![0; bytes])?;
                }
                *started = Some(first_frame_at);
                on_started(first_frame_at);
            } else if waiting_since.elapsed() >= FIRST_FRAME_TIMEOUT {
                bail!("等待首帧超时，请确认显示器已开启或目标窗口未最小化");
            }
        }
        if let Some(started) = *started {
            if let Some(tracker) = &mut click_tracker {
                tracker.poll();
            }
            if let Some(mic) = mic {
                mic.receive(|packet| callback.lock().send_audio(packet))?;
            }
            if limit.is_some_and(|limit| started.elapsed() >= limit) {
                break;
            }
        }
        thread::sleep(POLL_INTERVAL);
    }
    Ok(())
}

type RecordingControl = CaptureControl<Recording, VideoEncoderError>;

struct RecordingFlags {
    path: PathBuf,
    audio: Option<(u16, u32)>,
    zoom: Option<ZoomFrames>,
}

struct Recording {
    flags: RecordingFlags,
    encoder: Option<VideoEncoder>,
    output_size: (u32, u32),
    zoom_warning: Option<String>,
    first_timestamp: Option<i64>,
    first_frame_at: Option<Instant>,
    last_slot: i64,
    frames: u64,
}

impl Recording {
    fn send_audio(&mut self, pcm: &[u8]) -> Result<()> {
        if let Some(encoder) = &mut self.encoder {
            // windows-capture timestamps PCM from its running sample count.
            encoder
                .send_audio_buffer(pcm, 0)
                .context("无法写入麦克风音频")?;
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), VideoEncoderError> {
        self.encoder.take().map_or(Ok(()), VideoEncoder::finish)
    }
}

impl GraphicsCaptureApiHandler for Recording {
    type Flags = RecordingFlags;
    type Error = VideoEncoderError;

    fn new(context: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            flags: context.flags,
            encoder: None,
            output_size: (0, 0),
            zoom_warning: None,
            first_timestamp: None,
            first_frame_at: None,
            last_slot: -1,
            frames: 0,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _capture: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let frame_received_at = Instant::now();
        let timestamp = frame.timestamp()?.Duration;
        let origin = *self.first_timestamp.get_or_insert(timestamp);
        let slot = (timestamp - origin) * i64::from(FRAME_RATE) / FRAME_TICKS_PER_SECOND;
        if slot <= self.last_slot {
            return Ok(());
        }

        if self.encoder.is_none() {
            let width = frame.width().next_multiple_of(2);
            let height = frame.height().next_multiple_of(2);
            self.encoder = Some(create_encoder(width, height, &self.flags)?);
            self.output_size = (width, height);
        }
        if let Some(encoder) = &mut self.encoder {
            let zoom = match &mut self.flags.zoom {
                Some(zoom) => zoom.render(frame, frame_received_at, self.output_size),
                None => Ok(None),
            };
            match zoom {
                Ok(Some(pixels)) => encoder.send_frame_buffer(pixels, timestamp)?,
                Ok(None) => encoder.send_frame(frame)?,
                Err(error) => {
                    self.zoom_warning.get_or_insert_with(|| {
                        format!("点击跟随处理失败，受影响的帧保留原画面（{error:#}）")
                    });
                    encoder.send_frame(frame)?;
                }
            }
            self.first_frame_at.get_or_insert(frame_received_at);
            self.last_slot = slot;
            self.frames += 1;
        }
        Ok(())
    }
}

fn create_encoder(
    width: u32,
    height: u32,
    flags: &RecordingFlags,
) -> Result<VideoEncoder, VideoEncoderError> {
    let video = VideoSettingsBuilder::new(width, height)
        .sub_type(VideoSettingsSubType::H264)
        .frame_rate(FRAME_RATE)
        .bitrate(12_000_000);
    let audio = match flags.audio {
        Some((channels, sample_rate)) => AudioSettingsBuilder::new()
            .channel_count(u32::from(channels))
            .sample_rate(sample_rate)
            .bitrate(160_000),
        None => AudioSettingsBuilder::new().disabled(true),
    };
    VideoEncoder::new(video, audio, ContainerSettingsBuilder::new(), &flags.path)
}
