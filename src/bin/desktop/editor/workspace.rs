use std::collections::{HashSet, VecDeque};
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use eframe::egui::{self, Align, Layout, RichText, vec2};
use takeseam::PRODUCT_NAME;
use takeseam::editor_timeline::{
    OverlayText, PRIMARY_AUDIO, PRIMARY_VIDEO, TextStyle, Timeline, TrackId, TrackKind,
    TransitionSpec,
};

use super::asset_cards::{self, MaterialAction};
use super::asset_info;
use super::background::{BackgroundSettings, BackgroundSource};
use super::background_panel;
use super::canvas_format::CanvasFormat;
use super::cursor_effect::{
    CursorClickEffect, CursorEffect, CursorStyle, CursorTelemetry, ZOOM_HOLD_RANGE, ZOOM_PAN_RANGE,
    ZOOM_SCALE_RANGE, ZOOM_TRANSITION_RANGE, timeline_time_for_sample,
};
use super::dock_model::{DockState, Pane};
use super::export::{ExportJob, ExportQuality, ExportResolution, ExportSettings};
use super::feature_panel::{self, FeatureTab};
use super::file_menu::{self, FileAction};
use super::inspector_details;
use super::layout;
use super::library::{AUDIO_EXTENSIONS, Library, filename};
use super::media::{IMAGE_EXTENSIONS, MediaInfo, MediaTools, PreviewQuality};
use super::player::{Player, Span};
use super::proxy_cache::ProxyCache;
use super::preview_viewport::PreviewViewport;
use super::project;
use super::speech::{SpeechCue, SpeechJob};
use super::subtitle_canvas::SubtitleCanvas;
use super::text_panel::TextInspector;
use super::thumbnails::Thumbnails;
use super::timeline_view::{CursorTrackData, TimelineView, TrackDestination};
use super::transition_panel;
#[cfg(windows)]
use super::window_frame;
use super::{dock_presets, dock_view};
use crate::open;
use crate::ui_icons::{ButtonStyle, Icon, icon_button};
use crate::ui_theme;
use takeseam::i18n::{Language, Locale, key};

const PREVIEW_SCRUB_STEP_US: u64 = 100_000;

#[cfg(test)]
#[path = "workspace/drag_drop_tests.rs"]
mod drag_drop_tests;
#[cfg(test)]
#[path = "workspace/cursor_tracks_tests.rs"]
mod cursor_tracks_tests;
#[path = "workspace/narration.rs"]
mod narration;
#[path = "workspace/overlays.rs"]
mod overlays;
#[cfg(test)]
#[path = "workspace/project_lifecycle_tests.rs"]
mod project_lifecycle_tests;

pub(super) struct Workspace {
    logo: egui::TextureHandle,
    docking: Option<DockState>,
    tools: Option<MediaTools>,
    tools_error: Option<String>,
    library: Library,
    feature_tab: FeatureTab,
    transition_panel: transition_panel::State,
    narration: narration::Narration,
    text_inspector: TextInspector,
    subtitle_canvas: SubtitleCanvas,
    material_info: Option<PathBuf>,
    thumbnails: Thumbnails,
    preview_thumbnails: Thumbnails,
    proxy_cache: ProxyCache,
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
    cursor_effect: CursorEffect,
    saved_cursor_effect: CursorEffect,
    cursor_marker_cache: CursorMarkerCache,
    markers: Vec<u64>,
    saved_markers: Vec<u64>,
    position: u64,
    player: Player,
    volume_preview: Option<(u64, i16)>,
    preview_viewport: PreviewViewport,
    pending_additions: VecDeque<MaterialAddition>,
    export: ExportJob,
    export_settings: Option<ExportSettings>,
    notice: Option<Notice>,
    media_error: Option<String>,
    focused: bool,
    closing_after_export: bool,
    home: bool,
    speech_job: Option<SpeechJob>,
}

struct Notice {
    text: String,
    warning: bool,
    loading: bool,
}

struct MaterialAddition {
    path: PathBuf,
    destination: TrackDestination,
    at: Option<u64>,
}

#[derive(Default)]
struct CursorMarkerCache {
    key: Option<u64>,
    markers: Vec<CursorTrackData>,
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
        let preview_thumbnails = Thumbnails::preview(tools.clone(), ctx);
        let proxy_cache = ProxyCache::new(tools.clone(), ctx, library.directory());
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
            transition_panel: transition_panel::State::default(),
            narration: narration::Narration::default(),
            text_inspector: TextInspector::default(),
            subtitle_canvas: SubtitleCanvas::default(),
            material_info: None,
            thumbnails,
            preview_thumbnails,
            proxy_cache,
            material_drag: asset_cards::MaterialDrag::default(),
            timeline: Timeline::from_tracks(Vec::new()).expect("empty timeline is valid"),
            selected_clip: None,
            project: None,
            saved_revision: 0,
            canvas_format: CanvasFormat::Original,
            saved_canvas_format: CanvasFormat::Original,
            background: BackgroundSettings::default(),
            saved_background: BackgroundSettings::default(),
            cursor_effect: CursorEffect::default_for(1),
            saved_cursor_effect: CursorEffect::default_for(1),
            cursor_marker_cache: CursorMarkerCache::default(),
            markers: Vec::new(),
            saved_markers: Vec::new(),
            position: 0,
            player: Player::default(),
            volume_preview: None,
            preview_viewport: PreviewViewport::default(),
            pending_additions: VecDeque::new(),
            export: ExportJob::default(),
            export_settings: None,
            notice: None,
            media_error: None,
            focused: false,
            closing_after_export: false,
            home: !cfg!(test),
            speech_job: None,
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
            loading: false,
        });
    }

    fn show_loading_notice(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice {
            text: text.into(),
            warning: false,
            loading: true,
        });
    }

    fn project_is_dirty(&self) -> bool {
        self.timeline.revision() != self.saved_revision
            || self.canvas_format != self.saved_canvas_format
            || self.background != self.saved_background
            || self.cursor_effect != self.saved_cursor_effect
            || self.markers != self.saved_markers
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
                let extensions = [video.as_slice(), IMAGE_EXTENSIONS, AUDIO_EXTENSIONS].concat();
                dialog
                    .set_title("导入素材")
                    .add_filter("视频、图片和音频", &extensions)
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
            if let TrackDestination::New { kind, before } = destination {
                if kind == TrackKind::Video && !media.has_video && media.has_audio {
                    destination = TrackDestination::New {
                        kind: TrackKind::Audio,
                        before,
                    };
                } else if kind == TrackKind::Audio && media.has_video && !media.has_audio {
                    destination = TrackDestination::New {
                        kind: TrackKind::Video,
                        before,
                    };
                }
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
        self.prepare_current_proxy();
        if self.cursor_effect.end_us == 1 && self.duration() > 1 {
            self.cursor_effect.end_us = self.duration();
        }
        let proxy_path = self.preview_proxy_path();
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
            Span::at(&self.timeline, self.position).map(|span| {
                span.with_excluded_overlay(exclude).with_volume_override(
                    self.volume_preview.map(|(id, _)| id),
                    self.volume_preview.map(|(_, volume)| volume),
                )
            })
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
                span.with_video_decode_path(proxy_path.clone())
                    .with_canvas_format(self.canvas_format)
                    .with_background(&self.background)
                    .with_cursor_effect(&self.cursor_effect)
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
        self.open_project_path(ctx, path);
    }

    fn open_project_path(&mut self, ctx: &egui::Context, path: PathBuf) {
        match project::load_with_cursor(&path) {
            Ok((timeline, position, canvas_format, background, cursor_effect, markers)) => {
                for clip in timeline.all_clips().filter(|clip| clip.text.is_none()) {
                    self.library.insert(clip.path.clone(), false);
                }
                self.timeline = timeline;
                self.timeline_view = TimelineView::with_media(self.tools.clone(), ctx);
                self.cursor_marker_cache = CursorMarkerCache::default();
                self.saved_revision = self.timeline.revision();
                self.canvas_format = canvas_format;
                self.saved_canvas_format = canvas_format;
                self.saved_background = background.clone();
                self.background = background;
                self.saved_cursor_effect = cursor_effect.clone();
                self.cursor_effect = cursor_effect;
                self.saved_markers = markers.clone();
                self.markers = markers;
                self.position = position;
                self.selected_clip = None;
                self.text_inspector = TextInspector::default();
                self.subtitle_canvas.cancel();
                self.preview_viewport.reset();
                self.pending_additions.clear();
                self.player.playing = false;
                self.player.reset();
                self.project = Some(path);
                self.proxy_cache.set_project(self.project.as_deref());
                self.home = false;
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
        match project::save_with_cursor(
            &path,
            &self.timeline,
            self.position,
            self.canvas_format,
            &self.background,
            &self.cursor_effect,
            &self.markers,
        ) {
            Ok(()) => {
                self.project = Some(path);
                self.saved_revision = self.timeline.revision();
                self.saved_canvas_format = self.canvas_format;
                self.saved_background.clone_from(&self.background);
                self.saved_cursor_effect.clone_from(&self.cursor_effect);
                self.saved_markers.clone_from(&self.markers);
                self.show_notice("工程已保存", false);
            }
            Err(error) => self.show_notice(format!("{error:#}"), true),
        }
    }

    fn export_video(&mut self, ctx: &egui::Context) {
        if !self.commit_overlay_draft() {
            return;
        }
        if self.tools.is_none() {
            self.show_notice("FFmpeg 不可用，无法导出", true);
            return;
        }
        self.export_settings = Some(ExportSettings::new(self.library.directory()));
        ctx.request_repaint();
    }

    fn start_export(&mut self, ctx: &egui::Context, mut settings: ExportSettings) {
        let Some(tools) = self.tools.clone() else {
            self.show_notice("FFmpeg 不可用，无法导出", true);
            return;
        };
        if let Err(error) = settings.resolve_output(self.library.directory()) {
            self.show_notice(format!("{error:#}"), true);
            self.export_settings = Some(settings);
            return;
        }
        self.player.playing = false;
        self.player.reset();
        if let Err(error) = self.export.start(
            ctx,
            tools,
            self.timeline.tracks().to_vec(),
            settings,
            self.canvas_format,
            self.background.clone(),
            self.cursor_effect.clone(),
        ) {
            self.show_notice(format!("{error:#}"), true);
        } else {
            self.show_notice("正在导出时间线…", false);
        }
    }

    fn export_panel(&mut self, ctx: &egui::Context) {
        let Some(mut settings) = self.export_settings.take() else {
            return;
        };
        let mut open = true;
        let mut start = false;
        let mut cancel = false;
        let locale = Locale::from_context(ctx);
        let colors = ui_theme::palette(ctx);
        let preview_texture = self.player.texture().cloned();
        let preview_size = self.player.texture_composition_size();
        let output_directory = self.library.directory().to_owned();
        let content = ctx.content_rect();

        egui::Area::new(egui::Id::new("export-settings-backdrop"))
            .order(egui::Order::Middle)
            .fixed_pos(content.min)
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_min_size(content.size());
                ui.painter().rect_filled(
                    ui.max_rect(),
                    0.0,
                    egui::Color32::from_black_alpha(if ctx.theme() == egui::Theme::Dark {
                        150
                    } else {
                        82
                    }),
                );
                ui.allocate_response(content.size(), egui::Sense::click_and_drag());
            });

        egui::Window::new(locale.text(key::EXPORT_SETTINGS))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .title_bar(true)
            .default_pos(content.center() - egui::vec2(430.0, 260.0))
            .default_width(860.0)
            .frame(
                egui::Frame::new()
                    .fill(colors.surface)
                    .stroke(egui::Stroke::new(1.0, colors.border))
                    .corner_radius(14)
                    .inner_margin(egui::Margin::symmetric(16, 14)),
            )
            .show(ctx, |ui| {
                ui.set_min_size(egui::vec2(820.0, 500.0));
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);

                let section = |ui: &mut egui::Ui, title: &str| {
                    ui.add_space(2.0);
                    ui.label(RichText::new(title).size(12.0).strong().color(colors.text));
                    ui.separator();
                };
                let field_label = |ui: &mut egui::Ui, label: &str| {
                    ui.label(RichText::new(label).size(11.0).color(colors.muted));
                };

                ui.columns(2, |columns| {
                    let (left_column, right_column) = columns.split_at_mut(1);
                    let preview_ui = &mut left_column[0];
                    let settings_ui = &mut right_column[0];
                    preview_ui.set_min_width(350.0);
                    settings_ui.set_min_width(430.0);

                    section(preview_ui, locale.text(key::EXPORT_PREVIEW));
                    egui::Frame::new()
                        .fill(colors.field)
                        .stroke(egui::Stroke::new(1.0, colors.border))
                        .corner_radius(8)
                        .inner_margin(egui::Margin::same(8))
                        .show(preview_ui, |ui| {
                            let max_size = egui::vec2(340.0, 340.0);
                            let (source_width, source_height) = preview_size.unwrap_or((16, 9));
                            let scale = f32::min(
                                max_size.x / source_width.max(1) as f32,
                                max_size.y / source_height.max(1) as f32,
                            );
                            let image_size = egui::vec2(
                                source_width as f32 * scale,
                                source_height as f32 * scale,
                            );
                            ui.allocate_ui_with_layout(
                                max_size,
                                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                                |ui| {
                                    if let Some(texture) = &preview_texture {
                                        ui.add(
                                            egui::Image::new((texture.id(), image_size))
                                                .fit_to_exact_size(image_size)
                                                .corner_radius(6),
                                        );
                                    } else {
                                        ui.label(
                                            RichText::new(locale.text(key::PREVIEW))
                                                .size(12.0)
                                                .color(colors.muted),
                                        );
                                    }
                                },
                            );
                        });
                    preview_ui.add_space(8.0);
                    preview_ui.label(
                        RichText::new("当前时间线预览")
                            .size(11.0)
                            .color(colors.muted),
                    );

                    settings_ui.vertical(|ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("export-settings-scroll")
                            .max_height(405.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                section(ui, locale.text(key::EXPORT_OUTPUT));
                                egui::Frame::new()
                                    .fill(colors.field)
                                    .stroke(egui::Stroke::new(1.0, colors.border))
                                    .corner_radius(8)
                                    .inner_margin(egui::Margin::symmetric(10, 8))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(locale.text(key::EXPORT_TITLE))
                                                    .size(11.0)
                                                    .color(colors.muted),
                                            );
                                            let changed = ui
                                                .add_sized(
                                                    egui::vec2(ui.available_width(), 28.0),
                                                    egui::TextEdit::singleline(&mut settings.title),
                                                )
                                                .changed();
                                            if changed {
                                                settings.title = settings
                                                    .title
                                                    .chars()
                                                    .map(|character| {
                                                        if r#"<>:"/\|?*"#.contains(character) {
                                                            '_'
                                                        } else {
                                                            character
                                                        }
                                                    })
                                                    .collect();
                                                if settings.title.trim().is_empty() {
                                                    settings.title =
                                                        format!("{}-剪辑", PRODUCT_NAME);
                                                }
                                                settings.output_text = output_directory
                                                    .join(format!("{}.mp4", settings.title.trim()))
                                                    .to_string_lossy()
                                                    .into_owned();
                                            }
                                        });
                                        ui.add_sized(
                                            egui::vec2(ui.available_width(), 32.0),
                                            egui::TextEdit::singleline(&mut settings.output_text)
                                                .hint_text(locale.text(key::EXPORT_PATH_HINT)),
                                        );
                                        ui.add_space(4.0);
                                        ui.label(
                                            RichText::new(locale.text(key::EXPORT_PATH_HINT))
                                                .size(10.0)
                                                .color(colors.muted),
                                        );
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(locale.text(key::EXPORT_TIMELINE))
                                                    .size(11.0)
                                                    .color(colors.muted),
                                            );
                                            ui.add_enabled(
                                                false,
                                                egui::Button::new(
                                                    locale.text(key::EXPORT_MAIN_TIMELINE),
                                                ),
                                            );
                                        });
                                    });

                                section(ui, "视频");
                                egui::Frame::new()
                                    .fill(colors.field)
                                    .stroke(egui::Stroke::new(1.0, colors.border))
                                    .corner_radius(8)
                                    .inner_margin(egui::Margin::symmetric(10, 8))
                                    .show(ui, |ui| {
                                        egui::Grid::new("export-video-grid")
                                            .num_columns(2)
                                            .spacing(egui::vec2(14.0, 8.0))
                                            .show(ui, |ui| {
                                                field_label(ui, locale.text(key::EXPORT_FORMAT));
                                                ui.add_sized(
                                                    egui::vec2(190.0, 28.0),
                                                    egui::Button::new(locale.text(key::EXPORT_MP4)),
                                                );
                                                ui.end_row();

                                                field_label(ui, "FPS");
                                                ui.add_sized(
                                                    egui::vec2(190.0, 28.0),
                                                    egui::Button::new(format!(
                                                        "{} fps",
                                                        settings.fps
                                                    )),
                                                );
                                                ui.end_row();

                                                field_label(
                                                    ui,
                                                    locale.text(key::EXPORT_RESOLUTION),
                                                );
                                                egui::ComboBox::from_id_salt("export-resolution")
                                                    .width(190.0)
                                                    .selected_text(match settings.resolution {
                                                        ExportResolution::Canvas => locale
                                                            .text(key::EXPORT_RESOLUTION_CANVAS),
                                                        ExportResolution::Hd => {
                                                            locale.text(key::EXPORT_RESOLUTION_720)
                                                        }
                                                        ExportResolution::FullHd => {
                                                            locale.text(key::EXPORT_RESOLUTION_1080)
                                                        }
                                                        ExportResolution::UltraHd => {
                                                            locale.text(key::EXPORT_RESOLUTION_4K)
                                                        }
                                                    })
                                                    .show_ui(ui, |ui| {
                                                        ui.selectable_value(
                                                            &mut settings.resolution,
                                                            ExportResolution::Canvas,
                                                            locale.text(
                                                                key::EXPORT_RESOLUTION_CANVAS,
                                                            ),
                                                        );
                                                        ui.selectable_value(
                                                            &mut settings.resolution,
                                                            ExportResolution::Hd,
                                                            locale.text(key::EXPORT_RESOLUTION_720),
                                                        );
                                                        ui.selectable_value(
                                                            &mut settings.resolution,
                                                            ExportResolution::FullHd,
                                                            locale
                                                                .text(key::EXPORT_RESOLUTION_1080),
                                                        );
                                                        ui.selectable_value(
                                                            &mut settings.resolution,
                                                            ExportResolution::UltraHd,
                                                            locale.text(key::EXPORT_RESOLUTION_4K),
                                                        );
                                                    });
                                                ui.end_row();

                                                field_label(ui, locale.text(key::EXPORT_QUALITY));
                                                egui::ComboBox::from_id_salt("export-quality")
                                                    .width(190.0)
                                                    .selected_text(match settings.quality {
                                                        ExportQuality::High => {
                                                            locale.text(key::EXPORT_QUALITY_HIGH)
                                                        }
                                                        ExportQuality::Standard => locale
                                                            .text(key::EXPORT_QUALITY_STANDARD),
                                                        ExportQuality::Small => {
                                                            locale.text(key::EXPORT_QUALITY_SMALL)
                                                        }
                                                    })
                                                    .show_ui(ui, |ui| {
                                                        ui.selectable_value(
                                                            &mut settings.quality,
                                                            ExportQuality::High,
                                                            locale.text(key::EXPORT_QUALITY_HIGH),
                                                        );
                                                        ui.selectable_value(
                                                            &mut settings.quality,
                                                            ExportQuality::Standard,
                                                            locale
                                                                .text(key::EXPORT_QUALITY_STANDARD),
                                                        );
                                                        ui.selectable_value(
                                                            &mut settings.quality,
                                                            ExportQuality::Small,
                                                            locale.text(key::EXPORT_QUALITY_SMALL),
                                                        );
                                                    });
                                                ui.end_row();

                                                field_label(ui, locale.text(key::EXPORT_ENCODER));
                                                ui.add_enabled(
                                                    false,
                                                    egui::Button::new(
                                                        locale.text(key::EXPORT_AUTO),
                                                    ),
                                                );
                                                ui.end_row();
                                            });
                                    });

                                section(ui, "音频与背景");
                                egui::Frame::new()
                                    .fill(colors.field)
                                    .stroke(egui::Stroke::new(1.0, colors.border))
                                    .corner_radius(8)
                                    .inner_margin(egui::Margin::symmetric(10, 8))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.checkbox(
                                                &mut settings.audio,
                                                locale.text(key::EXPORT_AUDIO),
                                            );
                                            ui.add_enabled_ui(settings.audio, |ui| {
                                                ui.label(
                                                    RichText::new(
                                                        locale.text(key::EXPORT_AUDIO_BITRATE),
                                                    )
                                                    .size(11.0)
                                                    .color(colors.muted),
                                                );
                                                egui::ComboBox::from_id_salt(
                                                    "export-audio-bitrate",
                                                )
                                                .width(120.0)
                                                .selected_text(format!(
                                                    "{} kbps",
                                                    settings.audio_bitrate
                                                ))
                                                .show_ui(ui, |ui| {
                                                    for bitrate in [128, 192, 256] {
                                                        ui.selectable_value(
                                                            &mut settings.audio_bitrate,
                                                            bitrate,
                                                            format!("{bitrate} kbps"),
                                                        );
                                                    }
                                                });
                                            });
                                        });
                                        ui.checkbox(
                                            &mut settings.include_background,
                                            locale.text(key::EXPORT_BACKGROUND),
                                        );
                                        ui.checkbox(
                                            &mut settings.include_subtitles,
                                            locale.text(key::EXPORT_SUBTITLES),
                                        );
                                    });
                            });
                        ui.separator();
                        ui.horizontal(|ui| {
                            let duration = layout::time(self.duration());
                            let video_mbps = match settings.quality {
                                ExportQuality::High => 12.0,
                                ExportQuality::Standard => 8.0,
                                ExportQuality::Small => 5.0,
                            };
                            let audio_mbps = if settings.audio {
                                settings.audio_bitrate as f32 / 1000.0
                            } else {
                                0.0
                            };
                            let size = self.duration() as f32 / 1_000_000.0
                                * (video_mbps + audio_mbps)
                                / 8.0;
                            ui.label(
                                RichText::new(locale.format(
                                    key::EXPORT_ESTIMATE,
                                    &[("duration", &duration), ("size", &format!("{size:.1} MB"))],
                                ))
                                .size(10.0)
                                .color(colors.muted),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add_sized(
                                            egui::vec2(108.0, 32.0),
                                            egui::Button::new(locale.text(key::CANCEL))
                                                .fill(colors.control),
                                        )
                                        .clicked()
                                    {
                                        cancel = true;
                                    }
                                    if ui
                                        .add_enabled(
                                            !self.export.running(),
                                            egui::Button::new(locale.text(key::EXPORT_START))
                                                .fill(colors.accent_fill)
                                                .min_size(egui::vec2(108.0, 32.0)),
                                        )
                                        .clicked()
                                    {
                                        start = true;
                                    }
                                },
                            );
                        });
                    });
                });
            });
        if start {
            self.start_export(ctx, settings);
        } else if !cancel && open {
            self.export_settings = Some(settings);
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
            FeatureTab::Transitions => self.transition_tools(ui),
        }
    }

    fn transition_tools(&mut self, ui: &mut egui::Ui) {
        self.material_drag.hide(ui.ctx());
        if let Some(choice) = self.transition_panel.show(ui, self.selected_clip) {
            if let Some(id) = self.selected_clip {
                let _ = self.timeline.set_clip_transition_in(
                    id,
                    Some(TransitionSpec {
                        id: choice.id.to_owned(),
                        duration_us: 300_000,
                        name: choice.name.to_owned(),
                        role: choice.role.label().to_owned(),
                    }),
                );
                match (choice.role, choice.name) {
                    (super::transition_panel::TransitionRole::Entrance, _) => {
                        let _ = self.timeline.set_clip_fade_in(id, 300_000);
                    }
                    (super::transition_panel::TransitionRole::Exit, _) => {
                        let _ = self.timeline.set_clip_fade_out(id, 300_000);
                    }
                    (_, "黑场淡入") => {
                        let _ = self.timeline.set_clip_fade_in(id, 300_000);
                    }
                    (_, "黑场淡出") => {
                        let _ = self.timeline.set_clip_fade_out(id, 300_000);
                    }
                    (_, "黑场淡入淡出") => {
                        let _ = self.timeline.set_clip_fade_in(id, 300_000);
                        let _ = self.timeline.set_clip_fade_out(id, 300_000);
                    }
                    _ => {}
                }
                self.show_notice(format!("已选择转场：{}", choice.name), false);
            } else {
                self.show_notice("请先选择时间线片段", true);
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
        let selected_path = self.library.selected_path();
        let selected_scope = selected_path.map(|path| {
            if self.feature_tab == FeatureTab::Audio && self.library.is_audio(path) {
                MaterialScope::Audio
            } else {
                MaterialScope::All
            }
        });
        let insert_track = selected_path
            .zip(selected_scope)
            .and_then(|(path, scope)| self.active_media_track(path, scope));
        let add_enabled =
            self.tools.is_some() && selected_path.is_some() && self.pending_additions.is_empty();
        let mut position = if self.timeline.is_empty() {
            0
        } else {
            self.position
        };
        let cursor_tracks = cursor_tracks(&self.timeline, &mut self.cursor_marker_cache);
        self.timeline_view.set_cursor_tracks(cursor_tracks);
        self.timeline_view.set_user_markers(self.markers.clone());
        let input = self.timeline_view.draw_with_cursor_effect(
            ui,
            &mut self.timeline,
            &mut position,
            &mut self.selected_clip,
            &mut self.cursor_effect,
            add_enabled,
        );
        let recognize_audio_track = input.recognize_audio_track;
        let mut transition_notice = None;
        if let Some(action) = input.marker_action {
            match action {
                super::timeline_view::MarkerAction::Add(at) => {
                    self.markers.push(at.min(self.timeline.duration()));
                    self.markers.sort_unstable();
                    self.markers.dedup();
                }
                super::timeline_view::MarkerAction::Remove(at) => {
                    if let Some(index) = self.markers.iter().position(|marker| *marker == at) {
                        self.markers.remove(index);
                    }
                }
            }
        }
        if let Some(drop) = input.transition_drop {
            let metadata_clip = match drop.role {
                super::transition_panel::TransitionRole::Exit => drop.outgoing,
                _ => drop.incoming,
            };
            let _ = self.timeline.set_clip_transition_in(
                metadata_clip,
                Some(TransitionSpec {
                    id: drop.id.to_owned(),
                    duration_us: 300_000,
                    name: drop.name.to_owned(),
                    role: drop.role.label().to_owned(),
                }),
            );
            match (drop.role, drop.name) {
                (super::transition_panel::TransitionRole::Entrance, _) => {
                    let _ = self.timeline.set_clip_fade_in(drop.incoming, 300_000);
                }
                (super::transition_panel::TransitionRole::Exit, _) => {
                    let _ = self.timeline.set_clip_fade_out(drop.outgoing, 300_000);
                }
                (_, "黑场淡入") => {
                    let _ = self.timeline.set_clip_fade_in(drop.incoming, 300_000);
                }
                (_, "黑场淡出") => {
                    let _ = self.timeline.set_clip_fade_out(drop.outgoing, 300_000);
                }
                (_, "黑场淡入淡出") => {
                    let _ = self.timeline.set_clip_fade_in(drop.incoming, 300_000);
                    let _ = self.timeline.set_clip_fade_out(drop.outgoing, 300_000);
                }
                _ => {}
            }
            transition_notice = Some(format!("已应用转场：{}", drop.name));
        }
        let volume_changed = input.volume_changed;
        self.volume_preview = input.volume_preview;
        if let Some(drop) = input.material_drop {
            self.queue_material(ui.ctx(), drop.path, drop.destination, Some(drop.at));
        } else if input.add_material
            && let Some(path) = selected_path
            && let Some(scope) = selected_scope
        {
            let destination = insert_track.map(TrackDestination::Existing).or_else(|| {
                let info = self.library.info(path)?;
                let kind = if scope == MaterialScope::Audio || (!info.has_video && info.has_audio) {
                    TrackKind::Audio
                } else {
                    TrackKind::Video
                };
                Some(TrackDestination::New { kind, before: None })
            });
            if let Some(destination) = destination {
                self.queue_material(ui.ctx(), path.to_owned(), destination, None);
            }
        }
        if let Some(track) = recognize_audio_track {
            self.start_speech(track);
        }
        if let Some(error) = input.error {
            self.show_notice(error, true);
        } else if let Some(message) = input.clipboard_feedback {
            self.show_notice(message, false);
        }
        if let Some(message) = transition_notice {
            self.show_notice(message, false);
        }
        if input.playhead_seek {
            self.player.begin_seek();
        }
        if !self.timeline.is_empty() {
            self.position = position;
        }
        if input.playhead_seek {
            self.prepare_and_play_scrub_audio(self.position);
        }
        if input.playhead_seek_finished {
            self.player.end_seek();
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
            if !volume_changed {
                self.position = self.position.min(self.timeline.duration());
                self.player.playing = false;
                self.player.reset();
            }
        } else if self.position != old_position && !input.playhead_seek {
            self.player.reset();
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        if self.home {
            self.home_screen(ui);
            return;
        }
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
            layout::status(ui, fault, true, false);
        } else if !self.pending_additions.is_empty() {
            layout::status(ui, "正在读取素材…", false, true);
        } else if let Some(notice) = &self.notice {
            layout::status(ui, &notice.text, notice.warning, notice.loading);
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
            Pane::Inspector => {
                egui::ScrollArea::vertical()
                    .id_salt("inspector-scroll")
                    .show(ui, |ui| self.inspector(ui));
            }
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
            || self.player.audio_error().is_some()
        {
            "预览不可用"
        } else {
            self.player.blank_message().unwrap_or("正在读取预览…")
        };
        let preview_placeholder = if self.player.texture().is_none() {
            self.preview_placeholder()
        } else {
            None
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
        let title = self.preview_title();
        let preview_texture = if self.player.is_scrubbing() {
            self.player.texture().cloned().or_else(|| {
                preview_placeholder
                    .as_ref()
                    .map(|(texture, _)| texture.clone())
            })
        } else {
            self.player
                .texture()
                .cloned()
                .or_else(|| preview_placeholder.as_ref().map(|(texture, _)| texture.clone()))
        };
        let preview_size = self
            .player
            .texture_composition_size();
        let preview_size = if self.player.is_scrubbing() {
            preview_size.or_else(|| preview_placeholder.as_ref().map(|(_, size)| *size))
        } else {
            preview_size.or_else(|| preview_placeholder.as_ref().map(|(_, size)| *size))
        };
        let mut canvas_edit = None;
        let subtitle_canvas = &mut self.subtitle_canvas;
        let input = layout::monitor(
            ui,
            &mut self.preview_viewport,
            preview_texture.as_ref(),
            preview_size,
            &title,
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
        if let Some(jump) = input.jump {
            self.subtitle_canvas.cancel();
            self.player.playing = false;
            self.position = jump.min(duration);
            self.player.reset();
        }
        if let Some(step) = input.step {
            self.subtitle_canvas.cancel();
            self.player.playing = false;
            let delta = step.saturating_mul(33_333);
            self.position = self.position.saturating_add_signed(delta).min(duration);
            self.player.reset();
        }
        if let Some(seek) = input.seek {
            self.subtitle_canvas.cancel();
            self.player.begin_seek();
            self.position = seek;
            self.prepare_and_play_scrub_audio(self.position);
        }
        if input.seek_finished {
            self.player.end_seek();
        }
        if input.toggle {
            self.subtitle_canvas.cancel();
            self.player.toggle(&mut self.position, duration);
        }
    }

    fn preview_placeholder(&mut self) -> Option<(egui::TextureHandle, (u32, u32))> {
        let (path, timestamp_us, _size) = self.preview_source_at()?;
        let path = if self.player.is_scrubbing()
            && let Some(info) = self.library.info(&path)
        {
            self.proxy_cache
                .ensure(&path, info)
                .unwrap_or(path)
        } else {
            path
        };
        if let Some(texture) = self
            .preview_thumbnails
            .nearest_frame(&path, timestamp_us, 120_000)
        {
            return Some((texture.clone(), Self::texture_size(&texture)));
        }
        let timestamp_us = timestamp_us / PREVIEW_SCRUB_STEP_US * PREVIEW_SCRUB_STEP_US;
        if let Some(texture) = self.preview_thumbnails.frame(&path, timestamp_us).cloned() {
            return Some((texture.clone(), Self::texture_size(&texture)));
        }
        if self.player.is_scrubbing() {
            return None;
        }
        if let Some(texture) = self
            .timeline_view
            .video_thumbnail_nearest(&path, timestamp_us, 2_000_000)
        {
            return Some((texture.clone(), Self::texture_size(&texture)));
        }
        if !self.timeline.is_empty() {
            return None;
        }
        self.thumbnails
            .texture(&path)
            .cloned()
            .map(|texture| (texture.clone(), Self::texture_size(&texture)))
    }

    fn audio_span_at<'a>(
        timeline: &'a Timeline,
        library: &Library,
        position: u64,
    ) -> Option<Span<'a>> {
        if timeline.is_empty() {
            let path = library.selected_path()?;
            let info = library.info(path)?;
            Some(Span::for_asset(path, info))
        } else {
            Span::at(timeline, position)
        }
    }

    fn prepare_and_play_scrub_audio(&mut self, position: u64) {
        let Some(tools) = self.tools.clone() else {
            return;
        };
        if let Some(span) = Self::audio_span_at(&self.timeline, &self.library, position) {
            self.player.prepare_scrub_audio(
                Some(&tools),
                span,
                &mut self.library,
                position,
            );
        }
        if let Some(span) = Self::audio_span_at(&self.timeline, &self.library, position) {
            self.player.play_scrub_audio(
                Some(&tools),
                span,
                &mut self.library,
                position,
            );
        }
    }

    fn preview_title(&self) -> String {
        if let Some(track) = self
            .timeline
            .tracks()
            .iter()
            .find(|track| track.kind == TrackKind::Video && track.enabled)
            && let Some(clip) = track
                .clips
                .iter()
                .find(|clip| clip.start <= self.position && self.position < clip.end())
        {
            return clip
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
        }
        self.library
            .selected_path()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "预览".to_owned())
    }

    fn preview_source_at(&self) -> Option<(PathBuf, u64, (u32, u32))> {
        if !self.timeline.is_empty() {
            for track in self.timeline.tracks() {
                if track.kind != TrackKind::Video || !track.enabled {
                    continue;
                }
                if let Some(clip) = track
                    .clips
                    .iter()
                    .find(|clip| clip.start <= self.position && self.position < clip.end())
                {
                    let elapsed = self.position.saturating_sub(clip.start);
                    let source_time = clip.source_start.saturating_add(
                        elapsed
                            .saturating_mul(u64::from(clip.speed_milli.max(1)))
                            .checked_div(1000)
                            .unwrap_or(u64::MAX),
                    );
                    let size = self
                        .library
                        .info(&clip.path)
                        .map(|info| (info.width, info.height))
                        .filter(|(width, height)| *width > 0 && *height > 0)
                        .unwrap_or((16, 9));
                    return Some((clip.path.clone(), source_time, size));
                }
            }
            return None;
        }
        let path = self.library.selected_path()?.to_owned();
        let size = self
            .library
            .info(&path)
            .map(|info| (info.width, info.height))
            .filter(|(width, height)| *width > 0 && *height > 0)
            .unwrap_or((16, 9));
        Some((path, 0, size))
    }

    fn preview_proxy_path(&mut self) -> Option<PathBuf> {
        if self.player.quality() == PreviewQuality::Full
            || (!self.player.playing && !self.player.is_scrubbing())
        {
            return None;
        }
        let (path, _, _) = self.preview_source_at()?;
        let info = self.library.info(&path)?;
        self.proxy_cache.ensure(&path, info)
    }

    fn prepare_current_proxy(&mut self) {
        let Some((path, _, _)) = self.preview_source_at() else {
            return;
        };
        let Some(info) = self.library.info(&path) else {
            return;
        };
        let _ = self.proxy_cache.ensure(&path, info);
    }

    fn prepare_project_proxies(&mut self) {
        let paths = self
            .timeline
            .tracks()
            .iter()
            .filter(|track| track.kind == TrackKind::Video && track.enabled)
            .flat_map(|track| track.clips.iter().map(|clip| clip.path.clone()))
            .collect::<HashSet<_>>();
        for path in paths {
            if let Some(info) = self.library.info(&path) {
                let _ = self.proxy_cache.ensure(&path, info);
            }
        }
    }

    fn prepare_current_audio_cache(&mut self) {
        let Some(span) = Self::audio_span_at(&self.timeline, &self.library, self.position) else {
            return;
        };
        self.player.prepare_scrub_audio(
            self.tools.as_ref(),
            span,
            &mut self.library,
            self.position,
        );
    }

    fn texture_size(texture: &egui::TextureHandle) -> (u32, u32) {
        let [width, height] = texture.size();
        (width as u32, height as u32)
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        if self.timeline_view.cursor_track_selected() && self.cursor_telemetry_available() {
            self.cursor_effect_inspector(ui);
            ui.separator();
        }
        if self.overlay_inspector(ui) {
            return;
        }
        if let Some(id) = self.selected_clip
            && let Some(clip) = self.timeline.clip(id).cloned()
        {
            inspector_details::media(ui, &clip.path, self.library.info(&clip.path), Some(&clip));
            ui.separator();
            self.clip_inspector_controls(ui, id);
        } else if !self.timeline.is_empty() {
            inspector_details::sequence(ui, &self.timeline);
        } else if let Some(path) = self.library.selected_path() {
            inspector_details::media(ui, path, self.library.info(path), None);
        }
    }

    fn clip_inspector_controls(&mut self, ui: &mut egui::Ui, id: u64) {
        let Some(clip) = self.timeline.clip(id).cloned() else {
            return;
        };
        let kind = self.timeline.track_for_clip(id).map(|track| track.kind);
        let colors = ui_theme::palette(ui.ctx());
        ui.label(RichText::new("片段属性").size(12.0).strong().color(colors.text));
        ui.add_space(4.0);

        ui.collapsing("速度与方向", |ui| {
            let mut speed = clip.speed_milli;
            egui::ComboBox::from_id_salt(("clip-speed", id))
                .width(ui.available_width())
                .selected_text(format!("{}x", f32::from(speed) / 1000.0))
                .show_ui(ui, |ui| {
                    for value in [500_u16, 1000, 1500, 2000] {
                        ui.selectable_value(
                            &mut speed,
                            value,
                            format!("{}x", f32::from(value) / 1000.0),
                        );
                    }
                });
            if speed != clip.speed_milli {
                if let Err(error) = self.timeline.set_clip_speed(id, speed) {
                    self.show_notice(error.to_string(), true);
                }
            }

            let mut reverse = clip.reverse;
            if ui.checkbox(&mut reverse, "倒放").changed()
                && let Err(error) = self.timeline.set_clip_reverse(id, reverse)
            {
                self.show_notice(error.to_string(), true);
            }
        });

        if matches!(kind, Some(TrackKind::Audio | TrackKind::Video)) {
            ui.collapsing("音频", |ui| {
                let mut volume = clip.volume_db_tenths;
                if ui
                    .add(egui::Slider::new(&mut volume, -600..=120).text("音量").suffix(" dB"))
                    .changed()
                    && let Err(error) = self.timeline.set_clip_volume(id, volume)
                {
                    self.show_notice(error.to_string(), true);
                }
                if ui.button("重置音量").clicked()
                    && let Err(error) = self.timeline.set_clip_volume(id, 0)
                {
                    self.show_notice(error.to_string(), true);
                }
            });
        }

        if matches!(kind, Some(TrackKind::Audio | TrackKind::Video)) {
            ui.collapsing("淡化", |ui| {
                let max_fade = clip.duration().min(5_000_000);
                let mut fade_in = clip.fade_in_us;
                if ui
                    .add(egui::Slider::new(&mut fade_in, 0..=max_fade).text("淡入"))
                    .changed()
                    && let Err(error) = self.timeline.set_clip_fade_in(id, fade_in)
                {
                    self.show_notice(error.to_string(), true);
                }
                let mut fade_out = clip.fade_out_us;
                if ui
                    .add(egui::Slider::new(&mut fade_out, 0..=max_fade).text("淡出"))
                    .changed()
                    && let Err(error) = self.timeline.set_clip_fade_out(id, fade_out)
                {
                    self.show_notice(error.to_string(), true);
                }
            });
        }

        if let Some(transition) = &clip.transition_in {
            ui.collapsing("转场", |ui| {
                ui.label(format!(
                    "{} · {:.1}s",
                    transition.id,
                    transition.duration_us as f32 / 1_000_000.0
                ));
                if ui.button("清除转场").clicked()
                    && let Err(error) = self.timeline.set_clip_transition_in(id, None)
                {
                    self.show_notice(error.to_string(), true);
                }
            });
        }

        let group = self
            .timeline_view
            .selected_group(self.selected_clip)
            .to_vec();
        if group.len() > 1 {
            ui.collapsing("片段关系", |ui| {
                ui.horizontal(|ui| {
                    if ui.button("链接选中片段").clicked()
                        && let Err(error) = self.timeline.link_clips(&group)
                    {
                        self.show_notice(error.to_string(), true);
                    }
                    if ui.button("取消链接").clicked()
                        && let Err(error) = self.timeline.unlink_clips(&group)
                    {
                        self.show_notice(error.to_string(), true);
                    }
                });
            });
        }
    }

    fn cursor_telemetry_available(&self) -> bool {
        self.timeline
            .tracks()
            .iter()
            .filter(|track| track.kind == TrackKind::Video)
            .flat_map(|track| &track.clips)
            .any(|clip| {
                PathBuf::from(format!("{}.cursor.json", clip.path.display())).is_file()
            })
    }

    fn cursor_effect_inspector(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("鼠标轨道")
            .default_open(true)
            .show(ui, |ui| {
                let duration = self.duration().max(1);
                ui.label("拖动时间线中的鼠标效果块可移动，拖两端可调整范围。");
                ui.add(
                    egui::Slider::new(&mut self.cursor_effect.start_us, 0..=duration)
                        .text("开始"),
                );
                self.cursor_effect.start_us = self.cursor_effect.start_us.min(duration - 1);
                ui.add(
                    egui::Slider::new(
                        &mut self.cursor_effect.end_us,
                        (self.cursor_effect.start_us + 1)..=duration,
                    )
                    .text("结束"),
                );
                ui.checkbox(&mut self.cursor_effect.enabled, "显示鼠标");
                self.cursor_zoom_inspector(ui);
                ui.label("光标样式");
                let palette = ui_theme::palette(ui.ctx());
                let preview_key = egui::Id::new("cursor-style-theme-previews");
                let preview_textures = ui
                    .data(|data| data.get_temp::<Vec<(CursorStyle, egui::TextureHandle)>>(preview_key))
                    .unwrap_or_else(|| {
                        let textures = CursorStyle::ALL
                            .into_iter()
                            .filter_map(|style| {
                                let sprite = style.sprite()?;
                                let image = egui::ColorImage::from_rgba_unmultiplied(
                                    [sprite.width as usize, sprite.height as usize],
                                    &sprite.rgba,
                                );
                                let filter = if style == CursorStyle::Pixel {
                                    egui::TextureOptions::NEAREST
                                } else {
                                    egui::TextureOptions::LINEAR
                                };
                                Some((
                                    style,
                                    ui.ctx().load_texture(
                                        format!("cursor-style-{}", style.label()),
                                        image,
                                        filter,
                                    ),
                                ))
                            })
                            .collect::<Vec<_>>();
                        ui.data_mut(|data| data.insert_temp(preview_key, textures.clone()));
                        textures
                    });
                ui.horizontal_wrapped(|ui| {
                    for style in CursorStyle::ALL {
                        let selected = self.cursor_effect.cursor_style == style;
                        let (rect, response) =
                            ui.allocate_exact_size(vec2(62.0, 48.0), egui::Sense::click());
                        let tile_fill = if selected {
                            palette.control.gamma_multiply(0.9)
                        } else if response.hovered() || response.has_focus() {
                            palette.hover
                        } else {
                            palette.surface
                        };
                        ui.painter().rect_filled(rect, 5.0, tile_fill);
                        ui.painter().rect_stroke(
                            rect,
                            5.0,
                            egui::Stroke::new(
                                if selected { 1.5_f32 } else { 1.0_f32 },
                                if selected { palette.accent } else { palette.border },
                            ),
                            egui::StrokeKind::Inside,
                        );

                        let preview_rect = egui::Rect::from_center_size(
                            rect.center_top() + vec2(0.0, 20.0),
                            vec2(34.0, 34.0),
                        );
                        if let Some((_, texture)) = preview_textures
                            .iter()
                            .find(|(candidate, _)| *candidate == style)
                        {
                            ui.painter().image(
                                texture.id(),
                                preview_rect,
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE,
                            );
                        } else {
                            Icon::Cursor.paint_scaled(
                                ui.painter(),
                                preview_rect.center(),
                                palette.text,
                                1.2,
                            );
                        }
                        ui.painter().text(
                            rect.center_bottom() - vec2(0.0, 6.0),
                            egui::Align2::CENTER_CENTER,
                            style.label(),
                            egui::FontId::proportional(9.5),
                            if selected { palette.accent } else { palette.text },
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::WidgetType::Button,
                                response.enabled(),
                                selected,
                                style.label(),
                            )
                        });
                        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            self.cursor_effect.cursor_style = style;
                        }
                    }
                });
                ui.add(
                    egui::Slider::new(&mut self.cursor_effect.cursor_scale, 0.5..=4.0)
                        .text("鼠标大小"),
                );
                ui.horizontal(|ui| {
                    ui.label("点击反馈");
                    for (effect, label) in [
                        (CursorClickEffect::Ring, "波纹"),
                        (CursorClickEffect::Dot, "圆点"),
                        (CursorClickEffect::None, "无"),
                    ] {
                        if ui
                            .selectable_label(self.cursor_effect.click_effect == effect, label)
                            .clicked()
                        {
                            self.cursor_effect.click_effect = effect;
                        }
                    }
                });
            });
    }

    fn cursor_zoom_inspector(&mut self, ui: &mut egui::Ui) {
        let effect = &mut self.cursor_effect;
        let reset_transition = ui
            .horizontal_wrapped(|ui| {
                ui.strong("点击放大");
                ui.selectable_label(effect.zoom_transition_us.is_none(), "自动过渡")
                    .on_hover_text("OpenScreen 的默认过渡，放大 1.52 秒，回退 1.02 秒")
                    .clicked()
            })
            .inner;
        let mut transition_us = effect.zoom_transition_us();
        let original_transition_us = transition_us;
        ui.checkbox(&mut effect.zoom_follow_cursor, "镜头跟随鼠标")
            .on_hover_text("放大时随录制的鼠标移动。关闭后按点击位置平移");

        egui::Grid::new("cursor-zoom-settings")
            .num_columns(2)
            .min_col_width(0.0)
            .min_row_height(20.0)
            .spacing(vec2(6.0, 3.0))
            .show(ui, |ui| {
                ui.spacing_mut().interact_size.y = 20.0;
                let label = ui.label("倍率");
                ui.add(
                    egui::DragValue::new(&mut effect.zoom_scale)
                        .range(ZOOM_SCALE_RANGE)
                        .speed(0.01)
                        .fixed_decimals(2)
                        .suffix("×"),
                )
                .labelled_by(label.id)
                .on_hover_text("设为 1 倍可关闭点击放大");
                ui.end_row();

                for (label, help, duration_us, range_us, enabled) in [
                    (
                        "过渡",
                        "自动时显示回退时长。手动修改后，放大和回退共用这个时长",
                        &mut transition_us,
                        ZOOM_TRANSITION_RANGE,
                        true,
                    ),
                    (
                        "停留",
                        "放大后保持的时长",
                        &mut effect.zoom_hold_us,
                        ZOOM_HOLD_RANGE,
                        true,
                    ),
                    (
                        "平移",
                        "关闭镜头跟随时，在连续点击位置之间平移的时长",
                        &mut effect.zoom_pan_us,
                        ZOOM_PAN_RANGE,
                        !effect.zoom_follow_cursor,
                    ),
                ] {
                    let label = ui.label(label).on_hover_text(help);
                    const US_PER_SECOND: f64 = 1_000_000.0;
                    let mut seconds = *duration_us as f64 / US_PER_SECOND;
                    let range_seconds = (*range_us.start() as f64 / US_PER_SECOND)
                        ..=(*range_us.end() as f64 / US_PER_SECOND);
                    if ui
                        .add_enabled(
                            enabled,
                            egui::DragValue::new(&mut seconds)
                                .range(range_seconds)
                                .speed(0.01)
                                .fixed_decimals(2)
                                .suffix(" 秒"),
                        )
                        .labelled_by(label.id)
                        .on_hover_text(help)
                        .changed()
                    {
                        *duration_us = (seconds * US_PER_SECOND).round() as u64;
                    }
                    ui.end_row();
                }
                let label = ui.label("跟随平滑");
                ui.add_enabled(
                    effect.zoom_follow_cursor,
                    egui::DragValue::new(&mut effect.smoothing)
                        .range(0.0..=1.0)
                        .speed(0.01)
                        .fixed_decimals(2),
                )
                .labelled_by(label.id)
                .on_hover_text("调低时镜头更快追上鼠标，调高时镜头移动更平缓");
                ui.end_row();
            });

        if reset_transition {
            effect.zoom_transition_us = None;
        } else if transition_us != original_transition_us {
            effect.zoom_transition_us = Some(transition_us);
        }
    }

    fn home_screen(&mut self, ui: &mut egui::Ui) {
        let colors = ui_theme::palette(ui.ctx());
        let projects = self.home_projects();
        let area = ui.available_rect_before_wrap();
        let content_width = (area.width() - 32.0).clamp(0.0, 1120.0);
        let content_rect = egui::Rect::from_center_size(area.center(), vec2(content_width, area.height()));
        let background = egui::Color32::from_rgb(10, 13, 18);
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 0.0, background);
        painter.circle_filled(
            egui::pos2(area.right() - 80.0, area.top() + 80.0),
            190.0,
            colors.accent.gamma_multiply(0.035),
        );
        painter.circle_filled(
            egui::pos2(area.left() + 40.0, area.bottom() - 28.0),
            120.0,
            egui::Color32::from_rgb(34, 48, 72).gamma_multiply(0.22),
        );
        let mut content = ui.new_child(egui::UiBuilder::new().max_rect(content_rect));

        content.add_space(18.0);
        content.horizontal(|ui| {
            ui.add(
                egui::Image::new(&self.logo)
                    .fit_to_exact_size(vec2(34.0, 34.0))
                    .corner_radius(10.0),
            );
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(PRODUCT_NAME).size(15.0).strong().color(colors.text));
                ui.label(RichText::new("编辑工作台").size(11.0).color(colors.muted));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new("本地工程").size(11.0).color(colors.muted));
            });
        });

        content.add_space(40.0);
        let hero_width = content.available_width();
        let (hero, _) = content.allocate_exact_size(vec2(hero_width, 270.0), egui::Sense::hover());
        let mut hero_ui = content.new_child(egui::UiBuilder::new().max_rect(hero));
        let gap = 26.0;
        let left_width = ((hero_width - gap) * 0.62).max(0.0);
        let right_width = (hero_width - left_width - gap).max(0.0);
        let left_rect = egui::Rect::from_min_size(hero.min, vec2(left_width, hero.height()));
        let right_rect = egui::Rect::from_min_size(
            egui::pos2(left_rect.right() + gap, hero.top()),
            vec2(right_width, hero.height()),
        );
        let mut left = hero_ui.new_child(egui::UiBuilder::new().max_rect(left_rect));
        egui::Frame::new()
            .fill(colors.surface.gamma_multiply(0.48))
            .corner_radius(16.0)
            .stroke(egui::Stroke::new(1.0, colors.border))
            .inner_margin(20.0)
            .show(&mut left, |ui| {
                ui.label(
                    RichText::new("从一条空时间线开始")
                        .size(12.0)
                        .color(colors.accent),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new("把下一段剪辑\n放在这里")
                        .size(32.0)
                        .strong()
                        .color(colors.text),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new("录制、整理素材，然后把想法剪成一条清晰的时间线。")
                        .size(13.0)
                        .color(colors.muted),
                );
                ui.add_space(14.0);
                let button_gap = ui.spacing().item_spacing.x;
                let button_width = ((ui.available_width() - button_gap) * 0.5).max(0.0);
                let button_size = vec2(button_width, 54.0);
                ui.horizontal(|ui| {
                    if Self::gradient_action_button(ui, button_size, "创建项目", "从空白时间线开始") {
                        self.new_project(ui.ctx());
                    }
                    if Self::plain_action_button(
                        ui,
                        button_size,
                        "打开项目",
                        "选择已有工程",
                        colors.control,
                        colors.border,
                    ) && let Some(path) = rfd::FileDialog::new()
                        .set_title("打开剪辑工程")
                        .add_filter(format!("{} 工程", PRODUCT_NAME), &["takeproj"])
                        .pick_file()
                    {
                        self.open_project_path(ui.ctx(), path);
                    }
                });
            });
        let mut right = hero_ui.new_child(egui::UiBuilder::new().max_rect(right_rect));
        egui::Frame::new()
                .fill(colors.surface.gamma_multiply(0.86))
                .corner_radius(16.0)
                .stroke(egui::Stroke::new(1.0, colors.border))
                .inner_margin(20.0)
                .show(&mut right, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("工作区脉冲").size(13.0).strong().color(colors.text));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.colored_label(colors.accent, "●");
                            });
                        });
                        ui.add_space(18.0);
                        ui.label(
                            RichText::new(format!("{:02}", projects.len()))
                                .size(34.0)
                                .strong()
                                .color(colors.text),
                        );
                        ui.label(RichText::new("最近保存的工程").size(11.0).color(colors.muted));
                        ui.add_space(18.0);
                        ui.separator();
                        ui.add_space(10.0);
                        ui.label(RichText::new("工程目录").size(11.0).color(colors.muted));
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(self.library.directory().to_string_lossy())
                                .size(11.0)
                                .color(colors.text),
                        );
                });

        content.add_space(32.0);
        content.horizontal(|ui| {
            ui.label(RichText::new("最近打开").size(17.0).strong().color(colors.text));
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!("{} 个工程", projects.len()))
                    .size(11.0)
                    .color(colors.muted),
            );
        });
        content.add_space(14.0);

        if projects.is_empty() {
            egui::Frame::new()
                .fill(colors.surface.gamma_multiply(0.82))
                .corner_radius(14.0)
                .stroke(egui::Stroke::new(1.0, colors.border))
                .inner_margin(24.0)
                .show(&mut content, |ui| {
                    ui.allocate_ui_with_layout(
                        vec2(ui.available_width(), 120.0),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.label(RichText::new("＋").size(24.0).color(colors.accent));
                            ui.add_space(12.0);
                            ui.vertical(|ui| {
                                ui.label(RichText::new("还没有最近工程").size(13.0).color(colors.text));
                                ui.label(
                                    RichText::new("新建一个空白工程，开始建立你的第一条时间线。")
                                        .size(11.0)
                                        .color(colors.muted),
                                );
                            });
                        });
                });
        } else {
            egui::ScrollArea::horizontal()
                .id_salt("home-recent-projects")
                .show(&mut content, |ui| {
                    ui.horizontal(|ui| {
                        for (path, modified) in &projects {
                            let thumbnail = self
                                .project_thumbnail_source(path)
                                .and_then(|source| self.thumbnails.texture(&source).cloned());
                            let mut open_card = false;
                            let card = ui.allocate_ui([238.0, 164.0].into(), |ui| {
                                egui::Frame::new()
                                    .fill(colors.surface)
                                    .corner_radius(14.0)
                                    .stroke(egui::Stroke::new(1.0, colors.border))
                                    .inner_margin(12.0)
                                    .show(ui, |ui| {
                                        if let Some(thumbnail) = &thumbnail {
                                            ui.add(
                                                egui::Image::new(thumbnail)
                                                    .fit_to_exact_size(vec2(212.0, 92.0))
                                                    .corner_radius(9.0),
                                            );
                                        } else {
                                            let placeholder = ui.allocate_exact_size(
                                                vec2(212.0, 92.0),
                                                egui::Sense::hover(),
                                            );
                                            ui.painter().rect_filled(
                                                placeholder.0,
                                                9.0,
                                                colors.control,
                                            );
                                            ui.painter().text(
                                                placeholder.0.center(),
                                                egui::Align2::CENTER_CENTER,
                                                "暂无预览",
                                                egui::FontId::proportional(11.0),
                                                colors.muted,
                                            );
                                        }
                                        ui.add_space(8.0);
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(filename(path))
                                                    .size(13.0)
                                                    .strong()
                                                    .color(colors.text),
                                            );
                                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                                ui.label(
                                                    RichText::new("→").size(15.0).color(colors.accent),
                                                );
                                            });
                                        });
                                        ui.add_space(3.0);
                                        ui.label(
                                            RichText::new(format_modified(*modified))
                                                .size(11.0)
                                                .color(colors.muted),
                                        );
                                    });
                            });
                            if card.response.clicked() {
                                open_card = true;
                            }
                            if open_card {
                                self.open_project_path(ui.ctx(), path.clone());
                            }
                            ui.add_space(10.0);
                        }
                    });
                });
        }
    }
    fn gradient_action_button(
        ui: &mut egui::Ui,
        size: egui::Vec2,
        label: &str,
        hint: &str,
    ) -> bool {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        let left = egui::Color32::from_rgb(47, 107, 223);
        let right = egui::Color32::from_rgb(117, 69, 205);
        let mut mesh = egui::Mesh::default();
        let first = mesh.vertices.len() as u32;
        mesh.colored_vertex(rect.left_top(), left);
        mesh.colored_vertex(rect.right_top(), right);
        mesh.colored_vertex(rect.right_bottom(), right.gamma_multiply(0.82));
        mesh.colored_vertex(rect.left_bottom(), left.gamma_multiply(0.82));
        mesh.add_triangle(first, first + 1, first + 2);
        mesh.add_triangle(first, first + 2, first + 3);
        ui.painter().add(egui::Shape::mesh(mesh));
        ui.painter().rect_stroke(
            rect,
            12.0,
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(48)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            egui::pos2(rect.left() + 16.0, rect.top() + 10.0),
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::proportional(14.0),
            egui::Color32::WHITE,
        );
        ui.painter().text(
            egui::pos2(rect.left() + 16.0, rect.bottom() - 10.0),
            egui::Align2::LEFT_BOTTOM,
            hint,
            egui::FontId::proportional(10.0),
            egui::Color32::from_white_alpha(190),
        );
        ui.painter().text(
            egui::pos2(rect.right() - 16.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            "→",
            egui::FontId::proportional(19.0),
            egui::Color32::WHITE,
        );
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    }

    fn plain_action_button(
        ui: &mut egui::Ui,
        size: egui::Vec2,
        label: &str,
        hint: &str,
        fill: egui::Color32,
        border: egui::Color32,
    ) -> bool {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        ui.painter().rect_filled(rect, 12.0, fill);
        ui.painter().rect_stroke(
            rect,
            12.0,
            egui::Stroke::new(1.0, border),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            egui::pos2(rect.left() + 16.0, rect.top() + 10.0),
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::proportional(14.0),
            ui.visuals().text_color(),
        );
        ui.painter().text(
            egui::pos2(rect.left() + 16.0, rect.bottom() - 10.0),
            egui::Align2::LEFT_BOTTOM,
            hint,
            egui::FontId::proportional(10.0),
            ui.visuals().weak_text_color(),
        );
        ui.painter().text(
            egui::pos2(rect.right() - 16.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            "→",
            egui::FontId::proportional(19.0),
            ui.visuals().text_color(),
        );
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    }

    fn home_projects(&self) -> Vec<(PathBuf, SystemTime)> {
        let mut projects = fs::read_dir(self.library.directory())
            .ok()
            .into_iter()
            .flat_map(|entries| entries.flatten())
            .filter_map(|entry| {
                let path = entry.path();
                let is_project = path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("takeproj"));
                if !is_project {
                    return None;
                }
                let modified = entry.metadata().ok()?.modified().ok()?;
                Some((path, modified))
            })
            .collect::<Vec<_>>();
        projects.sort_unstable_by_key(|(_, modified)| std::cmp::Reverse(*modified));
        projects
    }

    fn project_thumbnail_source(&self, path: &Path) -> Option<PathBuf> {
        let (timeline, _, _, _, _, _) = project::load_with_cursor(path).ok()?;
        timeline
            .tracks()
            .iter()
            .filter(|track| track.kind == TrackKind::Video)
            .flat_map(|track| &track.clips)
            .find(|clip| clip.text.is_none())
            .map(|clip| clip.path.clone())
    }

    fn start_speech(&mut self, track_id: TrackId) {
        if self.speech_job.is_some() {
            self.show_notice("语音识别正在进行中", false);
            return;
        }
        let Some(tools) = self.tools.clone() else {
            self.show_notice("FFmpeg 不可用，无法进行语音识别", true);
            return;
        };
        let Some(track) = self.timeline.track(track_id).cloned() else {
            self.show_notice("音频轨道已不存在", true);
            return;
        };
        match SpeechJob::start(tools, track) {
            Ok(job) => {
                self.speech_job = Some(job);
                self.show_loading_notice("正在使用本地模型识别语音…");
            }
            Err(error) => self.show_notice(format!("{error:#}"), true),
        }
    }

    fn poll_speech(&mut self, ctx: &egui::Context) {
        let Some(job) = &mut self.speech_job else {
            return;
        };
        let Some(result) = job.poll() else {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
            return;
        };
        self.speech_job = None;
        let (track_id, result) = result;
        match result {
            Ok(cues) => self.insert_speech_cues(track_id, cues),
            Err(error) => self.show_notice(format!("{error:#}"), true),
        }
    }

    fn insert_speech_cues(&mut self, source_track: TrackId, cues: Vec<SpeechCue>) {
        let previous_caption_tracks = self
            .timeline
            .tracks()
            .iter()
            .filter(|track| track.kind == TrackKind::Caption)
            .map(|track| track.id)
            .collect::<Vec<_>>();
        let segments = cues
            .into_iter()
            .map(|cue| {
                (
                    cue.start_us,
                    cue.duration_us,
                    OverlayText {
                        content: cue.text,
                        style: TextStyle::default(),
                    },
                )
            })
            .collect::<Vec<_>>();
        if segments.is_empty() {
            self.show_notice("没有识别到语音内容", true);
            return;
        }
        match self
            .timeline
            .insert_text_batch(TrackKind::Caption, segments)
        {
            Ok(added) => {
                for track in previous_caption_tracks {
                    let _ = self.timeline.remove_track(track);
                }
                if let Some(&(track, id)) = added.first() {
                    if self.timeline.track(source_track).is_some() {
                        let _ = self.timeline.move_track_before(track, Some(source_track));
                    }
                    self.select_overlay(track, id, self.position);
                }
                if self.project.is_some() {
                    self.save_project(false);
                } else {
                    self.show_notice(format!("已生成 {} 条字幕", added.len()), false);
                }
            }
            Err(error) => self.show_notice(error.to_string(), true),
        }
    }

    fn new_project(&mut self, ctx: &egui::Context) {
        self.timeline = Timeline::from_tracks(Vec::new()).expect("empty timeline is valid");
        self.timeline_view = TimelineView::with_media(self.tools.clone(), ctx);
        self.cursor_marker_cache = CursorMarkerCache::default();
        self.library.clear_selection();
        self.selected_clip = None;
        self.position = 0;
        self.cursor_effect = CursorEffect::default_for(1);
        self.saved_revision = self.timeline.revision();
        self.saved_cursor_effect = self.cursor_effect.clone();
        self.background = BackgroundSettings::default();
        self.saved_background = self.background.clone();
        self.canvas_format = CanvasFormat::Original;
        self.saved_canvas_format = CanvasFormat::Original;
        self.markers.clear();
        self.saved_markers.clear();
        self.pending_additions.clear();
        self.volume_preview = None;
        self.material_info = None;
        self.text_inspector = TextInspector::default();
        self.subtitle_canvas.cancel();
        self.preview_viewport.reset();
        self.export_settings = None;
        self.media_error = None;
        self.player.playing = false;
        self.player.reset();
        match new_project_path(self.library.directory()).and_then(|path| {
            project::save_with_cursor(
                &path,
                &self.timeline,
                self.position,
                self.canvas_format,
                &self.background,
                &self.cursor_effect,
                &self.markers,
            )?;
            Ok(path)
        }) {
            Ok(path) => {
                self.project = Some(path);
                self.proxy_cache.set_project(self.project.as_deref());
                self.saved_revision = self.timeline.revision();
                self.show_notice("新工程已创建并保存在本地", false);
            }
            Err(error) => {
                self.project = None;
                self.proxy_cache.set_project(None);
                self.show_notice(format!("新工程已创建，但无法保存到本地（{error:#}）"), true);
            }
        }
        self.home = false;
    }
}

fn new_project_path(directory: &Path) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(directory).context("无法创建本地工程目录")?;
    for suffix in 1_u32.. {
        let name = if suffix == 1 {
            String::from("未命名工程.takeproj")
        } else {
            format!("未命名工程 ({suffix}).takeproj")
        };
        let path = directory.join(name);
        if !path.try_exists()? {
            return Ok(path);
        }
    }
    anyhow::bail!("无法生成新的工程文件名")
}

fn format_modified(modified: SystemTime) -> String {
    let Ok(age) = modified.elapsed() else {
        return String::from("本地工程");
    };
    let label = if age.as_secs() < 60 {
        String::from("刚刚修改")
    } else if age.as_secs() < 3_600 {
        format!("{} 分钟前修改", age.as_secs() / 60)
    } else if age.as_secs() < 86_400 {
        format!("{} 小时前修改", age.as_secs() / 3_600)
    } else {
        format!("{} 天前修改", age.as_secs() / 86_400)
    };
    label
}

fn cursor_tracks<'a>(timeline: &Timeline, cache: &'a mut CursorMarkerCache) -> &'a [CursorTrackData] {
    let key = cursor_marker_cache_key(timeline);
    if cache.key != Some(key) {
        let mut cursor_tracks = Vec::new();
        for clip in timeline
            .tracks()
            .iter()
            .filter(|track| track.kind == TrackKind::Video)
            .flat_map(|track| &track.clips)
        {
            let path = PathBuf::from(format!("{}.cursor.json", clip.path.display()));
            if !path.is_file() {
                continue;
            }
            let Ok(telemetry) = CursorTelemetry::load(&path) else {
                continue;
            };
            let mut markers = telemetry
                .samples
                .iter()
                .filter(|sample| sample.pressed)
                .filter_map(|sample| timeline_time_for_sample(clip, sample.at_us))
                .collect::<Vec<_>>();
            markers.sort_unstable();
            markers.dedup();
            cursor_tracks.push(CursorTrackData {
                clip_id: clip.id,
                markers,
            });
        }
        *cache = CursorMarkerCache {
            key: Some(key),
            markers: cursor_tracks,
        };
    }
    &cache.markers
}

fn cursor_marker_cache_key(timeline: &Timeline) -> u64 {
    let mut hasher = DefaultHasher::new();
    for track in timeline
        .tracks()
        .iter()
        .filter(|track| track.kind == TrackKind::Video)
    {
        track.id.hash(&mut hasher);
        for clip in &track.clips {
            clip.id.hash(&mut hasher);
            clip.path.hash(&mut hasher);
            clip.start.hash(&mut hasher);
            clip.source_start.hash(&mut hasher);
            clip.source_end.hash(&mut hasher);
            clip.speed_milli.hash(&mut hasher);
            clip.reverse.hash(&mut hasher);
            clip.timeline_duration_us.hash(&mut hasher);
            let path = PathBuf::from(format!("{}.cursor.json", clip.path.display()));
            match fs::metadata(path) {
                Ok(metadata) => {
                    true.hash(&mut hasher);
                    metadata.len().hash(&mut hasher);
                    metadata
                        .modified()
                        .ok()
                        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                        .map(|modified| (modified.as_secs(), modified.subsec_nanos()))
                        .hash(&mut hasher);
                }
                Err(_) => false.hash(&mut hasher),
            }
        }
    }
    hasher.finish()
}

impl eframe::App for Workspace {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        #[cfg(windows)]
        window_frame::restore_clipboard_shortcut(ctx, raw_input);
        self.material_drag.process(ctx, raw_input);
        self.transition_panel.process(ctx, raw_input);
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
        self.proxy_cache.poll();
        self.prepare_current_proxy();
        self.prepare_project_proxies();
        self.prepare_current_audio_cache();
        self.poll_speech(ctx);
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
        self.export_panel(&ctx);
        self.sync_preview(&ctx);
    }
}


