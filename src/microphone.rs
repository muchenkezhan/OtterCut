use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};

use anyhow::{Context as _, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputDevice {
    pub id: String,
    pub name: String,
}

pub fn input_devices() -> Result<Vec<InputDevice>> {
    Ok(cpal::default_host()
        .input_devices()
        .context("无法列出麦克风设备")?
        .enumerate()
        .filter_map(|(index, device)| {
            let id = device.id().ok()?.to_string();
            let name = device.description().map_or_else(
                |_| format!("麦克风 {}", index + 1),
                |description| description.name().to_owned(),
            );
            Some(InputDevice { id, name })
        })
        .collect())
}

fn open_input_device(device: Option<&str>) -> Result<Device> {
    let host = cpal::default_host();
    if let Some(device) = device {
        let id = device.parse().context("麦克风设备标识无效")?;
        return host.device_by_id(&id).context("找不到所选麦克风");
    }
    host.default_input_device().context("找不到默认麦克风")
}

pub struct Microphone {
    stream: Stream,
    packets: Receiver<Vec<u8>>,
    errors: Receiver<String>,
    pub channels: u16,
    pub sample_rate: u32,
}

impl Microphone {
    pub fn open_default() -> Result<Self> {
        Self::open(None)
    }

    pub fn open(device: Option<&str>) -> Result<Self> {
        let device = open_input_device(device)?;
        let default = device
            .default_input_config()
            .context("无法读取麦克风格式")?;
        let supported_format = |format| {
            matches!(
                format,
                SampleFormat::F32 | SampleFormat::I16 | SampleFormat::I32 | SampleFormat::U16
            )
        };
        let selected = if matches!(default.channels(), 1 | 2)
            && matches!(default.sample_rate(), 44_100 | 48_000)
            && supported_format(default.sample_format())
        {
            default
        } else {
            device
                .supported_input_configs()
                .context("无法读取麦克风可用格式")?
                .filter(|format| {
                    matches!(format.channels(), 1 | 2) && supported_format(format.sample_format())
                })
                .find_map(|format| {
                    format
                        .try_with_sample_rate(48_000)
                        .or_else(|| format.try_with_sample_rate(44_100))
                })
                .context("麦克风需要支持 44.1 或 48 kHz 的单声道或双声道 PCM 格式")?
        };
        let format = selected.sample_format();
        let config = selected.config();

        let (packets_tx, packets) = mpsc::sync_channel(64);
        let (errors_tx, errors) = mpsc::sync_channel(1);
        let stream = match format {
            SampleFormat::F32 => input::<f32>(&device, &config, packets_tx, errors_tx),
            SampleFormat::I16 => input::<i16>(&device, &config, packets_tx, errors_tx),
            SampleFormat::I32 => input::<i32>(&device, &config, packets_tx, errors_tx),
            SampleFormat::U16 => input::<u16>(&device, &config, packets_tx, errors_tx),
            other => bail!("麦克风采样格式暂不受支持（{other}）"),
        }
        .context("无法打开麦克风")?;

        Ok(Self {
            stream,
            packets,
            errors,
            channels: config.channels,
            sample_rate: config.sample_rate,
        })
    }

    pub fn start(&self) -> Result<()> {
        self.stream.play().context("无法启动麦克风")
    }

    pub fn pause(&self) -> Result<()> {
        self.stream.pause().context("无法停止麦克风")
    }

    pub fn receive(&self, send: impl FnMut(&[u8]) -> Result<()>) -> Result<()> {
        receive_packets(&self.packets, &self.errors, send)
    }

    pub fn finish(self, send: impl FnMut(&[u8]) -> Result<()>) -> Result<()> {
        let paused = self.pause();
        let Self {
            stream,
            packets,
            errors,
            ..
        } = self;
        // WASAPI queues the pause. Dropping the stream joins its callback thread.
        drop(stream);
        receive_packets(&packets, &errors, send)?;
        paused
    }
}

/// A separate input stream for measuring peak level without buffering audio.
pub struct InputMeter {
    _stream: Stream,
    peak: Arc<AtomicU32>,
    errors: Receiver<String>,
}

impl InputMeter {
    pub fn open(device: Option<&str>) -> Result<Self> {
        let device = open_input_device(device)?;
        let default = device
            .default_input_config()
            .context("无法读取麦克风格式")?;
        let config = default.config();
        let peak = Arc::new(AtomicU32::new(0));
        let callback_peak = Arc::clone(&peak);
        let (errors_tx, errors) = mpsc::sync_channel(1);
        let stream = match default.sample_format() {
            SampleFormat::F32 => meter_input::<f32>(&device, &config, callback_peak, errors_tx),
            SampleFormat::F64 => meter_input::<f64>(&device, &config, callback_peak, errors_tx),
            SampleFormat::I8 => meter_input::<i8>(&device, &config, callback_peak, errors_tx),
            SampleFormat::I16 => meter_input::<i16>(&device, &config, callback_peak, errors_tx),
            SampleFormat::I24 => {
                meter_input::<cpal::I24>(&device, &config, callback_peak, errors_tx)
            }
            SampleFormat::I32 => meter_input::<i32>(&device, &config, callback_peak, errors_tx),
            SampleFormat::I64 => meter_input::<i64>(&device, &config, callback_peak, errors_tx),
            SampleFormat::U8 => meter_input::<u8>(&device, &config, callback_peak, errors_tx),
            SampleFormat::U16 => meter_input::<u16>(&device, &config, callback_peak, errors_tx),
            SampleFormat::U24 => {
                meter_input::<cpal::U24>(&device, &config, callback_peak, errors_tx)
            }
            SampleFormat::U32 => meter_input::<u32>(&device, &config, callback_peak, errors_tx),
            SampleFormat::U64 => meter_input::<u64>(&device, &config, callback_peak, errors_tx),
            other => bail!("麦克风电平暂不支持采样格式 {other}"),
        }
        .context("无法打开麦克风电平采集")?;
        stream.play().context("无法启动麦克风电平采集")?;
        Ok(Self {
            _stream: stream,
            peak,
            errors,
        })
    }

    pub fn level(&mut self) -> Result<f32> {
        if let Ok(error) = self.errors.try_recv() {
            bail!("麦克风电平采集失败（{error}）");
        }
        Ok(f32::from_bits(self.peak.swap(0, Ordering::Relaxed)))
    }
}

fn meter_input<T>(
    device: &Device,
    config: &StreamConfig,
    peak: Arc<AtomicU32>,
    errors: SyncSender<String>,
) -> Result<Stream, cpal::BuildStreamError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device.build_input_stream(
        config,
        move |samples: &[T], _| {
            // Positive finite f32 values have the same ordering as their bit patterns.
            peak.fetch_max(peak_level(samples).to_bits(), Ordering::Relaxed);
        },
        move |error| {
            let _ = errors.try_send(error.to_string());
        },
        None,
    )
}

fn peak_level<T: Copy>(samples: &[T]) -> f32
where
    f32: FromSample<T>,
{
    samples
        .iter()
        .fold(0.0_f32, |peak, &sample| {
            peak.max(f32::from_sample(sample).abs())
        })
        .min(1.0)
}

fn receive_packets(
    packets: &Receiver<Vec<u8>>,
    errors: &Receiver<String>,
    mut send: impl FnMut(&[u8]) -> Result<()>,
) -> Result<()> {
    for packet in packets.try_iter() {
        send(&packet)?;
    }
    if let Ok(error) = errors.try_recv() {
        bail!("麦克风采集失败（{error}）");
    }
    Ok(())
}

fn input<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    packets: SyncSender<Vec<u8>>,
    errors: SyncSender<String>,
) -> Result<Stream, cpal::BuildStreamError>
where
    T: SizedSample,
    i16: FromSample<T>,
{
    let overflow = errors.clone();
    device.build_input_stream(
        config,
        move |samples: &[T], _| {
            let mut pcm = Vec::with_capacity(samples.len() * size_of::<i16>());
            for &sample in samples {
                pcm.extend_from_slice(&i16::from_sample(sample).to_le_bytes());
            }
            if !pcm.is_empty() && matches!(packets.try_send(pcm), Err(TrySendError::Full(_))) {
                let _ = overflow.try_send(String::from("编码未能跟上麦克风输入"));
            }
        },
        move |error| {
            let _ = errors.try_send(error.to_string());
        },
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::peak_level;

    #[test]
    fn peak_meter_normalizes_signed_and_unsigned_samples() {
        assert_eq!(peak_level::<i16>(&[]), 0.0);
        assert_eq!(peak_level(&[0_i16, 8192, -16_384]), 0.5);
        assert_eq!(peak_level(&[128_u8]), 0.0);
        assert_eq!(peak_level(&[128_u8, 64]), 0.5);
        assert_eq!(peak_level(&[f32::NAN, -1.2]), 1.0);
        assert_eq!(peak_level(&[f32::NAN]), 0.0);
    }
}
