use eframe::egui::{self, Align, Align2, Color32, FontId, Layout, RichText, Sense, Stroke, vec2};
use takeseam::editor_timeline::{
    Clip, OverlayText, TextAppearance, TextPlacement, TextPosition, TextStyle, TextStyleChanges,
    Timeline, TrackKind,
};

use crate::ui_icons::Icon;
use crate::ui_theme::upper_palette;

use super::feature_panel::{FeatureTab, action_row};

#[path = "text_panel/batch.rs"]
mod batch;

#[derive(Clone, Copy)]
pub(super) enum TextTool {
    Caption,
    ImportSrt,
    Title,
    Text,
    Watermark,
}

impl TextTool {
    pub(super) fn preset(self) -> Option<(TrackKind, OverlayText)> {
        let (kind, content, size, placement, bold) = match self {
            Self::Caption => (
                TrackKind::Caption,
                "新字幕",
                48,
                TextPlacement::Bottom,
                false,
            ),
            Self::Title => (TrackKind::Text, "标题", 72, TextPlacement::Top, true),
            Self::Text => (
                TrackKind::Text,
                "画面文字",
                48,
                TextPlacement::Center,
                false,
            ),
            Self::Watermark => (TrackKind::Text, "水印", 26, TextPlacement::Top, false),
            Self::ImportSrt => return None,
        };
        Some((
            kind,
            OverlayText {
                content: content.into(),
                style: TextStyle {
                    size,
                    placement,
                    bold,
                    ..TextStyle::default()
                },
            },
        ))
    }
}

pub(super) fn tools(ui: &mut egui::Ui, tab: FeatureTab) -> Option<TextTool> {
    let actions: &[(TextTool, Icon, &str)] = match tab {
        FeatureTab::Captions => &[
            (TextTool::Caption, Icon::Captions, "添加字幕"),
            (TextTool::ImportSrt, Icon::FolderPlus, "导入 SRT"),
        ],
        FeatureTab::Text => &[
            (TextTool::Title, Icon::Text, "添加标题"),
            (TextTool::Text, Icon::Text, "画面文字"),
            (TextTool::Watermark, Icon::Text, "添加水印"),
        ],
        _ => return None,
    };

    let mut selected = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        for &(action, icon, label) in actions {
            if action_row(ui, icon, label) {
                selected = Some(action);
            }
        }
    });
    selected
}

#[derive(Default)]
pub(super) struct TextInspector {
    draft: Option<Draft>,
    batch: Option<batch::BatchDraft>,
}

struct Draft {
    id: u64,
    original: OverlayText,
    text: OverlayText,
}

impl TextInspector {
    pub(super) fn show_batch(
        &mut self,
        ui: &mut egui::Ui,
        selected: &[(u64, TextStyle)],
    ) -> Option<TextStyleChanges> {
        self.draft = None;
        if selected.is_empty() {
            self.clear_batch();
            return None;
        }
        if self
            .batch
            .as_ref()
            .is_none_or(|batch| !batch.matches(selected))
        {
            self.batch = Some(batch::BatchDraft::new(selected));
        }
        self.batch.as_mut()?.show(ui)
    }

    pub(super) fn pending_batch_edit(
        &self,
        timeline: &Timeline,
    ) -> Option<(Vec<u64>, TextStyleChanges)> {
        self.batch.as_ref()?.pending_edit(timeline)
    }

    pub(super) fn clear_batch(&mut self) {
        self.batch = None;
    }

    pub(super) fn synchronize_batch_position(
        &mut self,
        before: &[(u64, TextStyle)],
        position: TextPosition,
    ) {
        if let Some(batch) = &mut self.batch {
            batch.synchronize_position(before, position);
        }
    }

    pub(super) fn pending_edit(&self, timeline: &Timeline) -> Option<(u64, OverlayText)> {
        let draft = self.draft.as_ref()?;
        let live = timeline.clip(draft.id)?.text.as_ref()?;
        (live == &draft.original && draft.text != draft.original)
            .then(|| (draft.id, draft.text.clone()))
    }

    pub(super) fn synchronize_transform(
        &mut self,
        id: u64,
        before: &OverlayText,
        after: &OverlayText,
    ) {
        if let Some(draft) = self.draft.as_mut()
            && draft.id == id
            && &draft.original == before
        {
            draft.original.clone_from(after);
            draft.text.style.position = after.style.position;
            draft.text.style.size = after.style.size;
        }
    }

    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        selected: Option<(&Clip, TrackKind)>,
    ) -> Option<(u64, OverlayText)> {
        self.clear_batch();
        let Some((clip, kind)) = selected else {
            self.draft = None;
            return None;
        };
        let Some(text) = &clip.text else {
            self.draft = None;
            return None;
        };
        if self
            .draft
            .as_ref()
            .is_none_or(|draft| draft.id != clip.id || &draft.original != text)
        {
            self.draft = Some(Draft {
                id: clip.id,
                original: text.clone(),
                text: text.clone(),
            });
        }

        let draft = self.draft.as_mut()?;
        let colors = upper_palette(ui.ctx());
        egui::ScrollArea::vertical()
            .id_salt("text-inspector-fields")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let spacing = ui.spacing_mut();
                spacing.item_spacing = vec2(8.0, 4.0);
                spacing.interact_size.y = 24.0;

                let label = if kind == TrackKind::Caption {
                    "字幕内容"
                } else {
                    "文字内容"
                };
                let caption = ui.label(RichText::new(label).size(11.0).color(colors.muted));
                ui.add(
                    egui::TextEdit::multiline(&mut draft.text.content)
                        .id_salt(("overlay-content", draft.id))
                        .desired_rows(3)
                        .margin(egui::Margin::symmetric(8, 4))
                        .desired_width(ui.available_width()),
                )
                .labelled_by(caption.id);
                ui.add_space(8.0);

                let updates = style_fields(
                    ui,
                    Some(draft.text.style.size),
                    Some(draft.text.style.bold),
                    Some(draft.text.style.into()),
                );
                if let Some(size) = updates.size {
                    draft.text.style.size = size;
                }
                if let Some(bold) = updates.bold {
                    draft.text.style.bold = bold;
                }
                if let Some(placement) = updates.placement {
                    draft.text.style.placement = placement;
                    draft.text.style.position = None;
                }

                let valid = draft.text.validate().is_ok();
                ui.add_space(6.0);
                ui.label(RichText::new("样式预设").size(11.0).color(colors.muted));
                let preset = style_presets(ui, Some(draft.text.style.appearance), valid)
                    .filter(|preset| *preset != draft.text.style.appearance);
                if let Some(preset) = preset {
                    draft.text.style.appearance = preset;
                }

                let edited = draft.text != draft.original;
                let too_long = draft
                    .text
                    .content
                    .chars()
                    .nth(OverlayText::MAX_CHARS)
                    .is_some();
                if too_long {
                    ui.label(
                        RichText::new(format!("最多 {} 字符", OverlayText::MAX_CHARS))
                            .size(11.0)
                            .color(colors.danger),
                    );
                }
                let apply = apply_button(ui, edited && valid);
                (apply || preset.is_some()).then(|| (draft.id, draft.text.clone()))
            })
            .inner
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PlacementValue {
    Anchor(TextPlacement),
    Custom(TextPosition),
}

impl From<TextStyle> for PlacementValue {
    fn from(style: TextStyle) -> Self {
        style
            .position
            .map_or(Self::Anchor(style.placement), Self::Custom)
    }
}

fn style_fields(
    ui: &mut egui::Ui,
    size: Option<u16>,
    bold: Option<bool>,
    current_placement: Option<PlacementValue>,
) -> TextStyleChanges {
    let colors = upper_palette(ui.ctx());
    let mut changes = TextStyleChanges::default();
    let inline_bold = ui.available_width() >= 136.0;
    let bold_control = |ui: &mut egui::Ui| {
        let mut value = bold.unwrap_or(false);
        ui.add(egui::Checkbox::new(&mut value, "粗体").indeterminate(bold.is_none()))
            .changed()
            .then_some(value)
    };

    ui.horizontal(|ui| {
        let label = ui.add_sized(
            [28.0, 24.0],
            egui::Label::new(RichText::new("字号").size(11.0).color(colors.muted)),
        );
        let mixed = TextStyle::MIN_SIZE - 1;
        let mut value = size.unwrap_or(mixed);
        let field =
            egui::DragValue::new(&mut value).range(TextStyle::MIN_SIZE..=TextStyle::MAX_SIZE);
        let field = if size.is_none() {
            field
                .clamp_existing_to_range(false)
                .custom_formatter(|value, _| {
                    if value < f64::from(TextStyle::MIN_SIZE) {
                        "混合".into()
                    } else {
                        format!("{value:.0}")
                    }
                })
        } else {
            field
        };
        let changed = ui
            .add_sized([44.0, 24.0], field)
            .labelled_by(label.id)
            .changed();
        if changed && value >= TextStyle::MIN_SIZE {
            changes.size = Some(value.min(TextStyle::MAX_SIZE));
        }
        if inline_bold {
            changes.bold = bold_control(ui);
        }
    });
    ui.horizontal(|ui| {
        let label = ui.add_sized(
            [28.0, 24.0],
            egui::Label::new(RichText::new("位置").size(11.0).color(colors.muted)),
        );
        egui::ComboBox::from_id_salt("overlay-placement")
            .width((ui.available_width() - ui.spacing().button_padding.x * 2.0).max(32.0))
            .truncate()
            .selected_text(match current_placement {
                Some(PlacementValue::Anchor(placement)) => placement_label(placement),
                Some(PlacementValue::Custom(_)) => "自定义",
                None => "混合",
            })
            .show_ui(ui, |ui| {
                for placement in [
                    TextPlacement::Top,
                    TextPlacement::Center,
                    TextPlacement::Bottom,
                ] {
                    let selected = current_placement == Some(PlacementValue::Anchor(placement));
                    if ui
                        .selectable_label(selected, placement_label(placement))
                        .clicked()
                    {
                        changes.placement = Some(placement);
                    }
                }
            })
            .response
            .labelled_by(label.id);
    });
    if !inline_bold {
        ui.horizontal(|ui| {
            ui.add_space(36.0);
            changes.bold = bold_control(ui);
        });
    }
    changes
}

fn apply_button(ui: &mut egui::Ui, enabled: bool) -> bool {
    ui.add_space(8.0);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.add_enabled(
            enabled,
            egui::Button::new(RichText::new("应用").size(12.0))
                .min_size(vec2(56.0, 24.0))
                .frame(enabled),
        )
        .clicked()
    })
    .inner
}

fn style_presets(
    ui: &mut egui::Ui,
    current: Option<TextAppearance>,
    enabled: bool,
) -> Option<TextAppearance> {
    const GAP: f32 = 5.0;
    let width = ui.available_width();
    let columns = if width >= 110.0 { 2 } else { 1 };
    let card_width = ((width - GAP * (columns - 1) as f32) / columns as f32).floor();
    let colors = upper_palette(ui.ctx());
    let mut chosen = None;
    for row in TextAppearance::ALL.chunks(columns) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for &preset in row {
                let selected = Some(preset) == current;
                let label = preset_label(preset);
                let response = ui
                    .add_enabled_ui(enabled, |ui| {
                        let (rect, response) =
                            ui.allocate_exact_size(vec2(card_width, 46.0), Sense::click());
                        response.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::WidgetType::SelectableLabel,
                                response.enabled(),
                                selected,
                                label,
                            )
                        });

                        let painter = ui.painter().with_clip_rect(rect);
                        let fill = if response.hovered() || response.is_pointer_button_down_on() {
                            colors.hover
                        } else {
                            colors.surface
                        };
                        let border = if selected {
                            Stroke::new(1.5_f32, colors.accent)
                        } else if response.has_focus() {
                            Stroke::new(1.5_f32, colors.text)
                        } else {
                            Stroke::new(1.0_f32, colors.border)
                        };
                        painter.rect(rect, 5, fill, border, egui::StrokeKind::Inside);
                        let sample = egui::Rect::from_min_size(
                            rect.min + vec2(4.0, 4.0),
                            vec2((rect.width() - 8.0).max(0.0), 23.0),
                        );
                        paint_preset_sample(&painter, sample, preset);
                        painter.text(
                            egui::pos2(rect.center().x, rect.bottom() - 9.5),
                            Align2::CENTER_CENTER,
                            label,
                            FontId::proportional(11.0),
                            if selected { colors.text } else { colors.muted },
                        );
                        response
                    })
                    .inner
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(label)
                    .on_disabled_hover_text("修正内容后可套用样式");
                if response.clicked() {
                    chosen = Some(preset);
                }
            }
        });
    }
    chosen
}

fn paint_preset_sample(painter: &egui::Painter, rect: egui::Rect, preset: TextAppearance) {
    let painter = painter.with_clip_rect(rect);
    painter.rect_filled(rect, 3, Color32::from_gray(99));
    let sample = painter.layout_no_wrap(
        "字幕".into(),
        FontId::proportional(12.0),
        Color32::PLACEHOLDER,
    );
    let position = rect.center() - sample.size() * 0.5;
    let paint = preset.paint();
    let rgba = |[r, g, b, a]: [u8; 4]| Color32::from_rgba_unmultiplied(r, g, b, a);
    if let Some(background) = paint.background {
        let patch = egui::Rect::from_center_size(rect.center(), sample.size() + vec2(8.0, 3.0));
        painter.rect_filled(patch, 0, rgba(background));
    }
    let outline = rgba(paint.outline);
    if paint.shadow > 0 {
        let shift = paint.shadow as f32 * 0.35;
        painter.galley(position + vec2(shift, shift), sample.clone(), outline);
    }
    if paint.outline_width > 0 {
        let radius = paint.outline_width as f32 * 0.35;
        for (x, y) in [
            (-1.0, 0.0),
            (1.0, 0.0),
            (0.0, -1.0),
            (0.0, 1.0),
            (-0.7, -0.7),
            (0.7, -0.7),
            (-0.7, 0.7),
            (0.7, 0.7),
        ] {
            painter.galley(position + vec2(x, y) * radius, sample.clone(), outline);
        }
    }
    painter.galley(position, sample, rgba(paint.foreground));
}

fn preset_label(preset: TextAppearance) -> &'static str {
    match preset {
        TextAppearance::Outline => "白字黑边",
        TextAppearance::Plain => "纯白字",
        TextAppearance::DarkBox => "深底白字",
        TextAppearance::LightBox => "浅底黑字",
        TextAppearance::Gold => "金色描边",
        TextAppearance::Shadow => "白字阴影",
    }
}

fn placement_label(placement: TextPlacement) -> &'static str {
    match placement {
        TextPlacement::Top => "顶部",
        TextPlacement::Center => "居中",
        TextPlacement::Bottom => "底部",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use takeseam::editor_timeline::TextPosition;

    #[test]
    fn transforming_a_caption_preserves_unapplied_inspector_edits() {
        let before = OverlayText {
            content: "原字幕".into(),
            style: TextStyle::default(),
        };
        let mut after = before.clone();
        after.style.size = 64;
        after.style.position = Some(TextPosition { x: 3400, y: 5700 });
        let mut pending = before.clone();
        pending.content = "待应用字幕".into();
        pending.style.bold = true;
        pending.style.appearance = TextAppearance::Gold;
        let mut inspector = TextInspector {
            draft: Some(Draft {
                id: 7,
                original: before.clone(),
                text: pending,
            }),
            ..Default::default()
        };

        inspector.synchronize_transform(7, &before, &after);

        let draft = inspector.draft.unwrap();
        assert_eq!(draft.original, after);
        assert_eq!(draft.text.style.position, after.style.position);
        assert_eq!(draft.text.style.size, after.style.size);
        assert_eq!(draft.text.content, "待应用字幕");
        assert!(draft.text.style.bold);
        assert_eq!(draft.text.style.appearance, TextAppearance::Gold);
    }

    #[test]
    fn clicking_a_preset_commits_pending_text_with_the_current_transform() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let original = OverlayText {
            content: "原字幕".into(),
            style: TextStyle {
                size: 64,
                position: Some(TextPosition { x: 6200, y: 7700 }),
                ..TextStyle::default()
            },
        };
        let mut timeline = Timeline::default();
        let (_, id) = timeline
            .insert_text(TrackKind::Caption, 0, 1_000_000, original.clone())
            .unwrap();
        let clip = timeline.clip(id).unwrap();
        let mut pending = original.clone();
        pending.content = "待应用字幕".into();
        let mut inspector = TextInspector {
            draft: Some(Draft {
                id,
                original: original.clone(),
                text: pending.clone(),
            }),
            ..Default::default()
        };
        let frame = |events| {
            let mut edit = None;
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(176.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| edit = inspector.show(ui, Some((clip, TrackKind::Caption))),
            );
            (output, edit)
        };
        pending.style.appearance = TextAppearance::Gold;
        assert_eq!(click_preset(frame, "金色描边"), Some((id, pending)));
    }

    #[test]
    fn mixed_appearance_can_use_the_primary_captions_preset_for_all() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let selected = [
            (7, TextStyle::default()),
            (
                8,
                TextStyle {
                    appearance: TextAppearance::Gold,
                    size: 72,
                    ..TextStyle::default()
                },
            ),
        ];
        let mut inspector = TextInspector::default();
        let frame = |events| {
            let mut edit = None;
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(176.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| edit = inspector.show_batch(ui, &selected),
            );
            (output, edit)
        };
        assert_eq!(
            click_preset(frame, "白字黑边"),
            Some(TextStyleChanges {
                appearance: Some(TextAppearance::Outline),
                ..Default::default()
            }),
        );
    }

    fn click_preset<T>(
        mut frame: impl FnMut(Vec<egui::Event>) -> (egui::FullOutput, T),
        label: &str,
    ) -> T {
        let bounds = frame(Vec::new())
            .0
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .into_iter()
            .find_map(|(_, node)| {
                (node.label() == Some(label))
                    .then(|| node.bounds())
                    .flatten()
            })
            .unwrap();
        let point = egui::pos2(
            ((bounds.x0 + bounds.x1) * 0.5) as f32,
            ((bounds.y0 + bounds.y1) * 0.5) as f32,
        );
        let button = |pressed| egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(point), button(true)]);
        frame(vec![button(false)]).1
    }
}
