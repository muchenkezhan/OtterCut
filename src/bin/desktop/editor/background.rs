use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::dynamic_recipe::DynamicRecipe;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct BackgroundSettings {
    pub source: BackgroundSource,
    pub motion: BackgroundMotion,
    pub blur: u8,
    pub inset: u8,
    pub radius: u8,
    pub shadow: BackgroundShadow,
    pub frame: BackgroundFrame,
}

impl Default for BackgroundSettings {
    fn default() -> Self {
        Self {
            source: BackgroundSource::Off,
            motion: BackgroundMotion::None,
            blur: 0,
            inset: 10,
            radius: 18,
            shadow: BackgroundShadow::Soft,
            frame: BackgroundFrame::None,
        }
    }
}

impl BackgroundSettings {
    pub(super) fn enabled(&self) -> bool {
        !matches!(self.source, BackgroundSource::Off)
    }

    pub(super) fn is_valid(&self) -> bool {
        self.blur <= 100
            && self.inset <= 40
            && self.radius <= 80
            && !matches!(&self.source, BackgroundSource::Image(path) if path.as_os_str().is_empty())
            && !matches!(&self.source, BackgroundSource::Dynamic(recipe) if !recipe.is_valid())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BackgroundSource {
    Off,
    Solid([u8; 3]),
    Gradient(BackdropPreset),
    Wallpaper(BackdropPreset),
    Dynamic(DynamicRecipe),
    Image(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BackdropPreset {
    Tide,
    Dusk,
    Glacier,
    Ink,
    Copper,
    Slate,
}

impl BackdropPreset {
    pub(super) const ALL: [Self; 6] = [
        Self::Tide,
        Self::Dusk,
        Self::Glacier,
        Self::Ink,
        Self::Copper,
        Self::Slate,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Tide => "潮汐",
            Self::Dusk => "暮色",
            Self::Glacier => "冰川",
            Self::Ink => "墨夜",
            Self::Copper => "铜砂",
            Self::Slate => "青岩",
        }
    }

    pub(super) fn sample(self, u: f32, v: f32, patterned: bool) -> [u8; 3] {
        let (colors, phase): ([[u8; 3]; 3], f32) = match self {
            Self::Tide => ([[10, 35, 59], [16, 100, 143], [84, 189, 182]], 0.4),
            Self::Dusk => ([[48, 32, 79], [141, 72, 123], [236, 152, 127]], 1.1),
            Self::Glacier => ([[28, 57, 101], [77, 129, 189], [153, 217, 226]], 1.9),
            Self::Ink => ([[21, 21, 36], [61, 56, 102], [112, 97, 166]], 2.7),
            Self::Copper => ([[63, 35, 40], [145, 71, 54], [229, 161, 109]], 3.4),
            Self::Slate => ([[27, 40, 47], [68, 91, 103], [154, 178, 183]], 4.2),
        };
        let diagonal = u * 0.68 + v * 0.32;
        let t = if patterned {
            let flow = (u * 7.1 + v * 4.8 + phase + (v * 6.4 + phase).sin()).sin();
            let folds = (u * 13.2 - v * 7.3 + flow * 1.5).cos();
            diagonal * 0.72 + 0.22 + 0.18 * flow + 0.08 * folds
        } else {
            diagonal
        };
        let t = t.clamp(0.0, 1.0) * 2.0;
        let left = (t as usize).min(1);
        let amount = t - left as f32;
        std::array::from_fn(|channel| {
            let a = f32::from(colors[left][channel]);
            let b = f32::from(colors[left + 1][channel]);
            (a + (b - a) * amount).round() as u8
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BackgroundMotion {
    None,
    Drift,
    Aurora,
    Wave,
}

impl BackgroundMotion {
    pub(super) const ALL: [Self; 4] = [Self::None, Self::Drift, Self::Aurora, Self::Wave];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::None => "无",
            Self::Drift => "漂移",
            Self::Aurora => "极光",
            Self::Wave => "波浪",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BackgroundShadow {
    None,
    Soft,
    Medium,
    Strong,
}

impl BackgroundShadow {
    pub(super) const ALL: [Self; 4] = [Self::None, Self::Soft, Self::Medium, Self::Strong];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::None => "无",
            Self::Soft => "轻",
            Self::Medium => "中",
            Self::Strong => "强",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BackgroundFrame {
    None,
    Window,
    Laptop,
    Phone,
    Monitor,
}

impl BackgroundFrame {
    pub(super) const ALL: [Self; 5] = [
        Self::None,
        Self::Window,
        Self::Laptop,
        Self::Phone,
        Self::Monitor,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::None => "无",
            Self::Window => "窗口",
            Self::Laptop => "笔记本",
            Self::Phone => "手机",
            Self::Monitor => "显示器",
        }
    }
}
