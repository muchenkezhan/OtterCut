use std::fs;
use std::io::{self, BufWriter, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use eframe::egui;
use takeseam::microphone::Microphone;
use tempfile::{Builder, NamedTempFile};

const WAV_HEADER_SIZE: usize = 44;
const RIFF_OVERHEAD: u32 = 36;
const BYTES_PER_SAMPLE: u16 = 2;

pub(super) struct NarrationCapture {
    stop: Option<Sender<()>>,
    events: Option<Receiver<CaptureEvent>>,
    started_at: Option<Instant>,
}

enum CaptureEvent {
    Started(Instant),
    Finished(Result<RecordedNarration>),
}

impl NarrationCapture {
    pub(super) fn start(directory: PathBuf, ctx: &egui::Context) -> Result<Self> {
        let (stop, stop_requested) = mpsc::channel();
        let (events, incoming) = mpsc::channel();
        let ctx = ctx.clone();
        thread::Builder::new()
            .name(format!("{} narration capture", takeseam::PRODUCT_NAME))
            .spawn(move || {
                let result = record(&directory, &stop_requested, || {
                    events
                        .send(CaptureEvent::Started(Instant::now()))
                        .map_err(|_| anyhow!("已取消旁白录制"))?;
                    ctx.request_repaint();
                    Ok(())
                });
                if events.send(CaptureEvent::Finished(result)).is_ok() {
                    ctx.request_repaint();
                }
            })
            .context("无法启动旁白录制任务")?;
        Ok(Self {
            stop: Some(stop),
            events: Some(incoming),
            started_at: None,
        })
    }

    pub(super) fn started_at(&self) -> Option<Instant> {
        self.started_at
    }

    pub(super) fn request_stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }

    pub(super) fn poll(&mut self) -> Option<Result<RecordedNarration>> {
        loop {
            let finished = match self.events.as_ref()?.try_recv() {
                Ok(CaptureEvent::Started(at)) => {
                    self.started_at = Some(at);
                    continue;
                }
                Ok(CaptureEvent::Finished(result)) => result,
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => Err(anyhow!("旁白录制任务已中断")),
            };
            self.events = None;
            return Some(finished);
        }
    }
}

pub(super) struct RecordedNarration {
    output: NamedTempFile,
    pub(super) duration_us: u64,
}

impl RecordedNarration {
    pub(super) fn persist(self) -> Result<PathBuf> {
        let path = self.output.path().with_extension("wav");
        self.output
            .persist_noclobber(&path)
            .with_context(|| format!("无法保存旁白录音（{}）", path.display()))?;
        Ok(path)
    }
}

fn record(
    directory: &Path,
    stop_requested: &Receiver<()>,
    notify_started: impl FnOnce() -> Result<()>,
) -> Result<RecordedNarration> {
    check_start_requested(stop_requested)?;
    fs::create_dir_all(directory).context("无法创建旁白目录")?;
    let mic = Microphone::open_default()?;
    let mut wav = WavRecording::new(directory, mic.channels, mic.sample_rate)?;
    check_start_requested(stop_requested)?;
    mic.start()?;
    notify_started()?;

    loop {
        match stop_requested.recv_timeout(Duration::from_millis(10)) {
            Ok(()) => break,
            Err(RecvTimeoutError::Timeout) => mic.receive(|packet| wav.write_packet(packet))?,
            Err(RecvTimeoutError::Disconnected) => bail!("已取消旁白录制"),
        }
    }
    mic.finish(|packet| wav.write_packet(packet))?;
    wav.finish()
}

fn check_start_requested(stop_requested: &Receiver<()>) -> Result<()> {
    ensure!(
        matches!(stop_requested.try_recv(), Err(TryRecvError::Empty)),
        "已取消旁白录制"
    );
    Ok(())
}

struct WavRecording {
    output: BufWriter<NamedTempFile>,
    channels: u16,
    sample_rate: u32,
    data_size: u32,
}

impl WavRecording {
    fn new(directory: &Path, channels: u16, sample_rate: u32) -> Result<Self> {
        let staged = Builder::new()
            .prefix("旁白-")
            .suffix(".part")
            .tempfile_in(directory)
            .context("无法创建旁白临时文件")?;
        let mut output = BufWriter::new(staged);
        output
            .write_all(&[0; WAV_HEADER_SIZE])
            .context("无法写入旁白文件")?;
        Ok(Self {
            output,
            channels,
            sample_rate,
            data_size: 0,
        })
    }

    fn write_packet(&mut self, pcm: &[u8]) -> Result<()> {
        let data_size = u32::try_from(pcm.len())
            .ok()
            .and_then(|size| self.data_size.checked_add(size))
            .filter(|size| *size <= u32::MAX - RIFF_OVERHEAD)
            .context("旁白录音已超过 WAV 文件大小上限")?;
        self.output.write_all(pcm).context("无法写入旁白音频")?;
        self.data_size = data_size;
        Ok(())
    }

    fn finish(mut self) -> Result<RecordedNarration> {
        ensure!(self.data_size > 0, "没有录到声音，请重新录制");
        let block_align = self.channels * BYTES_PER_SAMPLE;
        let bytes_per_second = self.sample_rate * u32::from(block_align);
        ensure!(
            self.data_size.is_multiple_of(u32::from(block_align)),
            "旁白音频帧不完整"
        );

        self.output.rewind().context("无法定位旁白文件头")?;
        self.write_header(block_align, bytes_per_second)
            .context("无法封装旁白文件")?;
        let output = self
            .output
            .into_inner()
            .map_err(|error| error.into_error())
            .context("无法写完旁白文件")?;
        output.as_file().sync_all().context("无法同步旁白文件")?;
        Ok(RecordedNarration {
            output,
            duration_us: u64::from(self.data_size) * 1_000_000 / u64::from(bytes_per_second),
        })
    }

    fn write_header(&mut self, block_align: u16, bytes_per_second: u32) -> io::Result<()> {
        self.output.write_all(b"RIFF")?;
        self.output
            .write_all(&(self.data_size + RIFF_OVERHEAD).to_le_bytes())?;
        self.output.write_all(b"WAVEfmt ")?;
        self.output.write_all(&16_u32.to_le_bytes())?;
        self.output.write_all(&1_u16.to_le_bytes())?;
        self.output.write_all(&self.channels.to_le_bytes())?;
        self.output.write_all(&self.sample_rate.to_le_bytes())?;
        self.output.write_all(&bytes_per_second.to_le_bytes())?;
        self.output.write_all(&block_align.to_le_bytes())?;
        self.output
            .write_all(&(BYTES_PER_SAMPLE * 8).to_le_bytes())?;
        self.output.write_all(b"data")?;
        self.output.write_all(&self.data_size.to_le_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narration_wav_preserves_pcm_and_publishes_a_valid_header() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut wav = WavRecording::new(directory.path(), 2, 48_000)?;
        let pcm = [1, 2, 3, 4, 5, 6, 7, 8];
        wav.write_packet(&pcm[..4])?;
        wav.write_packet(&pcm[4..])?;

        let recorded = wav.finish()?;
        assert_eq!(recorded.duration_us, 41);
        let path = recorded.persist()?;
        assert_eq!(path.extension().and_then(|ext| ext.to_str()), Some("wav"));
        assert_eq!(fs::read_dir(directory.path())?.count(), 1);

        let bytes = fs::read(path)?;
        assert_eq!(
            &bytes[..WAV_HEADER_SIZE],
            b"RIFF\x2c\x00\x00\x00WAVEfmt \x10\x00\x00\x00\x01\x00\x02\x00\x80\xbb\x00\x00\x00\xee\x02\x00\x04\x00\x10\x00data\x08\x00\x00\x00"
        );
        assert_eq!(&bytes[WAV_HEADER_SIZE..], pcm.as_slice());
        Ok(())
    }
}
