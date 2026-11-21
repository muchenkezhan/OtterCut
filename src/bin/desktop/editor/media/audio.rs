use std::fmt::Write;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use eframe::egui;

use super::{
    Decoder, MediaSlice, PlaybackClock, PreviewError, append_clip_volume_filter, hidden_command,
    local_media, seconds,
};

pub(super) struct AudioDecode {
    pub(super) executable: PathBuf,
    pub(super) sources: Vec<MediaSlice>,
    pub(super) duration_us: u64,
    pub(super) shared: Arc<PlaybackClock>,
    pub(super) decoder: Arc<Decoder>,
    pub(super) errors: mpsc::Sender<PreviewError>,
    pub(super) ctx: egui::Context,
}

impl AudioDecode {
    pub(super) fn run(self) {
        if let Err(error) = self.play()
            && !self.decoder.cancelled()
        {
            let _ = self
                .errors
                .send(PreviewError::Audio(format!("预览音频不可用（{error}）")));
        }
        self.shared.audio_ready.store(true, Ordering::Release);
        self.ctx.request_repaint();
    }

    fn play(&self) -> Result<()> {
        let device = cpal::default_host()
            .default_output_device()
            .context("未找到默认音频输出设备")?;
        let cfg = device
            .default_output_config()
            .context("无法读取默认音频输出格式")?;
        let config = cfg.config();
        let (packets, receiver) = mpsc::sync_channel(18);
        let mut player = AudioOutput {
            receiver,
            clock: self.shared.clone(),
            packet: Vec::new(),
            packet_start: 0,
            next_frame: None,
            config: config.clone(),
        };
        let errors = self.errors.clone();
        let ctx = self.ctx.clone();
        let report = move |error| {
            let _ = errors.send(PreviewError::Audio(format!("音频输出中断（{error}）")));
            ctx.request_repaint();
        };
        macro_rules! output {
            ($sample:ty) => {
                device.build_output_stream(
                    &config,
                    move |samples: &mut [$sample], info| player.fill(samples, info),
                    report,
                    None,
                )
            };
        }
        let stream = match cfg.sample_format() {
            SampleFormat::F32 => output!(f32),
            SampleFormat::F64 => output!(f64),
            SampleFormat::I16 => output!(i16),
            SampleFormat::I32 => output!(i32),
            SampleFormat::U16 => output!(u16),
            other => bail!("当前音频输出格式不受支持（{other}）"),
        }
        .context("无法打开音频输出")?;
        stream.play().context("无法启动音频输出")?;

        let command = mix_command(
            &self.executable,
            &self.sources,
            self.duration_us,
            config.sample_rate,
            config.channels,
        )?;
        let audio_packet_size =
            (config.sample_rate / 50) as usize * usize::from(config.channels) * 4;
        self.decoder.decode(command, |mut stdout| {
            let mut bytes = vec![0; audio_packet_size];
            while !self.decoder.cancelled() {
                let mut len = 0;
                while len < bytes.len() {
                    let read = stdout.read(&mut bytes[len..])?;
                    if read == 0 {
                        break;
                    }
                    len += read;
                }
                if len == 0 {
                    break;
                }
                let packet = bytes[..len]
                    .chunks_exact(4)
                    .map(|sample| f32::from_le_bytes(sample.try_into().unwrap()))
                    .collect();
                let mut packet = packet;
                loop {
                    if self.decoder.cancelled() {
                        return Ok(());
                    }
                    match packets.try_send(packet) {
                        Ok(()) => break,
                        Err(mpsc::TrySendError::Full(returned)) => {
                            packet = returned;
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(mpsc::TrySendError::Disconnected(_)) => bail!("音频输出已关闭"),
                    }
                }
                self.shared.audio_ready.store(true, Ordering::Release);
            }
            Ok(())
        })?;
        drop(packets);
        self.shared.audio_ready.store(true, Ordering::Release);
        self.ctx.request_repaint();
        // Keep the device alive while the last decoded packets leave its buffer.
        while !self.decoder.cancelled() {
            if self.shared.epoch.get().is_some_and(|epoch| {
                epoch.elapsed()
                    >= Duration::from_micros(self.duration_us) + Duration::from_millis(100)
            }) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        drop(stream);
        Ok(())
    }
}

fn mix_command(
    executable: &Path,
    sources: &[MediaSlice],
    duration: u64,
    rate: u32,
    channels: u16,
) -> Result<Command> {
    let mut command = hidden_command(executable);
    command.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
    let mut filters = String::new();
    let interval = seconds(duration);
    for (index, source) in sources.iter().enumerate() {
        let path = local_media(&source.path)?;
        let start = seconds(source.range.start);
        let length = seconds(source.range.end.saturating_sub(source.range.start).max(1));
        command
            .args([
                "-protocol_whitelist",
                "file,pipe",
                "-threads",
                "2",
                "-ss",
                &start,
                "-t",
                &length,
                "-i",
            ])
            .arg(path);
        write!(
            filters,
            "[{index}:a:0]asetpts=PTS-STARTPTS,aresample={rate}:async=1:first_pts=0,aformat=sample_fmts=fltp:channel_layouts=stereo,apad,atrim=duration={interval}"
        )?;
        append_clip_volume_filter(&mut filters, source.volume_db_tenths);
        write!(filters, "[a{index}];")?;
    }
    for index in 0..sources.len() {
        write!(filters, "[a{index}]")?;
    }
    if sources.len() == 1 {
        filters.push_str("anull[mix]");
    } else {
        write!(
            filters,
            "amix=inputs={}:normalize=0:dropout_transition=0,alimiter=limit=0.95:level=false:latency=true[mix]",
            sources.len()
        )?;
    }
    command
        .args([
            "-filter_complex_threads",
            "2",
            "-filter_complex",
            &filters,
            "-map",
            "[mix]",
            "-t",
            &interval,
            "-ac",
            &channels.to_string(),
            "-ar",
            &rate.to_string(),
            "-c:a",
            "pcm_f32le",
            "-f",
            "f32le",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    Ok(command)
}

struct AudioOutput {
    receiver: Receiver<Vec<f32>>,
    clock: Arc<PlaybackClock>,
    packet: Vec<f32>,
    packet_start: usize,
    next_frame: Option<i64>,
    config: StreamConfig,
}

impl AudioOutput {
    fn fill<T: SizedSample + FromSample<f32>>(
        &mut self,
        samples: &mut [T],
        info: &cpal::OutputCallbackInfo,
    ) {
        samples.fill(T::EQUILIBRIUM);
        let Some(epoch) = self.clock.epoch.get().copied() else {
            return;
        };
        let channels = usize::from(self.config.channels);
        let frame = self.next_frame.unwrap_or_else(|| {
            let timestamp = info.timestamp();
            let delay = timestamp
                .playback
                .duration_since(&timestamp.callback)
                .unwrap_or_default();
            let device_time = Instant::now() + delay;
            let time = if device_time >= epoch {
                (device_time - epoch).as_secs_f64()
            } else {
                -(epoch - device_time).as_secs_f64()
            };
            (time * f64::from(self.config.sample_rate)).round() as i64
        });
        let frames = samples.len() / channels;
        self.next_frame = Some(frame.saturating_add(frames as i64));
        let first = frame.unsigned_abs().min(frames as u64) as usize;
        let mut written = if frame < 0 { first * channels } else { 0 };
        let mut cursor = frame.max(0) as usize * channels;
        while written < samples.len() {
            while cursor >= self.packet_start + self.packet.len() {
                let Ok(packet) = self.receiver.try_recv() else {
                    return;
                };
                self.packet_start += self.packet.len();
                self.packet = packet;
            }
            let offset = cursor - self.packet_start;
            let count = (self.packet.len() - offset).min(samples.len() - written);
            for (output, &source) in samples[written..written + count]
                .iter_mut()
                .zip(&self.packet[offset..offset + count])
            {
                *output = T::from_sample(source);
            }
            cursor += count;
            written += count;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::media::MediaTools;

    #[test]
    #[ignore = "requires local FFmpeg and FFprobe"]
    fn preview_mix_contains_simultaneous_sources() {
        let tools = MediaTools::discover().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let mut sources = [440, 880].map(|frequency| {
            let path = temp.path().join(format!("{frequency}.wav"));
            let status = hidden_command(&tools.ffmpeg)
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("sine=frequency={frequency}:sample_rate=48000:duration=1"),
                    "-c:a",
                    "pcm_s16le",
                ])
                .arg(&path)
                .status()
                .unwrap();
            assert!(status.success());
            let info = tools.probe(&path).unwrap();
            assert!(info.has_audio);
            assert!(!info.has_video);
            MediaSlice {
                path,
                range: 200_000..700_000,
                volume_db_tenths: 0,
            }
        });
        let mix = |sources: &[MediaSlice]| {
            let output = mix_command(&tools.ffmpeg, sources, 500_000, 48_000, 1)
                .unwrap()
                .output()
                .unwrap();
            assert!(output.status.success());
            let samples: Vec<_> = output
                .stdout
                .chunks_exact(4)
                .map(|bytes| f64::from(f32::from_le_bytes(bytes.try_into().unwrap())))
                .collect();
            assert_eq!(samples.len(), 24_000);
            samples
        };
        let strength = |samples: &[f64], frequency: f64| {
            let (sin, cos) =
                samples
                    .iter()
                    .enumerate()
                    .fold((0.0, 0.0), |(sin, cos), (i, sample)| {
                        let phase = std::f64::consts::TAU * frequency * i as f64 / 48_000.0;
                        (sin + sample * phase.sin(), cos + sample * phase.cos())
                    });
            sin.hypot(cos) * 2.0 / samples.len() as f64
        };
        let original = mix(&sources);
        assert!(strength(&original, 440.0) > 0.055);
        assert!(strength(&original, 880.0) > 0.055);
        assert!(strength(&original, 1760.0) < 0.025);

        sources[0].volume_db_tenths = -120;
        let adjusted = mix(&sources);
        let ratio = strength(&adjusted, 440.0) / strength(&original, 440.0);
        assert!((ratio - 0.251).abs() < 0.02, "adjusted tone ratio {ratio}");
        let ratio = strength(&adjusted, 880.0) / strength(&original, 880.0);
        assert!((ratio - 1.0).abs() < 0.02, "unchanged tone ratio {ratio}");
    }
}
