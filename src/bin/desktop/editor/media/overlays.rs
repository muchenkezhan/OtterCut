use std::fmt::Write as _;
use std::fs;
use std::ops::Range;
use std::path::Path;

use anyhow::{Context as _, Result};
use takeseam::editor_timeline::{
    OverlayText, TextAppearance, TextPlacement, TextPosition, Track, TrackKind,
};

pub(in crate::editor) const SCRIPT_NAME: &str = "takeseam-overlay.ass";
pub(in crate::editor) const VIDEO_FPS: u64 = 30;
pub(in crate::editor) const DEFAULT_CANVAS: (u32, u32) = (1920, 1080);

#[derive(Clone)]
pub(super) struct Cue {
    start: u64,
    end: u64,
    layer: u8,
    text: OverlayText,
}

#[derive(Default)]
pub(in crate::editor) struct Overlays(Vec<Cue>);

impl Overlays {
    pub(in crate::editor) fn in_range(tracks: &[Track], range: Range<u64>) -> Self {
        Self::in_range_excluding(tracks, range, None)
    }

    pub(in crate::editor) fn in_range_excluding(
        tracks: &[Track],
        range: Range<u64>,
        excluded: Option<u64>,
    ) -> Self {
        let mut cues = Vec::new();
        // ASS stacks collisions within each layer. Start with the lowest row.
        for track in tracks
            .iter()
            .rev()
            .filter(|track| track.enabled && track.kind.is_overlay())
        {
            let layer = u8::from(track.kind == TrackKind::Text);
            let first = track
                .clips
                .partition_point(|clip| clip.end() <= range.start);
            cues.extend(
                track.clips[first..]
                    .iter()
                    .take_while(|clip| clip.start < range.end)
                    .filter(|clip| Some(clip.id) != excluded)
                    .filter_map(|clip| {
                        Some(Cue {
                            start: clip.start,
                            end: clip.end(),
                            layer,
                            text: clip.text.as_ref()?.clone(),
                        })
                    }),
            );
        }
        Self(cues)
    }

    pub(in crate::editor) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(in crate::editor) fn write_for_playback(
        &self,
        directory: &Path,
        size: (u32, u32),
        range: Range<u64>,
    ) -> Result<bool> {
        let clip_to_frames = |time: u64| {
            centiseconds_of_next_frame(time.clamp(range.start, range.end) - range.start)
        };
        self.write_script(directory, size, |cue| {
            let start = clip_to_frames(cue.start);
            let end = clip_to_frames(cue.end);
            (start < end).then_some((start, end))
        })
    }

    pub(super) fn write_for_frame(
        &self,
        directory: &Path,
        size: (u32, u32),
        position: u64,
    ) -> Result<bool> {
        self.write_script(directory, size, |cue| {
            (cue.start <= position && position < cue.end).then_some((0, 100))
        })
    }

    fn write_script(
        &self,
        directory: &Path,
        size: (u32, u32),
        times: impl Fn(&Cue) -> Option<(u64, u64)>,
    ) -> Result<bool> {
        let mut events = self
            .0
            .iter()
            .filter_map(|cue| Some((cue, times(cue)?)))
            .peekable();
        if events.peek().is_none() {
            return Ok(false);
        }
        // Keep the script height fixed so font size remains proportional to the canvas.
        let script_width = (u64::from(size.0) * 1080 / u64::from(size.1)).max(1);
        let x_margin = (script_width / 16).max(1);
        let mut script = format!(
            "[Script Info]\nScriptType: v4.00+\nPlayResX: {script_width}\nPlayResY: 1080\nWrapStyle: 0\nScaledBorderAndShadow: yes\n\n\
             [V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n"
        );
        for appearance in TextAppearance::ALL {
            write_style(&mut script, x_margin, appearance);
        }
        script.push_str(
            "\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
        );
        for (cue, (start, end)) in events {
            let style = &cue.text.style;
            let _ = write!(
                script,
                "Dialogue: {},{},{},{},,0,0,0,,{{",
                cue.layer,
                ass_time(start),
                ass_time(end),
                style_name(style.appearance),
            );
            if let Some(position) = style.position {
                let scale = f64::from(TextPosition::MAX);
                let x = f64::from(position.x) * script_width as f64 / scale;
                let y = f64::from(position.y) * 1080.0 / scale;
                let _ = write!(script, "\\an5\\pos({x:.3},{y:.3})");
            } else {
                let alignment = match style.placement {
                    TextPlacement::Top => 8,
                    TextPlacement::Center => 5,
                    TextPlacement::Bottom => 2,
                };
                let _ = write!(script, "\\an{alignment}");
            }
            let _ = write!(script, "\\fs{}\\b{}}}", style.size, u8::from(style.bold),);
            write_plain_text(&mut script, &cue.text.content);
            script.push('\n');
        }
        fs::write(directory.join(SCRIPT_NAME), script).context("无法写入字幕合成数据")?;
        Ok(true)
    }
}

fn style_name(appearance: TextAppearance) -> &'static str {
    match appearance {
        TextAppearance::Outline => "Default",
        TextAppearance::Plain => "Plain",
        TextAppearance::DarkBox => "DarkBox",
        TextAppearance::LightBox => "LightBox",
        TextAppearance::Gold => "Gold",
        TextAppearance::Shadow => "Shadow",
    }
}

fn ass_color([r, g, b, a]: [u8; 4]) -> u32 {
    u32::from_be_bytes([255 - a, b, g, r])
}

fn write_style(script: &mut String, x_margin: u64, appearance: TextAppearance) {
    let paint = appearance.paint();
    let primary = ass_color(paint.foreground);
    let outline = ass_color(paint.outline);
    // In libass, style 4 draws one background rectangle behind the text block;
    // the shadow distance becomes its padding.
    let (border_style, back, shadow) = match paint.background {
        Some(background) => (4, ass_color(background), paint.background_padding()),
        None => (1, if paint.shadow > 0 { outline } else { 0 }, paint.shadow),
    };
    let _ = writeln!(
        script,
        "Style: {},Microsoft YaHei,48,&H{primary:08X},&H{primary:08X},&H{outline:08X},&H{back:08X},0,0,0,0,100,100,0,0,{border_style},{},{},2,{x_margin},{x_margin},56,1",
        style_name(appearance),
        paint.outline_width,
        shadow,
    );
}

pub(in crate::editor) fn sequence_size(source: Option<super::MediaInfo>) -> (u32, u32) {
    let Some(source) = source else {
        return DEFAULT_CANVAS;
    };
    let scale = (f64::from(DEFAULT_CANVAS.0) / f64::from(source.width))
        .min(f64::from(DEFAULT_CANVAS.1) / f64::from(source.height))
        .min(1.0);
    let dimension = |value| ((f64::from(value) * scale).floor() as u32 & !1).max(2);
    (dimension(source.width), dimension(source.height))
}

fn centiseconds_of_next_frame(time: u64) -> u64 {
    // Scope cue boundaries to the first 30 fps frame at or after the edit point.
    let frame = (u128::from(time) * u128::from(VIDEO_FPS)).div_ceil(1_000_000);
    (frame * 100 / u128::from(VIDEO_FPS)) as u64
}

fn ass_time(value: u64) -> String {
    format!(
        "{}:{:02}:{:02}.{:02}",
        value / 360_000,
        value / 6_000 % 60,
        value / 100 % 60,
        value % 100,
    )
}

fn write_plain_text(output: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '\n' => output.push_str("\\N"),
            '\r' => (),
            '\t' => output.push(' '),
            '{' => output.push_str("\\{"),
            '}' => output.push_str("\\}"),
            // A zero-width word joiner keeps a literal backslash from forming an ASS escape.
            '\\' => output.push_str("\\\u{2060}"),
            value if value.is_control() => (),
            value => output.push(value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cue_boundaries_follow_the_next_frame_and_keep_text_literal() {
        assert_eq!(centiseconds_of_next_frame(0), 0);
        assert_eq!(centiseconds_of_next_frame(1), 3);
        assert_eq!(centiseconds_of_next_frame(33_333), 3);
        assert_eq!(centiseconds_of_next_frame(33_334), 6);
        assert_eq!(ass_time(360_123), "1:00:01.23");
        let mut out = String::new();
        write_plain_text(&mut out, "字幕{\\b1}\\N\r\nText\0");
        assert_eq!(out, "字幕\\{\\\u{2060}b1\\}\\\u{2060}N\\NText");
    }
}
