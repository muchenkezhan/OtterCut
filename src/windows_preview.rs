use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::{Error as FrameError, Frame};
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    GraphicsCaptureItemType, MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

use crate::windows_recorder::CaptureSource;

const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(4);
const POLL_INTERVAL: Duration = Duration::from_millis(30);
const FRAME_INTERVAL: Duration = Duration::from_millis(250);
const DISPLAY_PREVIEW_LIMIT: (usize, usize) = (320, 180);
const WINDOW_PREVIEW_LIMIT: (usize, usize) = (480, 270);

pub struct PreviewFrame {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Runs a preview until cancelled, the source closes, or capture fails.
/// The callback runs on the Windows capture thread and should return promptly.
pub fn capture(
    source: CaptureSource,
    stop: &AtomicBool,
    on_frame: impl FnMut(PreviewFrame) + Send + 'static,
) -> Result<()> {
    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }

    let seen_frame = Arc::new(AtomicBool::new(false));
    let limit = match source {
        CaptureSource::Monitor(_) => DISPLAY_PREVIEW_LIMIT,
        CaptureSource::Window(_) => WINDOW_PREVIEW_LIMIT,
    };
    let flags = PreviewFlags {
        seen_frame: Arc::clone(&seen_frame),
        on_frame,
        limit,
    };
    let control = match source {
        CaptureSource::Monitor(monitor) => start_capture(monitor, flags)?,
        CaptureSource::Window(window) => start_capture(window, flags)?,
    };

    let started = Instant::now();
    loop {
        if stop.load(Ordering::Relaxed) {
            return control.stop().context("无法停止画面预览");
        }
        if control.is_finished() {
            control.wait().context("预览采集失败")?;
            bail!("录制来源已关闭，请重新选择来源");
        }
        if !seen_frame.load(Ordering::Relaxed) && started.elapsed() >= FIRST_FRAME_TIMEOUT {
            control.stop().context("预览首帧超时，无法停止采集")?;
            bail!("等待预览画面超时，请确认录制来源仍然可见");
        }
        thread::sleep(POLL_INTERVAL);
    }
}

struct PreviewFlags<F> {
    seen_frame: Arc<AtomicBool>,
    on_frame: F,
    limit: (usize, usize),
}

struct Preview<F> {
    flags: PreviewFlags<F>,
    last_sample: Option<Instant>,
}

fn start_capture<T, F>(
    item: T,
    flags: PreviewFlags<F>,
) -> Result<CaptureControl<Preview<F>, FrameError>>
where
    T: TryInto<GraphicsCaptureItemType> + Send + 'static,
    F: FnMut(PreviewFrame) + Send + 'static,
{
    Preview::<F>::start_free_threaded(Settings::new(
        item,
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::Default,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        flags,
    ))
    .context("无法启动录制来源的画面预览")
}

impl<F: FnMut(PreviewFrame) + Send + 'static> GraphicsCaptureApiHandler for Preview<F> {
    type Flags = PreviewFlags<F>;
    type Error = FrameError;

    fn new(context: Context<Self::Flags>) -> std::result::Result<Self, Self::Error> {
        Ok(Self {
            flags: context.flags,
            last_sample: None,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _: InternalCaptureControl,
    ) -> std::result::Result<(), Self::Error> {
        let now = Instant::now();
        if self
            .last_sample
            .is_some_and(|previous| now - previous < FRAME_INTERVAL)
        {
            return Ok(());
        }
        self.last_sample = Some(now);

        let mut buffer = frame.buffer()?;
        let width = buffer.width() as usize;
        let height = buffer.height() as usize;
        let row_pitch = buffer.row_pitch() as usize;
        let pixels = buffer.as_raw_buffer();
        if let Some(preview) = thumbnail(width, height, row_pitch, pixels, self.flags.limit) {
            (self.flags.on_frame)(preview);
            self.flags.seen_frame.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
}

fn thumbnail(
    source_width: usize,
    source_height: usize,
    row_pitch: usize,
    bgra: &[u8],
    (max_width, max_height): (usize, usize),
) -> Option<PreviewFrame> {
    if source_width == 0
        || source_height == 0
        || row_pitch < source_width * 4
        || bgra.len() < row_pitch * source_height
    {
        return None;
    }
    let scale = (max_width as f64 / source_width as f64)
        .min(max_height as f64 / source_height as f64)
        .min(1.0);
    let width = ((source_width as f64 * scale).round() as usize).max(1);
    let height = ((source_height as f64 * scale).round() as usize).max(1);
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let source_y = (2 * y + 1) * source_height / (2 * height);
        let start = source_y * row_pitch;
        for x in 0..width {
            let source_x = (2 * x + 1) * source_width / (2 * width);
            let pixel = start + source_x * 4;
            rgba.extend_from_slice(&[bgra[pixel + 2], bgra[pixel + 1], bgra[pixel], 255]);
        }
    }
    Some(PreviewFrame {
        width,
        height,
        rgba,
    })
}
