use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

const SETTINGS_VERSION: u32 = 1;
const MAX_SETTINGS_BYTES: u64 = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Pane {
    Materials,
    Preview,
    Inspector,
    Timeline,
}

impl Pane {
    pub(super) const ALL: [Self; 4] = [
        Self::Materials,
        Self::Preview,
        Self::Inspector,
        Self::Timeline,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Materials => "素材",
            Self::Preview => "预览",
            Self::Inspector => "属性",
            Self::Timeline => "时间线",
        }
    }

    fn bit(self) -> u8 {
        1 << match self {
            Self::Materials => 0,
            Self::Preview => 1,
            Self::Inspector => 2,
            Self::Timeline => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Placement {
    Left,
    Right,
    Top,
    Bottom,
    Swap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DockNode {
    Pane(Pane),
    Split {
        axis: Axis,
        ratio: f32,
        first: Box<DockNode>,
        second: Box<DockNode>,
    },
}

impl DockNode {
    pub(super) const MIN_RATIO: f32 = 0.05;
    pub(super) const MAX_RATIO: f32 = 0.95;

    fn split(axis: Axis, ratio: f32, first: Self, second: Self) -> Self {
        Self::Split {
            axis,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    fn without(self, removed: Pane) -> Option<Self> {
        match self {
            Self::Pane(pane) => (pane != removed).then_some(Self::Pane(pane)),
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => match (first.without(removed), second.without(removed)) {
                (Some(first), Some(second)) => Some(Self::split(axis, ratio, first, second)),
                (Some(remaining), None) | (None, Some(remaining)) => Some(remaining),
                (None, None) => None,
            },
        }
    }

    fn split_at(&mut self, target: Pane, source: Pane, axis: Axis, source_first: bool) -> bool {
        match self {
            Self::Pane(pane) if *pane == target => {
                let (first, second) = if source_first {
                    (source, target)
                } else {
                    (target, source)
                };
                *self = Self::split(axis, 0.5, Self::Pane(first), Self::Pane(second));
                true
            }
            Self::Pane(_) => false,
            Self::Split { first, second, .. } => {
                first.split_at(target, source, axis, source_first)
                    || second.split_at(target, source, axis, source_first)
            }
        }
    }

    fn swap(&mut self, source: Pane, target: Pane) {
        match self {
            Self::Pane(pane) if *pane == source => *pane = target,
            Self::Pane(pane) if *pane == target => *pane = source,
            Self::Pane(_) => {}
            Self::Split { first, second, .. } => {
                first.swap(source, target);
                second.swap(source, target);
            }
        }
    }

    fn valid(&self) -> bool {
        let expected = (1 << Pane::ALL.len()) - 1;
        self.pane_bits(0) == Some(expected)
    }

    fn pane_bits(&self, depth: usize) -> Option<u8> {
        match self {
            Self::Pane(pane) => Some(pane.bit()),
            Self::Split {
                ratio,
                first,
                second,
                ..
            } if depth + 1 < Pane::ALL.len()
                && (Self::MIN_RATIO..=Self::MAX_RATIO).contains(ratio) =>
            {
                let first = first.pane_bits(depth + 1)?;
                let second = second.pane_bits(depth + 1)?;
                (first & second == 0).then_some(first | second)
            }
            Self::Split { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Preset {
    Editing,
    Preview,
    Timeline,
    Custom,
}

impl Preset {
    pub(super) const ALL: [Self; 4] = [Self::Editing, Self::Preview, Self::Timeline, Self::Custom];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Editing => "默认剪辑",
            Self::Preview => "预览优先",
            Self::Timeline => "时间线优先",
            Self::Custom => "自定义",
        }
    }

    fn layout(self) -> Option<DockNode> {
        let (top, left, right) = match self {
            Self::Editing => (0.56, 0.23, 0.17),
            Self::Preview => (0.72, 0.17, 0.14),
            Self::Timeline => (0.40, 0.23, 0.17),
            Self::Custom => return None,
        };
        Some(DockNode::split(
            Axis::Vertical,
            top,
            DockNode::split(
                Axis::Horizontal,
                left,
                DockNode::Pane(Pane::Materials),
                DockNode::split(
                    Axis::Horizontal,
                    1.0 - right / (1.0 - left),
                    DockNode::Pane(Pane::Preview),
                    DockNode::Pane(Pane::Inspector),
                ),
            ),
            DockNode::Pane(Pane::Timeline),
        ))
    }
}

pub(super) struct DockState {
    pub(super) root: DockNode,
    pub(super) active: Preset,
    custom: Option<DockNode>,
}

impl Default for DockState {
    fn default() -> Self {
        Self {
            root: Preset::Editing.layout().unwrap(),
            active: Preset::Editing,
            custom: None,
        }
    }
}

impl DockState {
    pub(super) fn load() -> Self {
        preference_path()
            .and_then(|path| Self::load_from(&path))
            .unwrap_or_default()
    }

    pub(super) fn save(&self) -> Result<()> {
        if let Some(path) = preference_path() {
            self.save_to(&path)?;
        }
        Ok(())
    }

    pub(super) fn can_select(&self, preset: Preset) -> bool {
        preset != Preset::Custom || self.custom.is_some()
    }

    pub(super) fn select(&mut self, preset: Preset) -> bool {
        let layout = match preset {
            Preset::Custom => self.custom.clone(),
            preset => preset.layout(),
        };
        let Some(layout) = layout else {
            return false;
        };
        if self.active == preset && self.root == layout {
            return false;
        }
        self.active = preset;
        self.root = layout;
        true
    }

    pub(super) fn move_pane(&mut self, source: Pane, target: Pane, placement: Placement) -> bool {
        if source == target || !self.root.valid() {
            return false;
        }
        let mut root = self.root.clone();
        let (axis, source_first) = match placement {
            Placement::Left => (Axis::Horizontal, true),
            Placement::Right => (Axis::Horizontal, false),
            Placement::Top => (Axis::Vertical, true),
            Placement::Bottom => (Axis::Vertical, false),
            Placement::Swap => {
                root.swap(source, target);
                self.root = root;
                return self.commit_custom();
            }
        };
        let Some(mut root) = root.without(source) else {
            return false;
        };
        if !root.split_at(target, source, axis, source_first) || self.root == root {
            return false;
        }
        self.root = root;
        self.commit_custom()
    }

    /// Records a completed splitter resize or a panel arrangement change.
    pub(super) fn commit_custom(&mut self) -> bool {
        if !self.root.valid()
            || match self.active {
                Preset::Custom => self.custom.as_ref() == Some(&self.root),
                preset => preset.layout().as_ref() == Some(&self.root),
            }
        {
            return false;
        }
        self.custom = Some(self.root.clone());
        self.active = Preset::Custom;
        true
    }

    fn load_from(path: &Path) -> Option<Self> {
        let mut bytes = Vec::new();
        File::open(path)
            .ok()?
            .take(MAX_SETTINGS_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 > MAX_SETTINGS_BYTES {
            return None;
        }
        let preferences: Preferences = serde_json::from_slice(&bytes).ok()?;
        if preferences.version != SETTINGS_VERSION {
            return None;
        }
        let custom = preferences.custom.filter(DockNode::valid);
        let mut state = Self {
            custom,
            ..Self::default()
        };
        state.select(preferences.active);
        Some(state)
    }

    fn save_to(&self, path: &Path) -> Result<()> {
        let directory = path.parent().context("工作区设置目录不可用")?;
        ensure!(self.root.valid(), "工作区布局无效");
        fs::create_dir_all(directory).context("无法创建工作区设置目录")?;
        let mut staged =
            tempfile::NamedTempFile::new_in(directory).context("无法创建工作区设置临时文件")?;
        let preferences = Preferences {
            version: SETTINGS_VERSION,
            active: self.active,
            custom: self.custom.clone(),
        };
        serde_json::to_writer(&mut staged, &preferences).context("无法写入工作区设置")?;
        staged.write_all(b"\n")?;
        staged.as_file().sync_all()?;
        staged
            .persist(path)
            .map_err(|error| error.error)
            .context("无法保存工作区设置")?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct Preferences {
    version: u32,
    active: Preset,
    custom: Option<DockNode>,
}

fn preference_path() -> Option<PathBuf> {
    Some(takeseam::config_directory()?.join("workspace.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_panels_never_duplicates_or_loses_one() {
        for placement in [
            Placement::Left,
            Placement::Right,
            Placement::Top,
            Placement::Bottom,
            Placement::Swap,
        ] {
            for source in Pane::ALL {
                for target in Pane::ALL {
                    let mut state = DockState::default();
                    assert!(!state.can_select(Preset::Custom));
                    let before = state.root.clone();
                    let moved = state.move_pane(source, target, placement);
                    assert!(state.root.valid());
                    assert_eq!(moved, state.root != before);
                    assert_eq!(state.active == Preset::Custom, moved);
                }
            }
        }
    }

    #[test]
    fn preset_switch_preserves_and_recalls_last_manual_layout() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        let mut state = DockState::default();
        assert!(state.move_pane(Pane::Inspector, Pane::Materials, Placement::Swap));
        let DockNode::Split { ratio, .. } = &mut state.root else {
            unreachable!()
        };
        *ratio = 0.48;
        assert!(state.commit_custom());
        assert!(!state.commit_custom());
        let custom = state.root.clone();
        assert!(state.select(Preset::Preview));
        assert!(state.select(Preset::Timeline));
        state.save_to(&path).unwrap();

        let mut reopened = DockState::load_from(&path).unwrap();
        assert_eq!(reopened.active, Preset::Timeline);
        assert!(reopened.select(Preset::Custom));
        assert_eq!(reopened.root, custom);
        reopened.save_to(&path).unwrap();
        let reopened = DockState::load_from(&path).unwrap();
        assert_eq!(reopened.active, Preset::Custom);
        assert_eq!(reopened.root, custom);
    }

    #[test]
    fn invalid_or_oversized_preferences_cannot_hide_panels() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        let invalid = Preferences {
            version: SETTINGS_VERSION,
            active: Preset::Custom,
            custom: Some(DockNode::split(
                Axis::Vertical,
                0.98,
                DockNode::Pane(Pane::Timeline),
                DockNode::Pane(Pane::Timeline),
            )),
        };
        fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        let state = DockState::load_from(&path).unwrap();
        assert_eq!(state.active, Preset::Editing);
        assert!(state.root.valid());
        assert!(!state.can_select(Preset::Custom));
        fs::write(&path, vec![b' '; MAX_SETTINGS_BYTES as usize + 1]).unwrap();
        assert!(DockState::load_from(&path).is_none());
    }
}
