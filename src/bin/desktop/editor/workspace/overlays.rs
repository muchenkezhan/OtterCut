use std::collections::HashSet;

use eframe::egui;
use takeseam::editor_timeline::{
    OverlayText, TextStyle, TextStyleChanges, Timeline, TrackId, TrackKind,
};

use super::super::subrip;
use super::super::subtitle_canvas::CanvasTransform;
use super::super::text_panel::{self, TextTool};
use super::Workspace;

const DEFAULT_DURATION_US: u64 = 3_000_000;

impl Workspace {
    pub(super) fn visible_overlay<'a>(
        timeline: &'a Timeline,
        selection: Option<u64>,
        group: &[u64],
        position: u64,
    ) -> Option<(u64, &'a OverlayText)> {
        let id = selection?;
        let position = position.min(timeline.duration().saturating_sub(1));
        let primary = timeline
            .track_for_clip(id)
            .filter(|track| track.enabled && track.kind.is_overlay())
            .and_then(|track| track.clip_at(position))
            .filter(|clip| clip.id == id);
        let clip = primary.or_else(|| {
            if group.is_empty() {
                return None;
            }
            timeline
                .tracks()
                .iter()
                .filter(|track| track.enabled && track.kind.is_overlay())
                .filter_map(|track| track.clip_at(position))
                .find(|clip| group.contains(&clip.id))
        })?;
        Some((clip.id, clip.text.as_ref()?))
    }

    pub(super) fn apply_canvas_transform(
        &mut self,
        ctx: &egui::Context,
        id: u64,
        transform: CanvasTransform,
    ) {
        let Some(before) = self.timeline.clip(id).and_then(|clip| clip.text.clone()) else {
            return;
        };
        let style = match transform {
            CanvasTransform::Move(position) => {
                let styles = self.selected_overlay_styles();
                if styles.len() > 1 {
                    let ids = styles.iter().map(|&(id, _)| id).collect::<Vec<_>>();
                    let changes = TextStyleChanges {
                        position: Some(position),
                        ..TextStyleChanges::default()
                    };
                    if self.update_overlay_styles(&ids, changes) {
                        self.text_inspector
                            .synchronize_batch_position(&styles, position);
                        ctx.request_repaint();
                    }
                    return;
                }
                TextStyle {
                    position: Some(position),
                    ..before.style
                }
            }
            CanvasTransform::Resize(style) => style,
        };
        let after = OverlayText {
            style,
            ..before.clone()
        };
        match self.timeline.update_text(id, after.clone()) {
            Ok(()) => {
                self.text_inspector
                    .synchronize_transform(id, &before, &after);
                self.notice = None;
                ctx.request_repaint();
            }
            Err(error) => self.show_notice(error.to_string(), true),
        }
    }

    pub(super) fn overlay_tools(&mut self, ui: &mut egui::Ui) {
        let Some(tool) = text_panel::tools(ui, self.feature_tab) else {
            return;
        };
        if matches!(tool, TextTool::ImportSrt) {
            self.import_srt();
            return;
        }
        let Some((kind, text)) = tool.preset() else {
            return;
        };
        let at = self.position.min(self.timeline.duration());
        match self
            .timeline
            .insert_text(kind, at, DEFAULT_DURATION_US, text)
        {
            Ok((track, id)) => {
                self.select_overlay(track, id, at);
                self.notice = None;
            }
            Err(error) => self.show_notice(error.to_string(), true),
        }
    }

    fn selected_overlay_styles(&self) -> Vec<(u64, TextStyle)> {
        let selection = self.timeline_view.selected_group(self.selected_clip);
        if selection.is_empty() {
            return Vec::new();
        }
        let ids = selection.iter().copied().collect::<HashSet<_>>();
        self.timeline
            .all_clips()
            .filter(|clip| ids.contains(&clip.id))
            .filter_map(|clip| Some((clip.id, clip.text.as_ref()?.style)))
            .collect()
    }

    pub(super) fn overlay_inspector(&mut self, ui: &mut egui::Ui) -> bool {
        let mut styles = self.selected_overlay_styles();
        if let Some((ids, changes)) = self.text_inspector.pending_batch_edit(&self.timeline)
            && ids.iter().copied().ne(styles.iter().map(|&(id, _)| id))
            && self.commit_overlay_styles(&ids, changes)
        {
            styles = self.selected_overlay_styles();
        }
        if let Some((id, text)) = self.text_inspector.pending_edit(&self.timeline)
            && (self.selected_clip != Some(id) || !styles.is_empty())
            && !text.content.trim().is_empty()
            && self.commit_overlay_edit(id, text)
        {
            styles = self.selected_overlay_styles();
        }
        if !styles.is_empty() {
            if let Some(changes) = self.text_inspector.show_batch(ui, &styles) {
                let ids = styles.iter().map(|&(id, _)| id).collect::<Vec<_>>();
                if self.commit_overlay_styles(&ids, changes) {
                    ui.ctx().request_repaint();
                }
            }
            return true;
        }

        let selected = self.selected_clip.and_then(|id| {
            Some((
                self.timeline.clip(id)?,
                self.timeline.track_for_clip(id)?.kind,
            ))
        });
        let has_overlay = selected.is_some_and(|(clip, _)| clip.text.is_some());
        if let Some((id, text)) = self.text_inspector.show(ui, selected)
            && self.commit_overlay_edit(id, text)
        {
            ui.ctx().request_repaint();
        }
        has_overlay
    }

    pub(super) fn commit_overlay_draft(&mut self) -> bool {
        if let Some((ids, changes)) = self.text_inspector.pending_batch_edit(&self.timeline) {
            return self.commit_overlay_styles(&ids, changes);
        }
        match self.text_inspector.pending_edit(&self.timeline) {
            Some((id, text)) => self.commit_overlay_edit(id, text),
            None => true,
        }
    }

    fn commit_overlay_styles(&mut self, ids: &[u64], changes: TextStyleChanges) -> bool {
        if self.update_overlay_styles(ids, changes) {
            self.text_inspector.clear_batch();
            true
        } else {
            false
        }
    }

    fn update_overlay_styles(&mut self, ids: &[u64], changes: TextStyleChanges) -> bool {
        match self.timeline.update_text_styles(ids, changes) {
            Ok(()) => {
                self.notice = None;
                true
            }
            Err(error) => {
                self.show_notice(error.to_string(), true);
                false
            }
        }
    }

    fn commit_overlay_edit(&mut self, id: u64, text: OverlayText) -> bool {
        match self.timeline.update_text(id, text) {
            Ok(()) => {
                self.notice = None;
                true
            }
            Err(error) => {
                self.show_notice(error.to_string(), true);
                false
            }
        }
    }

    fn import_srt(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("导入 SRT 字幕")
            .set_directory(self.library.directory())
            .add_filter("SRT 字幕", &["srt"])
            .pick_file()
        else {
            return;
        };
        let cues = match subrip::read(&path) {
            Ok(cues) => cues,
            Err(error) => {
                self.show_notice(format!("{error:#}"), true);
                return;
            }
        };
        let mut segments = cues
            .into_iter()
            .map(|cue| {
                let text = OverlayText {
                    content: cue.text,
                    style: TextStyle::default(),
                };
                (cue.start_us, cue.duration_us, text)
            })
            .collect::<Vec<_>>();
        segments.sort_by_key(|(start, _, _)| *start);
        let start = segments.first().map_or(0, |segment| segment.0);
        match self
            .timeline
            .insert_text_batch(TrackKind::Caption, segments)
        {
            Ok(added) => {
                if let Some(&(track, id)) = added.first() {
                    self.select_overlay(track, id, start);
                }
                self.show_notice(format!("已导入 {} 条字幕", added.len()), false);
            }
            Err(error) => self.show_notice(error.to_string(), true),
        }
    }

    pub(super) fn select_overlay(&mut self, track: TrackId, id: u64, at: u64) {
        self.selected_clip = Some(id);
        self.position = at;
        self.timeline_view.reveal_added_track(track);
        self.player.playing = false;
        self.player.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_follows_selected_captions_as_the_playhead_moves() {
        let mut timeline = Timeline::default();
        let caption = |content: &str| OverlayText {
            content: content.into(),
            style: TextStyle::default(),
        };
        let mut insert = |at, content| {
            timeline
                .insert_text(TrackKind::Caption, at, 1_000_000, caption(content))
                .unwrap()
                .1
        };
        let first = insert(0, "第一条");
        let second = insert(1_000_000, "第二条");
        insert(2_000_000, "第三条");
        let group = [first, second];
        let visible =
            |at| Workspace::visible_overlay(&timeline, Some(first), &group, at).map(|(id, _)| id);

        assert_eq!(visible(500_000), Some(first));
        assert_eq!(visible(1_500_000), Some(second));
        assert_eq!(visible(2_500_000), None);
    }
}
