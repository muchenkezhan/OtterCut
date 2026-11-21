use std::collections::HashMap;

use eframe::egui::{self, RichText, vec2};
use takeseam::editor_timeline::{TextPosition, TextStyle, TextStyleChanges, Timeline};

use super::{PlacementValue, apply_button, style_fields, style_presets, upper_palette};

pub(super) struct BatchDraft {
    id: egui::Id,
    original: Vec<(u64, TextStyle)>,
    changes: TextStyleChanges,
}

impl BatchDraft {
    pub(super) fn new(selected: &[(u64, TextStyle)]) -> Self {
        let ids = selected.iter().map(|(id, _)| id).collect::<Vec<_>>();
        Self {
            id: egui::Id::new(("batch-text-inspector", ids)),
            original: selected.to_vec(),
            changes: TextStyleChanges::default(),
        }
    }

    pub(super) fn matches(&self, selected: &[(u64, TextStyle)]) -> bool {
        self.original.as_slice() == selected
    }

    pub(super) fn synchronize_position(
        &mut self,
        before: &[(u64, TextStyle)],
        position: TextPosition,
    ) {
        if self.matches(before) {
            for (_, style) in &mut self.original {
                style.position = Some(position);
            }
            self.changes.placement = None;
        }
    }

    pub(super) fn pending_edit(&self, timeline: &Timeline) -> Option<(Vec<u64>, TextStyleChanges)> {
        if self.changes == TextStyleChanges::default() {
            return None;
        }
        let current: HashMap<_, _> = timeline
            .tracks()
            .iter()
            .flat_map(|track| &track.clips)
            .filter_map(|clip| Some((clip.id, clip.text.as_ref()?.style)))
            .collect();
        self.original
            .iter()
            .all(|(id, style)| current.get(id) == Some(style))
            .then(|| {
                (
                    self.original.iter().map(|(id, _)| *id).collect(),
                    self.changes,
                )
            })
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui) -> Option<TextStyleChanges> {
        let colors = upper_palette(ui.ctx());
        egui::ScrollArea::vertical()
            .id_salt(self.id)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let spacing = ui.spacing_mut();
                spacing.item_spacing = vec2(8.0, 4.0);
                spacing.interact_size.y = 24.0;
                ui.label(
                    RichText::new(format!("已选 {} 个文本片段", self.original.len()))
                        .size(11.0)
                        .color(colors.muted),
                );
                ui.add_space(8.0);

                let original_size = self.shared(|style| style.size);
                let original_bold = self.shared(|style| style.bold);
                let original_position = self.shared(PlacementValue::from);
                let fields = style_fields(
                    ui,
                    self.changes.size.or(original_size),
                    self.changes.bold.or(original_bold),
                    self.changes
                        .placement
                        .map(PlacementValue::Anchor)
                        .or(original_position),
                );
                if let Some(size) = fields.size {
                    self.changes.size = (Some(size) != original_size).then_some(size);
                }
                if let Some(bold) = fields.bold {
                    self.changes.bold = (Some(bold) != original_bold).then_some(bold);
                }
                if let Some(placement) = fields.placement {
                    self.changes.placement = (Some(PlacementValue::Anchor(placement))
                        != original_position)
                        .then_some(placement);
                }

                ui.add_space(6.0);
                ui.label(RichText::new("样式预设").size(11.0).color(colors.muted));
                let original_appearance = self.shared(|style| style.appearance);
                let selected = self.changes.appearance.or(original_appearance);
                let preset =
                    style_presets(ui, selected, true).filter(|&preset| Some(preset) != selected);
                if let Some(preset) = preset {
                    self.changes.appearance =
                        (Some(preset) != original_appearance).then_some(preset);
                }

                let has_changes = self.changes != TextStyleChanges::default();
                let apply = apply_button(ui, has_changes);
                (has_changes && (apply || preset.is_some())).then_some(self.changes)
            })
            .inner
    }

    fn shared<T: Copy + PartialEq>(&self, get: impl Fn(TextStyle) -> T) -> Option<T> {
        let first = get(self.original.first()?.1);
        self.original
            .iter()
            .skip(1)
            .all(|(_, style)| get(*style) == first)
            .then_some(first)
    }
}
