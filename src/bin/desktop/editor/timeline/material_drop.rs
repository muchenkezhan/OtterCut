use eframe::egui::{
    self, CursorIcon, DragAndDrop, Event, PointerButton, Rect, Stroke, StrokeKind, Ui,
};
use takeseam::editor_timeline::{Timeline, TimelineError, TrackId, TrackKind};

use crate::ui_theme;

use super::super::asset_cards::DraggedAsset;
use super::canvas::{SECOND, Viewport, paint_drop_guide};
use super::edge_drop::TrackGap;
use super::track_rows::Rows;
use super::{MaterialDrop, TrackDestination};

pub(super) struct Target<'a> {
    pub(super) canvas: Rect,
    pub(super) rows: Rows<'a>,
    pub(super) active_track: TrackId,
    pub(super) playhead: u64,
    pub(super) snap: bool,
}

pub(super) fn is_dragging(ctx: &egui::Context) -> bool {
    DragAndDrop::has_payload_of_type::<DraggedAsset>(ctx)
}

pub(super) fn interact(
    ui: &Ui,
    painter: &egui::Painter,
    view: Viewport,
    target: Target,
    timeline: &Timeline,
) -> Result<Option<MaterialDrop>, TimelineError> {
    let Some(asset) = DragAndDrop::payload::<DraggedAsset>(ui.ctx()) else {
        return Ok(None);
    };
    let (pointer, released, held, focused) = ui.input(|input| {
        let release = input.events.iter().rev().find_map(|event| match event {
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                ..
            } => Some(*pos),
            _ => None,
        });
        (
            release.or(input.pointer.interact_pos()),
            input.pointer.primary_released(),
            input.pointer.primary_down(),
            input.viewport().focused.unwrap_or(true),
        )
    });
    if !focused || (!held && !released) {
        DragAndDrop::clear_payload(ui.ctx());
        return Ok(None);
    }
    let Some(pointer) =
        pointer.filter(|pos| target.canvas.intersect(ui.clip_rect()).contains(*pos))
    else {
        return Ok(None);
    };
    let gap = TrackGap::at(target.rows, timeline, pointer);
    let destination = if timeline.is_empty() {
        let kind = empty_timeline_kind(&asset);
        Some(TrackDestination::New { kind, before: None })
    } else if let Some(gap) = gap {
        let prefer_audio =
            asset.has_audio == Some(true) && (asset.has_video == Some(false) || pointer.y > gap.y);
        let order = if prefer_audio {
            [TrackKind::Audio, TrackKind::Video]
        } else {
            [TrackKind::Video, TrackKind::Audio]
        };
        order
            .into_iter()
            .find(|&kind| gap.allows(kind) && fits(kind, &asset))
            .map(|kind| TrackDestination::New {
                kind,
                before: gap.before,
            })
    } else {
        let track = target
            .rows
            .at(timeline, pointer.y)
            .or_else(|| timeline.track(target.active_track))
            .ok_or(TimelineError::TrackNotFound(target.active_track))?;
        fits(track.kind, &asset).then_some(TrackDestination::Existing(track.id))
    };
    let Some(destination) = destination else {
        ui.ctx().set_cursor_icon(CursorIcon::NotAllowed);
        if released {
            DragAndDrop::clear_payload(ui.ctx());
            return Err(TimelineError::IncompatibleTrack);
        }
        return Ok(None);
    };
    let raw_at = view.time_at(pointer.x.max(view.rect.left()));
    let tolerance = if target.snap {
        (7.0 / view.scale * SECOND) as u64
    } else {
        0
    };
    let at = insertion_point(timeline, destination, raw_at, target.playhead, tolerance);
    if released {
        return Ok(
            DragAndDrop::take_payload::<DraggedAsset>(ui.ctx()).map(|asset| MaterialDrop {
                path: asset.path.clone(),
                destination,
                at,
            }),
        );
    }
    ui.ctx().set_cursor_icon(CursorIcon::Copy);
    if let (Some(gap), TrackDestination::New { kind: _, .. }) = (gap, destination) {
        gap.paint_target(painter, view, at, target.rows);
    } else if let TrackDestination::Existing(id) = destination
        && let Some(index) = timeline.tracks().iter().position(|row| row.id == id)
    {
        let row = target.rows.rect(view, index);
        paint(
            &painter.with_clip_rect(view.rect),
            view,
            row,
            target.rows.area,
            at,
            asset.duration_us,
        );
    }
    Ok(None)
}

fn fits(kind: TrackKind, asset: &DraggedAsset) -> bool {
    match kind {
        TrackKind::Text | TrackKind::Caption => false,
        TrackKind::Video => asset.has_video != Some(false),
        TrackKind::Audio => asset.has_audio != Some(false),
    }
}

fn empty_timeline_kind(asset: &DraggedAsset) -> TrackKind {
    if asset.has_video != Some(false) {
        TrackKind::Video
    } else {
        TrackKind::Audio
    }
}

fn insertion_point(
    timeline: &Timeline,
    destination: TrackDestination,
    at: u64,
    playhead: u64,
    tolerance: u64,
) -> u64 {
    let clip_boundary = |at| match destination {
        TrackDestination::Existing(id) => timeline.material_insertion_point_on_track(id, at),
        TrackDestination::New { .. } => at,
    };
    let boundary = clip_boundary(at);
    if boundary != at || tolerance == 0 {
        return boundary;
    }
    let playhead = clip_boundary(playhead);
    let edges = timeline
        .all_clips()
        .flat_map(|clip| [clip.start, clip.end()]);
    let snapped = [0, playhead]
        .into_iter()
        .chain(edges)
        .filter(|target| target.abs_diff(at) <= tolerance)
        .min_by_key(|target| target.abs_diff(at))
        .unwrap_or(at);
    clip_boundary(snapped)
}

fn paint(
    painter: &egui::Painter,
    view: Viewport,
    row: Rect,
    rows: Rect,
    at: u64,
    duration: Option<u64>,
) {
    let colors = ui_theme::palette(painter.ctx());
    let body = painter.with_clip_rect(rows);
    if let Some(duration) = duration.filter(|duration| *duration > 0) {
        let ghost = view.clip_rect(row, at, at.saturating_add(duration));
        body.rect(
            ghost,
            0,
            colors.accent.gamma_multiply(0.17),
            Stroke::new(1.0_f32, colors.accent),
            StrokeKind::Inside,
        );
    }
    paint_drop_guide(
        painter,
        view,
        at,
        row.bottom().clamp(rows.top(), rows.bottom()),
        &colors,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_timeline_prefers_video_for_av_media() {
        let asset = DraggedAsset {
            path: "screen.mp4".into(),
            duration_us: Some(1_000_000),
            has_video: Some(true),
            has_audio: Some(true),
        };
        assert_eq!(empty_timeline_kind(&asset), TrackKind::Video);
    }

    #[test]
    fn empty_timeline_uses_audio_for_audio_only_media() {
        let asset = DraggedAsset {
            path: "voice.wav".into(),
            duration_us: Some(1_000_000),
            has_video: Some(false),
            has_audio: Some(true),
        };
        assert_eq!(empty_timeline_kind(&asset), TrackKind::Audio);
    }
}
