use std::fs;
use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::path::Path;
use std::process::{Child, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};

use super::{CameraPreviewCommand, RecordingRequest, SavedRecording, macos};

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const FINALIZE_TIMEOUT: Duration = Duration::from_secs(30);

pub enum RecordingEvent {
    Started,
    CameraPreviewVisibility(bool),
    CameraPreviewClosed,
    CameraPreviewUnavailable,
    CameraUnavailable,
}

pub fn record(
    request: RecordingRequest,
    stopped: &AtomicBool,
    on_started: impl FnOnce(),
) -> Result<Option<SavedRecording>> {
    let mut on_started = Some(on_started);
    record_with_camera_preview(request, stopped, &mpsc::channel().1, |event| {
        if matches!(event, RecordingEvent::Started)
            && let Some(notify) = on_started.take()
        {
            notify();
        }
    })
}

pub fn record_with_camera_preview(
    request: RecordingRequest,
    stopped: &AtomicBool,
    camera_preview: &mpsc::Receiver<CameraPreviewCommand>,
    on_event: impl FnMut(RecordingEvent),
) -> Result<Option<SavedRecording>> {
    if stopped.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let output = std::path::absolute(&request.output).context("无法解析录像路径")?;
    ensure!(!output.try_exists()?, "已有同名录像：{}", output.display());
    let parent = output.parent().context("缺少录像保存目录")?;
    let temporary = tempfile::tempdir_in(parent).context("无法在保存目录暂存录像")?;
    let staged = temporary.path().join("recording.mp4");
    let staged_camera = request
        .camera_device
        .as_ref()
        .map(|_| temporary.path().join("recording-camera.mp4"));
    let mut child = macos::start_recording(
        &staged,
        request.microphone,
        request.microphone_device.as_deref(),
        request.source.as_deref(),
        request.camera_device.as_deref(),
        request.camera_mode.as_ref(),
        staged_camera.as_deref(),
        request.click_follow,
    )?;
    let stdout = child.stdout.take().expect("capture helper stdout is piped");
    let stderr = child.stderr.take().expect("capture helper stderr is piped");
    let (events_tx, events) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if events_tx.send(line).is_err() {
                break;
            }
        }
    });
    let diagnostics = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = BufReader::new(stderr).read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).trim().to_owned()
    });
    let mut state = CaptureState {
        on_event,
        started: None,
        cancelled: false,
        camera_saved: false,
    };
    let outcome = supervise(
        &mut child,
        stopped,
        request.duration,
        &events,
        camera_preview,
        &mut state,
    );
    if outcome.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = reader.join();
    let diagnostics = diagnostics.join().unwrap_or_default();
    state.receive(&events);
    let status = outcome?;
    if state.cancelled {
        return Ok(None);
    }
    if !status.success() {
        let detail = if diagnostics.is_empty() {
            status.to_string()
        } else {
            diagnostics
        };
        bail!("macOS 录制失败（{detail}）");
    }
    if state.started.is_none() && stopped.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let camera_output = if state.camera_saved {
        staged_camera
            .as_deref()
            .map(|staged| camera_recording_path(&output).map(|output| (staged, output)))
            .transpose()?
    } else {
        None
    };
    let warning = (!diagnostics.is_empty()).then_some(diagnostics);
    let camera_path = camera_output.as_ref().map(|(_, output)| output.clone());
    publish_capture(&staged, &output, camera_output)?;
    Ok(Some(SavedRecording {
        path: output,
        camera_path,
        warning,
    }))
}

struct CaptureState<F> {
    on_event: F,
    started: Option<Instant>,
    cancelled: bool,
    camera_saved: bool,
}

impl<F: FnMut(RecordingEvent)> CaptureState<F> {
    fn receive(&mut self, events: &mpsc::Receiver<String>) {
        while let Ok(event) = events.try_recv() {
            match event.as_str() {
                "recording" if self.started.is_none() => {
                    self.started = Some(Instant::now());
                    (self.on_event)(RecordingEvent::Started);
                }
                "camera-preview-closed" => (self.on_event)(RecordingEvent::CameraPreviewClosed),
                "camera-preview-shown" => {
                    (self.on_event)(RecordingEvent::CameraPreviewVisibility(true))
                }
                "camera-preview-hidden" => {
                    (self.on_event)(RecordingEvent::CameraPreviewVisibility(false))
                }
                "camera-preview-unavailable" => {
                    (self.on_event)(RecordingEvent::CameraPreviewUnavailable)
                }
                "camera-unavailable" => (self.on_event)(RecordingEvent::CameraUnavailable),
                "cancelled" => self.cancelled = true,
                "camera-recorded" => self.camera_saved = true,
                _ => {}
            }
        }
    }
}

fn supervise(
    child: &mut Child,
    stopped: &AtomicBool,
    duration: Option<Duration>,
    events: &mpsc::Receiver<String>,
    camera_preview: &mpsc::Receiver<CameraPreviewCommand>,
    state: &mut CaptureState<impl FnMut(RecordingEvent)>,
) -> Result<ExitStatus> {
    let mut stop_sent: Option<Instant> = None;
    let mut preview_commands_available = true;
    loop {
        state.receive(events);
        if let Some(status) = child.try_wait().context("无法检查录像进程状态")? {
            return Ok(status);
        }
        if let Some(sent) = stop_sent {
            ensure!(sent.elapsed() < FINALIZE_TIMEOUT, "等待 macOS 保存录像超时");
        } else if stopped.load(Ordering::Relaxed)
            || state
                .started
                .zip(duration)
                .is_some_and(|(start, limit)| start.elapsed() >= limit)
        {
            if let Some(mut stdin) = child.stdin.take()
                && let Err(error) = stdin.write_all(b"stop\n")
                && error.kind() != io::ErrorKind::BrokenPipe
            {
                return Err(error).context("无法通知 macOS 保存录像");
            }
            stop_sent = Some(Instant::now());
        } else if preview_commands_available && let Some(stdin) = child.stdin.as_mut() {
            for command in camera_preview.try_iter() {
                if command.write_to(stdin).is_err() {
                    preview_commands_available = false;
                    (state.on_event)(RecordingEvent::CameraPreviewUnavailable);
                    break;
                }
            }
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn camera_recording_path(output: &Path) -> Result<std::path::PathBuf> {
    let stem = output
        .file_stem()
        .context("录像路径缺少文件名")?
        .to_string_lossy();
    Ok(output.with_file_name(format!("{stem}-摄像头.mp4")))
}

fn publish_capture(
    staged: &Path,
    output: &Path,
    camera: Option<(&Path, std::path::PathBuf)>,
) -> Result<()> {
    let file = fs::metadata(staged).context("系统未生成录像文件")?;
    ensure!(file.is_file() && file.len() > 0, "系统生成的录像为空");
    if let Some((staged_camera, camera_output)) = &camera {
        let camera = fs::metadata(staged_camera).context("系统未生成摄像头录像")?;
        ensure!(
            camera.is_file() && camera.len() > 0,
            "系统生成的摄像头录像为空"
        );
        ensure!(!camera_output.try_exists()?, "摄像头录像已存在");
    }
    tempfile::TempPath::try_from_path(staged)?
        .persist_noclobber(output)
        .with_context(|| format!("无法保存录像到 {}", output.display()))?;
    if let Some((staged_camera, camera_output)) = &camera {
        if let Err(error) =
            tempfile::TempPath::try_from_path(staged_camera)?.persist_noclobber(camera_output)
        {
            let _ = fs::remove_file(output);
            return Err(error)
                .with_context(|| format!("无法保存摄像头录像到 {}", camera_output.display()));
        }
    }
    let staged_cursor = std::path::PathBuf::from(format!("{}.cursor.json", staged.display()));
    if staged_cursor.is_file() {
        let output_cursor = std::path::PathBuf::from(format!("{}.cursor.json", output.display()));
        if let Err(error) =
            tempfile::TempPath::try_from_path(&staged_cursor)?.persist_noclobber(&output_cursor)
        {
            let _ = fs::remove_file(output);
            if let Some((_, camera_output)) = &camera {
                let _ = fs::remove_file(camera_output);
            }
            return Err(error).context("无法保存鼠标轨迹文件");
        }
    }
    Ok(())
}
