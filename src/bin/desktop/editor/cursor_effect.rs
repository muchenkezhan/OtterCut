use std::ops::{Range, RangeInclusive};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use takeseam::editor_timeline::Clip;

use super::media::CursorViewport;
use openscreen_motion::{pan_ease, zoom_ease};
pub(super) use openscreen_motion::ZoomSpring;

#[path = "cursor_effect/openscreen_motion.rs"]
mod openscreen_motion;

const TELEMETRY_VERSION: u32 = 1;
const MAX_SAMPLES: usize = 2_000_000;
const CLICK_ZOOM_HOLD_US: u64 = 1_300_000;
const CLICK_ZOOM_MIN_RAMP_US: u64 = 240_000;
const CLICK_ZOOM_PAN_US: u64 = 1_000_000;
const OPENSCREEN_ZOOM_IN_US: u64 = 1_522_575;
const OPENSCREEN_ZOOM_OUT_US: u64 = 1_015_050;
const OPENSCREEN_ZOOM_IN_OVERLAP_US: u64 = 500_000;
const OPENSCREEN_CONNECTED_GAP_US: u64 = 1_500_000;
pub(super) const ZOOM_SCALE_RANGE: RangeInclusive<f32> = 1.0..=5.0;
pub(super) const ZOOM_TRANSITION_RANGE: RangeInclusive<u64> = CLICK_ZOOM_MIN_RAMP_US..=3_000_000;
pub(super) const ZOOM_HOLD_RANGE: RangeInclusive<u64> = 0..=5_000_000;
pub(super) const ZOOM_PAN_RANGE: RangeInclusive<u64> = 50_000..=3_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct CursorSample {
    pub at_us: u64,
    pub x: f32,
    pub y: f32,
    #[serde(default)]
    pub pressed: bool,
}

impl CursorSample {
    pub(super) fn is_active_click(&self, deleted_cursor_clicks_us: &[u64]) -> bool {
        self.pressed && deleted_cursor_clicks_us.binary_search(&self.at_us).is_err()
    }
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

    pub(super) fn sample_at(
        &self,
        at_us: u64,
        deleted_cursor_clicks_us: &[u64],
    ) -> Option<CursorSample> {
        let index = self.samples.partition_point(|sample| sample.at_us <= at_us);
        match (
            index.checked_sub(1).and_then(|left| self.samples.get(left)),
            self.samples.get(index),
        ) {
            (Some(left), Some(right)) if right.at_us > left.at_us => {
                let amount = (at_us - left.at_us) as f32 / (right.at_us - left.at_us) as f32;
                Some(CursorSample {
                    at_us,
                    x: left.x + (right.x - left.x) * amount,
                    y: left.y + (right.y - left.y) * amount,
                    pressed: left.is_active_click(deleted_cursor_clicks_us)
                        || right.is_active_click(deleted_cursor_clicks_us),
                })
            }
            (Some(sample), _) | (None, Some(sample)) => Some(CursorSample {
                at_us,
                pressed: sample.is_active_click(deleted_cursor_clicks_us),
                ..*sample
            }),
            (None, None) => None,
        }
    }

    pub(super) fn for_video(path: &Path) -> Option<Self> {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(".cursor.json");
        Self::load(&PathBuf::from(sidecar)).ok()
    }

    pub(super) fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)
            .with_context(|| format!("无法读取鼠标轨迹（{}）", path.display()))?;
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ZoomFrame {
    pub(super) scale: f32,
    /// Crop center in normalized canvas coordinates.
    pub(super) center: [f32; 2],
}

#[derive(Default)]
pub(super) struct CursorZoom {
    shots: Vec<ZoomShot>,
    scale: f64,
    pan_us: u64,
    follow: bool,
    smoothing: f64,
    active: Range<u64>,
}

struct ZoomShot {
    start: u64,
    full_at: u64,
    return_at: u64,
    end: u64,
    anchors: Vec<ZoomAnchor>,
}

struct ZoomAnchor {
    time: u64,
    point: [f32; 2],
    pan_start: u64,
}

impl CursorZoom {
    /// Build once for the trimmed clip. All shot times are relative to the clip start.
    ///
    /// Adapt OpenScreen's zoomRegionUtils to OtterCut's recorded clicks. The click
    /// marks the original region's full-zoom point, 500 ms after its region start.
    pub(super) fn new(
        telemetry: &CursorTelemetry,
        deleted_cursor_clicks_us: &[u64],
        source: Range<u64>,
        speed_milli: u16,
        reverse: bool,
        active: Range<u64>,
        effect: &CursorEffect,
    ) -> Self {
        let maximum = effect.zoom_scale;
        if !maximum.is_finite() || maximum <= 1.0 || source.is_empty() || active.is_empty() {
            return Self::default();
        }
        let scale = f64::from(maximum.min(*ZOOM_SCALE_RANGE.end()));
        let zoom_out_us = effect.zoom_transition_us();
        let zoom_in_us = effect.zoom_in_us();
        let hold_us = effect.zoom_hold_us.min(*ZOOM_HOLD_RANGE.end());
        let pan_us = effect.zoom_pan_us.clamp(*ZOOM_PAN_RANGE.start(), *ZOOM_PAN_RANGE.end());
        let mut shots: Vec<ZoomShot> = Vec::new();
        let first = telemetry.samples.partition_point(|sample| sample.at_us < source.start);
        let end = telemetry.samples.partition_point(|sample| sample.at_us < source.end);
        let samples = &telemetry.samples[first..end];
        for index in 0..samples.len() {
            let index = if reverse { samples.len() - 1 - index } else { index };
            let sample = samples[index];
            if !sample.is_active_click(deleted_cursor_clicks_us)
                || ![sample.x, sample.y].iter().all(|value| (0.0..=1.0).contains(value))
            {
                continue;
            }
            let time = timeline_offset_for_sample(&source, speed_milli, reverse, sample.at_us);
            if !active.contains(&time) {
                continue;
            }
            let point = [sample.x, sample.y];
            let start = active.start.max(time.saturating_sub(zoom_in_us));
            let end = active.end.min(time.saturating_add(hold_us + zoom_out_us));
            // Keep a short return ramp when the clip ends during the selected hold.
            let min_return_us = CLICK_ZOOM_MIN_RAMP_US.min((end - time) / 2);
            let return_at = (end - min_return_us).min(time.saturating_add(hold_us));
            let region_start = time.saturating_sub(OPENSCREEN_ZOOM_IN_OVERLAP_US);
            if let Some(shot) = shots.last_mut()
                && region_start <= shot.return_at.saturating_add(OPENSCREEN_CONNECTED_GAP_US)
            {
                let previous = shot.anchors.last_mut().unwrap();
                if previous.time == time {
                    previous.point = point;
                    continue;
                }
                shot.anchors.push(ZoomAnchor {
                    time,
                    point,
                    pan_start: shot.return_at.min(time),
                });
                shot.return_at = return_at;
                shot.end = end;
            } else {
                shots.push(ZoomShot {
                    start,
                    full_at: time,
                    return_at,
                    end,
                    anchors: vec![ZoomAnchor { time, point, pan_start: start }],
                });
            }
        }
        Self {
            shots,
            scale,
            pan_us,
            follow: effect.zoom_follow_cursor,
            smoothing: f64::from(effect.smoothing.clamp(0.0, 1.0)),
            active,
        }
    }

    pub(super) fn frame(
        &self,
        offset_us: u64,
        cursor: Option<CursorSample>,
        viewport: CursorViewport,
        spring: &mut ZoomSpring,
        animated: bool,
    ) -> Option<ZoomFrame> {
        let resolve_focus = |point| {
            let margin = 0.5 / self.scale;
            viewport.map(point).map(|coordinate| f64::from(coordinate).clamp(margin, 1.0 - margin))
        };
        let target = if let Some((shot, strength)) = self.active_shot(offset_us) {
            let raw_focus = cursor
                .filter(|sample| {
                    self.follow
                        && [sample.x, sample.y]
                            .iter()
                            .all(|coordinate| (0.0..=1.0).contains(coordinate))
                })
                .map(|sample| resolve_focus([sample.x, sample.y]));
            let focus = spring
                .follow_focus(
                    raw_focus,
                    offset_us,
                    self.smoothing,
                    animated && offset_us >= shot.full_at,
                )
                .unwrap_or_else(|| shot.focus_at(offset_us, self.pan_us, resolve_focus));
            self.transform(strength, focus)
        } else {
            spring.follow_focus(None, offset_us, self.smoothing, false);
            [1.0, 0.0, 0.0]
        };
        let [scale, x, y] = spring.step(
            target,
            viewport.surface(),
            offset_us,
            animated && self.active.contains(&offset_us),
        );
        zoom_frame(scale, x, y)
    }

    fn active_shot(&self, offset_us: u64) -> Option<(&ZoomShot, f64)> {
        // Separate regions can have overlapping ramps. OpenScreen takes the strongest
        // one, preferring the later region on a tie.
        let first = self.shots.partition_point(|shot| shot.end <= offset_us);
        let mut selected = None;
        let mut best_strength = 0.0;
        for shot in self.shots[first..].iter().take_while(|shot| shot.start <= offset_us) {
            let strength = shot.strength_at(offset_us);
            if strength == 0.0 || strength < best_strength {
                continue;
            }
            best_strength = strength;
            selected = Some((shot, strength));
        }
        selected
    }

    fn transform(&self, strength: f64, focus: [f64; 2]) -> [f64; 3] {
        [
            1.0 + (self.scale - 1.0) * strength,
            (0.5 - focus[0] * self.scale) * strength,
            (0.5 - focus[1] * self.scale) * strength,
        ]
    }

    #[cfg(test)]
    fn at(&self, offset_us: u64) -> Option<ZoomFrame> {
        let (shot, strength) = self.active_shot(offset_us)?;
        let margin = 0.5 / self.scale;
        let focus = shot.focus_at(offset_us, self.pan_us, |point| {
            point.map(|coordinate| f64::from(coordinate).clamp(margin, 1.0 - margin))
        });
        let [scale, x, y] = self.transform(strength, focus);
        zoom_frame(scale, x, y)
    }
}

fn zoom_frame(scale: f64, x: f64, y: f64) -> Option<ZoomFrame> {
    if scale <= 1.0 {
        return None;
    }
    let margin = 0.5 / scale;
    Some(ZoomFrame {
        scale: scale as f32,
        center: [x, y].map(|translation| {
            ((0.5 - translation) / scale).clamp(margin, 1.0 - margin) as f32
        }),
    })
}

impl ZoomShot {
    fn strength_at(&self, time: u64) -> f64 {
        if time >= self.end || time < self.start {
            return 0.0;
        }
        // A trim can cut through a ramp. Use the available interval so the camera
        // remains at the original framing at the effect boundary.
        if time < self.full_at {
            zoom_ease((time - self.start) as f64 / (self.full_at - self.start) as f64)
        } else if time <= self.return_at {
            1.0
        } else {
            1.0 - zoom_ease((time - self.return_at) as f64 / (self.end - self.return_at) as f64)
        }
    }

    fn focus_at(
        &self,
        time: u64,
        pan_us: u64,
        resolve_focus: impl Fn([f32; 2]) -> [f64; 2],
    ) -> [f64; 2] {
        let index = self.anchors.partition_point(|anchor| anchor.pan_start <= time);
        let anchor = &self.anchors[index.saturating_sub(1)];
        let target = resolve_focus(anchor.point);
        if index <= 1 {
            return target;
        }
        let next_start = self.anchors.get(index).map_or(self.return_at, |next| next.pan_start);
        let end = anchor.pan_start.saturating_add(pan_us).min(next_start);
        if time >= end {
            return target;
        }
        let previous = resolve_focus(self.anchors[index - 2].point);
        let progress = pan_ease((time - anchor.pan_start) as f64 / (end - anchor.pan_start) as f64);
        [
            previous[0] + (target[0] - previous[0]) * progress,
            previous[1] + (target[1] - previous[1]) * progress,
        ]
    }
}

pub(super) fn timeline_time_for_sample(clip: &Clip, source_time_us: u64) -> Option<u64> {
    let source = clip.source_start..clip.source_end;
    if !source.contains(&source_time_us) {
        return None;
    }
    let offset =
        timeline_offset_for_sample(&source, clip.speed_milli, clip.reverse, source_time_us);
    let timeline_time = clip.start.saturating_add(offset);
    (timeline_time < clip.end()).then_some(timeline_time)
}

fn timeline_offset_for_sample(
    source: &Range<u64>,
    speed_milli: u16,
    reverse: bool,
    at_us: u64,
) -> u64 {
    let source_offset = if reverse {
        source.end - 1 - at_us
    } else {
        at_us - source.start
    };
    source_offset.saturating_mul(1000) / u64::from(speed_milli.max(1))
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CursorStyle {
    #[default]
    Default,
    StudioInk,
    Prism,
    Coral,
    Pixel,
    StarSprout,
    GlassLens,
}

pub(super) struct CursorSprite {
    pub(super) rgba: Vec<u8>,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) hotspot: [f32; 2],
}

impl CursorStyle {
    pub(super) const ALL: [Self; 7] = [
        Self::Default,
        Self::StudioInk,
        Self::Prism,
        Self::Coral,
        Self::Pixel,
        Self::StarSprout,
        Self::GlassLens,
    ];

    pub(super) fn sprite(self) -> Option<&'static CursorSprite> {
        static STUDIO_INK: OnceLock<CursorSprite> = OnceLock::new();
        static PRISM: OnceLock<CursorSprite> = OnceLock::new();
        static CORAL: OnceLock<CursorSprite> = OnceLock::new();
        static PIXEL: OnceLock<CursorSprite> = OnceLock::new();
        static STAR_SPROUT: OnceLock<CursorSprite> = OnceLock::new();
        static GLASS_LENS: OnceLock<CursorSprite> = OnceLock::new();

        let (slot, png, hotspot): (_, &[u8], [f32; 2]) = match self {
            Self::Default => return None,
            Self::StudioInk => (
                &STUDIO_INK,
                include_bytes!("../../../../assets/cursors/studio-ink.png"),
                [6.2304, 2.0992],
            ),
            Self::Prism => (
                &PRISM,
                include_bytes!("../../../../assets/cursors/prism-glow.png"),
                [6.3456, 2.0672],
            ),
            Self::Coral => (
                &CORAL,
                include_bytes!("../../../../assets/cursors/pop-coral.png"),
                [10.4768, 2.1792],
            ),
            Self::Pixel => (
                &PIXEL,
                include_bytes!("../../../../assets/cursors/pixel-candy.png"),
                [6.5, 2.0],
            ),
            Self::StarSprout => (
                &STAR_SPROUT,
                include_bytes!("../../../../assets/cursors/star-sprout.png"),
                [4.7232, 2.1152],
            ),
            Self::GlassLens => (
                &GLASS_LENS,
                include_bytes!("../../../../assets/cursors/glass-lens.png"),
                [9.744, 0.816],
            ),
        };

        Some(slot.get_or_init(|| {
            let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
                .expect("bundled cursor artwork is a valid PNG image")
                .into_rgba8();
            let (width, height) = image.dimensions();
            CursorSprite {
                rgba: image.into_raw(),
                width,
                height,
                // OpenScreen specifies hotspots on a 32 by 32 coordinate grid.
                hotspot: hotspot.map(|coordinate| coordinate / 32.0),
            }
        }))
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoom_transition_us: Option<u64>,
    #[serde(default = "default_zoom_hold_us")]
    pub zoom_hold_us: u64,
    #[serde(default = "default_zoom_pan_us")]
    pub zoom_pan_us: u64,
    #[serde(default = "default_true")]
    pub zoom_follow_cursor: bool,
    #[serde(default = "default_smoothing")]
    pub smoothing: f32,
    #[serde(default)]
    pub click_effect: CursorClickEffect,
    #[serde(default)]
    pub cursor_style: CursorStyle,
}

impl CursorEffect {
    pub(super) fn default_for(duration_us: u64) -> Self {
        Self {
            id: 1,
            start_us: 0,
            end_us: duration_us.max(1),
            enabled: true,
            cursor_scale: 1.0,
            zoom_scale: default_zoom_scale(),
            zoom_transition_us: None,
            zoom_hold_us: default_zoom_hold_us(),
            zoom_pan_us: default_zoom_pan_us(),
            zoom_follow_cursor: true,
            smoothing: 0.7,
            click_effect: CursorClickEffect::Ring,
            cursor_style: CursorStyle::Default,
        }
    }

    pub(super) fn contains(&self, at_us: u64) -> bool {
        self.enabled && self.start_us <= at_us && at_us < self.end_us
    }

    pub(super) fn zoom_transition_us(&self) -> u64 {
        let duration = self.zoom_transition_us.unwrap_or(OPENSCREEN_ZOOM_OUT_US);
        duration.clamp(*ZOOM_TRANSITION_RANGE.start(), *ZOOM_TRANSITION_RANGE.end())
    }

    pub(super) fn zoom_in_us(&self) -> u64 {
        self.zoom_transition_us.map_or(OPENSCREEN_ZOOM_IN_US, |_| self.zoom_transition_us())
    }

    pub(super) fn validate(&self) -> Result<()> {
        ensure!(self.start_us < self.end_us, "鼠标效果时间范围无效");
        ensure!((0.5..=4.0).contains(&self.cursor_scale), "鼠标大小无效");
        ensure!(ZOOM_SCALE_RANGE.contains(&self.zoom_scale), "鼠标放大倍数无效");
        ensure!(
            self.zoom_transition_us
                .is_none_or(|duration| ZOOM_TRANSITION_RANGE.contains(&duration)),
            "点击放大过渡时长无效"
        );
        ensure!(ZOOM_HOLD_RANGE.contains(&self.zoom_hold_us), "点击放大停留时长无效");
        ensure!(ZOOM_PAN_RANGE.contains(&self.zoom_pan_us), "点击跟随平移时长无效");
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
    1.8
}

fn default_zoom_hold_us() -> u64 {
    CLICK_ZOOM_HOLD_US
}

fn default_zoom_pan_us() -> u64 {
    CLICK_ZOOM_PAN_US
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
        let sample = telemetry.sample_at(500_000, &[]).unwrap();
        assert!((sample.x - 0.5).abs() < 0.001);
        assert!((sample.y - 0.5).abs() < 0.001);
        assert!(sample.pressed);
    }

    #[test]
    fn deleted_clicks_preserve_pointer_positions_and_other_click_effects() {
        let mut telemetry = recorded_clicks(&[(1_000_000, 0.4, 0.5)]);
        telemetry.push(CursorSample {
            at_us: 1_040_000,
            x: 0.6,
            y: 0.5,
            pressed: false,
        });
        telemetry.push(CursorSample {
            at_us: 6_000_000,
            x: 0.5,
            y: 0.4,
            pressed: true,
        });
        let deleted = [1_000_000];
        let unedited = telemetry.sample_at(1_020_000, &[]).unwrap();
        let edited = telemetry.sample_at(1_020_000, &deleted).unwrap();
        assert!(unedited.pressed);
        assert_eq!(
            edited,
            CursorSample {
                pressed: false,
                ..unedited
            }
        );
        assert!(!telemetry.sample_at(0, &deleted).unwrap().pressed);
        assert!(telemetry.sample_at(6_000_000, &deleted).unwrap().pressed);

        let clip = 0..9_000_000;
        let effect = zoom_effect(2.0, clip.end);
        let zoom = CursorZoom::new(&telemetry, &deleted, clip.clone(), 1000, false, clip, &effect);
        assert_eq!(zoom.at(1_000_000), None);
        assert_eq!(zoom.at(6_000_000).unwrap().scale, 2.0);
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
            cursor_style: CursorStyle::Default,
            ..CursorEffect::default_for(1)
        };
        assert!(effect.validate().is_err());
    }

    #[test]
    fn legacy_cursor_effect_defaults_to_the_default_cursor_style() {
        let effect: CursorEffect = serde_json::from_str(
            r#"{
                "id": 1,
                "start_us": 0,
                "end_us": 1000,
                "enabled": true,
                "cursor_scale": 1.0,
                "zoom_scale": 1.6,
                "smoothing": 0.7,
                "click_effect": "ring"
            }"#,
        )
        .unwrap();

        assert_eq!(effect.cursor_style, CursorStyle::Default);
        assert_eq!(effect.zoom_transition_us, None);
        assert_eq!(effect.zoom_in_us(), 1_522_575);
        assert_eq!(effect.zoom_transition_us(), 1_015_050);
        assert!(effect.zoom_follow_cursor);
        assert_eq!(effect.zoom_hold_us, CLICK_ZOOM_HOLD_US);
        assert_eq!(effect.zoom_pan_us, CLICK_ZOOM_PAN_US);
    }

    #[test]
    fn cursor_style_survives_project_serialization() {
        let mut effect = CursorEffect::default_for(1_000);
        effect.cursor_style = CursorStyle::Prism;

        let saved = serde_json::to_string(&effect).unwrap();
        let restored: CursorEffect = serde_json::from_str(&saved).unwrap();

        assert_eq!(restored.cursor_style, CursorStyle::Prism);
    }

    fn recorded_clicks(points: &[(u64, f32, f32)]) -> CursorTelemetry {
        let mut telemetry = CursorTelemetry::new(1920, 1080);
        for &(at_us, x, y) in points {
            telemetry.push(CursorSample {
                at_us,
                x,
                y,
                pressed: true,
            });
        }
        telemetry
    }

    fn zoom_effect(scale: f32, duration_us: u64) -> CursorEffect {
        let mut effect = CursorEffect::default_for(duration_us);
        effect.zoom_scale = scale;
        effect
    }

    #[test]
    fn click_zoom_eases_in_before_the_click_and_returns_to_wide() {
        let telemetry = recorded_clicks(&[(2_000_000, 0.63, 0.4)]);
        let effect = zoom_effect(1.6, 6_000_000);
        let zoom = CursorZoom::new(&telemetry, &[], 0..6_000_000, 1000, false, 0..6_000_000, &effect);

        assert_eq!(zoom.at(477_424), None);
        let approaching = zoom.at(1_500_000).unwrap();
        assert!((1.0..1.6).contains(&approaching.scale));
        assert!(approaching.center[0] > 0.5 && approaching.center[0] < 0.63);
        let full = zoom.at(2_000_000).unwrap();
        assert!((full.scale - 1.6).abs() < 1e-5);
        assert_eq!(full.center, [0.63, 0.4]);
        assert_eq!(zoom.at(3_200_000), Some(full));
        let returning = zoom.at(3_700_000).unwrap();
        assert!((1.0..1.6).contains(&returning.scale));
        assert_eq!(zoom.at(4_315_050), None);
    }

    #[test]
    fn click_zoom_controls_change_transition_hold_and_pan_timing() {
        let single_click = recorded_clicks(&[(2_000_000, 0.4, 0.5)]);
        let default_effect = zoom_effect(1.6, 6_000_000);
        let mut custom_effect = default_effect.clone();
        custom_effect.zoom_transition_us = Some(2_000_000);
        custom_effect.zoom_hold_us = 0;
        custom_effect.zoom_pan_us = 100_000;

        let default = CursorZoom::new(
            &single_click,
            &[],
            0..6_000_000,
            1000,
            false,
            0..6_000_000,
            &default_effect,
        );
        let custom = CursorZoom::new(
            &single_click,
            &[],
            0..6_000_000,
            1000,
            false,
            0..6_000_000,
            &custom_effect,
        );

        assert!(
            custom.at(1_500_000).unwrap().scale > default.at(1_500_000).unwrap().scale,
            "longer transition should make the zoom more advanced before the click"
        );
        assert!(
            custom.at(2_200_000).unwrap().scale < default.at(2_200_000).unwrap().scale,
            "zero hold should start the return sooner"
        );

        let nearby_clicks = recorded_clicks(&[(2_000_000, 0.4, 0.5), (3_500_000, 0.6, 0.5)]);
        let default_pan = CursorZoom::new(
            &nearby_clicks,
            &[],
            0..6_000_000,
            1000,
            false,
            0..6_000_000,
            &default_effect,
        );
        custom_effect.zoom_hold_us = default_effect.zoom_hold_us;
        let custom_pan = CursorZoom::new(
            &nearby_clicks,
            &[],
            0..6_000_000,
            1000,
            false,
            0..6_000_000,
            &custom_effect,
        );
        assert!(
            custom_pan.at(3_400_000).unwrap().center[0]
                > default_pan.at(3_400_000).unwrap().center[0],
            "shorter pan should arrive at the next focus sooner"
        );
    }

    #[test]
    fn nearby_clicks_keep_the_scale_and_pan_to_the_next_click() {
        let telemetry = recorded_clicks(&[(2_000_000, 0.4, 0.5), (3_500_000, 0.6, 0.5)]);
        let effect = zoom_effect(1.6, 6_000_000);
        let zoom = CursorZoom::new(&telemetry, &[], 0..6_000_000, 1000, false, 0..6_000_000, &effect);

        let first = zoom.at(2_900_000).unwrap();
        let panning = zoom.at(3_550_000).unwrap();
        let landed = zoom.at(4_300_000).unwrap();
        assert_eq!(
            first,
            ZoomFrame {
                scale: 1.6,
                center: [0.4, 0.5]
            }
        );
        assert_eq!(panning.scale, 1.6);
        assert!(panning.center[0] > 0.4 && panning.center[0] < 0.6);
        assert_eq!(
            landed,
            ZoomFrame {
                scale: 1.6,
                center: [0.6, 0.5]
            }
        );
    }

    #[test]
    fn zoom_tracks_pointer_motion_between_clicks_and_snaps_on_seek() {
        use crate::editor::background::BackgroundSettings;
        use crate::editor::media::{MediaInfo, cursor_viewport};

        let size = (1920, 1080);
        let viewport = cursor_viewport(
            size,
            size,
            MediaInfo {
                duration_us: 6_000_000,
                width: size.0,
                height: size.1,
                has_video: true,
                has_audio: false,
            },
            &BackgroundSettings::default(),
        );
        let mut telemetry = recorded_clicks(&[(2_000_000, 0.4, 0.5)]);
        telemetry.push(CursorSample { at_us: 2_400_000, x: 0.65, y: 0.5, pressed: false });
        let mut effect = zoom_effect(2.0, 6_000_000);
        let clip = 0..6_000_000;
        let following = CursorZoom::new(&telemetry, &[], clip.clone(), 1000, false, clip.clone(), &effect);
        effect.zoom_follow_cursor = false;
        let fixed = CursorZoom::new(&telemetry, &[], clip.clone(), 1000, false, clip, &effect);
        let mut spring = ZoomSpring::default();
        let mut center = 0.4;
        for time in (2_000_000..=2_400_000).step_by(20_000) {
            let frame = following.frame(time, telemetry.sample_at(time, &[]), viewport, &mut spring, true).unwrap();
            assert!(frame.center[0] >= center);
            center = frame.center[0];
        }
        assert!((0.5..0.65).contains(&center));
        let time = 2_400_000;
        let sample = telemetry.sample_at(time, &[]);
        assert_eq!(following.frame(time, sample, viewport, &mut spring, false).unwrap().center[0], 0.65);
        assert_eq!(fixed.frame(time, sample, viewport, &mut spring, false).unwrap().center[0], 0.4);
    }

    #[test]
    fn click_zoom_respects_trim_reverse_speed_and_active_bounds() {
        let telemetry = recorded_clicks(&[
            (1_000_000, 0.5, 0.5),
            (4_000_000, 0.98, 0.02),
            (8_000_000, 0.5, 0.5),
        ]);
        let effect = zoom_effect(2.0, 2_400_000);
        let zoom = CursorZoom::new(
            &telemetry,
            &[],
            2_000_000..8_000_000,
            2000,
            true,
            1_200_000..2_400_000,
            &effect,
        );

        assert_eq!(zoom.at(1_200_000), None);
        assert!(zoom.at(1_200_001).unwrap().scale - 1.0 < 1e-4);
        let full = zoom.at(1_999_999).unwrap();
        assert_eq!(
            full,
            ZoomFrame {
                scale: 2.0,
                center: [0.75, 0.25]
            }
        );
        assert!(zoom.at(2_399_999).is_none_or(|frame| frame.scale - 1.0 < 1e-4));
        assert_eq!(zoom.at(2_400_000), None);
        let normal_effect = zoom_effect(1.0, 8_000_000);
        let normal = CursorZoom::new(
            &telemetry,
            &[],
            0..8_000_000,
            1000,
            false,
            0..8_000_000,
            &normal_effect,
        );
        let trimmed_effect = zoom_effect(1.6, 3_000_000);
        assert_eq!(normal.at(4_000_000), None);
        let trimmed = CursorZoom::new(
            &telemetry,
            &[],
            5_000_000..8_000_000,
            1000,
            false,
            0..3_000_000,
            &trimmed_effect,
        );
        assert_eq!(trimmed.at(1_000_000), None);
    }

    #[test]
    fn cursor_zoom_defaults_for_new_and_legacy_projects() {
        let effect = CursorEffect::default_for(1_000);
        let mut json = serde_json::to_value(&effect).unwrap();
        json.as_object_mut().unwrap().remove("zoom_scale");
        let legacy: CursorEffect = serde_json::from_value(json).unwrap();

        assert_eq!(effect.zoom_scale, 1.8);
        assert_eq!(legacy.zoom_scale, 1.8);
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
            deleted_cursor_clicks_us: Vec::new(),
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
            deleted_cursor_clicks_us: Vec::new(),
        };
        assert_eq!(timeline_time_for_sample(&clip, 7_000_000), Some(1_999_999));
        assert_eq!(timeline_time_for_sample(&clip, 2_000_000), Some(6_999_999));
        clip.speed_milli = 2000;
        assert_eq!(timeline_time_for_sample(&clip, 7_000_000), Some(1_499_999));
    }
}
