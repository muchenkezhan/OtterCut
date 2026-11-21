use std::fs::File;
use std::io::Read as _;
use std::path::Path;

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};

use super::background::BackgroundMotion;

const RECIPE_VERSION: u8 = 1;
const MAX_RECIPE_BYTES: u64 = 8 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DynamicRecipe {
    pub version: u8,
    pub name: String,
    pub colors: [[u8; 3]; 3],
    pub pattern: DynamicPattern,
    pub motion: BackgroundMotion,
    pub speed: u16,
    pub intensity: u8,
    pub angle: u16,
    pub focus: [u8; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DynamicPattern {
    Ribbons,
    Curtain,
    Ripple,
    Halo,
    Grid,
    Dots,
    Meteors,
}

impl DynamicRecipe {
    pub(super) fn modern_presets() -> [Self; 6] {
        use BackgroundMotion::Drift;
        use DynamicPattern::{Dots, Grid, Meteors};

        [
            (
                "深蓝网格",
                [[9, 19, 31], [43, 76, 101], [123, 185, 199]],
                Grid,
                52,
                28,
                0,
                [66, 34],
            ),
            (
                "雾白网格",
                [[245, 248, 250], [194, 209, 220], [128, 161, 184]],
                Grid,
                46,
                25,
                0,
                [35, 30],
            ),
            (
                "蓝灰网点",
                [[15, 22, 34], [60, 82, 107], [145, 170, 196]],
                Dots,
                51,
                29,
                0,
                [59, 41],
            ),
            (
                "纸白网点",
                [[247, 247, 244], [208, 211, 208], [155, 166, 172]],
                Dots,
                42,
                24,
                0,
                [37, 49],
            ),
            (
                "寒星流线",
                [[6, 11, 24], [34, 62, 108], [157, 222, 252]],
                Meteors,
                108,
                77,
                142,
                [63, 35],
            ),
            (
                "琥珀流星",
                [[17, 12, 23], [94, 54, 48], [248, 199, 147]],
                Meteors,
                92,
                72,
                38,
                [35, 38],
            ),
        ]
        .map(
            |(name, colors, pattern, speed, intensity, angle, focus)| Self {
                version: RECIPE_VERSION,
                name: name.into(),
                colors,
                pattern,
                motion: Drift,
                speed,
                intensity,
                angle,
                focus,
            },
        )
    }

    pub(super) fn presets() -> [Self; 4] {
        [
            Self {
                version: RECIPE_VERSION,
                name: "蓝潮".into(),
                colors: [[8, 23, 48], [24, 92, 152], [101, 223, 215]],
                pattern: DynamicPattern::Ribbons,
                motion: BackgroundMotion::Drift,
                speed: 100,
                intensity: 74,
                angle: 333,
                focus: [55, 43],
            },
            Self {
                version: RECIPE_VERSION,
                name: "暮金".into(),
                colors: [[40, 21, 46], [157, 66, 59], [242, 184, 103]],
                pattern: DynamicPattern::Ripple,
                motion: BackgroundMotion::Wave,
                speed: 92,
                intensity: 68,
                angle: 18,
                focus: [37, 64],
            },
            Self {
                version: RECIPE_VERSION,
                name: "极夜".into(),
                colors: [[9, 15, 39], [74, 54, 138], [157, 214, 242]],
                pattern: DynamicPattern::Curtain,
                motion: BackgroundMotion::Aurora,
                speed: 74,
                intensity: 82,
                angle: 23,
                focus: [53, 45],
            },
            Self {
                version: RECIPE_VERSION,
                name: "冰湖".into(),
                colors: [[17, 40, 63], [49, 119, 152], [168, 226, 229]],
                pattern: DynamicPattern::Halo,
                motion: BackgroundMotion::Drift,
                speed: 66,
                intensity: 55,
                angle: 306,
                focus: [63, 31],
            },
        ]
    }

    pub(super) fn is_valid(&self) -> bool {
        self.version == RECIPE_VERSION
            && !self.name.trim().is_empty()
            && self.name.chars().count() <= 40
            && !self.name.chars().any(char::is_control)
            && (10..=300).contains(&self.speed)
            && self.intensity <= 100
            && self.angle < 360
            && self.focus.iter().all(|&position| position <= 100)
    }

    pub(super) fn read(path: &Path) -> Result<Self> {
        let file = File::open(path).context("无法打开动态配方")?;
        ensure!(file.metadata()?.is_file(), "动态配方需为 JSON 文件");
        let mut bytes = Vec::new();
        file.take(MAX_RECIPE_BYTES + 1)
            .read_to_end(&mut bytes)
            .context("无法读取动态配方")?;
        ensure!(bytes.len() as u64 <= MAX_RECIPE_BYTES, "动态配方超过 8 KiB");
        let mut recipe: Self = serde_json::from_slice(&bytes).context("动态配方 JSON 格式无效")?;
        ensure!(recipe.is_valid(), "动态配方的版本、名称或参数无效");
        recipe.name = recipe.name.trim().to_owned();
        Ok(recipe)
    }

    pub(super) fn sample(&self, u: f32, v: f32) -> [u8; 3] {
        let aspect = 16.0 / 9.0;
        if matches!(
            self.pattern,
            DynamicPattern::Grid | DynamicPattern::Dots | DynamicPattern::Meteors
        ) {
            // Keep fine lines and short tails visible when thumbnails are sampled down.
            let samples = [
                (-0.0026, -0.0035),
                (0.0026, -0.0035),
                (-0.0026, 0.0035),
                (0.0026, 0.0035),
            ]
            .map(|(x, y)| self.sample_in(u + x, v + y, aspect));
            return std::array::from_fn(|channel| {
                let sum: u16 = samples
                    .iter()
                    .map(|sample| u16::from(sample[channel]))
                    .sum();
                ((sum + 2) / 4) as u8
            });
        }
        self.sample_in(u, v, aspect)
    }

    pub(super) fn sample_in(&self, u: f32, v: f32, aspect: f32) -> [u8; 3] {
        match self.pattern {
            DynamicPattern::Grid => return self.grid(u, v, aspect, false),
            DynamicPattern::Dots => return self.grid(u, v, aspect, true),
            DynamicPattern::Meteors => {
                let mut color = self.meteor_base(u, v, aspect);
                for layer in 0..2 {
                    let [r, g, b, a] = self.meteor_layer(u, v, aspect, layer);
                    let opacity = f32::from(a) / 255.0;
                    color = std::array::from_fn(|channel| {
                        mix(color[channel], [r, g, b][channel], opacity)
                    });
                }
                return color;
            }
            _ => {}
        }
        let (sin, cos) = f32::from(self.angle).to_radians().sin_cos();
        let u = u - f32::from(self.focus[0]) * 0.01;
        let v = v - f32::from(self.focus[1]) * 0.01;
        let x = cos * u + sin * v;
        let y = -sin * u + cos * v;

        // Layer the broad gradients and fine highlights before mapping to the palette.
        let soft_band = |distance: f32, width: f32| {
            let relative = distance / width;
            (-relative * relative).exp()
        };
        let level = match self.pattern {
            DynamicPattern::Ribbons => {
                let bend = 0.13 * (x * 6.0).sin() + 0.045 * (x * 11.0 + 0.8).cos();
                let wide = soft_band(y - bend + 0.04, 0.16);
                let edge = soft_band(y - bend - 0.078, 0.032);
                let echo = soft_band(y + bend * 0.36 + 0.24, 0.075);
                0.12 + 0.50 * wide + 0.26 * edge + 0.19 * echo + 0.11 * x
            }
            DynamicPattern::Curtain => {
                let shift = 0.11 * (y * 6.5 + x * 2.0).sin();
                let pleat = (24.0 * (x - shift)).cos() * 0.5 + 0.5;
                let veil = soft_band(x - shift, 0.39) * soft_band(y + 0.07, 0.52);
                let glint = soft_band(y + shift * 0.55 + 0.19, 0.11);
                0.09 + veil * (0.30 + 0.21 * pleat) + 0.27 * glint
            }
            DynamicPattern::Ripple => {
                let distance = (x * 1.17 + y * 0.32).hypot(y * 1.74);
                let rings = (distance * 21.0 - 0.4).cos() * 0.5 + 0.5;
                let glow = soft_band(distance - 0.16, 0.67);
                0.12 + glow * (0.51 + 0.20 * rings * rings) + 0.14 * x - 0.12 * y
            }
            DynamicPattern::Halo => {
                let core = soft_band((x * 0.74).hypot(y * 1.55), 0.36);
                let rim = soft_band((x * 0.95).hypot(y * 1.46) - 0.31, 0.14);
                let reflection = soft_band(y + 0.46 + x * 0.13, 0.14);
                0.11 + 0.68 * core + 0.20 * rim + 0.19 * reflection
            }
            DynamicPattern::Grid | DynamicPattern::Dots | DynamicPattern::Meteors => unreachable!(),
        };
        self.palette(level)
    }

    fn grid(&self, u: f32, v: f32, aspect: f32, dots: bool) -> [u8; 3] {
        let [width, height] = canvas_scale(aspect);
        let [x, y] = self.rotated((u - 0.5) * width, (v - 0.5) * height);
        let focal_x = (u - f32::from(self.focus[0]) * 0.01) * width;
        let focal_y = (v - f32::from(self.focus[1]) * 0.01) * height;
        let glow = fade(focal_x.hypot(focal_y), 0.06, 1.12);

        let pitch = if dots { 0.082 } else { 0.098 };
        let dx = wrapped_distance(x, pitch).abs();
        let dy = wrapped_distance(y, pitch).abs();
        let field = if dots {
            let radius = 0.0048 + 0.0025 * glow;
            let dot = fade(dx.hypot(dy), radius - 0.001, radius + 0.0025);
            dot * (0.19 + 0.17 * glow)
        } else {
            let line = fade(dx.min(dy), 0.0007, 0.0027);
            let major_distance = wrapped_distance(x, pitch * 4.0)
                .abs()
                .min(wrapped_distance(y, pitch * 4.0).abs());
            let major = fade(major_distance, 0.001, 0.0031);
            line * (0.11 + 0.15 * glow) + major * (0.055 + 0.078 * glow)
        };
        self.palette(0.018 + 0.105 * glow + field)
    }

    pub(super) fn meteor_base(&self, u: f32, v: f32, aspect: f32) -> [u8; 3] {
        let [width, height] = canvas_scale(aspect);
        let fx = (u - f32::from(self.focus[0]) * 0.01) * width;
        let fy = (v - f32::from(self.focus[1]) * 0.01) * height;
        let light = fade((fx * 0.8).hypot(fy * 1.3), 0.05, 1.0) * 0.16;
        let mut star: f32 = 0.0;
        for (x, y, strength) in [
            (0.09, 0.15, 0.27),
            (0.22, 0.43, 0.31),
            (0.32, 0.26, 0.23),
            (0.41, 0.73, 0.35),
            (0.47, 0.10, 0.24),
            (0.55, 0.52, 0.23),
            (0.66, 0.24, 0.32),
            (0.71, 0.68, 0.29),
            (0.83, 0.46, 0.25),
            (0.91, 0.16, 0.28),
            (0.93, 0.86, 0.35),
            (0.15, 0.88, 0.25),
        ] {
            let dx = ((u - x) * width).abs();
            let dy = ((v - y) * height).abs();
            if dx < 0.0045 && dy < 0.0045 {
                star = star.max(strength * fade(dx.hypot(dy), 0.0008, 0.0045));
            }
        }
        self.palette(0.016 + light + star)
    }

    pub(super) fn meteor_layer(&self, u: f32, v: f32, aspect: f32, layer: usize) -> [u8; 4] {
        const DISTANT: [(f32, f32, f32, f32); 5] = [
            (0.11, 0.25, 0.073, 0.66),
            (0.28, 0.70, 0.088, 0.64),
            (0.43, 0.18, 0.056, 0.58),
            (0.73, 0.85, 0.092, 0.71),
            (0.90, 0.37, 0.071, 0.62),
        ];
        const NEAR: [(f32, f32, f32, f32); 4] = [
            (0.18, 0.55, 0.167, 0.90),
            (0.48, 0.35, 0.186, 0.98),
            (0.68, 0.60, 0.143, 0.84),
            (0.86, 0.14, 0.178, 0.94),
        ];
        let (seeds, radius, luma) = match layer {
            0 => (DISTANT.as_slice(), 0.0020, 0.60),
            _ => (NEAR.as_slice(), 0.0035, 0.95),
        };
        let [width, height] = canvas_scale(aspect);
        let mut opacity: f32 = 0.0;
        for &(x, y, length, brightness) in seeds {
            let dx = wrapped_distance(u - x, 1.0) * width;
            let dy = wrapped_distance(v - y, 1.0) * height;
            if dx.abs().max(dy.abs()) > length + 0.012 {
                continue;
            }
            let [along, across] = self.rotated(dx, dy);
            if along < -length || along > 0.012 || across.abs() > 0.012 {
                continue;
            }
            let taper = (1.0 + along / length).clamp(0.0, 1.0);
            let core = fade(across.abs(), 0.00045, radius * (0.65 + 0.35 * taper));
            let tail = taper * taper * core * fade(along, 0.0, radius);
            let head = fade(along.hypot(across), radius * 0.48, radius * 1.50);
            let bloom = fade(across.abs(), radius, radius * 3.0)
                * 0.18
                * taper
                * fade(along, 0.0, radius * 2.0);
            opacity = opacity.max(brightness * (tail * 0.83 + bloom).max(head));
        }
        let [r, g, b] = self.palette(luma);
        [r, g, b, (opacity.clamp(0.0, 1.0) * 255.0).round() as u8]
    }

    fn rotated(&self, x: f32, y: f32) -> [f32; 2] {
        let (sin, cos) = f32::from(self.angle).to_radians().sin_cos();
        [cos * x + sin * y, -sin * x + cos * y]
    }

    fn palette(&self, level: f32) -> [u8; 3] {
        let level = level.clamp(0.0, 1.0) * 2.0;
        let left = (level as usize).min(1);
        let blend = level - left as f32;
        std::array::from_fn(|channel| {
            mix(
                self.colors[left][channel],
                self.colors[left + 1][channel],
                blend,
            )
        })
    }
}

fn canvas_scale(aspect: f32) -> [f32; 2] {
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        16.0 / 9.0
    };
    [aspect.max(1.0), aspect.recip().max(1.0)]
}

fn wrapped_distance(value: f32, period: f32) -> f32 {
    (value + period * 0.5).rem_euclid(period) - period * 0.5
}

fn fade(value: f32, start: f32, end: f32) -> f32 {
    let t = ((value - start) / (end - start)).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

fn mix(from: u8, to: u8, blend: f32) -> u8 {
    let from = f32::from(from);
    (from + (f32::from(to) - from) * blend).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipes_round_trip_and_reject_extra_fields() {
        let recipe = DynamicRecipe::presets()[0].clone();
        let mut json = serde_json::to_value(&recipe).unwrap();
        assert_eq!(
            serde_json::from_value::<DynamicRecipe>(json.clone()).unwrap(),
            recipe
        );
        json["script"] = serde_json::Value::String("anything".into());
        assert!(serde_json::from_value::<DynamicRecipe>(json).is_err());
    }

    #[test]
    fn import_rejects_out_of_range_values_and_oversized_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recipe.json");
        let mut recipe = DynamicRecipe::presets()[1].clone();
        std::fs::write(&path, serde_json::to_vec(&recipe).unwrap()).unwrap();
        assert_eq!(DynamicRecipe::read(&path).unwrap(), recipe);

        recipe.speed = 301;
        std::fs::write(&path, serde_json::to_vec(&recipe).unwrap()).unwrap();
        assert!(DynamicRecipe::read(&path).is_err());
        std::fs::write(&path, vec![b' '; MAX_RECIPE_BYTES as usize + 1]).unwrap();
        assert!(DynamicRecipe::read(&path).is_err());
    }

    #[test]
    fn each_preset_has_a_palette_and_focus_changes_its_image() {
        for mut recipe in DynamicRecipe::presets() {
            assert!(recipe.is_valid());
            let focus = recipe.focus.map(|value| f32::from(value) / 100.0);
            let original = recipe.sample(focus[0], focus[1]);
            assert_ne!(original, recipe.sample(0.02, 0.98));
            recipe.focus = [0, 0];
            assert_ne!(original, recipe.sample(focus[0], focus[1]));
        }
    }

    #[test]
    fn modern_presets_round_trip_and_keep_dots_round_in_wide_canvases() {
        let presets = DynamicRecipe::modern_presets();
        assert_eq!(
            presets.map(|recipe| recipe.pattern),
            [
                DynamicPattern::Grid,
                DynamicPattern::Grid,
                DynamicPattern::Dots,
                DynamicPattern::Dots,
                DynamicPattern::Meteors,
                DynamicPattern::Meteors,
            ]
        );
        for recipe in DynamicRecipe::modern_presets() {
            assert!(recipe.is_valid());
            assert_eq!(
                serde_json::from_slice::<DynamicRecipe>(&serde_json::to_vec(&recipe).unwrap())
                    .unwrap(),
                recipe
            );
        }

        let mut dots = DynamicRecipe::modern_presets()[2].clone();
        dots.focus = [50, 50];
        let square = dots.sample_in(0.582, 0.5, 1.0);
        let wide = dots.sample_in(0.541, 0.5, 2.0);
        assert!(
            square
                .into_iter()
                .zip(wide)
                .all(|(a, b)| a.abs_diff(b) <= 1)
        );
        assert_ne!(wide, dots.sample_in(0.527, 0.5, 2.0));
    }

    #[test]
    fn meteor_layers_wrap_and_place_tails_behind_their_heads() {
        let mut recipe = DynamicRecipe::modern_presets()[4].clone();
        recipe.angle = 0;
        let layer = |u, v, depth| recipe.meteor_layer(u, v, 2.0, depth);

        let tail = layer(0.13, 0.55, 1)[3];
        let head = layer(0.18, 0.55, 1)[3];
        assert!(head > tail && tail > 0);
        assert_eq!(layer(0.20, 0.55, 1)[3], 0);
        assert_ne!(layer(0.18, 0.55, 0), layer(0.18, 0.55, 1));
        assert_eq!(layer(0.13, 0.55, 1), layer(1.13, 0.55, 1));
    }
}
