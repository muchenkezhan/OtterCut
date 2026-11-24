use std::ops::Range;
use std::path::Path;

use eframe::egui::{self, Align2, Color32, FontId, Mesh, Painter, Rect, Stroke, pos2, vec2};
use takeseam::editor_timeline::{Clip, TrackKind};

use crate::ui_icons::Icon;
use crate::ui_theme::Palette;

use super::super::media::MediaTools;
use super::super::thumbnails::Thumbnails;
use super::super::waveforms::{Waveform, Waveforms};
use super::ClipLane;
use super::canvas::{SECOND, Viewport, timecode};

const CAPTION_HEIGHT: f32 = 16.0;
const WAVE_STEP: f32 = 1.2;
const WAVE_FEATHER: f32 = 0.6;
const WAVE_BASELINE_INSET: f32 = 1.0;
const TILE_WIDTH: f64 = 58.0;

pub(super) struct ClipMedia {
    video: Thumbnails,
    audio: Waveforms,
}

impl ClipMedia {
    pub(super) fn new(tools: Option<MediaTools>, ctx: &egui::Context) -> Self {
        Self {
            video: Thumbnails::timeline(tools.clone(), ctx),
            audio: Waveforms::new(tools, ctx),
        }
    }

    pub(super) fn video_nearest_frame(
        &mut self,
        path: &Path,
        timestamp_us: u64,
        tolerance_us: u64,
    ) -> Option<egui::TextureHandle> {
        self.video
            .nearest_frame(path, timestamp_us, tolerance_us)
    }
}

pub(super) fn paint(
    painter: &Painter,
    rect: Rect,
    lane: ClipLane,
    clip: &Clip,
    name: &str,
    colors: &Palette,
    media: Option<&mut ClipMedia>,
    volume_db_tenths: i16,
) {
    if lane.kind.is_overlay() {
        paint_caption(painter, rect, name, None, colors);
        return;
    }
    let caption_bottom = rect.top() + CAPTION_HEIGHT;
    let caption = Rect::from_min_max(rect.min, pos2(rect.right(), caption_bottom));
    paint_caption(painter, caption, name, Some(clip.duration()), colors);

    let body = Rect::from_min_max(
        pos2(rect.left() + 1.0, caption_bottom),
        rect.right_bottom() - vec2(1.0, 1.0),
    );
    let visible = body.intersect(painter.clip_rect());
    if !visible.is_positive() || visible.width() < 7.0 {
        return;
    }

    let media_painter = painter.with_clip_rect(body);
    match lane.kind {
        TrackKind::Text | TrackKind::Caption => return,
        TrackKind::Video => {
            media_painter.rect_filled(body, 0, colors.monitor);
            let rendered = media.is_some_and(|media| {
                paint_filmstrip(&media_painter, body, lane, clip, &mut media.video, colors)
            });
            if !rendered && visible.width() >= 26.0 && visible.height() >= 18.0 {
                Icon::Media.paint(&media_painter, visible.center(), colors.muted);
            }
        }
        TrackKind::Audio => {
            let (ink, fill) = if lane.enabled {
                (colors.audio_wave, colors.audio_clip)
            } else {
                (colors.clip_border, colors.control)
            };
            media_painter.rect_filled(body, 0, fill);
            let envelope =
                media.and_then(|media| media.audio.envelope(&clip.path, clip.media_duration));
            let gain = 10.0_f32.powf(f32::from(volume_db_tenths) / 200.0);
            paint_waveform(
                &media_painter,
                body,
                lane.view,
                clip,
                envelope,
                ink,
                fill.lerp_to_gamma(ink, 0.28),
                gain,
            );
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct VolumeLine(Rect);

impl VolumeLine {
    pub(super) fn for_clip(rect: Rect, trim_handle: f32) -> Option<Self> {
        let inset = trim_handle + 2.0;
        let area = Rect::from_min_max(
            pos2(rect.left() + inset, rect.top() + CAPTION_HEIGHT + 4.0),
            pos2(rect.right() - inset, rect.bottom() - 5.0),
        );
        (area.width() >= 8.0 && area.height() >= 12.0).then_some(Self(area))
    }

    pub(super) fn y_at(self, db_tenths: i16) -> f32 {
        let middle = self.0.center().y;
        let (edge, fraction) = if db_tenths >= 0 {
            (
                self.0.top(),
                f32::from(db_tenths) / f32::from(Clip::MAX_VOLUME_DB_TENTHS),
            )
        } else {
            (
                self.0.bottom(),
                (f32::from(db_tenths) / f32::from(Clip::MIN_VOLUME_DB_TENTHS)).sqrt(),
            )
        };
        egui::lerp(middle..=edge, fraction.clamp(0.0, 1.0))
    }

    pub(super) fn value_at(self, y: f32) -> i16 {
        let middle = self.0.center().y;
        let value = if y < middle {
            ((middle - y) / (middle - self.0.top())).clamp(0.0, 1.0)
                * f32::from(Clip::MAX_VOLUME_DB_TENTHS)
        } else {
            ((y - middle) / (self.0.bottom() - middle))
                .clamp(0.0, 1.0)
                .powi(2)
                * f32::from(Clip::MIN_VOLUME_DB_TENTHS)
        };
        value.round() as i16
    }

    pub(super) fn hit(self, db_tenths: i16) -> Rect {
        let y = self.y_at(db_tenths);
        Rect::from_x_y_ranges(self.0.x_range(), (y - 4.0)..=(y + 4.0))
    }

    pub(super) fn paint(
        self,
        painter: &Painter,
        volume_db_tenths: i16,
        highlighted: bool,
        colors: &Palette,
    ) {
        let visible = self.0.intersect(painter.clip_rect());
        if !visible.is_positive() {
            return;
        }
        let y = self.y_at(volume_db_tenths);
        if !visible.y_range().contains(y) {
            return;
        }
        let ink = if highlighted {
            colors.accent
        } else {
            colors.text.gamma_multiply(0.7)
        };
        painter.hline(visible.x_range(), y, Stroke::new(1.0_f32, ink));
        if highlighted {
            painter.circle_filled(pos2(visible.left() + 2.0, y), 2.0, ink);
            let value = format!("{:+.1} dB", f32::from(volume_db_tenths) / 10.0);
            let badge = painter.layout_no_wrap(value, FontId::monospace(10.0), colors.text);
            let size = badge.size() + vec2(8.0, 4.0);
            if size.x + 4.0 <= visible.width() {
                let below = y - size.y < painter.clip_rect().top();
                let top = if below { y + 3.0 } else { y - size.y - 3.0 };
                let rect = Rect::from_min_size(pos2(visible.right() - size.x - 2.0, top), size);
                painter.rect_filled(rect, 3, colors.surface);
                painter.galley(
                    Align2::CENTER_CENTER
                        .anchor_size(rect.center(), badge.size())
                        .min,
                    badge,
                    colors.text,
                );
            }
        }
    }
}

fn paint_caption(
    painter: &Painter,
    caption: Rect,
    name: &str,
    duration: Option<u64>,
    colors: &Palette,
) {
    let visible = caption
        .intersect(painter.clip_rect())
        .shrink2(vec2(6.0, 0.0));
    if !visible.is_positive() {
        return;
    }
    let mut name_rect = visible;
    let caption_painter = painter.with_clip_rect(visible);
    let baseline = caption.center().y;
    if visible.width() >= 168.0
        && let Some(duration) = duration
    {
        let stamp =
            painter.layout_no_wrap(timecode(duration), FontId::monospace(9.5), colors.muted);
        let x = visible.right() - stamp.size().x;
        let y = baseline - stamp.size().y * 0.5;
        caption_painter.galley(pos2(x, y), stamp, colors.muted);
        name_rect.max.x = x - 8.0;
    }
    let mut layout = egui::text::LayoutJob::simple(
        name.to_owned(),
        FontId::proportional(11.0),
        colors.text,
        name_rect.width(),
    );
    layout.wrap.max_rows = 1;
    let title = painter.layout_job(layout);
    let y = baseline - title.size().y * 0.5;
    caption_painter
        .with_clip_rect(name_rect)
        .galley(pos2(name_rect.left(), y), title, colors.text);
}

fn paint_filmstrip(
    painter: &Painter,
    body: Rect,
    lane: ClipLane,
    clip: &Clip,
    frames: &mut Thumbnails,
    colors: &Palette,
) -> bool {
    let visible = painter.clip_rect();
    let tint = if lane.enabled {
        Color32::WHITE
    } else {
        Color32::WHITE.gamma_multiply(0.38)
    };
    let mut rendered = false;
    for (range, timestamp) in film_tiles(clip, lane.view, visible) {
        let tile = Rect::from_min_max(
            pos2(
                x_at_source(lane.view, clip, range.start).max(body.left()),
                body.top(),
            ),
            pos2(
                x_at_source(lane.view, clip, range.end).min(body.right()),
                body.bottom(),
            ),
        );
        if !tile.intersect(visible).is_positive() {
            continue;
        }
        if let Some(frame) = frames.frame(&clip.path, timestamp) {
            painter.image(
                frame.id(),
                tile,
                cover_uv(frame.size_vec2(), tile.size()),
                tint,
            );
            rendered = true;
        } else {
            painter.rect_filled(tile, 0, colors.track);
        }
        if range.start > clip.source_start {
            painter.vline(
                tile.left(),
                tile.y_range(),
                Stroke::new(1.0_f32, Color32::from_black_alpha(110)),
            );
        }
    }
    rendered
}

fn film_tiles(
    clip: &Clip,
    view: Viewport,
    visible: Rect,
) -> impl Iterator<Item = (Range<u64>, u64)> + '_ {
    // Powers of two make repeated zoom steps and trimmings share the same source-frame cache keys.
    let interval = (TILE_WIDTH * SECOND / view.scale).log2().round().exp2() as u64;
    let interval = interval.max(1);
    let first = source_at_x(view, clip, visible.left()) / interval;
    let last = source_at_x(view, clip, visible.right()).saturating_sub(1) / interval;
    (first..=last).filter_map(move |index| {
        let start = index.saturating_mul(interval);
        let left = start.max(clip.source_start);
        let right = start.saturating_add(interval).min(clip.source_end);
        if left >= right {
            return None;
        }
        let sample = start.saturating_add(interval / 2).clamp(left, right - 1);
        Some((left..right, sample))
    })
}

fn cover_uv(source: egui::Vec2, destination: egui::Vec2) -> Rect {
    let source_aspect = source.x / source.y;
    let destination_aspect = destination.x / destination.y;
    let width = (destination_aspect / source_aspect).min(1.0);
    let height = (source_aspect / destination_aspect).min(1.0);
    Rect::from_center_size(pos2(0.5, 0.5), vec2(width, height))
}

fn paint_waveform(
    painter: &Painter,
    body: Rect,
    view: Viewport,
    clip: &Clip,
    envelope: Option<&Waveform>,
    ink: Color32,
    baseline: Color32,
    gain: f32,
) {
    painter.hline(
        body.x_range(),
        body.bottom() - WAVE_BASELINE_INSET,
        Stroke::new(1.0_f32, baseline),
    );
    let visible = painter.clip_rect();
    let Some(envelope) = envelope else {
        if visible.width() >= 26.0 && visible.height() >= 18.0 {
            Icon::Audio.paint(painter, visible.center(), ink);
        }
        return;
    };

    let mesh = waveform_mesh(body, visible, ink, |left, right| {
        envelope
            .peaks(
                source_at_x(view, clip, left),
                source_at_x(view, clip, right),
            )
            .map(|peak| peak * gain)
    });
    if !mesh.indices.is_empty() {
        painter.add(egui::Shape::mesh(mesh));
    }
}

fn waveform_mesh(
    body: Rect,
    visible: Rect,
    ink: Color32,
    mut peaks: impl FnMut(f32, f32) -> [f32; 2],
) -> Mesh {
    let baseline = body.bottom() - WAVE_BASELINE_INSET;
    let peak_height = (baseline - body.top() - 3.0).max(1.0);
    let first = ((visible.left() - body.left()) / WAVE_STEP).floor() as usize;
    let last = ((visible.right() - body.left()) / WAVE_STEP).ceil() as usize;
    let mut mesh = Mesh::default();
    let mut previous_audible = false;

    for index in first..=last {
        let x = (body.left() + index as f32 * WAVE_STEP).min(body.right());
        let left = (x - WAVE_STEP * 0.5).max(body.left());
        let right = (x + WAVE_STEP * 0.5).min(body.right());
        let [low, high] = peaks(left, right);
        let amplitude = low.abs().max(high.abs()).min(1.0);
        let height = amplitude * peak_height;
        let top = baseline - height;
        let audible = height > 0.0;
        let start = mesh.vertices.len() as u32;

        for (y, color) in [
            (top - WAVE_FEATHER.min(height), Color32::TRANSPARENT),
            (top, ink),
            (baseline, ink),
        ] {
            mesh.colored_vertex(pos2(x, y), color);
        }

        // Join the peak edge and the flat base without drawing across silence.
        if start >= 3 && (audible || previous_audible) {
            for row in 0..2 {
                let previous = start + row - 3;
                let current = start + row;
                mesh.add_triangle(previous, current, previous + 1);
                mesh.add_triangle(previous + 1, current, current + 1);
            }
        }
        previous_audible = audible;
    }
    mesh
}

fn source_at_x(view: Viewport, clip: &Clip, x: f32) -> u64 {
    let offset = view
        .time_at(x)
        .saturating_sub(clip.start)
        .min(clip.duration());
    clip.source_start.saturating_add(offset)
}

fn x_at_source(view: Viewport, clip: &Clip, timestamp: u64) -> f32 {
    view.x(clip
        .start
        .saturating_add(timestamp.saturating_sub(clip.source_start)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveform_rises_from_the_bottom_and_keeps_silence_flat() {
        let body = Rect::from_min_size(pos2(0.0, 0.0), vec2(64.0, 32.0));
        let ink = Color32::LIGHT_BLUE;
        let mesh = waveform_mesh(body, body, ink, |left, right| {
            if right <= 18.0 {
                [-0.18, 0.32]
            } else if left >= 44.0 {
                [-0.8, 0.2]
            } else {
                [0.0, 0.0]
            }
        });
        assert!(mesh.is_valid());
        assert!(!mesh.indices.is_empty());
        let baseline = body.bottom() - WAVE_BASELINE_INSET;
        assert!(mesh.vertices.iter().all(|vertex| vertex.pos.y <= baseline));
        for (x, amplitude) in [(12.0, 0.32), (48.0, 0.8)] {
            let mut solid = mesh
                .vertices
                .iter()
                .filter(|vertex| (vertex.pos.x - x).abs() < 0.01 && vertex.color == ink);
            let expected = baseline - amplitude * (baseline - body.top() - 3.0);
            assert!((solid.next().unwrap().pos.y - expected).abs() < 0.01);
            assert_eq!(solid.next().unwrap().pos.y, baseline);
        }
        assert!(mesh.triangles().all(|triangle| {
            let x = triangle.map(|index| mesh.vertices[index as usize].pos.x);
            x.iter().all(|x| *x <= 20.0) || x.iter().all(|x| *x >= 42.0)
        }));
    }

    #[test]
    fn visible_filmstrip_samples_are_bounded_by_the_trimmed_source() {
        let clip = Clip {
            id: 0,
            path: "clip.mp4".into(),
            text: None,
            media_duration: 160_000_000,
            source_start: 30_250_000,
            source_end: 65_664_000,
            start: 57_000_000,
            volume_db_tenths: 0,
            fade_in_us: 0,
            fade_out_us: 0,
            transition_in: None,
            speed_milli: 1000,
            reverse: false,
            timeline_duration_us: 0,
            group_id: None,
        };
        let view = Viewport {
            rect: Rect::from_min_size(pos2(100.0, 0.0), vec2(420.0, 36.0)),
            scale: 58.0,
            scroll: 65_000_000.0,
        };
        let tiles: Vec<_> = film_tiles(&clip, view, view.rect).collect();
        assert!(tiles.len() > 4 && tiles.len() < 12);
        assert_eq!(source_at_x(view, &clip, view.rect.left()), 38_250_000);
        let (first, _) = &tiles[0];
        assert!(first.contains(&38_250_000));
        assert!(
            tiles
                .windows(2)
                .all(|pair| pair[0].0.end == pair[1].0.start)
        );
        for (tile, timestamp) in tiles {
            assert!(tile.contains(&timestamp));
            assert!((clip.source_start..clip.source_end).contains(&timestamp));
        }
        let left_edge = Viewport {
            scroll: clip.start as f64,
            ..view
        };
        let (first, timestamp) = film_tiles(&clip, left_edge, left_edge.rect).next().unwrap();
        assert_eq!(first.start, clip.source_start);
        assert!(first.contains(&timestamp));
    }
}
