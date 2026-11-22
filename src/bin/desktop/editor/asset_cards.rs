use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Color32, CornerRadius, DragAndDrop, Event, FontId, PointerButton, Pos2, Rect, Stroke,
    StrokeKind, pos2, vec2,
};
use takeseam::editor_timeline::TrackKind;

use crate::ui_icons::Icon;
use crate::ui_theme::{Palette, upper_palette};
use takeseam::i18n::{Locale, key};

use super::feature_panel::action_row;
use super::layout;
use super::library::{Library, filename, has_audio_extension};
use super::thumbnails::Thumbnails;

const GRID_GAP: f32 = 8.0;
const MIN_CARD_WIDTH: f32 = 110.0;
const CAPTION_HEIGHT: f32 = 22.0;
const CARD_RADIUS: u8 = 6;

#[derive(Default)]
pub(super) struct AssetInput {
    pub selected: Option<usize>,
    pub action: Option<MaterialAction>,
}

pub(super) enum MaterialAction {
    Import,
    Add(PathBuf),
    Inspect(PathBuf),
    OpenFolder(PathBuf),
    CopyPath(PathBuf),
    Remove(PathBuf),
}

#[derive(Clone)]
pub(super) struct DraggedAsset {
    pub path: PathBuf,
    pub duration_us: Option<u64>,
    pub has_video: Option<bool>,
    pub has_audio: Option<bool>,
}

#[derive(Default)]
pub(super) struct MaterialDrag {
    viewport: Option<egui::ViewportId>,
    cards: Vec<CardHit>,
    press: Option<CardPress>,
    released_drag: bool,
}

struct CardHit {
    rect: Rect,
    layer: egui::LayerId,
    asset: DraggedAsset,
}

struct CardPress {
    origin: Pos2,
    asset: DraggedAsset,
    started: bool,
}

impl MaterialDrag {
    pub(super) fn hide(&mut self, ctx: &egui::Context) {
        self.cancel(ctx);
        self.cards.clear();
        self.viewport = None;
    }

    pub(super) fn process(&mut self, ctx: &egui::Context, raw: &egui::RawInput) {
        if self.viewport != Some(raw.viewport_id) {
            return;
        }
        self.released_drag = false;
        if !raw.focused {
            self.cancel(ctx);
            return;
        }

        // Native input can deliver the press, movement and release in the same frame.
        let threshold = ctx.options(|options| options.input_options.max_click_dist);
        for event in &raw.events {
            match *event {
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: true,
                    ..
                } => {
                    self.cancel(ctx);
                    let layer = ctx.layer_id_at(pos);
                    self.press = self.cards.iter().rev().find_map(|card| {
                        (card.rect.contains(pos) && layer.is_none_or(|layer| layer == card.layer))
                            .then(|| CardPress {
                                origin: pos,
                                asset: card.asset.clone(),
                                started: false,
                            })
                    });
                }
                Event::PointerMoved(pos) => self.move_to(ctx, pos, threshold),
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: false,
                    ..
                } => {
                    self.move_to(ctx, pos, threshold);
                    self.released_drag |= self.press.take().is_some_and(|press| press.started);
                }
                Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                } => self.cancel(ctx),
                Event::PointerGone if self.press.is_some() => self.cancel(ctx),
                _ => {}
            }
        }
    }

    fn move_to(&mut self, ctx: &egui::Context, pos: Pos2, threshold: f32) {
        if let Some(press) = &mut self.press
            && !press.started
            && press.origin.distance_sq(pos) > threshold * threshold
        {
            press.started = true;
            DragAndDrop::set_payload(ctx, press.asset.clone());
        }
    }

    fn cancel(&mut self, ctx: &egui::Context) {
        self.released_drag |= self.press.take().is_some_and(|press| press.started);
        if DragAndDrop::has_payload_of_type::<DraggedAsset>(ctx) {
            DragAndDrop::clear_payload(ctx);
        }
    }
}

pub(super) fn show(
    ui: &mut egui::Ui,
    library: &Library,
    thumbnails: &mut Thumbnails,
    drag: &mut MaterialDrag,
    add_to: impl Fn(&Path) -> Option<TrackKind>,
) -> AssetInput {
    show_grid(ui, library, thumbnails, drag, add_to, false)
}

pub(super) fn show_audio(
    ui: &mut egui::Ui,
    library: &Library,
    thumbnails: &mut Thumbnails,
    drag: &mut MaterialDrag,
    add_to: impl Fn(&Path) -> Option<TrackKind>,
) -> AssetInput {
    show_grid(ui, library, thumbnails, drag, add_to, true)
}

fn show_grid(
    ui: &mut egui::Ui,
    library: &Library,
    thumbnails: &mut Thumbnails,
    drag: &mut MaterialDrag,
    add_to: impl Fn(&Path) -> Option<TrackKind>,
    audio_only: bool,
) -> AssetInput {
    drag.viewport = Some(ui.ctx().viewport_id());
    drag.cards.clear();
    // Register the panel before its cards so cards and scrollbars receive overlapping clicks.
    let background = ui.interact(
        ui.available_rect_before_wrap(),
        ui.id().with(("materials-context", audio_only)),
        egui::Sense::click(),
    );
    let show_menus = !DragAndDrop::has_payload_of_type::<DraggedAsset>(ui.ctx());
    let audio_indices = audio_only.then(|| {
        library
            .paths
            .iter()
            .enumerate()
            .filter_map(|(index, path)| library.is_audio(path).then_some(index))
            .collect::<Vec<_>>()
    });
    let indices = audio_indices.as_deref();
    let card_count = indices.map_or(library.paths.len(), <[usize]>::len);
    let locale = Locale::from_context(ui.ctx());
    let import_label = if audio_only {
        locale.text(key::IMPORT_AUDIO)
    } else {
        locale.text(key::IMPORT_MEDIA)
    };
    if card_count == 0 {
        let action = if !audio_only && action_row(ui, Icon::FolderPlus, import_label) {
            Some(MaterialAction::Import)
        } else {
            show_menus
                .then(|| import_menu(&background, import_label))
                .flatten()
        };
        return AssetInput {
            action,
            ..Default::default()
        };
    }

    ui.add_space(4.0);
    let scroll = &ui.spacing().scroll;
    let available =
        (ui.available_width() - scroll.allocated_width().max(scroll.bar_width)).max(1.0);
    let columns = ((available + GRID_GAP) / (MIN_CARD_WIDTH + GRID_GAP))
        .floor()
        .max(1.0) as usize;
    let width = ((available - (columns - 1) as f32 * GRID_GAP) / columns as f32).floor();
    let height = width * 9.0 / 16.0 + CAPTION_HEIGHT;
    let card_id = ui.id().with(("material-card", audio_only));

    let mut result = egui::ScrollArea::vertical()
        .id_salt(("materials-grid", audio_only))
        .auto_shrink([false, false])
        .scroll_source(egui::scroll_area::ScrollSource {
            scroll_bar: true,
            drag: false,
            mouse_wheel: true,
        })
        .show_rows(ui, height, card_count.div_ceil(columns), |ui, rows| {
            let mut result = AssetInput::default();
            for row in rows {
                let begin = row * columns;
                let end = (begin + columns).min(card_count);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GRID_GAP;
                    for position in begin..end {
                        let index = indices.map_or(position, |indices| indices[position]);
                        let response = card(
                            ui,
                            card_id,
                            vec2(width, height),
                            library,
                            thumbnails,
                            index,
                            drag,
                        );
                        if response.clicked() {
                            result.selected = Some(index);
                        }
                        if show_menus {
                            let path = &library.paths[index];
                            let media = library.info(path);
                            let can_add = add_to(path).is_some_and(|kind| {
                                media.is_none_or(|info| match kind {
                                    TrackKind::Video => info.has_video,
                                    TrackKind::Audio => info.has_audio,
                                    TrackKind::Caption | TrackKind::Text => false,
                                })
                            });
                            if let Some(action) = material_menu(&response, path, can_add) {
                                result.action = Some(action);
                            }
                        }
                    }
                });
            }
            result
        })
        .inner;
    paint_drag_preview(ui);
    result.selected = result.selected.filter(|_| !drag.released_drag);
    if show_menus && result.action.is_none() {
        result.action = import_menu(&background, import_label);
    }
    result
}

fn import_menu(response: &egui::Response, label: &str) -> Option<MaterialAction> {
    egui::Popup::context_menu(response)
        .show(|ui| {
            layout::compact_menu(ui);
            if ui.button(label).clicked() {
                ui.close();
                Some(MaterialAction::Import)
            } else {
                None
            }
        })
        .and_then(|menu| menu.inner)
}

fn material_menu(response: &egui::Response, path: &Path, can_add: bool) -> Option<MaterialAction> {
    egui::Popup::context_menu(response)
        .show(|ui| {
            layout::compact_menu(ui);
            let locale = Locale::from_context(ui.ctx());
            let action = if ui
                .add_enabled(can_add, egui::Button::new(locale.text(key::ADD_TO_TRACK)))
                .clicked()
            {
                MaterialAction::Add(path.to_owned())
            } else if ui.button(locale.text(key::VIEW_INFO)).clicked() {
                MaterialAction::Inspect(path.to_owned())
            } else if ui.button(locale.text(key::OPEN_FOLDER)).clicked() {
                MaterialAction::OpenFolder(path.to_owned())
            } else if ui.button(locale.text(key::COPY_PATH)).clicked() {
                MaterialAction::CopyPath(path.to_owned())
            } else {
                ui.separator();
                if ui.button(locale.text(key::REMOVE_ASSET)).clicked() {
                    MaterialAction::Remove(path.to_owned())
                } else {
                    return None;
                }
            };
            ui.close();
            Some(action)
        })
        .and_then(|menu| menu.inner)
}

fn card(
    ui: &mut egui::Ui,
    id: egui::Id,
    size: egui::Vec2,
    library: &Library,
    thumbnails: &mut Thumbnails,
    index: usize,
    drag: &mut MaterialDrag,
) -> egui::Response {
    let path = &library.paths[index];
    let name = filename(path);
    let selected = library.selected == Some(index);
    let (_, rect) = ui.allocate_space(size);
    let response = ui.interact(rect, id.with(path), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            response.enabled(),
            selected,
            &name,
        )
    });
    if response.gained_focus() {
        response.scroll_to_me(None);
    }
    let info = library.info(path);
    let pending_audio = info.is_none() && has_audio_extension(path);
    if ui.is_rect_visible(rect) {
        drag.cards.push(CardHit {
            rect: response.interact_rect,
            layer: ui.layer_id(),
            asset: DraggedAsset {
                path: path.clone(),
                duration_us: info.map(|info| info.duration_us),
                has_video: info
                    .map(|info| info.has_video)
                    .or(pending_audio.then_some(false)),
                has_audio: info
                    .map(|info| info.has_audio)
                    .or(pending_audio.then_some(true)),
            },
        });
        let painter = ui.painter();
        let colors = upper_palette(ui.ctx());
        let image = Rect::from_min_max(rect.min, rect.max - vec2(0.0, CAPTION_HEIGHT));
        let audio = pending_audio || info.is_some_and(|info| !info.has_video);
        let just_hovered = response.hovered() || response.is_pointer_button_down_on();
        let fill = if selected {
            colors.selection
        } else if just_hovered {
            colors.hover
        } else {
            colors.control
        };
        painter.rect_filled(rect, CARD_RADIUS, fill);
        let texture = (!audio).then(|| thumbnails.texture(path)).flatten();
        paint_media(painter, image, texture, audio, colors);

        let outline = if response.has_focus() {
            Stroke::new(2.0_f32, colors.accent)
        } else if selected {
            Stroke::new(1.0_f32, colors.accent)
        } else if just_hovered {
            Stroke::new(1.0_f32, colors.border)
        } else {
            Stroke::NONE
        };
        painter.rect_stroke(rect, CARD_RADIUS, outline, StrokeKind::Inside);

        if let Some(info) = info {
            paint_duration(painter, image, info.duration_us);
        }
        let name = path
            .file_stem()
            .and_then(|part| part.to_str())
            .unwrap_or(&name);
        let mut caption = egui::text::LayoutJob::simple(
            name.to_owned(),
            FontId::proportional(11.5),
            colors.text,
            (rect.width() - 12.0).max(1.0),
        );
        caption.wrap.max_rows = 1;
        let galley = painter.layout_job(caption);
        let baseline = image.bottom() + CAPTION_HEIGHT * 0.5;
        painter.galley(
            pos2(rect.left() + 6.0, baseline - galley.size().y * 0.5),
            galley,
            colors.text,
        );
    }
    if DragAndDrop::has_payload_of_type::<DraggedAsset>(ui.ctx()) {
        response.on_hover_cursor(egui::CursorIcon::Grabbing)
    } else {
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(path.display().to_string())
    }
}

fn paint_media(
    painter: &egui::Painter,
    image: Rect,
    texture: Option<&egui::TextureHandle>,
    audio: bool,
    colors: Palette,
) {
    let corners = CornerRadius {
        nw: CARD_RADIUS,
        ne: CARD_RADIUS,
        sw: 0,
        se: 0,
    };
    let matte = if audio {
        colors.audio_clip
    } else {
        colors.monitor
    };
    painter.rect_filled(image, corners, matte);
    if let Some(texture) = texture {
        let pixels = texture.size_vec2();
        let scale = (image.width() / pixels.x).min(image.height() / pixels.y);
        let bounds = Rect::from_center_size(image.center(), pixels * scale);
        painter.add(
            egui::epaint::RectShape::filled(bounds, corners, Color32::WHITE)
                .with_texture(texture.id(), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0))),
        );
    } else {
        let (icon, ink) = if audio {
            (Icon::Audio, colors.audio_wave)
        } else {
            (Icon::Media, colors.muted)
        };
        icon.paint(painter, image.center(), ink);
    }
}

fn paint_duration(painter: &egui::Painter, image: Rect, duration_us: u64) {
    let label = painter.layout_no_wrap(
        format_duration(duration_us),
        FontId::monospace(10.0),
        Color32::WHITE,
    );
    let size = label.size() + vec2(8.0, 2.0);
    let max = image.max - vec2(4.0, 4.0);
    let badge = Rect::from_min_max(max - size, max);
    painter.rect_filled(badge, 3, Color32::from_black_alpha(214));
    painter.galley(badge.min + vec2(4.0, 1.0), label, Color32::WHITE);
}

fn paint_drag_preview(ui: &egui::Ui) {
    let ctx = ui.ctx();
    let Some(asset) = egui::DragAndDrop::payload::<DraggedAsset>(ctx) else {
        return;
    };
    let Some(pointer) = ui.input(|input| {
        input
            .pointer
            .hover_pos()
            .filter(|_| input.pointer.primary_down())
    }) else {
        return;
    };
    let bounds = ctx.content_rect().shrink(6.0);
    if !bounds.contains(pointer) || bounds.width() < 48.0 || bounds.height() < 30.0 {
        return;
    }
    let colors = upper_palette(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        ui.id().with("material-drag-preview"),
    ));
    let mut caption = egui::text::LayoutJob::simple(
        filename(&asset.path),
        FontId::proportional(12.0),
        colors.text,
        (bounds.width() - 35.0).min(176.0),
    );
    caption.wrap.max_rows = 1;
    let label = painter.layout_job(caption);
    let size = vec2(label.size().x + 35.0, 28.0);
    let origin = pos2(
        (pointer.x + 12.0).min(bounds.right() - size.x),
        (pointer.y - 12.0 - size.y).max(bounds.top()),
    );
    let rect = Rect::from_min_size(origin, size);
    painter.rect(
        rect,
        7,
        colors.surface,
        Stroke::new(1.0_f32, colors.accent),
        StrokeKind::Inside,
    );
    let icon = if asset.has_video == Some(false) {
        Icon::Audio
    } else {
        Icon::Media
    };
    icon.paint(
        &painter,
        pos2(rect.left() + 14.0, rect.center().y),
        colors.accent,
    );
    painter.galley(
        pos2(rect.left() + 27.0, rect.center().y - label.size().y / 2.0),
        label,
        colors.text,
    );
}

fn format_duration(duration_us: u64) -> String {
    let seconds = duration_us / 1_000_000;
    let hours = seconds / 3600;
    let minutes = (seconds / 60) % 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

#[cfg(test)]
mod context_menu_tests {
    use super::*;
    use crate::editor::media::MediaInfo;

    #[test]
    fn context_menus_use_the_clicked_card_and_open_on_empty_panel_space() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.mp4");
        let second = directory.path().join("second.wav");
        let mut library = Library::new(directory.path().to_owned(), None, &ctx);
        library.insert(first, true);
        library.insert(second.clone(), false);
        let mut thumbnails = Thumbnails::new(None, &ctx);
        let mut drag = MaterialDrag::default();
        let mut frame = |events| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(480.0, 320.0))),
                events,
                ..Default::default()
            };
            drag.process(&ctx, &input);
            let mut result = AssetInput::default();
            let widgets = ctx.run_ui(input, |ui| {
                result = show(ui, &library, &mut thumbnails, &mut drag, |_| {
                    Some(TrackKind::Video)
                });
            });
            (widgets, result)
        };
        let center = |widgets: egui::FullOutput, name| {
            let bounds = widgets
                .platform_output
                .accesskit_update
                .unwrap()
                .nodes
                .iter()
                .find_map(|(_, node)| {
                    (node.label() == Some(name))
                        .then(|| node.bounds())
                        .flatten()
                })
                .unwrap_or_else(|| panic!("未找到菜单或卡片 {name}"));
            pos2(
                ((bounds.x0 + bounds.x1) * 0.5) as f32,
                ((bounds.y0 + bounds.y1) * 0.5) as f32,
            )
        };
        let click = |point, button| {
            let event = |pressed| Event::PointerButton {
                pos: point,
                button,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            vec![Event::PointerMoved(point), event(true), event(false)]
        };

        let card = center(frame(Vec::new()).0, "second.wav");
        let (_, result) = frame(click(card, PointerButton::Secondary));
        assert_eq!(result.selected, None);
        assert!(result.action.is_none());
        // egui measures a popup before placing its buttons at the pointer on the next frame.
        let copy = center(frame(Vec::new()).0, "复制文件路径");
        let mut copy_events = click(copy, PointerButton::Primary);
        let release = copy_events.pop().unwrap();
        frame(copy_events);
        let (_, result) = frame(vec![release]);
        assert!(matches!(result.action, Some(MaterialAction::CopyPath(path)) if path == second));
        assert_eq!(result.selected, None);
        frame(Vec::new());

        frame(click(card, PointerButton::Secondary));
        let remove = center(frame(Vec::new()).0, "移除素材");
        let mut remove_events = click(remove, PointerButton::Primary);
        let release = remove_events.pop().unwrap();
        frame(remove_events);
        let (_, result) = frame(vec![release]);
        assert!(matches!(result.action, Some(MaterialAction::Remove(path)) if path == second));
        frame(Vec::new());

        frame(click(pos2(100.0, 260.0), PointerButton::Secondary));
        let import = center(frame(Vec::new()).0, "导入素材");
        let mut import_events = click(import, PointerButton::Primary);
        let release = import_events.pop().unwrap();
        frame(import_events);
        let (_, result) = frame(vec![release]);
        assert!(matches!(result.action, Some(MaterialAction::Import)));
    }

    #[test]
    fn audio_grid_selects_and_drags_the_original_asset_index() {
        let ctx = egui::Context::default();
        let directory = tempfile::tempdir().unwrap();
        let wave = directory.path().join("voice.WAV");
        let container = directory.path().join("audio-only.mp4");
        let mut library = Library::new(directory.path().to_owned(), None, &ctx);
        library.insert(directory.path().join("video.mp4"), false);
        library.insert(wave.clone(), false);
        let media = MediaInfo {
            duration_us: 2_000_000,
            width: 0,
            height: 0,
            has_audio: true,
            has_video: false,
        };
        library.add_fixture(
            directory.path().join("video-named-audio.mp3"),
            MediaInfo {
                width: 640,
                height: 360,
                has_video: true,
                ..media
            },
        );
        library.add_fixture(container.clone(), media);
        let mut thumbnails = Thumbnails::new(None, &ctx);
        let mut drag = MaterialDrag::default();
        let mut frame = |events| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(480.0, 320.0))),
                events,
                ..Default::default()
            };
            drag.process(&ctx, &input);
            let payload = DragAndDrop::payload::<DraggedAsset>(&ctx);
            let mut result = AssetInput::default();
            let _ = ctx.run_ui(input, |ui| {
                result = show_audio(ui, &library, &mut thumbnails, &mut drag, |_| {
                    Some(TrackKind::Audio)
                });
            });
            let cards = drag
                .cards
                .iter()
                .map(|card| (card.asset.path.clone(), card.rect.center()))
                .collect::<Vec<_>>();
            (result, cards, payload)
        };
        let (_, cards, _) = frame(Vec::new());
        let [(first, wave_center), (second, container_center)] = cards.as_slice() else {
            panic!("音频列表应包含且仅包含两条音频素材");
        };
        assert_eq!(first, &wave);
        assert_eq!(second, &container);
        let primary = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };

        frame(vec![
            Event::PointerMoved(*container_center),
            primary(*container_center, true),
        ]);
        let (selection, _, _) = frame(vec![primary(*container_center, false)]);
        assert_eq!(selection.selected, Some(3));

        frame(vec![
            Event::PointerMoved(*wave_center),
            primary(*wave_center, true),
        ]);
        let drop_at = pos2(420.0, 240.0);
        let (release, _, payload) =
            frame(vec![Event::PointerMoved(drop_at), primary(drop_at, false)]);
        let payload = payload.expect("应识别到音频素材拖放");
        assert_eq!(payload.path, wave);
        assert_eq!(payload.has_video, Some(false));
        assert_eq!(payload.has_audio, Some(true));
        assert_eq!(release.selected, None);
    }
}
