use std::path::Path;

use eframe::egui;

use crate::ui_theme::palette;

use super::layout::time;
use super::library::{Library, filename};

pub(super) fn show(ctx: &egui::Context, library: &mut Library, path: &Path) -> bool {
    let info = library.request(path);
    let colors = palette(ctx);
    let mut open = true;
    egui::Window::new("素材信息")
        .id(egui::Id::new("material-info"))
        .open(&mut open)
        .collapsible(false)
        .default_width(340.0)
        .show(ctx, |ui| {
            ui.label(egui::RichText::new(filename(path)).strong());
            match info {
                Some(Ok(info)) => {
                    let kind = match (info.has_video, info.has_audio) {
                        (true, true) => "视频和音频",
                        (true, false) => "视频",
                        (false, _) => "音频",
                    };
                    egui::Grid::new("material-info-fields")
                        .spacing(egui::vec2(18.0, 5.0))
                        .show(ui, |ui| {
                            ui.label("类型");
                            ui.label(kind);
                            ui.end_row();
                            ui.label("时长");
                            ui.label(time(info.duration_us));
                            ui.end_row();
                            if info.has_video {
                                ui.label("分辨率");
                                ui.label(format!("{} × {}", info.width, info.height));
                                ui.end_row();
                            }
                        });
                }
                Some(Err(error)) => {
                    ui.colored_label(colors.danger, error);
                }
                None => {
                    ui.label("正在读取素材…");
                }
            }
            ui.separator();
            ui.add(egui::Label::new(path.display().to_string()).wrap());
        });
    open
}
