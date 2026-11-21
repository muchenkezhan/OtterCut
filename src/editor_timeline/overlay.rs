//! Generated captions and on-screen text stored in the timeline.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{Clip, Timeline, TimelineError, Track, TrackId, TrackKind};
use super::{allocate_id, allocate_track_id, default_track_insertion};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextPlacement {
    Top,
    Center,
    Bottom,
}

/// The text's center, measured from the top-left corner in 1/10,000ths of the canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextPosition {
    pub x: u16,
    pub y: u16,
}

impl TextPosition {
    pub const MAX: u16 = 10_000;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAppearance {
    #[default]
    Outline,
    Plain,
    DarkBox,
    LightBox,
    Gold,
    Shadow,
}

/// Shared colors in RGBA and effect sizes at a 1080-pixel canvas height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextPaint {
    pub foreground: [u8; 4],
    pub outline: [u8; 4],
    pub outline_width: u8,
    pub shadow: u8,
    pub background: Option<[u8; 4]>,
}

impl TextPaint {
    pub const fn background_padding(self) -> u8 {
        if self.background.is_some() { 9 } else { 0 }
    }
}

impl TextAppearance {
    pub const ALL: [Self; 6] = [
        Self::Outline,
        Self::Plain,
        Self::DarkBox,
        Self::LightBox,
        Self::Gold,
        Self::Shadow,
    ];

    pub const fn paint(self) -> TextPaint {
        let white = [255, 255, 255, 255];
        let dark = [17, 17, 17, 255];
        match self {
            Self::Outline => TextPaint {
                foreground: white,
                outline: dark,
                outline_width: 3,
                shadow: 0,
                background: None,
            },
            Self::Plain => TextPaint {
                foreground: white,
                outline: dark,
                outline_width: 0,
                shadow: 0,
                background: None,
            },
            Self::DarkBox => TextPaint {
                foreground: white,
                outline: dark,
                outline_width: 0,
                shadow: 0,
                background: Some([12, 14, 18, 208]),
            },
            Self::LightBox => TextPaint {
                foreground: [23, 27, 35, 255],
                outline: dark,
                outline_width: 0,
                shadow: 0,
                background: Some([247, 247, 243, 238]),
            },
            Self::Gold => TextPaint {
                foreground: [255, 225, 132, 255],
                outline: [25, 18, 12, 255],
                outline_width: 3,
                shadow: 0,
                background: None,
            },
            Self::Shadow => TextPaint {
                foreground: white,
                outline: [10, 10, 14, 192],
                outline_width: 0,
                shadow: 5,
                background: None,
            },
        }
    }

    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextStyle {
    /// Font size in pixels at a 1080-pixel canvas height.
    pub size: u16,
    pub placement: TextPlacement,
    pub bold: bool,
    /// An explicit center overrides the preset placement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<TextPosition>,
    #[serde(default, skip_serializing_if = "TextAppearance::is_default")]
    pub appearance: TextAppearance,
}

impl TextStyle {
    pub const MIN_SIZE: u16 = 12;
    pub const MAX_SIZE: u16 = 144;
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            size: 48,
            placement: TextPlacement::Bottom,
            bold: false,
            position: None,
            appearance: TextAppearance::default(),
        }
    }
}

/// Updates only the specified style fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextStyleChanges {
    pub size: Option<u16>,
    pub bold: Option<bool>,
    /// Sets the preset placement and clears the custom position unless `position` is supplied.
    pub placement: Option<TextPlacement>,
    /// Sets a custom center that takes precedence over the preset placement.
    pub position: Option<TextPosition>,
    pub appearance: Option<TextAppearance>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayText {
    pub content: String,
    pub style: TextStyle,
}

impl OverlayText {
    pub const MAX_CHARS: usize = 4_096;

    pub fn validate(&self) -> Result<(), TimelineError> {
        if self.content.trim().is_empty()
            || self.content.chars().count() > Self::MAX_CHARS
            || self
                .content
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err(TimelineError::InvalidText);
        }
        if !(TextStyle::MIN_SIZE..=TextStyle::MAX_SIZE).contains(&self.style.size) {
            return Err(TimelineError::InvalidTextSize);
        }
        if self.style.position.is_some_and(|position| {
            position.x > TextPosition::MAX || position.y > TextPosition::MAX
        }) {
            return Err(TimelineError::InvalidTextPosition);
        }
        Ok(())
    }
}

impl Timeline {
    /// Adds generated text at a fixed position without moving existing clips.
    pub fn insert_text(
        &mut self,
        kind: TrackKind,
        at: u64,
        duration: u64,
        text: OverlayText,
    ) -> Result<(TrackId, u64), TimelineError> {
        let mut last_track_id = self.last_track_id;
        let result = self.edit(|tracks, last_id| {
            insert(
                tracks,
                last_id,
                &mut last_track_id,
                kind,
                at,
                duration,
                text,
            )
        })?;
        self.last_track_id = last_track_id;
        Ok(result)
    }

    /// Inserts a batch in input order and creates any required tracks in one undo step.
    pub fn insert_text_batch(
        &mut self,
        kind: TrackKind,
        segments: Vec<(u64, u64, OverlayText)>,
    ) -> Result<Vec<(TrackId, u64)>, TimelineError> {
        if !kind.is_overlay() {
            return Err(TimelineError::IncompatibleTrack);
        }
        let mut last_track_id = self.last_track_id;
        let result = self.edit(|tracks, last_id| {
            segments
                .into_iter()
                .map(|(at, duration, text)| {
                    insert(
                        tracks,
                        last_id,
                        &mut last_track_id,
                        kind,
                        at,
                        duration,
                        text,
                    )
                })
                .collect::<Result<_, _>>()
        })?;
        self.last_track_id = last_track_id;
        Ok(result)
    }

    pub fn update_text(&mut self, id: u64, text: OverlayText) -> Result<(), TimelineError> {
        let (track, index) = self.locate_clip(id)?;
        self.edit_track(track, |clips, _| {
            let current = clips[index]
                .text
                .as_mut()
                .ok_or(TimelineError::IncompatibleTrack)?;
            *current = text;
            Ok(())
        })
    }

    /// Applies a shared style change to selected text clips in a single undo step.
    pub fn update_text_styles(
        &mut self,
        ids: &[u64],
        changes: TextStyleChanges,
    ) -> Result<(), TimelineError> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut remaining: HashSet<_> = ids.iter().copied().collect();
        self.edit(|tracks, _| {
            for clip in tracks.iter_mut().flat_map(|track| &mut track.clips) {
                if !remaining.remove(&clip.id) {
                    continue;
                }
                let style = &mut clip
                    .text
                    .as_mut()
                    .ok_or(TimelineError::IncompatibleTrack)?
                    .style;
                style.size = changes.size.unwrap_or(style.size);
                style.bold = changes.bold.unwrap_or(style.bold);
                style.appearance = changes.appearance.unwrap_or(style.appearance);
                if let Some(placement) = changes.placement {
                    style.placement = placement;
                    style.position = changes.position;
                } else if let Some(position) = changes.position {
                    style.position = Some(position);
                }
            }
            if let Some(id) = ids.iter().find(|id| remaining.contains(id)) {
                return Err(TimelineError::ClipNotFound(*id));
            }
            Ok(())
        })
    }
}

fn insert(
    tracks: &mut Vec<Track>,
    last_id: &mut u64,
    last_track_id: &mut TrackId,
    kind: TrackKind,
    at: u64,
    duration: u64,
    text: OverlayText,
) -> Result<(TrackId, u64), TimelineError> {
    if !kind.is_overlay() {
        return Err(TimelineError::IncompatibleTrack);
    }
    if duration == 0 {
        return Err(TimelineError::InvalidSourceRange);
    }
    let end = at
        .checked_add(duration)
        .ok_or(TimelineError::TimeOutOfBounds)?;
    let id = allocate_id(last_id)?;
    let clip = Clip {
        text: Some(text),
        ..Clip::full_source(id, Default::default(), duration, at)
    };

    let existing = tracks
        .iter_mut()
        .filter(|track| track.enabled && track.kind == kind)
        .find_map(|track| {
            let index = track.clips.partition_point(|clip| clip.end() <= at);
            if track.clips.get(index).is_some_and(|clip| clip.start < end) {
                None
            } else {
                Some((track, index))
            }
        });
    if let Some((track, index)) = existing {
        track.clips.insert(index, clip);
        return Ok((track.id, id));
    }

    let track_id = allocate_track_id(last_track_id)?;
    let mut track = Track::new(track_id, kind);
    track.clips.push(clip);
    tracks.insert(default_track_insertion(tracks, kind), track);
    Ok((track_id, id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_timeline::{PRIMARY_AUDIO, PRIMARY_VIDEO};

    fn overlay(content: &str) -> OverlayText {
        OverlayText {
            content: content.into(),
            style: TextStyle::default(),
        }
    }

    #[cfg(windows)]
    #[test]
    fn legacy_styles_keep_their_placement_and_round_trip_new_appearance_and_position() {
        let legacy = serde_json::json!({
            "content": "旧字幕",
            "style": { "size": 38, "placement": "Top", "bold": true }
        });
        let mut text: OverlayText = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(text.style.placement, TextPlacement::Top);
        assert_eq!(text.style.position, None);
        assert_eq!(text.style.appearance, TextAppearance::Outline);
        assert_eq!(serde_json::to_value(&text).unwrap(), legacy);

        text.style.position = Some(TextPosition { x: 7_500, y: 4_800 });
        text.style.appearance = TextAppearance::DarkBox;
        let restored: OverlayText =
            serde_json::from_value(serde_json::to_value(&text).unwrap()).unwrap();
        assert_eq!(restored, text);
    }

    #[test]
    fn text_transforms_keep_their_history_and_reject_positions_outside_the_canvas() {
        let mut timeline = Timeline::default();
        let original = overlay("待调整字幕");
        let (_, id) = timeline
            .insert_text(TrackKind::Caption, 4, 12, original.clone())
            .unwrap();
        let transformed = OverlayText {
            content: original.content.clone(),
            style: TextStyle {
                size: 72,
                position: Some(TextPosition { x: 1_800, y: 6_200 }),
                appearance: TextAppearance::Gold,
                ..original.style
            },
        };
        timeline.update_text(id, transformed.clone()).unwrap();
        assert!(timeline.undo());
        assert_eq!(timeline.clip(id).unwrap().text.as_ref(), Some(&original));
        assert!(timeline.redo());
        assert_eq!(timeline.clip(id).unwrap().text.as_ref(), Some(&transformed));

        let mut outside = transformed.clone();
        outside.style.position = Some(TextPosition {
            x: TextPosition::MAX + 1,
            y: 5_000,
        });
        let revision = timeline.revision();
        assert_eq!(
            timeline.update_text(id, outside),
            Err(TimelineError::InvalidTextPosition)
        );
        assert_eq!(timeline.revision(), revision);
        assert_eq!(timeline.clip(id).unwrap().text.as_ref(), Some(&transformed));
    }

    #[test]
    fn applies_a_preset_to_selected_captions_on_multiple_tracks_in_one_undo_step() {
        let mut timeline = Timeline::default();
        let mut first = overlay("第一条字幕");
        first.style.size = 32;
        first.style.position = Some(TextPosition { x: 4_000, y: 7_000 });
        let mut second = overlay("第二条字幕");
        second.style.bold = true;
        second.style.placement = TextPlacement::Top;
        let (first_track, first_id) = timeline
            .insert_text(TrackKind::Caption, 0, 30, first.clone())
            .unwrap();
        let (second_track, second_id) = timeline
            .insert_text(TrackKind::Caption, 10, 30, second.clone())
            .unwrap();
        let unselected = overlay("未选中的字幕");
        let (_, unselected_id) = timeline
            .insert_text(TrackKind::Caption, 50, 30, unselected.clone())
            .unwrap();
        assert_ne!(first_track, second_track);
        let original = timeline.tracks().to_vec();

        timeline
            .update_text_styles(
                &[first_id, second_id],
                TextStyleChanges {
                    appearance: Some(TextAppearance::Gold),
                    ..TextStyleChanges::default()
                },
            )
            .unwrap();
        first.style.appearance = TextAppearance::Gold;
        second.style.appearance = TextAppearance::Gold;
        for (id, expected) in [
            (first_id, first),
            (second_id, second),
            (unselected_id, unselected),
        ] {
            assert_eq!(timeline.clip(id).unwrap().text.as_ref(), Some(&expected));
        }
        let edited = timeline.tracks().to_vec();
        assert!(timeline.undo());
        assert_eq!(timeline.tracks(), original);
        assert!(timeline.redo());
        assert_eq!(timeline.tracks(), edited);
    }

    #[test]
    fn batch_placement_clears_custom_positions_and_applies_size_and_bold() {
        let mut timeline = Timeline::default();
        let mut first = overlay("第一条");
        first.style.position = Some(TextPosition { x: 2_000, y: 8_000 });
        let (_, first_id) = timeline
            .insert_text(TrackKind::Caption, 0, 30, first)
            .unwrap();
        let (_, second_id) = timeline
            .insert_text(TrackKind::Caption, 30, 30, overlay("第二条"))
            .unwrap();
        timeline
            .update_text_styles(
                &[first_id, second_id],
                TextStyleChanges {
                    size: Some(72),
                    bold: Some(true),
                    placement: Some(TextPlacement::Center),
                    ..TextStyleChanges::default()
                },
            )
            .unwrap();
        let expected = TextStyle {
            size: 72,
            bold: true,
            placement: TextPlacement::Center,
            ..TextStyle::default()
        };
        for id in [first_id, second_id] {
            assert_eq!(
                timeline.clip(id).unwrap().text.as_ref().unwrap().style,
                expected
            );
        }
    }

    #[test]
    fn matches_the_selected_caption_centers_in_one_undo_step() {
        let mut timeline = Timeline::default();
        let mut first = overlay("第一条");
        first.style.size = 72;
        first.style.appearance = TextAppearance::Gold;
        first.style.position = Some(TextPosition { x: 2_000, y: 8_000 });
        let mut second = overlay("第二条");
        second.style.placement = TextPlacement::Top;
        second.style.bold = true;
        let unselected = overlay("未选中的字幕");
        let (_, first_id) = timeline
            .insert_text(TrackKind::Caption, 0, 30, first.clone())
            .unwrap();
        let (_, second_id) = timeline
            .insert_text(TrackKind::Caption, 30, 30, second.clone())
            .unwrap();
        let (_, unselected_id) = timeline
            .insert_text(TrackKind::Caption, 60, 30, unselected.clone())
            .unwrap();
        let original = timeline.tracks().to_vec();
        let position = Some(TextPosition { x: 6_300, y: 4_100 });

        timeline
            .update_text_styles(
                &[first_id, second_id],
                TextStyleChanges {
                    position,
                    ..TextStyleChanges::default()
                },
            )
            .unwrap();
        first.style.position = position;
        second.style.position = position;
        for (id, expected) in [
            (first_id, first),
            (second_id, second),
            (unselected_id, unselected),
        ] {
            assert_eq!(timeline.clip(id).unwrap().text.as_ref(), Some(&expected));
        }
        let edited = timeline.tracks().to_vec();
        assert!(timeline.undo());
        assert_eq!(timeline.tracks(), original);
        assert!(timeline.redo());
        assert_eq!(timeline.tracks(), edited);
    }

    #[test]
    fn invalid_batch_style_changes_leave_all_clips_and_history_intact() {
        let mut timeline = Timeline::default();
        let (_, caption_id) = timeline
            .insert_text(TrackKind::Caption, 0, 30, overlay("原字幕"))
            .unwrap();
        let video_id = timeline
            .insert_media_on_track(PRIMARY_VIDEO, "video.mp4".into(), 30, 0)
            .unwrap();
        let original = timeline.tracks().to_vec();
        let revision = timeline.revision();
        let changes = TextStyleChanges {
            bold: Some(true),
            ..TextStyleChanges::default()
        };

        assert_eq!(
            timeline.update_text_styles(&[caption_id, u64::MAX], changes),
            Err(TimelineError::ClipNotFound(u64::MAX))
        );
        assert_eq!(
            timeline.update_text_styles(&[caption_id, video_id], changes),
            Err(TimelineError::IncompatibleTrack)
        );
        assert_eq!(
            timeline.update_text_styles(
                &[caption_id],
                TextStyleChanges {
                    size: Some(TextStyle::MAX_SIZE + 1),
                    ..changes
                },
            ),
            Err(TimelineError::InvalidTextSize)
        );
        assert_eq!(
            timeline.update_text_styles(
                &[caption_id],
                TextStyleChanges {
                    position: Some(TextPosition {
                        x: TextPosition::MAX + 1,
                        y: 5_000,
                    }),
                    ..changes
                },
            ),
            Err(TimelineError::InvalidTextPosition)
        );
        assert_eq!(timeline.tracks(), original);
        assert_eq!(timeline.revision(), revision);
    }

    #[test]
    fn imports_overlapping_cues_without_moving_existing_captions() {
        let mut timeline = Timeline::default();
        let (first_track, existing) = timeline
            .insert_text(TrackKind::Caption, 10, 20, overlay("原字幕"))
            .unwrap();
        let original = timeline.tracks().to_vec();
        let inserted = timeline
            .insert_text_batch(
                TrackKind::Caption,
                vec![
                    (15, 20, overlay("第一条\n第二行")),
                    (35, 10, overlay("第二条")),
                ],
            )
            .unwrap();
        assert_ne!(inserted[0].0, first_track);
        assert_eq!(inserted[1].0, inserted[0].0);
        assert_eq!(timeline.clip(existing).unwrap().start, 10);
        assert_eq!(
            timeline
                .clip(inserted[0].1)
                .unwrap()
                .text
                .as_ref()
                .unwrap()
                .content,
            "第一条\n第二行"
        );
        let with_batch = timeline.tracks().to_vec();
        assert!(timeline.undo());
        assert_eq!(timeline.tracks(), original);
        assert!(timeline.redo());
        assert_eq!(timeline.tracks(), with_batch);
    }

    #[test]
    fn text_extends_on_both_sides_and_follows_existing_history() {
        let mut timeline = Timeline::default();
        let (_, id) = timeline
            .insert_text(TrackKind::Text, 24, 20, overlay("标题"))
            .unwrap();
        timeline.trim_start(id, 13, false).unwrap();
        timeline.trim_end(id, 52, false).unwrap();
        let right = timeline.split(id, 40).unwrap();
        timeline.update_text(right, overlay("新标题")).unwrap();
        timeline.move_clip(right, 44).unwrap();
        let left = timeline.clip(id).unwrap();
        assert_eq!((left.start, left.end()), (13, 40));
        assert_eq!(left.text.as_ref().unwrap(), &overlay("标题"));
        let edited = timeline.clip(right).unwrap();
        assert_eq!((edited.start, edited.end()), (44, 56));
        assert_eq!(edited.text.as_ref().unwrap(), &overlay("新标题"));
        assert!(timeline.undo());
        assert!(timeline.undo());
        assert_eq!(
            timeline.clip(right).unwrap().text.as_ref().unwrap(),
            &overlay("标题")
        );
    }

    #[test]
    fn keeps_visual_track_groups_and_rejects_invalid_changes_atomically() {
        let mut timeline = Timeline::default();
        let (caption, _) = timeline
            .insert_text(TrackKind::Caption, 0, 30, overlay("字幕"))
            .unwrap();
        let (title, _) = timeline
            .insert_text(TrackKind::Text, 0, 30, overlay("标题"))
            .unwrap();
        let video = timeline.add_track(TrackKind::Video).unwrap();
        let original = timeline.tracks().to_vec();
        assert_eq!(
            original.iter().map(|track| track.id).collect::<Vec<_>>(),
            vec![title, caption, video, 3, PRIMARY_VIDEO, PRIMARY_AUDIO]
        );
        assert_eq!(
            timeline.add_track_before(TrackKind::Video, Some(caption)),
            Err(TimelineError::IncompatibleTrack)
        );
        let revision = timeline.revision();
        assert_eq!(
            timeline.insert_text_batch(
                TrackKind::Caption,
                vec![(0, 10, overlay("新字幕")), (10, 10, overlay(" \t"))],
            ),
            Err(TimelineError::InvalidText)
        );
        assert_eq!(
            timeline.insert_media_on_track(caption, "video.mp4".into(), 20, 60),
            Err(TimelineError::IncompatibleTrack)
        );
        assert_eq!(timeline.tracks(), original);
        assert_eq!(timeline.revision(), revision);
    }
}
