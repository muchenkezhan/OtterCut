use takeseam::editor_timeline::{Clip, Timeline, TimelineError, TrackId};

use super::clip_media::VolumeLine;
use super::commands::{Edge, trim};

#[derive(Clone, Copy)]
pub(super) enum Gesture {
    Move,
    Reorder,
    Trim(Edge, bool),
    Roll(Edge),
    Slip,
    Slide,
    Volume { line: VolumeLine, offset: f32 },
    Transition(Edge),
}

impl Gesture {
    pub(super) fn volume_at(self, y: f32) -> Option<i16> {
        match self {
            Self::Volume { line, offset } => Some(line.value_at(y - offset)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct ClipDrag {
    pub(super) id: u64,
    pub(super) kind: Gesture,
    pub(super) origin: u64,
}

impl ClipDrag {
    pub(super) fn move_start(self, clip: &Clip, delta: i64) -> u64 {
        clip.start
            .saturating_add_signed(delta)
            .min(u64::MAX - clip.duration())
    }

    pub(super) fn snap(
        self,
        timeline: &Timeline,
        playhead: u64,
        delta: i64,
        tolerance: u64,
    ) -> i64 {
        let Some(clip) = timeline.clip(self.id) else {
            return delta;
        };
        let anchors: &[_] = match self.kind {
            Gesture::Trim(edge, _) | Gesture::Roll(edge) => &[edge.time(clip)],
            Gesture::Move | Gesture::Slide => &[clip.start, clip.end()],
            Gesture::Reorder | Gesture::Slip | Gesture::Volume { .. } | Gesture::Transition(_) => return delta,
        };
        let edges = timeline
            .all_clips()
            .filter(|other| other.id != self.id)
            .flat_map(|other| [other.start, other.end()]);
        let mut correction = 0;
        let mut nearest = tolerance + 1;
        for target in [0, playhead].into_iter().chain(edges) {
            for anchor in anchors {
                let projected = anchor.saturating_add_signed(delta);
                let distance = target.abs_diff(projected);
                // A shared cut should move as soon as the user drags it.
                if target != *anchor && distance < nearest {
                    nearest = distance;
                    correction = difference(target, projected);
                }
            }
        }
        delta.saturating_add(correction)
    }

    pub(super) fn propose(
        self,
        timeline: &Timeline,
        delta: i64,
        pointer: u64,
        target: TrackId,
        pointer_y: f32,
    ) -> Result<Draft, TimelineError> {
        let source = timeline
            .track_for_clip(self.id)
            .ok_or(TimelineError::ClipNotFound(self.id))?;
        let clips = &source.clips;
        let index = clips
            .iter()
            .position(|clip| clip.id == self.id)
            .ok_or(TimelineError::ClipNotFound(self.id))?;
        let clip = &clips[index];
        let previous = index.checked_sub(1).and_then(|index| clips.get(index));
        let next = clips.get(index + 1);
        let move_to = self.move_start(clip, delta);
        let mut draft = Draft {
            id: self.id,
            track: source.id,
            action: Edit::Move(move_to),
            ghost: None,
            guide: None,
            source_in: None,
        };
        match self.kind {
            Gesture::Move => {
                let end = move_to + clip.duration();
                if target != source.id {
                    let track = timeline
                        .track(target)
                        .ok_or(TimelineError::TrackNotFound(target))?;
                    if track.kind != source.kind {
                        return Err(TimelineError::IncompatibleTrack);
                    }
                    if track
                        .clips
                        .iter()
                        .any(|other| other.start < end && other.end() > move_to)
                    {
                        return Err(TimelineError::Overlap);
                    }
                    draft.track = target;
                    draft.ghost = Some((move_to, end));
                    return Ok(draft);
                }
                if clips
                    .iter()
                    .any(|other| other.id != self.id && other.start < end && other.end() > move_to)
                {
                    // Subtitle timing should survive a move that collides with its neighbor.
                    if source.kind.is_overlay() {
                        return Err(TimelineError::Overlap);
                    }
                    draft.reorder(clips, move_to + clip.duration() / 2);
                } else {
                    draft.ghost = Some((move_to, end));
                }
            }
            Gesture::Reorder => draft.reorder(clips, pointer),
            Gesture::Trim(edge, ripple) => {
                let delta = trim_delta(clip, edge, ripple, delta, previous, next);
                let at = edge.time(clip).saturating_add_signed(delta);
                draft.action = Edit::Trim(edge, at, ripple);
                let span = match (edge, ripple) {
                    (Edge::Start, false) => (at, clip.end()),
                    (Edge::Start, true) => (
                        clip.start,
                        clip.end().saturating_add_signed(delta.saturating_neg()),
                    ),
                    (Edge::End, _) => (clip.start, at),
                };
                draft.ghost = Some(span);
                draft.guide = Some(match (edge, ripple) {
                    (Edge::Start, false) => span.0,
                    _ => span.1,
                });
            }
            Gesture::Roll(edge) => {
                let pair = match edge {
                    Edge::Start => previous.map(|left| (left, clip)),
                    Edge::End => next.map(|right| (clip, right)),
                };
                let (left, right) = pair
                    .filter(|(left, right)| left.end() == right.start)
                    .ok_or(TimelineError::MissingAdjacentClip)?;
                let at = right
                    .start
                    .saturating_add_signed(adjacent_delta(left, right, delta));
                draft.action = Edit::Roll(left.id, at);
                draft.guide = Some(at);
            }
            Gesture::Slip => {
                let delta = delta.clamp(
                    difference(0, clip.source_start),
                    difference(clip.media_duration, clip.source_end),
                );
                draft.action = Edit::Slip(delta);
                draft.ghost = Some((clip.start, clip.end()));
                draft.source_in = Some(clip.source_start.saturating_add_signed(delta));
            }
            Gesture::Slide => {
                let left = previous
                    .filter(|left| left.end() == clip.start)
                    .ok_or(TimelineError::MissingAdjacentClip)?;
                let right = next
                    .filter(|right| clip.end() == right.start)
                    .ok_or(TimelineError::MissingAdjacentClip)?;
                let delta = adjacent_delta(left, right, delta);
                draft.action = Edit::Slide(delta);
                draft.ghost = Some((
                    clip.start.saturating_add_signed(delta),
                    clip.end().saturating_add_signed(delta),
                ));
            }
            Gesture::Volume { line, offset } => {
                draft.action = Edit::Volume(line.value_at(pointer_y - offset));
            }
            Gesture::Transition(edge) => {
                let Some(transition) = &clip.transition_in else {
                    return Err(TimelineError::ClipNotFound(clip.id));
                };
                let duration = transition_duration_at(clip, &transition.role, edge, pointer);
                draft.action = Edit::Transition(duration);
                draft.ghost = Some(transition_span(clip, &transition.role, duration));
            }
        }
        Ok(draft)
    }
}

enum Edit {
    Move(u64),
    Reorder(Option<u64>),
    Trim(Edge, u64, bool),
    Roll(u64, u64),
    Slip(i64),
    Slide(i64),
    Volume(i16),
    Transition(u64),
}

pub(super) struct Draft {
    id: u64,
    pub(super) track: TrackId,
    action: Edit,
    pub(super) ghost: Option<(u64, u64)>,
    pub(super) guide: Option<u64>,
    pub(super) source_in: Option<u64>,
}

impl Draft {
    pub(super) fn volume_value(&self) -> Option<i16> {
        match self.action {
            Edit::Volume(value) => Some(value),
            _ => None,
        }
    }
}

impl Draft {
    fn reorder(&mut self, clips: &[Clip], at: u64) {
        let mut others = clips.iter().filter(|clip| clip.id != self.id);
        let before = others
            .clone()
            .find(|clip| at < clip.start + clip.duration() / 2);
        self.action = Edit::Reorder(before.map(|clip| clip.id));
        self.guide = Some(before.map_or_else(
            || others.next_back().map_or(0, Clip::end),
            |clip| clip.start,
        ));
    }

    pub(super) fn commit(self, timeline: &mut Timeline) -> Result<(), TimelineError> {
        match self.action {
            Edit::Move(at) => timeline.move_to_track(self.id, self.track, at),
            Edit::Reorder(before) => timeline.reorder(self.id, before),
            Edit::Trim(edge, at, ripple) => trim(timeline, self.id, edge, at, ripple),
            Edit::Roll(left, at) => timeline.roll(left, at),
            Edit::Slip(delta) => timeline.slip(self.id, delta),
            Edit::Slide(delta) => timeline.slide(self.id, delta),
            Edit::Volume(db_tenths) => timeline.set_clip_volume(self.id, db_tenths),
            Edit::Transition(duration) => timeline.set_clip_transition_duration(self.id, duration),
        }
    }
}

fn trim_delta(
    clip: &Clip,
    edge: Edge,
    ripple: bool,
    delta: i64,
    previous: Option<&Clip>,
    next: Option<&Clip>,
) -> i64 {
    match edge {
        Edge::Start => {
            let earliest = if ripple {
                0
            } else {
                previous.map_or(0, Clip::end)
            };
            let min = difference(0, clip.trim_headroom()).max(difference(earliest, clip.start));
            delta.clamp(min, difference(clip.duration(), 1))
        }
        Edge::End => {
            let latest = if ripple {
                u64::MAX
            } else {
                next.map_or(u64::MAX, |clip| clip.start)
            };
            let max = difference(clip.trim_tailroom(), 0).min(difference(latest, clip.end()));
            delta.clamp(difference(1, clip.duration()), max)
        }
    }
}

fn transition_duration_at(clip: &Clip, role: &str, edge: Edge, pointer: u64) -> u64 {
    match role {
        "入场" => pointer.saturating_sub(clip.start).clamp(50_000, clip.duration()),
        "出场" => clip.end().saturating_sub(pointer).clamp(50_000, clip.duration()),
        _ => {
            let distance = match edge {
                Edge::Start => clip.start.abs_diff(pointer),
                Edge::End => pointer.abs_diff(clip.start),
            };
            distance
                .saturating_mul(2)
                .clamp(50_000, clip.duration().saturating_mul(2))
        }
    }
}

fn transition_span(clip: &Clip, role: &str, duration: u64) -> (u64, u64) {
    match role {
        "入场" => (clip.start, clip.start.saturating_add(duration)),
        "出场" => (clip.end().saturating_sub(duration), clip.end()),
        _ => (
            clip.start.saturating_sub(duration / 2),
            clip.start.saturating_add(duration / 2),
        ),
    }
}

fn adjacent_delta(left: &Clip, right: &Clip, delta: i64) -> i64 {
    let min = difference(1, left.duration()).max(difference(0, right.trim_headroom()));
    let max = difference(left.trim_tailroom(), 0).min(difference(right.duration(), 1));
    delta.clamp(min, max)
}

pub(super) fn difference(target: u64, start: u64) -> i64 {
    (i128::from(target) - i128::from(start)).clamp(i128::from(i64::MIN), i128::from(i64::MAX))
        as i64
}
