use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::OwnedFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};
use async_io::Timer;
use tempfile::{Builder, NamedTempFile};

use crate::platform::{RecordingRequest, SavedRecording};

const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(15);
const FINALIZE_TIMEOUT: Duration = Duration::from_secs(12);
const POLL_INTERVAL: Duration = Duration::from_millis(75);
// The MP4 headers alone are smaller than this. Wait for actual encoded payload.
const MIN_RECORDING_BYTES: u64 = 128;

pub(super) async fn record(
    remote: OwnedFd,
    node: u32,
    request: RecordingRequest,
    should_stop: impl Fn() -> bool,
    on_started: impl FnOnce(),
) -> Result<Option<SavedRecording>> {
    if should_stop() {
        return Ok(None);
    }
    let parent = request
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty());
    let parent = parent.unwrap_or_else(|| Path::new("."));
    ensure!(!request.output.exists(), "录制文件已经存在");
    let temporary = Builder::new()
        .prefix(".ottercut-")
        .suffix(".mp4")
        .tempfile_in(parent)
        .context("无法创建临时录像文件")?;
    let filename = temporary
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .context("无法确定临时录像文件名")?;
    let log = tempfile::tempfile().context("无法创建录屏日志")?;

    let mut gst = Command::new("gst-launch-1.0");
    gst.current_dir(parent)
        .process_group(0)
        // Stdio transfers the authorized PipeWire socket safely across exec.
        .stdin(Stdio::from(remote))
        .stdout(Stdio::null())
        .stderr(Stdio::from(log.try_clone()?))
        .args(["-q", "-e", "pipewiresrc", "fd=0", "keepalive-time=100"])
        .arg(format!("path={node}"))
        .args([
            "!",
            "queue",
            "!",
            "videoconvert",
            "!",
            "videoscale",
            "!",
            "videorate",
            "!",
            "video/x-raw,format=I420,framerate=30/1,width=(int)[2,2147483646,2],height=(int)[2,2147483646,2]",
            "!",
            "x264enc",
            "speed-preset=veryfast",
            "tune=zerolatency",
            "key-int-max=60",
            "bitrate=8000",
            "!",
            "h264parse",
            "!",
            "mp4mux",
            "name=mux",
            "!",
            "filesink",
            "buffer-mode=unbuffered",
        ])
        .arg(format!("location={filename}"));

    if request.microphone {
        gst.args([
            "pulsesrc",
            "!",
            "audioconvert",
            "!",
            "audioresample",
            "!",
            "avenc_aac",
            "!",
            "aacparse",
            "!",
            "queue",
            "!",
            "mux.",
        ]);
    }

    let mut encoder = Encoder(
        gst.spawn()
            .context("无法启动 gst-launch-1.0，请安装 GStreamer 和 PipeWire 插件")?,
    );
    // The command itself retains its configured stdio descriptors until dropped.
    drop(gst);

    let initialized_at = Instant::now();
    while !has_media(&temporary)? {
        if should_stop() {
            return Ok(None);
        }
        if let Some(status) = encoder.0.try_wait()? {
            bail!("{}", encoder_error(status, log));
        }
        ensure!(
            initialized_at.elapsed() < FIRST_FRAME_TIMEOUT,
            "GStreamer 没有收到录制画面"
        );
        Timer::after(POLL_INTERVAL).await;
    }
    if should_stop() {
        return Ok(None);
    }

    on_started();
    let started = Instant::now();
    let status = loop {
        if let Some(status) = encoder.0.try_wait()? {
            break status;
        }
        if should_stop()
            || request
                .duration
                .is_some_and(|duration| started.elapsed() >= duration)
        {
            break encoder.finish().await.context("GStreamer 封装录像失败")?;
        }
        Timer::after(POLL_INTERVAL).await;
    };
    ensure!(status.success(), "{}", encoder_error(status, log));
    ensure_complete_mp4(temporary.as_file())?;
    temporary.as_file().sync_all().context("无法写入录像")?;
    temporary
        .persist_noclobber(&request.output)
        .context("无法保存录像，目标位置可能已存在同名文件")?;
    Ok(Some(SavedRecording {
        path: request.output,
        camera_path: None,
        warning: None,
    }))
}

fn has_media(file: &NamedTempFile) -> Result<bool> {
    Ok(file.as_file().metadata()?.len() > MIN_RECORDING_BYTES)
}

fn ensure_complete_mp4(mut file: &File) -> Result<()> {
    let file_size = file.metadata()?.len();
    let mut offset = 0;
    let mut has_media = false;
    let mut has_movie = false;

    while offset < file_size {
        file.seek(SeekFrom::Start(offset))?;
        let mut header = [0; 8];
        file.read_exact(&mut header).context("录像文件不完整")?;
        let mut size = u64::from(u32::from_be_bytes(header[..4].try_into().unwrap()));
        let mut header_size = 8;
        if size == 1 {
            let mut extended_size = [0; 8];
            file.read_exact(&mut extended_size)
                .context("录像文件不完整")?;
            size = u64::from_be_bytes(extended_size);
            header_size = 16;
        } else if size == 0 {
            size = file_size - offset;
        }
        ensure!(
            size >= header_size && size <= file_size - offset,
            "录像文件不完整"
        );
        if size > header_size {
            has_media |= &header[4..] == b"mdat";
            has_movie |= &header[4..] == b"moov";
        }
        offset += size;
    }

    ensure!(has_media && has_movie, "录像缺少媒体数据或 MP4 索引");
    Ok(())
}

fn encoder_error(status: ExitStatus, mut log: File) -> String {
    let mut detail = Vec::new();
    if log.seek(SeekFrom::Start(0)).is_ok() {
        let _ = log.take(8192).read_to_end(&mut detail);
    }
    let detail = String::from_utf8_lossy(&detail);
    format!("GStreamer 录制失败（{status}）{}", detail.trim())
}

struct Encoder(Child);

impl Encoder {
    async fn finish(&mut self) -> Result<ExitStatus> {
        if let Some(status) = self.0.try_wait()? {
            return Ok(status);
        }
        // The child is exclusively owned here and cannot have its PID reused
        // before it is reaped. SIGINT makes gst-launch -e write the MP4 trailer.
        let signal_result = unsafe { libc::kill(self.0.id() as libc::pid_t, libc::SIGINT) };
        if signal_result == -1 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error.into());
            }
        }
        let deadline = Instant::now() + FINALIZE_TIMEOUT;
        loop {
            if let Some(status) = self.0.try_wait()? {
                return Ok(status);
            }
            ensure!(Instant::now() < deadline, "等待录像封装超时");
            Timer::after(POLL_INTERVAL).await;
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
