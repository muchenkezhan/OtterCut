use std::path::Path;

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use takeseam::editor_timeline::Clip;

const TELEMETRY_VERSION: u32 = 1;
const MAX_SAMPLES: usize = 2_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct CursorSample {
    pub at_us: u64,
    pub x: f32,
    pub y: f32,
    #[serde(default)]
    pub pressed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct CursorTelemetry {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub samples: Vec<CursorSample>,
}

impl CursorTelemetry {
    #[cfg(test)]
    pub(super) fn new(width: u32, height: u32) -> Self {
        Self {
            version: TELEMETRY_VERSION,
            width,
            height,
            samples: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn push(&mut self, sample: CursorSample) {
        if sample.x.is_finite()
            && sample.y.is_finite()
            && (0.0..=1.0).contains(&sample.x)
            && (0.0..=1.0).contains(&sample.y)
            && self
                .samples
                .last()
                .is_none_or(|previous| sample.at_us >= previous.at_us)
        {
            self.samples.push(sample);
        }
    }

    pub(super) fn sample_at(&self, at_us: u64) -> Option<CursorSample> {
        let index = self.samples.partition_point(|sample| sample.at_us <= at_us);
        match (self.samples.get(index.checked_sub(1)?), self.samples.get(index)) {
            (Some(left), Some(right)) if right.at_us > left.at_us => {
                let amount = (at_us - left.at_us) as f32 / (right.at_us - left.at_us) as f32;
                Some(CursorSample {
                    at_us,
                    x: left.x + (right.x - left.x) * amount,
                    y: left.y + (right.y - left.y) * amount,
                    pressed: left.pressed || right.pressed,
                })
            }
            (Some(sample), _) => Some(CursorSample { at_us, ..*sample }),
            (None, Some(sample)) => Some(CursorSample { at_us, ..*sample }),
            (None, None) => None,
        }
    }

    pub(super) fn zoom_scale_at(&self, at_us: u64, maximum: f32) -> f32 {
        let Some(click) = self
            .samples
            .iter()
            .rev()
            .find(|sample| sample.at_us <= at_us && sample.pressed)
        else {
            return 1.0;
        };
        let elapsed = at_us.saturating_sub(click.at_us) as f32 / 1_000_000.0;
        let progress = if elapsed < 0.27 {
            let t = elapsed / 0.27;
            t * t * (3.0 - 2.0 * t)
        } else if elapsed < 1.47 {
            1.0
        } else if elapsed < 1.89 {
            let t = (elapsed - 1.47) / 0.42;
            1.0 - t * t * (3.0 - 2.0 * t)
        } else {
            0.0
        };
        1.0 + (maximum.clamp(1.0, 3.0) - 1.0) * progress
    }

    pub(super) fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("无法读取鼠标轨迹（{}）", path.display()))?;
        ensure!(bytes.len() <= 64 * 1024 * 1024, "鼠标轨迹文件过大");
        let telemetry: Self = serde_json::from_slice(&bytes).context("鼠标轨迹格式无效")?;
        telemetry.validate()?;
        Ok(telemetry)
    }

    fn validate(&self) -> Result<()> {
        ensure!(self.version == TELEMETRY_VERSION, "鼠标轨迹版本不受支持");
        ensure!(self.width > 0 && self.height > 0, "鼠标轨迹画面尺寸无效");
        ensure!(self.samples.len() <= MAX_SAMPLES, "鼠标轨迹采样点过多");
        ensure!(
            self.samples.windows(2).all(|samples| {
                samples[0].at_us <= samples[1].at_us
                    && (0.0..=1.0).contains(&samples[1].x)
                    && (0.0..=1.0).contains(&samples[1].y)
            }),
            "鼠标轨迹时间顺序无效"
        );
        Ok(())
    }
}

pub(super) fn timeline_time_for_sample(clip: &Clip, source_time_us: u64) -> Option<u64> {
    if source_time_us < clip.source_start || source_time_us >= clip.source_end {
        return None;
    }
    let source_offset = if clip.reverse {
        clip.source_end.saturating_sub(1).saturating_sub(source_time_us)
    } else {
        source_time_us.saturating_sub(clip.source_start)
    };
    let timeline_offset = source_offset
        .saturating_mul(1000)
        .checked_div(u64::from(clip.speed_milli.max(1)))?;
    let timeline_time = clip.start.saturating_add(timeline_offset);
    (timeline_time < clip.end()).then_some(timeline_time)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CursorClickEffect {
    Ring,
    Dot,
    None,
}

impl Default for CursorClickEffect {
    fn default() -> Self {
        Self::Ring
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct CursorEffect {
    pub id: u64,
    pub start_us: u64,
    pub end_us: u64,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_cursor_scale")]
    pub cursor_scale: f32,
    #[serde(default = "default_zoom_scale")]
    pub zoom_scale: f32,
    #[serde(default = "default_smoothing")]
    pub smoothing: f32,
    #[serde(default)]
    pub click_effect: CursorClickEffect,
}

impl CursorEffect {
    pub(super) fn default_for(duration_us: u64) -> Self {
        Self {
            id: 1,
            start_us: 0,
            end_us: duration_us.max(1),
            enabled: true,
            cursor_scale: 1.0,
            zoom_scale: 1.0,
            smoothing: 0.7,
            click_effect: CursorClickEffect::Ring,
        }
    }

    pub(super) fn contains(&self, at_us: u64) -> bool {
        self.enabled && self.start_us <= at_us && at_us < self.end_us
    }

    pub(super) fn validate(&self) -> Result<()> {
        ensure!(self.start_us < self.end_us, "鼠标效果时间范围无效");
        ensure!((0.5..=4.0).contains(&self.cursor_scale), "鼠标大小无效");
        ensure!((1.0..=3.0).contains(&self.zoom_scale), "鼠标放大倍数无效");
        ensure!((0.0..=1.0).contains(&self.smoothing), "鼠标平滑参数无效");
        Ok(())
    }
}

fn default_true() -> bool {
    true
}

fn default_cursor_scale() -> f32 {
    1.0
}

fn default_zoom_scale() -> f32 {
    1.6
}

fn default_smoothing() -> f32 {
    0.7
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_interpolates_between_normalized_samples() {
        let mut telemetry = CursorTelemetry::new(1920, 1080);
        telemetry.push(CursorSample {
            at_us: 0,
            x: 0.0,
            y: 0.2,
            pressed: false,
        });
        telemetry.push(CursorSample {
            at_us: 1_000_000,
            x: 1.0,
            y: 0.8,
            pressed: true,
        });
        let sample = telemetry.sample_at(500_000).unwrap();
        assert!((sample.x - 0.5).abs() < 0.001);
        assert!((sample.y - 0.5).abs() < 0.001);
        assert!(sample.pressed);
    }

    #[test]
    fn invalid_cursor_effect_ranges_are_rejected() {
        let effect = CursorEffect {
            id: 1,
            start_us: 2,
            end_us: 1,
            enabled: true,
            cursor_scale: 1.0,
            zoom_scale: 1.6,
            smoothing: 0.7,
            click_effect: CursorClickEffect::Ring,
        };
        assert!(effect.validate().is_err());
    }

    #[test]
    fn maps_recorded_clicks_through_trimmed_and_sped_up_video() {
        let clip = Clip {
            id: 1,
            path: "recording.mp4".into(),
            media_duration: 10_000_000,
            source_start: 2_000_000,
            source_end: 8_000_000,
            start: 5_000_000,
            volume_db_tenths: 0,
            fade_in_us: 0,
            fade_out_us: 0,
            transition_in: None,
            speed_milli: 2000,
            reverse: false,
            timeline_duration_us: 0,
            group_id: None,
            text: None,
        };
        assert_eq!(timeline_time_for_sample(&clip, 4_000_000), Some(6_000_000));
        assert_eq!(timeline_time_for_sample(&clip, 8_000_000), None);
    }

    #[test]
    fn maps_recorded_clicks_backwards_for_reversed_video() {
        let mut clip = Clip {
            id: 1,
            path: "recording.mp4".into(),
            media_duration: 10_000_000,
            source_start: 2_000_000,
            source_end: 8_000_000,
            start: 1_000_000,
            volume_db_tenths: 0,
            fade_in_us: 0,
            fade_out_us: 0,
            transition_in: None,
            speed_milli: 1000,
            reverse: true,
            timeline_duration_us: 0,
            group_id: None,
            text: None,
        };
        assert_eq!(timeline_time_for_sample(&clip, 7_000_000), Some(1_999_999));
        assert_eq!(timeline_time_for_sample(&clip, 2_000_000), Some(6_999_999));
        clip.speed_milli = 2000;
        assert_eq!(timeline_time_for_sample(&clip, 7_000_000), Some(1_499_999));
    }
}
