use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CanvasFormat {
    #[default]
    Original,
    Preset(AspectRatio),
    Custom {
        width: u32,
        height: u32,
    },
}

impl CanvasFormat {
    pub(super) const MIN_CUSTOM_DIMENSION: u32 = 2;
    pub(super) const MAX_CUSTOM_DIMENSION: u32 = 4096;

    pub(super) fn dimensions(self) -> Option<(u32, u32)> {
        match self {
            Self::Original => None,
            Self::Preset(ratio) => Some(ratio.dimensions()),
            Self::Custom { width, height } => Some((width, height)),
        }
    }

    pub(super) fn is_original(self) -> bool {
        matches!(self, Self::Original)
    }

    pub(super) fn is_valid(self) -> bool {
        match self {
            Self::Custom { width, height } => [width, height].into_iter().all(|dimension| {
                (Self::MIN_CUSTOM_DIMENSION..=Self::MAX_CUSTOM_DIMENSION).contains(&dimension)
                    && dimension.is_multiple_of(2)
            }),
            _ => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AspectRatio {
    Landscape,
    Standard,
    CinemaWide,
    Panorama,
    Widescreen,
    Portrait,
    PortraitStandard,
    Phone58,
    Square,
    PortraitTall,
}

impl AspectRatio {
    pub(super) const ALL: [Self; 10] = [
        Self::Landscape,
        Self::Standard,
        Self::CinemaWide,
        Self::Panorama,
        Self::Widescreen,
        Self::Portrait,
        Self::PortraitStandard,
        Self::Phone58,
        Self::Square,
        Self::PortraitTall,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Landscape => "16:9",
            Self::Standard => "4:3",
            Self::CinemaWide => "2.35:1",
            Self::Panorama => "2:1",
            Self::Widescreen => "1.85:1",
            Self::Portrait => "9:16",
            Self::PortraitStandard => "3:4",
            Self::Phone58 => "5.8寸",
            Self::Square => "1:1",
            Self::PortraitTall => "1:2",
        }
    }

    pub(super) fn dimensions(self) -> (u32, u32) {
        match self {
            Self::Landscape => (1920, 1080),
            Self::Standard => (1440, 1080),
            Self::CinemaWide => (1880, 800),
            Self::Panorama => (1920, 960),
            Self::Widescreen => (1850, 1000),
            Self::Portrait => (1080, 1920),
            Self::PortraitStandard => (1080, 1440),
            Self::Phone58 => (750, 1624),
            Self::Square => (1080, 1080),
            Self::PortraitTall => (960, 1920),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_have_the_exact_ratios_and_even_dimensions() {
        let ratios = [
            (16, 9),
            (4, 3),
            (47, 20),
            (2, 1),
            (37, 20),
            (9, 16),
            (3, 4),
            (375, 812),
            (1, 1),
            (1, 2),
        ];
        for (ratio, (w, h)) in AspectRatio::ALL.into_iter().zip(ratios) {
            let (width, height) = CanvasFormat::Preset(ratio).dimensions().unwrap();
            assert_eq!(width * h, height * w, "{}", ratio.label());
            assert!(CanvasFormat::Custom { width, height }.is_valid());
        }
    }

    #[test]
    fn only_even_custom_dimensions_in_range_are_valid() {
        assert!(CanvasFormat::default().is_original());
        assert_eq!(CanvasFormat::Original.dimensions(), None);
        assert!(CanvasFormat::Original.is_valid());

        let valid = CanvasFormat::Custom {
            width: 4096,
            height: 2,
        };
        assert!(valid.is_valid());
        assert_eq!(valid.dimensions(), Some((4096, 2)));
        for (width, height) in [(0, 1080), (1920, 1081), (4098, 1080)] {
            assert!(!CanvasFormat::Custom { width, height }.is_valid());
        }
    }
}
