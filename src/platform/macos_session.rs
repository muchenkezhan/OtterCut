use std::fs;
use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::path::Path;
use std::process::{Child, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};

use super::{RecordingRequest, SavedRecording, macos};

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const FINALIZE_TIMEOUT: Duration = Duration::from_secs(30);

pub fn record(
    request: RecordingRequest,
    stopped: &AtomicBool,
    on_started: impl FnOnce(),
) -> Result<Option<SavedRecording>> {
    if stopped.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let output = std::path::absolute(&request.output).context("无法解析录像路径")?;
    ensure!(!output.try_exists()?, "已有同名录像：{}", output.display());
    let parent = output.parent().context("缺少录像保存目录")?;
    let temporary = tempfile::tempdir_in(parent).context("无法在保存目录暂存录像")?;
    let staged = temporary.path().join("recording.mp4");
    let mut child = macos::start_recording(&staged, request.microphone)?;
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
        notify_started: Some(on_started),
        started: None,
        cancelled: false,
    };
    let outcome = supervise(&mut child, stopped, request.duration, &events, &mut state);
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
    publish_capture(&staged, &output)?;
    Ok(Some(SavedRecording {
        path: output,
        warning: None,
    }))
}

struct CaptureState<F> {
    notify_started: Option<F>,
    started: Option<Instant>,
    cancelled: bool,
}

impl<F: FnOnce()> CaptureState<F> {
    fn receive(&mut self, events: &mpsc::Receiver<String>) {
        while let Ok(event) = events.try_recv() {
            match event.as_str() {
                "recording" => {
                    if let Some(notify) = self.notify_started.take() {
                        self.started = Some(Instant::now());
                        notify();
                    }
                }
                "cancelled" => self.cancelled = true,
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
    state: &mut CaptureState<impl FnOnce()>,
) -> Result<ExitStatus> {
    let mut stop_sent: Option<Instant> = None;
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
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn publish_capture(staged: &Path, output: &Path) -> Result<()> {
    let file = fs::metadata(staged).context("系统未生成录像文件")?;
    ensure!(file.is_file() && file.len() > 0, "系统生成的录像为空");
    tempfile::TempPath::try_from_path(staged)?
        .persist_noclobber(output)
        .with_context(|| format!("无法保存录像到 {}", output.display()))?;
    Ok(())
}
