use std::path::Path;

use eframe::egui::{self, Align, Layout, RichText};
use takeseam::editor_timeline::{Clip, Timeline};

use crate::ui_theme::upper_palette;

use super::{layout, library::filename, media::MediaInfo};

pub(super) fn media(ui: &mut egui::Ui, path: &Path, info: Option<MediaInfo>, clip: Option<&Clip>) {
    let colors = upper_palette(ui.ctx());
    let name = filename(path);
    ui.add_space(4.0);
    ui.add(
        egui::Label::new(RichText::new(&name).size(13.0).strong().color(colors.text)).truncate(),
    )
    .on_hover_text(&name);
    ui.add_space(4.0);
    if let Some(info) = info {
        if info.has_video {
            row(ui, "分辨率", format!("{} × {}", info.width, info.height));
        } else if info.has_audio {
            row(ui, "类型", "音频");
        }
    }
    if let Some(clip) = clip {
        row(ui, "时长", layout::time(clip.duration()));
        row(ui, "时间线入点", layout::time(clip.start));
    } else if let Some(info) = info {
        row(ui, "时长", layout::time(info.duration_us));
    }
}

pub(super) fn sequence(ui: &mut egui::Ui, timeline: &Timeline) {
    let colors = upper_palette(ui.ctx());
    ui.add_space(4.0);
    ui.label(
        RichText::new("时间线")
            .size(13.0)
            .strong()
            .color(colors.text),
    );
    ui.add_space(4.0);
    row(ui, "总时长", layout::time(timeline.duration()));
    row(ui, "轨道数", timeline.tracks().len().to_string());
}

fn row(ui: &mut egui::Ui, name: &str, value: impl AsRef<str>) {
    let colors = upper_palette(ui.ctx());
    let value = value.as_ref();
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(RichText::new(name).size(12.0).color(colors.muted)).truncate());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(value)
                        .size(11.0)
                        .monospace()
                        .color(colors.text),
                )
                .truncate(),
            )
            .on_hover_text(value);
        });
    });
}
