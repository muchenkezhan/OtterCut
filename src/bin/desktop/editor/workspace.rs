use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};
use takeseam::PRODUCT_NAME;
use takeseam::editor_timeline::{PRIMARY_AUDIO, PRIMARY_VIDEO, Timeline, TrackId, TrackKind};

use super::asset_cards::{self, MaterialAction};
use super::asset_info;
use super::background::{BackgroundSettings, BackgroundSource};
use super::background_panel;
use super::canvas_format::CanvasFormat;
use super::dock_model::{DockState, Pane};
use super::export::ExportJob;
use super::feature_panel::{self, FeatureTab};
use super::file_menu::{self, FileAction};
use super::inspector_details;
use super::layout;
use super::library::{AUDIO_EXTENSIONS, Library, filename};
use super::media::{MediaInfo, MediaTools};
use super::player::{Player, Span};
use super::preview_viewport::PreviewViewport;
use super::project;
use super::subtitle_canvas::SubtitleCanvas;
use super::text_panel::TextInspector;
use super::thumbnails::Thumbnails;
use super::timeline_view::{TimelineView, TrackDestination};
#[cfg(windows)]
use super::window_frame;
use super::{dock_presets, dock_view};
use crate::open;
use crate::ui_icons::{ButtonStyle, Icon, icon_button};
use crate::ui_theme;
use takeseam::i18n::{Language, Locale, key};

#[cfg(test)]
#[path = "workspace/drag_drop_tests.rs"]
mod drag_drop_tests;
#[path = "workspace/narration.rs"]
mod narration;
#[path = "workspace/overlays.rs"]
mod overlays;

pub(super) struct Workspace {
    logo: egui::TextureHandle,
    docking: Option<DockState>,
    tools: Option<MediaTools>,
    tools_error: Option<String>,
    library: Library,
    feature_tab: FeatureTab,
    narration: narration::Narration,
    text_inspector: TextInspector,
    subtitle_canvas: SubtitleCanvas,
    material_info: Option<PathBuf>,
    thumbnails: Thumbnails,
    material_drag: asset_cards::MaterialDrag,
    timeline: Timeline,
    timeline_view: TimelineView,
    selected_clip: Option<u64>,
    project: Option<PathBuf>,
    saved_revision: u64,
    canvas_format: CanvasFormat,
    saved_canvas_format: CanvasFormat,
    background: BackgroundSettings,
    saved_background: BackgroundSettings,
    position: u64,
    player: Player,
    preview_viewport: PreviewViewport,
    pending_additions: VecDeque<MaterialAddition>,
    export: ExportJob,
    notice: Option<Notice>,
    media_error: Option<String>,
    focused: bool,
    closing_after_export: bool,
}

struct Notice {
    text: String,
    warning: bool,
}

struct MaterialAddition {
    path: PathBuf,
    destination: TrackDestination,
    at: Option<u64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MaterialScope {
    All,
    Audio,
}

impl Workspace {
    pub(super) fn new(ctx: &egui::Context, logo: egui::TextureHandle, directory: PathBuf) -> Self {
        let (tools, tools_error) = match MediaTools::discover() {
            Ok(tools) => (Some(tools), None),
            Err(error) => (None, Some(format!("{error:#}"))),
        };
        let library = Library::new(directory, tools.as_ref(), ctx);
        let thumbnails = Thumbnails::new(tools.clone(), ctx);
        Self {
            logo,
            docking: Some(if cfg!(test) {
                DockState::default()
            } else {
                DockState::load()
            }),
            timeline_view: TimelineView::with_media(tools.clone(), ctx),
            tools,
            tools_error,
            library,
            feature_tab: FeatureTab::default(),
            narration: narration::Narration::default(),
            text_inspector: TextInspector::default(),
            subtitle_canvas: SubtitleCanvas::default(),
            material_info: None,
            thumbnails,
            material_drag: asset_cards::MaterialDrag::default(),
            timeline: Timeline::default(),
            selected_clip: None,
            project: None,
            saved_revision: 0,
            canvas_format: CanvasFormat::Original,
            saved_canvas_format: CanvasFormat::Original,
            background: BackgroundSettings::default(),
            saved_background: BackgroundSettings::default(),
            position: 0,
            player: Player::default(),
            preview_viewport: PreviewViewport::default(),
            pending_additions: VecDeque::new(),
            export: ExportJob::default(),
            notice: None,
            media_error: None,
            focused: false,
            closing_after_export: false,
        }
    }

    fn duration(&self) -> u64 {
        if !self.timeline.is_empty() {
            self.timeline.duration()
        } else {
            self.library
                .selected_path()
                .and_then(|path| self.library.info(path))
                .map_or_else(
                    || {
                        if matches!(self.background.source, BackgroundSource::Dynamic(_)) {
                            8_000_000
                        } else {
                            0
                        }
                    },
                    |info| info.duration_us,
                )
        }
    }

    fn show_notice(&mut self, text: impl Into<String>, warning: bool) {
        self.notice = Some(Notice {
            text: text.into(),
            warning,
        });
    }

    fn project_is_dirty(&self) -> bool {
        self.timeline.revision() != self.saved_revision
            || self.canvas_format != self.saved_canvas_format
            || self.background != self.saved_background
    }

    fn title(&self) -> String {
        let name = self
            .project
            .as_deref()
            .map(filename)
            .unwrap_or_else(|| String::from("未命名工程"));
        if self.project_is_dirty() {
            format!("{name} *")
        } else {
            name
        }
    }

    fn import(&mut self, scope: MaterialScope) {
        let dialog = rfd::FileDialog::new().set_directory(self.library.directory());
        let dialog = match scope {
            MaterialScope::All => {
                let video = ["mp4", "m4v", "mov", "mkv", "webm", "avi"];
                let extensions = [video.as_slice(), AUDIO_EXTENSIONS].concat();
                dialog
                    .set_title("导入素材")
                    .add_filter("视频和音频", &extensions)
            }
            MaterialScope::Audio => dialog
                .set_title("导入音频")
                .add_filter("音频文件", AUDIO_EXTENSIONS),
        };
        let Some(paths) = dialog.pick_files() else {
            return;
        };
        for (index, path) in paths.into_iter().enumerate() {
            self.library.retry(&path);
            let _ = self.library.request(&path);
            self.library.insert(path, index == 0);
        }
        if self.timeline.is_empty() {
            self.position = 0;
            self.player.playing = false;
            self.player.reset();
        }
        self.notice = None;
    }

    fn queue_material(
        &mut self,
        ctx: &egui::Context,
        path: PathBuf,
        destination: TrackDestination,
        at: Option<u64>,
    ) {
        self.pending_additions.push_back(MaterialAddition {
            path,
            destination,
            at,
        });
        self.notice = None;
        ctx.request_repaint();
    }

    fn add_materials_when_ready(&mut self) {
        while let Some(addition) = self.pending_additions.front() {
            let Some(info) = self
                .library
                .request(&addition.path)
                .map(|info| info.map_err(str::to_owned))
            else {
                break;
            };
            let addition = self
                .pending_additions
                .pop_front()
                .expect("material addition queue was nonempty");
            let media = match info {
                Ok(media) => media,
                Err(error) => {
                    self.show_notice(error, true);
                    continue;
                }
            };
            let mut destination = addition.destination;
            if let TrackDestination::New {
                kind: TrackKind::Video,
                before: Some(before),
            } = destination
                && !media.has_video
                && media.has_audio
                && self.timeline.tracks().windows(2).any(|pair| {
                    pair[0].kind.stack_order() <= TrackKind::Video.stack_order()
                        && pair[1].kind == TrackKind::Audio
                        && pair[1].id == before
                })
            {
                // At the video/audio seam either kind fits. The drag can precede probing.
                destination = TrackDestination::New {
                    kind: TrackKind::Audio,
                    before: Some(before),
                };
            }
            let (kind, start) = match destination {
                TrackDestination::Existing(id) => {
                    let Some(track) = self.timeline.track(id) else {
                        self.show_notice("目标轨道已被移除", true);
                        continue;
                    };
                    let start = addition.at.map_or_else(
                        || track.duration(),
                        |at| self.timeline.material_insertion_point_on_track(id, at),
                    );
                    (track.kind, start)
                }
                TrackDestination::New { kind, .. } => (kind, addition.at.unwrap_or_default()),
            };
            let incompatible = match kind {
                TrackKind::Video if !media.has_video => Some("把音频素材拖到音频轨"),
                TrackKind::Audio if !media.has_audio => Some("音频轨需要包含声音的素材"),
                TrackKind::Caption | TrackKind::Text => Some("请把媒体素材拖到视频轨或音频轨"),
                _ => None,
            };
            if let Some(message) = incompatible {
                self.show_notice(message, true);
                continue;
            }
            let inserted = match destination {
                TrackDestination::Existing(track) => self
                    .timeline
                    .insert_media_on_track(track, addition.path, media.duration_us, start)
                    .map(|clip| (None, clip)),
                TrackDestination::New { kind, before } => self
                    .timeline
                    .insert_media_on_new_track_before(
                        kind,
                        before,
                        addition.path,
                        media.duration_us,
                        start,
                    )
                    .map(|(track, clip)| (Some(track), clip)),
            };
            match inserted {
                Ok((track, id)) => {
                    if let Some(track) = track {
                        self.timeline_view.reveal_added_track(track);
                    }
                    self.position = start;
                    self.selected_clip = Some(id);
                    self.player.playing = false;
                    self.player.reset();
                }
                Err(error) => self.show_notice(error.to_string(), true),
            }
        }
    }

    fn sync_preview(&mut self, ctx: &egui::Context) {
        self.media_error = None;
        let span = if self.timeline.is_empty() {
            let asset = self.library.selected_path().map(Path::to_owned);
            if let Some(path) = asset {
                match self.library.request(&path) {
                    Some(Ok(media)) => Some(Span::for_asset(&path, media)),
                    Some(Err(error)) => {
                        self.media_error = Some(error.to_owned());
                        None
                    }
                    None => None,
                }
            } else {
                None
            }
        } else {
            // Prepare the editable caption as a separate layer while paused, so a drag
            // does not stop video decoding or replace the background frame.
            let exclude = if self.player.playing {
                None
            } else {
                Self::visible_overlay(
                    &self.timeline,
                    self.selected_clip,
                    self.timeline_view.selected_group(self.selected_clip),
                    self.position,
                )
                .map(|(id, _)| id)
            };
            Span::at(&self.timeline, self.position).map(|span| span.with_excluded_overlay(exclude))
        };
        if self.media_error.is_some() {
            self.player.playing = false;
        }
        let span = span.or_else(|| {
            self.background
                .enabled()
                .then(|| Span::for_background(self.duration()))
        });
        if let Some(error) = self.player.sync(
            ctx,
            self.tools.as_ref(),
            span.map(|span| {
                span.with_canvas_format(self.canvas_format)
                    .with_background(&self.background)
            }),
            &mut self.library,
            self.position,
        ) {
            self.media_error = Some(error);
        }
    }

    fn open_project(&mut self, ctx: &egui::Context) {
        if !self.commit_overlay_draft() {
            return;
        }
        if self.project_is_dirty()
            && rfd::MessageDialog::new()
                .set_title("打开剪辑工程")
                .set_description("当前工程有未保存的修改。继续打开另一个工程？")
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                != rfd::MessageDialogResult::Yes
        {
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .set_title("打开剪辑工程")
            .set_directory(self.library.directory())
            .add_filter(format!("{PRODUCT_NAME} 工程"), &["takeproj"])
            .pick_file()
        else {
            return;
        };
        match project::load(&path) {
            Ok((timeline, position, canvas_format, background)) => {
                for clip in timeline.all_clips().filter(|clip| clip.text.is_none()) {
                    self.library.insert(clip.path.clone(), false);
                }
                self.timeline = timeline;
                self.timeline_view = TimelineView::with_media(self.tools.clone(), ctx);
                self.saved_revision = self.timeline.revision();
                self.canvas_format = canvas_format;
                self.saved_canvas_format = canvas_format;
                self.saved_background = background.clone();
                self.background = background;
                self.position = position;
                self.selected_clip = None;
                self.text_inspector = TextInspector::default();
                self.subtitle_canvas.cancel();
                self.preview_viewport.reset();
                self.pending_additions.clear();
                self.player.playing = false;
                self.player.reset();
                self.project = Some(path);
                self.show_notice("工程已打开", false);
            }
            Err(error) => self.show_notice(format!("{error:#}"), true),
        }
    }

    fn save_project(&mut self, save_as: bool) {
        if !self.commit_overlay_draft() {
            return;
        }
        let path = if !save_as && self.project.is_some() {
            self.project.clone()
        } else {
            rfd::FileDialog::new()
                .set_title("保存剪辑工程")
                .set_directory(self.library.directory())
                .set_file_name(format!("{PRODUCT_NAME}.takeproj"))
                .add_filter(format!("{PRODUCT_NAME} 工程"), &["takeproj"])
                .save_file()
        };
        let Some(path) = path else { return };
        match project::save(
            &path,
            &self.timeline,
            self.position,
            self.canvas_format,
            &self.background,
        ) {
            Ok(()) => {
                self.project = Some(path);
                self.saved_revision = self.timeline.revision();
                self.saved_canvas_format = self.canvas_format;
                self.saved_background.clone_from(&self.background);
                self.show_notice("工程已保存", false);
            }
            Err(error) => self.show_notice(format!("{error:#}"), true),
        }
    }

    fn export_video(&mut self, ctx: &egui::Context) {
        if !self.commit_overlay_draft() {
            return;
        }
        let Some(tools) = self.tools.clone() else {
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .set_title("导出 MP4")
            .set_directory(self.library.directory())
            .set_file_name(format!("{PRODUCT_NAME}-剪辑.mp4"))
            .add_filter("MP4 视频", &["mp4"])
            .save_file()
        else {
            return;
        };
        self.player.playing = false;
        self.player.reset();
        if let Err(error) = self.export.start(
            ctx,
            tools,
            self.timeline.tracks().to_vec(),
            path,
            self.canvas_format,
            self.background.clone(),
        ) {
            self.show_notice(format!("{error:#}"), true);
        } else {
            self.show_notice("正在导出时间线…", false);
        }
    }

    fn header_actions(&mut self, ui: &mut egui::Ui, docking: &mut DockState) -> bool {
        ui.allocate_ui_with_layout(
            egui::vec2(68.0, 32.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                if self.export.running() {
                    if icon_button(ui, Icon::Stop, "取消导出", true, ButtonStyle::Quiet).clicked()
                    {
                        self.export.cancel();
                    }
                    ui.label(
                        RichText::new(format!(
                            "{}%",
                            (self.export.progress() * 100.0).floor() as u32
                        ))
                        .monospace()
                        .size(11.0),
                    );
                } else {
                    let can_export =
                        self.tools.is_some() && !self.timeline.is_empty() && !self.narration.busy();
                    if layout::export_button(ui, can_export).clicked() {
                        self.export_video(ui.ctx());
                    }
                }
            },
        );
        ui.separator();
        let (icon, label) = if ui.ctx().theme() == egui::Theme::Dark {
            (Icon::Sun, "切换到浅色模式")
        } else {
            (Icon::Moon, "切换到深色模式")
        };
        if icon_button(ui, icon, label, true, ButtonStyle::Quiet).clicked()
            && let Err(error) = ui_theme::toggle(ui.ctx())
        {
            self.show_notice(format!("无法保存外观设置（{error}）"), true);
        }
        let locale = Locale::from_context(ui.ctx());
        ui.menu_button(locale.text(key::LANGUAGE), |ui| {
            for language in [Language::ZhCn, Language::EnUs] {
                let selected = locale.language() == language;
                let label = match language {
                    Language::ZhCn => locale.text(key::LANGUAGE_ZH),
                    Language::EnUs => locale.text(key::LANGUAGE_EN),
                };
                if ui.selectable_label(selected, label).clicked() {
                    if let Err(error) = Locale::set_context_and_persist(ui.ctx(), language) {
                        self.show_notice(format!("无法保存语言设置（{error}）"), true);
                    }
                    ui.close();
                }
            }
        });
        dock_presets::show(ui, docking)
    }

    fn feature_panel(&mut self, ui: &mut egui::Ui) {
        feature_panel::tabs(ui, &mut self.feature_tab, self.narration.busy());
        match self.feature_tab {
            FeatureTab::Materials => self.materials(ui, MaterialScope::All),
            FeatureTab::Audio => {
                if feature_panel::action_row(ui, Icon::FolderPlus, "导入音频") {
                    self.import(MaterialScope::Audio);
                }
                self.narration_tools(ui);
                self.materials(ui, MaterialScope::Audio);
            }
            FeatureTab::Captions | FeatureTab::Text => {
                self.material_drag.hide(ui.ctx());
                self.overlay_tools(ui);
            }
            FeatureTab::Background => {
                self.material_drag.hide(ui.ctx());
                if let Some(background) =
                    background_panel::show(ui, &self.background, self.library.directory())
                {
                    self.background = background;
                    self.subtitle_canvas.cancel();
                    ui.ctx().request_repaint();
                }
            }
        }
    }

    fn active_media_track(&self, source: &Path, scope: MaterialScope) -> Option<TrackId> {
        let active = self.timeline_view.active_track(&self.timeline);
        Self::media_track(&self.timeline, active, self.library.info(source), scope)
    }

    fn media_track(
        timeline: &Timeline,
        active: TrackId,
        source: Option<MediaInfo>,
        scope: MaterialScope,
    ) -> Option<TrackId> {
        if timeline.track(active).is_some_and(|track| {
            !track.kind.is_overlay()
                && (scope == MaterialScope::All || track.kind == TrackKind::Audio)
        }) {
            return Some(active);
        }
        let audio = scope == MaterialScope::Audio
            || source.is_some_and(|info| !info.has_video && info.has_audio);
        let (primary, kind) = if audio {
            (PRIMARY_AUDIO, TrackKind::Audio)
        } else {
            (PRIMARY_VIDEO, TrackKind::Video)
        };
        timeline
            .track(primary)
            .filter(|track| track.kind == kind)
            .or_else(|| timeline.tracks().iter().find(|track| track.kind == kind))
            .map(|track| track.id)
    }

    fn materials(&mut self, ui: &mut egui::Ui, scope: MaterialScope) {
        let timeline = &self.timeline;
        let library = &self.library;
        let active = self.timeline_view.active_track(timeline);
        let can_add = self.tools.is_some() && self.pending_additions.is_empty();
        let add_to = |path: &Path| {
            if !can_add {
                return None;
            }
            let track = Self::media_track(timeline, active, library.info(path), scope)?;
            timeline.track(track).map(|track| track.kind)
        };
        let thumbnails = &mut self.thumbnails;
        let drag = &mut self.material_drag;
        let input = match scope {
            MaterialScope::All => asset_cards::show(ui, library, thumbnails, drag, add_to),
            MaterialScope::Audio => asset_cards::show_audio(ui, library, thumbnails, drag, add_to),
        };
        if let Some(index) = input.selected
            && self.library.selected != Some(index)
        {
            self.library.selected = Some(index);
            if self.timeline.is_empty() {
                self.position = 0;
                self.player.playing = false;
                self.player.reset();
            }
        }
        if let Some(action) = input.action {
            self.material_action(ui.ctx(), action, scope);
        }
    }

    fn material_action(
        &mut self,
        ctx: &egui::Context,
        action: MaterialAction,
        scope: MaterialScope,
    ) {
        match action {
            MaterialAction::Import => self.import(scope),
            MaterialAction::Add(path) => {
                if let Some(track) = self.active_media_track(&path, scope) {
                    self.queue_material(ctx, path, TrackDestination::Existing(track), None);
                }
            }
            MaterialAction::Inspect(path) => self.material_info = Some(path),
            MaterialAction::OpenFolder(path) => {
                if let Some(folder) = path.parent().filter(|folder| folder.is_dir()) {
                    if let Err(error) = open(folder) {
                        self.show_notice(format!("{error:#}"), true);
                    }
                } else {
                    self.show_notice("素材所在文件夹不可用", true);
                }
            }
            MaterialAction::CopyPath(path) => {
                ctx.copy_text(path.to_string_lossy().into_owned());
                self.show_notice("已复制文件路径", false);
            }
            MaterialAction::Remove(path) => {
                let was_previewed = self.timeline.is_empty()
                    && self.library.selected_path() == Some(path.as_path());
                if self.library.remove(&path) {
                    if self.material_info.as_deref() == Some(path.as_path()) {
                        self.material_info = None;
                    }
                    if was_previewed {
                        self.position = 0;
                        self.player.playing = false;
                        self.player.reset();
                    }
                    self.show_notice("已从素材库移除", false);
                    ctx.request_repaint();
                }
            }
        }
    }

    fn timeline(&mut self, ui: &mut egui::Ui) {
        let old_position = self.position;
        let old_revision = self.timeline.revision();
        let old_selection = self.selected_clip;
        let insert_track = self.library.selected_path().and_then(|path| {
            let scope = if self.feature_tab == FeatureTab::Audio && self.library.is_audio(path) {
                MaterialScope::Audio
            } else {
                MaterialScope::All
            };
            self.active_media_track(path, scope)
        });
        let add_enabled =
            self.tools.is_some() && insert_track.is_some() && self.pending_additions.is_empty();
        let mut position = if self.timeline.is_empty() {
            0
        } else {
            self.position
        };
        let input = self.timeline_view.draw(
            ui,
            &mut self.timeline,
            &mut position,
            &mut self.selected_clip,
            add_enabled,
        );
        if let Some(drop) = input.material_drop {
            self.queue_material(ui.ctx(), drop.path, drop.destination, Some(drop.at));
        } else if input.add_material
            && let Some(path) = self.library.selected_path()
            && let Some(track) = insert_track
        {
            self.queue_material(
                ui.ctx(),
                path.to_owned(),
                TrackDestination::Existing(track),
                None,
            );
        }
        if let Some(error) = input.error {
            self.show_notice(error, true);
        } else if let Some(message) = input.clipboard_feedback {
            self.show_notice(message, false);
        }
        if !self.timeline.is_empty() {
            self.position = position;
        }
        if old_selection != self.selected_clip
            && let Some(id) = self.selected_clip
            && let Some(track) = self
                .timeline
                .track_for_clip(id)
                .filter(|track| track.enabled)
            && let Some(clip) = track
                .clips
                .iter()
                .find(|clip| clip.id == id && clip.text.is_some())
            && !(clip.start..clip.end()).contains(&self.position)
        {
            self.position = clip.start;
            self.player.playing = false;
        }
        if self.timeline.revision() != old_revision {
            self.position = self.position.min(self.timeline.duration());
            self.player.playing = false;
            self.player.reset();
        } else if self.position != old_position {
            self.player.reset();
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        let logo = self.logo.clone();
        let title = self.title();
        let mut docking = self.docking.take().expect("workspace layout initialized");
        let can_open = !self.narration.busy();
        let mut file_action = None;
        let mut layout_changed = false;
        layout::header(
            ui,
            &logo,
            &title,
            |ui| file_action = file_menu::show(ui, can_open),
            |ui| layout_changed = self.header_actions(ui, &mut docking),
        );
        match file_action {
            Some(FileAction::Import) => self.import(MaterialScope::All),
            Some(FileAction::Open) => self.open_project(ui.ctx()),
            Some(FileAction::Save) => self.save_project(false),
            None => {}
        }
        let fault = self
            .tools_error
            .as_deref()
            .or(self.media_error.as_deref())
            .or(self.player.error())
            .or(self.player.audio_error());
        if let Some(fault) = fault {
            layout::status(ui, fault, true);
        } else if !self.pending_additions.is_empty() {
            layout::status(ui, "正在读取素材…", false);
        } else if let Some(notice) = &self.notice {
            layout::status(ui, &notice.text, notice.warning);
        }
        let caption = if !self.timeline.is_empty() {
            String::from("时间线预览")
        } else {
            self.library
                .selected_path()
                .map(filename)
                .unwrap_or_else(|| String::from("预览"))
        };
        layout_changed |= dock_view::show(ui, &mut docking, &caption, |pane, ui| match pane {
            Pane::Materials => self.feature_panel(ui),
            Pane::Preview => self.preview(ui),
            Pane::Inspector => self.inspector(ui),
            Pane::Timeline => self.timeline(ui),
        });
        if layout_changed
            && !cfg!(test)
            && let Err(error) = docking.save()
        {
            self.show_notice(format!("无法保存工作区（{error:#}）"), true);
        }
        if layout_changed {
            ui.ctx().request_repaint();
        }
        self.docking = Some(docking);
        if let Some(path) = &self.material_info
            && !asset_info::show(ui.ctx(), &mut self.library, path)
        {
            self.material_info = None;
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let duration = self.duration();
        let empty = if self.library.selected_path().is_none() && self.timeline.is_empty() {
            "导入素材开始剪辑"
        } else if self.tools_error.is_some()
            || self.media_error.is_some()
            || self.player.error().is_some()
        {
            "预览不可用"
        } else {
            self.player.blank_message().unwrap_or("正在读取预览…")
        };
        let selection = Self::visible_overlay(
            &self.timeline,
            self.selected_clip,
            self.timeline_view.selected_group(self.selected_clip),
            self.position,
        );
        if selection.is_none() {
            self.subtitle_canvas.cancel();
        }
        let frame_excludes_text = selection.is_some_and(|(id, _)| self.player.texture_excludes(id));
        let mut canvas_edit = None;
        let subtitle_canvas = &mut self.subtitle_canvas;
        let input = layout::monitor(
            ui,
            &mut self.preview_viewport,
            self.player.texture(),
            self.player.texture_composition_size(),
            empty,
            self.position,
            duration,
            self.player.playing,
            self.player.quality(),
            self.canvas_format,
            |ui, image| {
                if let Some((id, text)) = selection
                    && let Some(transform) =
                        subtitle_canvas.show(ui, image, id, text, frame_excludes_text)
                {
                    canvas_edit = Some((id, transform));
                }
            },
        );
        if canvas_edit.is_some() || self.subtitle_canvas.excluded_overlay().is_some() {
            self.player.playing = false;
        }
        if let Some((id, transform)) = canvas_edit {
            self.apply_canvas_transform(ui.ctx(), id, transform);
        }
        if let Some(quality) = input.quality {
            self.player.set_quality(quality);
            ui.ctx().request_repaint();
        }
        if let Some(canvas_format) = input.canvas_format {
            self.canvas_format = canvas_format;
            self.subtitle_canvas.cancel();
            ui.ctx().request_repaint();
        }
        if let Some(seek) = input.seek {
            self.subtitle_canvas.cancel();
            self.position = seek;
            self.player.reset();
        }
        if input.toggle {
            self.subtitle_canvas.cancel();
            self.player.toggle(&mut self.position, duration);
        }
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        if self.overlay_inspector(ui) {
            return;
        }
        if let Some(clip) = self.selected_clip.and_then(|id| self.timeline.clip(id)) {
            inspector_details::media(ui, &clip.path, self.library.info(&clip.path), Some(clip));
        } else if !self.timeline.is_empty() {
            inspector_details::sequence(ui, &self.timeline);
        } else if let Some(path) = self.library.selected_path() {
            inspector_details::media(ui, path, self.library.info(path), None);
        }
    }
}

impl eframe::App for Workspace {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        #[cfg(windows)]
        window_frame::restore_clipboard_shortcut(ctx, raw_input);
        self.material_drag.process(ctx, raw_input);
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ui_theme::sync(ctx);
        let focused = ctx.input(|input| input.viewport().focused.unwrap_or(false));
        if focused && !self.focused {
            self.library.refresh_recordings();
        }
        self.focused = focused;
        self.library.poll();
        if ctx.input(|input| input.viewport().close_requested()) {
            if self.narration.busy() {
                self.feature_tab = FeatureTab::Audio;
                self.narration.stop();
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else if self.export.running() {
                self.closing_after_export = true;
                self.export.cancel();
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }
        self.poll_narration(ctx);
        if let Some(result) = self.export.poll() {
            if self.closing_after_export {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                match result {
                    Ok(path) => self.show_notice(format!("导出完成 {}", path.display()), false),
                    Err(error) => self.show_notice(format!("{error:#}"), true),
                }
            }
        }
        self.add_materials_when_ready();
        let duration = self.duration();
        self.player.advance(ctx, &mut self.position, duration);
        self.sync_preview(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
            let save_as = ctx.input(|i| i.modifiers.shift);
            self.save_project(save_as);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Space) && !i.modifiers.command)
            && !ctx.egui_wants_keyboard_input()
        {
            let duration = self.duration();
            self.player.toggle(&mut self.position, duration);
        }
        self.draw(ui);
        self.sync_preview(&ctx);
    }
}
