use std::fmt::Write;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use eframe::egui;

use super::{
    Decoder, MediaSlice, PlaybackClock, PreviewError, append_clip_audio_filters, hidden_command,
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
        let source_ids: Vec<_> = self.sources.iter().map(|source| source.id).collect();
        let decoded_channels = source_ids.len().saturating_mul(2).max(2);
        let (packets, receiver) = mpsc::sync_channel(18);
        let mut player = AudioOutput {
            receiver,
            clock: self.shared.clone(),
            packet: Vec::new(),
            packet_start: 0,
            next_frame: None,
            source_ids,
            decoded_channels,
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
            decoded_channels as u16,
        )?;
        let audio_packet_size =
            (config.sample_rate / 50) as usize * decoded_channels * 4;
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

enum ScrubCommand {
    Play {
        executable: PathBuf,
        sources: Vec<MediaSlice>,
        duration_us: u64,
        timeline_start: u64,
        position: u64,
        sample_rate: u32,
        channels: u16,
    },
    Prepare {
        executable: PathBuf,
        sources: Vec<MediaSlice>,
        duration_us: u64,
        timeline_start: u64,
        position: u64,
        sample_rate: u32,
        channels: u16,
    },
    Stop,
}

pub(super) struct ScrubAudio {
    requests: mpsc::Sender<ScrubCommand>,
    state: Arc<Mutex<ScrubState>>,
    _stream: cpal::Stream,
}

struct ScrubState {
    samples: Vec<f32>,
    channels: usize,
}

struct ScrubCache {
    key: u64,
    samples: Vec<f32>,
    channels: u16,
    sample_rate: u32,
    timeline_start: u64,
    duration_us: u64,
}

impl ScrubAudio {
    pub(super) fn new() -> Result<Self> {
        let device = cpal::default_host()
            .default_output_device()
            .context("未找到默认音频输出设备")?;
        let cfg = device
            .default_output_config()
            .context("无法读取默认音频输出格式")?;
        let config = cfg.config();
        let state = Arc::new(Mutex::new(ScrubState {
            samples: Vec::new(),
            channels: 2,
        }));
        let output_state = Arc::clone(&state);
        let report = |_error| {};
        macro_rules! output {
            ($sample:ty) => {
                device.build_output_stream(
                    &config,
                    move |samples: &mut [$sample], _info| {
                        fill_scrub_samples(samples, &output_state, config.channels as usize)
                    },
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
        .context("无法打开拖动音频输出")?;
        stream.play().context("无法启动拖动音频输出")?;

        let (requests, incoming) = mpsc::channel();
        let worker_state = Arc::clone(&state);
        thread::Builder::new()
            .name("scrub-audio".into())
            .spawn(move || scrub_worker(incoming, worker_state))
            .context("无法启动拖动音频线程")?;
        Ok(Self {
            requests,
            state,
            _stream: stream,
        })
    }

    pub(super) fn play(
        &self,
        executable: &Path,
        sources: Vec<MediaSlice>,
        duration_us: u64,
        timeline_start: u64,
        position: u64,
    ) {
        let sample_rate = 48_000;
        let _ = self.requests.send(ScrubCommand::Play {
            executable: executable.to_owned(),
            sources,
            duration_us,
            timeline_start,
            position,
            sample_rate,
            channels: 2,
        });
    }

    pub(super) fn stop(&self) {
        let _ = self.requests.send(ScrubCommand::Stop);
        if let Ok(mut state) = self.state.lock() {
            state.samples.clear();
        }
    }

    pub(super) fn prepare(
        &self,
        executable: &Path,
        sources: Vec<MediaSlice>,
        duration_us: u64,
        timeline_start: u64,
        position: u64,
    ) {
        let _ = self.requests.send(ScrubCommand::Prepare {
            executable: executable.to_owned(),
            sources,
            duration_us,
            timeline_start,
            position,
            sample_rate: 48_000,
            channels: 2,
        });
    }
}

fn scrub_worker(receiver: Receiver<ScrubCommand>, state: Arc<Mutex<ScrubState>>) {
    let mut pending = None;
    let mut cache = None;
    loop {
        let command = pending.take().or_else(|| receiver.recv().ok());
        let Some(command) = command else { return };
        match command {
            ScrubCommand::Stop => {
                if let Ok(mut state) = state.lock() {
                    state.samples.clear();
                }
            }
            ScrubCommand::Play {
                executable,
                sources,
                duration_us,
                timeline_start,
                position,
                sample_rate,
                channels,
            } => {
                pending = play_scrub_audio_request(
                    &receiver,
                    &state,
                    &mut cache,
                    &executable,
                    &sources,
                    duration_us,
                    timeline_start,
                    position,
                    sample_rate,
                    channels,
                );
            }
            ScrubCommand::Prepare {
                executable,
                sources,
                duration_us,
                timeline_start,
                position,
                sample_rate,
                channels,
            } => {
                pending = prepare_scrub_audio_request(
                    &receiver,
                    &state,
                    &mut cache,
                    &executable,
                    &sources,
                    duration_us,
                    timeline_start,
                    position,
                    sample_rate,
                    channels,
                );
            }
        }
    }
}

fn play_scrub_audio_request(
    receiver: &Receiver<ScrubCommand>,
    state: &Arc<Mutex<ScrubState>>,
    cache: &mut Option<ScrubCache>,
    executable: &Path,
    sources: &[MediaSlice],
    duration_us: u64,
    timeline_start: u64,
    position: u64,
    sample_rate: u32,
    channels: u16,
) -> Option<ScrubCommand> {
    let key = scrub_cache_key(sources, duration_us, sample_rate, channels);
    if cache.as_ref().is_some_and(|cached| {
        cached.key == key
            && position >= cached.timeline_start
            && position < cached.timeline_start.saturating_add(cached.duration_us)
    }) {
        if let Some(cached) = cache.as_ref() {
            fill_cached_scrub_audio(state, cached, position);
        }
        return take_latest_scrub_command(receiver, None);
    }
    decode_scrub_audio_buffer(
        receiver,
        state,
        cache,
        executable,
        sources,
        duration_us,
        timeline_start,
        position,
        sample_rate,
        channels,
        key,
        true,
    )
}

fn decode_scrub_audio_buffer(
    receiver: &Receiver<ScrubCommand>,
    state: &Arc<Mutex<ScrubState>>,
    cache: &mut Option<ScrubCache>,
    executable: &Path,
    sources: &[MediaSlice],
    duration_us: u64,
    timeline_start: u64,
    position: u64,
    sample_rate: u32,
    channels: u16,
    key: u64,
    emit: bool,
) -> Option<ScrubCommand> {
    let mut command = mix_command(executable, sources, duration_us, sample_rate, channels).ok()?;
    let mut child = command.stdout(Stdio::piped()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let mut decoded = Vec::new();
    let mut bytes = vec![0_u8; (sample_rate / 50) as usize * usize::from(channels) * 4];
    let mut pending = None;
    loop {
        while let Ok(command) = receiver.try_recv() {
            if matches!(command, ScrubCommand::Stop) {
                let _ = child.kill();
                let _ = child.wait();
                return Some(command);
            }
            pending = Some(command);
        }
        let mut read = 0;
        while read < bytes.len() {
            match stdout.read(&mut bytes[read..]) {
                Ok(0) => {
                    let samples = decoded;
                    let cached = ScrubCache {
                        key,
                        samples,
                        channels,
                        sample_rate,
                        timeline_start,
                        duration_us,
                    };
                    if emit {
                        fill_cached_scrub_audio(state, &cached, position);
                    }
                    *cache = Some(cached);
                    return pending;
                }
                Ok(count) => read += count,
                Err(_) => return pending,
            }
        }
        decoded.extend(bytes[..read]
            .chunks_exact(4)
            .map(|sample| f32::from_le_bytes(sample.try_into().unwrap())));
    }
}

fn prepare_scrub_audio_request(
    receiver: &Receiver<ScrubCommand>,
    state: &Arc<Mutex<ScrubState>>,
    cache: &mut Option<ScrubCache>,
    executable: &Path,
    sources: &[MediaSlice],
    duration_us: u64,
    timeline_start: u64,
    position: u64,
    sample_rate: u32,
    channels: u16,
) -> Option<ScrubCommand> {
    let key = scrub_cache_key(sources, duration_us, sample_rate, channels);
    if cache.as_ref().is_some_and(|cached| {
        cached.key == key
            && cached.duration_us >= duration_us
            && position >= cached.timeline_start
            && position < cached.timeline_start.saturating_add(cached.duration_us)
    }) {
        return take_latest_scrub_command(receiver, None);
    }
    decode_scrub_audio_buffer(
        receiver,
        state,
        cache,
        executable,
        sources,
        duration_us,
        timeline_start,
        timeline_start,
        sample_rate,
        channels,
        key,
        false,
    )
}

fn take_latest_scrub_command(
    receiver: &Receiver<ScrubCommand>,
    initial: Option<ScrubCommand>,
) -> Option<ScrubCommand> {
    let mut latest = initial;
    while let Ok(command) = receiver.try_recv() {
        latest = Some(command);
    }
    latest
}

fn scrub_cache_key(
    sources: &[MediaSlice],
    _duration_us: u64,
    sample_rate: u32,
    channels: u16,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    sample_rate.hash(&mut hasher);
    channels.hash(&mut hasher);
    for source in sources {
        source.id.hash(&mut hasher);
        source.path.hash(&mut hasher);
        source.timeline_start.hash(&mut hasher);
        source.timeline_duration_us.hash(&mut hasher);
        source.volume_db_tenths.hash(&mut hasher);
        source.fade_in_us.hash(&mut hasher);
        source.fade_out_us.hash(&mut hasher);
        source.speed_milli.hash(&mut hasher);
        source.reverse.hash(&mut hasher);
    }
    hasher.finish()
}

fn fill_cached_scrub_audio(
    state: &Arc<Mutex<ScrubState>>,
    cache: &ScrubCache,
    position: u64,
) {
    let channels = usize::from(cache.channels.max(1));
    let offset = position
        .saturating_sub(cache.timeline_start)
        .saturating_mul(u64::from(cache.sample_rate))
        .checked_div(1_000_000)
        .unwrap_or(u64::MAX)
        .saturating_mul(channels as u64) as usize;
    let window = (cache.sample_rate as usize * 120 / 1000).saturating_mul(channels);
    let start = offset.min(cache.samples.len());
    let end = start.saturating_add(window).min(cache.samples.len());
    if let Ok(mut state) = state.lock() {
        state.channels = channels;
        state.samples.clear();
        state.samples.extend_from_slice(&cache.samples[start..end]);
    }
}

fn fill_scrub_samples<T: SizedSample + FromSample<f32>>(
    samples: &mut [T],
    state: &Arc<Mutex<ScrubState>>,
    output_channels: usize,
) {
    samples.fill(T::EQUILIBRIUM);
    let Ok(mut state) = state.try_lock() else {
        return;
    };
    let source_channels = state.channels.max(1);
    for frame in samples.chunks_mut(output_channels.max(1)) {
        let left = state.samples.first().copied().unwrap_or_default();
        let right = if source_channels > 1 {
            state.samples.get(1).copied().unwrap_or(left)
        } else {
            left
        };
        let remove = source_channels.min(state.samples.len());
        state.samples.drain(..remove);
        for (index, sample) in frame.iter_mut().enumerate() {
            *sample = T::from_sample(if index == 0 { left } else { right });
        }
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
        append_clip_audio_filters(
            &mut filters,
            source.volume_db_tenths,
            source.fade_in_us,
            source.fade_out_us,
            duration,
            source.speed_milli,
            source.reverse,
        );
        write!(filters, "[a{index}];")?;
    }
    for index in 0..sources.len() {
        write!(filters, "[a{index}]")?;
    }
    if sources.len() == 1 {
        filters.push_str("anull[mix]");
    } else {
        // Keep each source in its own stereo pair. The CPAL callback mixes those
        // pairs and applies the live gain atomically, so dragging a clip's volume
        // line never has to restart FFmpeg.
        write!(filters, "amerge=inputs={}[mix]", sources.len())?;
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
    source_ids: Vec<u64>,
    decoded_channels: usize,
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
        let output_channels = usize::from(self.config.channels);
        let decoded_channels = self.decoded_channels;
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
        let frames = samples.len() / output_channels;
        self.next_frame = Some(frame.saturating_add(frames as i64));
        let first = frame.unsigned_abs().min(frames as u64) as usize;
        let mut written = if frame < 0 {
            first * output_channels
        } else {
            0
        };
        let mut cursor = frame.max(0) as usize * decoded_channels;
        let gain_db_tenths = self
            .clock
            .live_gain_db_tenths
            .load(Ordering::Acquire);
        let gain = 10.0_f32.powf(f32::from(gain_db_tenths) / 200.0);
        while written < samples.len() {
            while cursor.saturating_add(decoded_channels) > self.packet_start + self.packet.len() {
                let Ok(packet) = self.receiver.try_recv() else {
                    return;
                };
                self.packet_start += self.packet.len();
                self.packet = packet;
            }
            let offset = cursor - self.packet_start;
            let mut left = 0.0_f32;
            let mut right = 0.0_f32;
            let live_source = self
                .clock
                .live_volume_source
                .load(Ordering::Acquire);
            for (source_index, source_id) in self.source_ids.iter().enumerate() {
                let source_gain = if *source_id == live_source { gain } else { 1.0 };
                left += self.packet[offset + source_index * 2] * source_gain;
                right += self.packet[offset + source_index * 2 + 1] * source_gain;
            }
            let mixed = [left, right];
            for channel in 0..output_channels {
                let value = if output_channels == 1 {
                    (left + right) * 0.5
                } else {
                    mixed[channel.min(1)]
                };
                samples[written + channel] = T::from_sample(value.clamp(-1.0, 1.0));
            }
            cursor += decoded_channels;
            written += output_channels;
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
                id: frequency as u64,
                path,
                range: 200_000..700_000,
                timeline_start: 0,
                timeline_duration_us: 500_000,
                volume_db_tenths: 0,
            fade_in_us: 0,
            fade_out_us: 0,
            speed_milli: 1000,
            reverse: false,
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

