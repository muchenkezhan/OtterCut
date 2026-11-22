use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, Result};
use eframe::egui;
use takeseam::PRODUCT_NAME;
use takeseam::editor_timeline::TrackKind;

use super::super::feature_panel::{self, NarrationAction, NarrationView};
use super::super::media::MediaInfo;
use super::super::narration_capture::NarrationCapture;
use super::Workspace;

#[derive(Default)]
pub(super) struct Narration {
    pending: Option<Pending>,
    error: Option<String>,
}

struct Pending {
    capture: NarrationCapture,
    at: u64,
    stopping: bool,
}

impl Narration {
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub(super) fn stop(&mut self) {
        if let Some(pending) = &mut self.pending
            && !pending.stopping
        {
            pending.capture.request_stop();
            pending.stopping = true;
        }
    }

    fn view(&self) -> NarrationView {
        match &self.pending {
            None => NarrationView::Idle,
            Some(pending) if pending.stopping => NarrationView::Saving,
            Some(pending) => pending
                .capture
                .started_at()
                .map_or(NarrationView::Starting, |started| {
                    NarrationView::Recording(started.elapsed())
                }),
        }
    }
}

impl Workspace {
    pub(super) fn narration_tools(&mut self, ui: &mut egui::Ui) {
        let action = feature_panel::narration_controls(
            ui,
            self.narration.view(),
            self.narration.error.as_deref(),
        );
        match action {
            Some(NarrationAction::Start) => self.start_narration(ui.ctx()),
            Some(NarrationAction::Stop) => self.narration.stop(),
            None => {}
        }
    }

    fn start_narration(&mut self, ctx: &egui::Context) {
        if self.narration.busy() {
            return;
        }
        let at = if self.timeline.is_empty() {
            0
        } else {
            self.position
        };
        let directory = self
            .project
            .as_deref()
            .and_then(std::path::Path::parent)
            .unwrap_or_else(|| self.library.directory())
            .join(format!("{PRODUCT_NAME} 旁白"));
        if let Some(error) = self.narration.error.take()
            && self
                .notice
                .as_ref()
                .is_some_and(|notice| notice.text == error)
        {
            self.notice = None;
        }
        self.player.playing = false;
        self.player.reset();
        match NarrationCapture::start(directory, ctx) {
            Ok(capture) => {
                self.narration.pending = Some(Pending {
                    capture,
                    at,
                    stopping: false,
                });
            }
            Err(error) => self.narration.error = Some(format!("{error:#}")),
        }
    }

    pub(super) fn poll_narration(&mut self, ctx: &egui::Context) {
        let Some(pending) = &mut self.narration.pending else {
            return;
        };
        let Some(result) = pending.capture.poll() else {
            if !pending.stopping && pending.capture.started_at().is_some() {
                ctx.request_repaint_after(Duration::from_millis(200));
            }
            return;
        };
        let at = pending.at;
        self.narration.pending = None;
        match result.and_then(|recording| {
            let duration = recording.duration_us;
            let path = recording.persist()?;
            self.insert_narration(path, duration, at)
        }) {
            Ok(()) => self.show_notice("旁白已加入时间线", false),
            Err(error) => {
                let message = format!("{error:#}");
                self.narration.error = Some(message.clone());
                self.show_notice(message, true);
            }
        }
    }

    fn insert_narration(&mut self, path: PathBuf, duration: u64, at: u64) -> Result<()> {
        self.library.insert_with_info(
            path.clone(),
            MediaInfo {
                duration_us: duration,
                width: 0,
                height: 0,
                has_video: false,
                has_audio: true,
            },
            true,
        );
        let end = at.checked_add(duration).context("旁白超出时间线范围")?;
        let active = self.timeline_view.active_track(&self.timeline);
        let target = self
            .timeline
            .tracks()
            .iter()
            .filter(|track| {
                track.kind == TrackKind::Audio
                    && track.enabled
                    && track
                        .clips
                        .iter()
                        .all(|clip| clip.end() <= at || clip.start >= end)
            })
            .min_by_key(|track| track.id != active);
        let (track, clip) = match target {
            Some(track) => (
                track.id,
                self.timeline
                    .insert_media_on_track(track.id, path, duration, at)?,
            ),
            None => {
                self.timeline
                    .insert_media_on_new_track(TrackKind::Audio, path, duration, at)?
            }
        };
        self.timeline_view.reveal_added_track(track);
        self.selected_clip = Some(clip);
        self.position = at;
        self.player.playing = false;
        self.player.reset();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use takeseam::editor_timeline::{PRIMARY_AUDIO, PRIMARY_VIDEO, Timeline};

    #[test]
    fn narration_keeps_its_start_without_displacing_existing_audio() {
        let ctx = egui::Context::default();
        let directory = tempfile::tempdir().unwrap();
        let logo = ctx.load_texture(
            "logo",
            egui::ColorImage::filled([1, 1], egui::Color32::WHITE),
            egui::TextureOptions::NEAREST,
        );
        let mut workspace = Workspace::new(&ctx, logo, directory.path().into());
        workspace.timeline = Timeline::default();
        workspace
            .timeline
            .insert_media_on_track(PRIMARY_VIDEO, "screen.mp4".into(), 5_000_000, 0)
            .unwrap();
        let existing = workspace
            .timeline
            .insert_media_on_track(PRIMARY_AUDIO, "existing.wav".into(), 1_000_000, 2_000_000)
            .unwrap();
        let original = workspace.timeline.tracks().to_vec();
        let path = directory.path().join("旁白-test.wav");

        for (start, new_track) in [(500_000, false), (1_500_000, true)] {
            workspace
                .insert_narration(path.clone(), 1_000_000, start)
                .unwrap();
            let selected = workspace.selected_clip.unwrap();
            let track = workspace.timeline.track_for_clip(selected).unwrap();
            assert_eq!(track.kind, TrackKind::Audio);
            assert_eq!(track.id != PRIMARY_AUDIO, new_track);
            assert_eq!(workspace.timeline.clip(selected).unwrap().start, start);
            assert_eq!(workspace.timeline.clip(existing).unwrap().start, 2_000_000);
            assert_eq!(workspace.library.selected_path(), Some(path.as_path()));
            assert!(workspace.library.info(&path).unwrap().has_audio);
            assert!(workspace.timeline.undo());
            assert_eq!(workspace.timeline.tracks(), original);
        }
    }
}
