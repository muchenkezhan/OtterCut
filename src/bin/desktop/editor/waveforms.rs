use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;

use anyhow::Result;
use eframe::egui;

use super::media::{Decoder, MediaTools, decoding_command, local_media};

const CACHE_LIMIT: usize = 24;
const QUEUE_LIMIT: usize = 6;
const SAMPLE_RATE: u64 = 8_000;
const MICROS_PER_SECOND: u64 = 1_000_000;
const MAX_BUCKETS: u64 = 65_536;
const MIN_BUCKET_FRAMES: u64 = SAMPLE_RATE / 200;

type Peak = [i16; 2];
const EMPTY: Peak = [i16::MAX, i16::MIN];

pub(super) struct Waveform {
    samples: u64,
    bucket_frames: u64,
    leaf_count: usize,
    peaks: Vec<Peak>,
}

impl Waveform {
    fn new(samples: u64, bucket_frames: u64, buckets: Vec<Peak>) -> Self {
        let leaf_count = buckets.len().next_power_of_two();
        let mut peaks = vec![EMPTY; leaf_count * 2];
        peaks[leaf_count..leaf_count + buckets.len()].copy_from_slice(&buckets);
        for index in (1..leaf_count).rev() {
            peaks[index] = merge(peaks[index * 2], peaks[index * 2 + 1]);
        }
        Self {
            samples,
            bucket_frames,
            leaf_count,
            peaks,
        }
    }

    /// Signed minimum and maximum over the source interval, at envelope resolution.
    pub(super) fn peaks(&self, from_us: u64, to_us: u64) -> [f32; 2] {
        let first =
            (u128::from(from_us) * u128::from(SAMPLE_RATE) / u128::from(MICROS_PER_SECOND)) as u64;
        let last = frames_ceil(to_us);
        if from_us >= to_us || first >= self.samples {
            return [0.0; 2];
        }

        // Query complete buckets in logarithmic time when a long clip is zoomed out.
        let mut left = self.leaf_count + (first / self.bucket_frames) as usize;
        let mut right =
            self.leaf_count + last.min(self.samples).div_ceil(self.bucket_frames) as usize;
        let mut peak = if last > self.samples { [0, 0] } else { EMPTY };
        while left < right {
            if !left.is_multiple_of(2) {
                peak = merge(peak, self.peaks[left]);
                left += 1;
            }
            if !right.is_multiple_of(2) {
                right -= 1;
                peak = merge(peak, self.peaks[right]);
            }
            left /= 2;
            right /= 2;
        }
        if peak == EMPTY {
            [0.0; 2]
        } else {
            peak.map(|sample| f32::from(sample) / 32_768.0)
        }
    }
}

pub(super) struct Waveforms {
    worker: Option<Worker>,
    pending: HashSet<PathBuf>,
    cache: HashMap<PathBuf, Cached>,
    order: u64,
}

struct Cached {
    duration: u64,
    last_used: u64,
    waveform: Option<Waveform>,
}

type Request = (PathBuf, u64);
type Update = (PathBuf, u64, Option<Waveform>);

struct Worker {
    requests: SyncSender<Request>,
    updates: Receiver<Update>,
    decoder: Arc<Decoder>,
}

impl Worker {
    fn start(tools: MediaTools, ctx: egui::Context) -> Option<Self> {
        let (requests, incoming) = mpsc::sync_channel::<Request>(QUEUE_LIMIT);
        let (outgoing, updates) = mpsc::channel();
        let decoder = Arc::new(Decoder::default());
        let background = Arc::clone(&decoder);
        thread::Builder::new()
            .name(format!("{} waveforms", takeseam::PRODUCT_NAME))
            .spawn(move || {
                while let Ok((path, duration)) = incoming.recv() {
                    if background.cancelled() {
                        break;
                    }
                    let waveform = decode(&tools, &path, duration, &background).ok();
                    if outgoing.send((path, duration, waveform)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .ok()?;
        Some(Self {
            requests,
            updates,
            decoder,
        })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.decoder.cancel();
    }
}

impl Waveforms {
    pub(super) fn new(tools: Option<MediaTools>, ctx: &egui::Context) -> Self {
        Self {
            worker: tools.and_then(|tools| Worker::start(tools, ctx.clone())),
            pending: HashSet::new(),
            cache: HashMap::new(),
            order: 0,
        }
    }

    pub(super) fn envelope(&mut self, path: &Path, duration: u64) -> Option<&Waveform> {
        let worker = self.worker.as_ref()?;
        for (path, duration, waveform) in worker.updates.try_iter() {
            self.pending.remove(&path);
            if !self.cache.contains_key(&path)
                && self.cache.len() >= CACHE_LIMIT
                && let Some(oldest) = self
                    .cache
                    .iter()
                    .min_by_key(|(_, cached)| cached.last_used)
                    .map(|(key, _)| key.clone())
            {
                self.cache.remove(&oldest);
            }
            self.order += 1;
            self.cache.insert(
                path,
                Cached {
                    duration,
                    last_used: self.order,
                    waveform,
                },
            );
        }

        if self
            .cache
            .get(path)
            .is_none_or(|entry| entry.duration != duration)
            && duration != 0
            && !self.pending.contains(path)
            && worker
                .requests
                .try_send((path.to_owned(), duration))
                .is_ok()
        {
            self.pending.insert(path.to_owned());
        }
        let cached = self
            .cache
            .get_mut(path)
            .filter(|entry| entry.duration == duration)?;
        self.order += 1;
        cached.last_used = self.order;
        cached.waveform.as_ref()
    }
}

fn merge(a: Peak, b: Peak) -> Peak {
    [a[0].min(b[0]), a[1].max(b[1])]
}

fn frames_ceil(time_us: u64) -> u64 {
    (u128::from(time_us) * u128::from(SAMPLE_RATE)).div_ceil(u128::from(MICROS_PER_SECOND)) as u64
}

fn decode(tools: &MediaTools, path: &Path, duration: u64, decoder: &Decoder) -> Result<Waveform> {
    let path = local_media(path)?;
    let limit = format!(
        "{}.{:06}",
        duration / MICROS_PER_SECOND,
        duration % MICROS_PER_SECOND
    );
    let rate = SAMPLE_RATE.to_string();
    let resample = format!("aresample={rate}:async=1:first_pts=0");
    let mut command = decoding_command(&tools.ffmpeg, &path, 0);
    command.args([
        "-t",
        &limit,
        "-map",
        "0:a:0",
        "-vn",
        "-sn",
        "-dn",
        "-filter_threads",
        "1",
        "-af",
        &resample,
        "-ac",
        "2",
        "-ar",
        &rate,
        "-c:a",
        "pcm_s16le",
        "-f",
        "s16le",
        "pipe:1",
    ]);
    decoder.decode(command, |stdout| read_peaks(stdout, frames_ceil(duration)))
}

fn read_peaks(mut input: impl Read, max_frames: u64) -> Result<Waveform> {
    let bucket_frames = max_frames.div_ceil(MAX_BUCKETS).max(MIN_BUCKET_FRAMES);
    let mut buckets = Vec::with_capacity(max_frames.div_ceil(bucket_frames) as usize);
    let mut bytes = [0; 16_384];
    let mut remaining = 0;
    let mut samples = 0;
    let mut peak = EMPTY;
    let mut in_bucket = 0;
    loop {
        let read = input.read(&mut bytes[remaining..])?;
        if read == 0 {
            break;
        }
        let data = read + remaining;
        let mut frames = bytes[..data].chunks_exact(4);
        let count = max_frames.saturating_sub(samples).min((data / 4) as u64) as usize;
        for frame in frames.by_ref().take(count) {
            let left = i16::from_le_bytes([frame[0], frame[1]]);
            let right = i16::from_le_bytes([frame[2], frame[3]]);
            peak = merge(peak, [left.min(right), left.max(right)]);
            samples += 1;
            in_bucket += 1;
            if in_bucket == bucket_frames {
                buckets.push(peak);
                peak = EMPTY;
                in_bucket = 0;
            }
        }
        remaining = frames.remainder().len();
        bytes.copy_within(data - remaining..data, 0);
    }
    if in_bucket > 0 {
        buckets.push(peak);
    }
    Ok(Waveform::new(samples, bucket_frames, buckets))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_the_source_interval_without_losing_the_other_stereo_channel() {
        let mut data = Vec::new();
        for (left, right) in [(0_i16, 0_i16), (8_192, -16_384), (16_384, -4_096)] {
            for _ in 0..MIN_BUCKET_FRAMES {
                data.extend(left.to_le_bytes());
                data.extend(right.to_le_bytes());
            }
        }
        let waveform = read_peaks(data.as_slice(), MIN_BUCKET_FRAMES * 3).unwrap();
        assert_eq!(waveform.peaks(0, 5_000), [0.0, 0.0]);
        assert_eq!(waveform.peaks(5_000, 10_000), [-0.5, 0.25]);
        assert_eq!(waveform.peaks(9_999, 14_999), [-0.5, 0.5]);
        assert_eq!(waveform.peaks(10_000, 15_000), [-0.125, 0.5]);
        assert_eq!(waveform.peaks(15_000, 18_000), [0.0, 0.0]);
    }

    #[test]
    #[ignore = "requires local FFmpeg and FFprobe"]
    fn decodes_real_audio_and_preserves_silence() {
        let tools = MediaTools::discover().unwrap();
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("voice.wav");
        let status = std::process::Command::new(&tools.ffmpeg)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc='if(lt(t,0.2),0,0.65*sin(2*PI*440*t))':s=8000:d=0.5",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        let waveform = decode(&tools, &path, 500_000, &Decoder::default()).unwrap();
        assert_eq!(waveform.peaks(20_000, 150_000), [0.0, 0.0]);
        let [low, high] = waveform.peaks(250_000, 450_000);
        assert!(low < -0.3 && high > 0.3, "{low}..{high}");
        assert_eq!(waveform.peaks(510_000, 550_000), [0.0, 0.0]);
    }
}
