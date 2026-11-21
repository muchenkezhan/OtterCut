use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Stroke, Theme};

const REFRESH_INTERVAL: Duration = Duration::from_millis(750);
const EDITOR_STYLE: &str = "takeseam-editor-style";
const ACCENT_FILL: Color32 = color(0x2F6BDF);
pub(crate) const ON_ACCENT_FILL: Color32 = Color32::WHITE;
const DARK_ACCENT: Color32 = color(0x5794EC);
const LIGHT_ACCENT: Color32 = color(0x265BC1);
const LIGHT_SELECTION: Color32 = color(0xDFE9F8);
const LIGHT_AUDIO_CLIP: Color32 = color(0xE8F1FC);

#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub canvas: Color32,
    pub surface: Color32,
    pub monitor: Color32,
    pub field: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_fill: Color32,
    pub on_accent: Color32,
    pub danger: Color32,
    pub recording: Color32,
    pub playhead: Color32,
    pub timecode: Color32,
    pub selection: Color32,
    pub hover: Color32,
    pub control: Color32,
    pub track: Color32,
    pub clip: Color32,
    pub audio_clip: Color32,
    pub audio_wave: Color32,
    pub clip_border: Color32,
    pub selected_clip_border: Color32,
}

// FilmCraft Darkest surfaces and blue accents, adapted to the OtterCut controls.
const DARK: Palette = Palette {
    canvas: color(0x000000),
    surface: color(0x1D1D1D),
    monitor: color(0x1D1D1D),
    field: color(0x0E0E0E),
    border: color(0x303030),
    text: color(0xD1D1D1),
    muted: color(0xB0B0B0),
    accent: DARK_ACCENT,
    accent_fill: ACCENT_FILL,
    on_accent: color(0x0E1522),
    danger: color(0xDC513D),
    recording: color(0xE34850),
    playhead: color(0x5895EC),
    timecode: color(0x4096F3),
    selection: color(0x27364D),
    hover: color(0x2C2C2C),
    control: color(0x242424),
    track: color(0x1D1D1D),
    clip: color(0x3A3A46),
    audio_clip: color(0x3A3A46),
    audio_wave: DARK_ACCENT,
    clip_border: color(0x505050),
    selected_clip_border: DARK_ACCENT,
};

const LIGHT: Palette = Palette {
    canvas: color(0xF1F1F2),
    surface: color(0xFFFFFF),
    monitor: color(0xF6F6F7),
    field: color(0xF6F6F7),
    border: color(0xDADADC),
    text: color(0x222224),
    muted: color(0x56565B),
    accent: LIGHT_ACCENT,
    accent_fill: ACCENT_FILL,
    on_accent: ON_ACCENT_FILL,
    danger: color(0xB33129),
    recording: color(0xB33129),
    playhead: LIGHT_ACCENT,
    timecode: LIGHT_ACCENT,
    selection: LIGHT_SELECTION,
    hover: color(0xECECEE),
    control: color(0xF0F0F1),
    track: color(0xF6F6F7),
    clip: color(0xE4E4E6),
    audio_clip: LIGHT_AUDIO_CLIP,
    audio_wave: LIGHT_ACCENT,
    clip_border: color(0xA5A5AC),
    selected_clip_border: LIGHT_ACCENT,
};

const EDITOR_LIGHT: Palette = Palette {
    canvas: color(0xF2F4F6),
    surface: color(0xFFFFFF),
    monitor: color(0xEEF1F3),
    field: color(0xEEF1F3),
    border: color(0xDDE1E6),
    text: color(0x20242A),
    muted: color(0x57616D),
    accent: LIGHT_ACCENT,
    accent_fill: ACCENT_FILL,
    on_accent: ON_ACCENT_FILL,
    danger: color(0xB33429),
    recording: color(0xB33429),
    playhead: LIGHT_ACCENT,
    timecode: LIGHT_ACCENT,
    selection: LIGHT_SELECTION,
    hover: color(0xF2F4F6),
    control: color(0xF0F2F4),
    track: color(0xFFFFFF),
    clip: color(0xE6EAEF),
    audio_clip: LIGHT_AUDIO_CLIP,
    audio_wave: LIGHT_ACCENT,
    clip_border: color(0xB7BEC8),
    selected_clip_border: LIGHT_ACCENT,
};

const UPPER_LIGHT: Palette = Palette {
    canvas: color(0xF2F4F6),
    surface: color(0xFFFFFF),
    monitor: color(0xEBEFF3),
    border: color(0xDDE3EA),
    muted: color(0x586573),
    hover: color(0xEFF3F7),
    control: color(0xF2F5F8),
    ..EDITOR_LIGHT
};

const fn color(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub(crate) fn palette(ctx: &egui::Context) -> Palette {
    let editor = ctx.data(|data| data.get_temp::<bool>(egui::Id::new(EDITOR_STYLE)));
    select_palette(ctx.theme(), editor.unwrap_or(false))
}

pub(crate) fn upper_palette(ctx: &egui::Context) -> Palette {
    match ctx.theme() {
        Theme::Dark => DARK,
        Theme::Light => UPPER_LIGHT,
    }
}

fn select_palette(theme: Theme, editor: bool) -> Palette {
    match (theme, editor) {
        (Theme::Dark, _) => DARK,
        (Theme::Light, false) => LIGHT,
        (Theme::Light, true) => EDITOR_LIGHT,
    }
}

#[derive(Clone)]
struct Preference {
    path: Option<PathBuf>,
    stored: Option<Theme>,
    checked: Instant,
}

impl Preference {
    fn id() -> egui::Id {
        egui::Id::new("takeseam-theme-preference")
    }
}

pub(crate) fn initialize(ctx: &egui::Context) {
    initialize_style(ctx, false);
}

pub(crate) fn initialize_editor(ctx: &egui::Context) {
    initialize_style(ctx, true);
}

fn initialize_style(ctx: &egui::Context, editor: bool) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(EDITOR_STYLE), editor));
    for theme in [Theme::Dark, Theme::Light] {
        let mut style = style(theme, select_palette(theme, editor));
        if editor {
            style.spacing.interact_size = egui::vec2(28.0, 28.0);
            style.spacing.item_spacing = egui::vec2(8.0, 4.0);
            style.visuals.window_corner_radius = egui::CornerRadius::same(6);
            for widget in [
                &mut style.visuals.widgets.noninteractive,
                &mut style.visuals.widgets.inactive,
                &mut style.visuals.widgets.hovered,
                &mut style.visuals.widgets.active,
                &mut style.visuals.widgets.open,
            ] {
                widget.corner_radius = egui::CornerRadius::same(5);
            }
            for role in [egui::TextStyle::Body, egui::TextStyle::Button] {
                style
                    .text_styles
                    .insert(role, egui::FontId::proportional(12.0));
            }
            style
                .text_styles
                .insert(egui::TextStyle::Monospace, egui::FontId::monospace(11.0));
        }
        ctx.set_style_of(theme, style);
    }
    set_fonts(ctx);

    let path = std::env::var_os("LOCALAPPDATA")
        .filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(path).join("TakeSeam").join("theme"));
    let stored = path.as_deref().and_then(read_preference);
    ctx.data_mut(|data| {
        data.insert_temp(
            Preference::id(),
            Preference {
                path,
                stored,
                checked: Instant::now(),
            },
        );
    });
    apply(ctx, stored.unwrap_or(Theme::Dark));
}

pub(crate) fn toggle(ctx: &egui::Context) -> io::Result<()> {
    let next = match ctx.theme() {
        Theme::Dark => Theme::Light,
        Theme::Light => Theme::Dark,
    };
    apply(ctx, next);

    let preference = ctx.data(|data| data.get_temp::<Preference>(Preference::id()));
    if let Some(mut preference) = preference
        && let Some(path) = preference.path.as_deref()
    {
        write_preference(path, next)?;
        preference.stored = Some(next);
        preference.checked = Instant::now();
        ctx.data_mut(|data| data.insert_temp(Preference::id(), preference));
    }
    Ok(())
}

pub(crate) fn sync(ctx: &egui::Context) {
    let Some(mut preference) = ctx.data(|data| data.get_temp::<Preference>(Preference::id()))
    else {
        return;
    };
    let Some(path) = preference.path.as_deref() else {
        return;
    };
    if preference.checked.elapsed() >= REFRESH_INTERVAL {
        // Startup and successful atomic writes are the only sources of persisted values.
        if let Some(stored) = read_preference(path)
            && preference.stored != Some(stored)
        {
            preference.stored = Some(stored);
            apply(ctx, stored);
        }
        preference.checked = Instant::now();
        ctx.data_mut(|data| data.insert_temp(Preference::id(), preference));
    }
    ctx.request_repaint_after(REFRESH_INTERVAL);
}

fn apply(ctx: &egui::Context, theme: Theme) {
    ctx.set_theme(theme);
    let window_theme = match theme {
        Theme::Dark => egui::SystemTheme::Dark,
        Theme::Light => egui::SystemTheme::Light,
    };
    ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(window_theme));
    ctx.request_repaint();
}

fn read_preference(path: &Path) -> Option<Theme> {
    match fs::read_to_string(path).ok()?.trim() {
        "dark" => Some(Theme::Dark),
        "light" => Some(Theme::Light),
        _ => None,
    }
}

fn write_preference(path: &Path, theme: Theme) -> io::Result<()> {
    let directory = path.parent().expect("theme preference has a parent folder");
    fs::create_dir_all(directory)?;
    let mut staged = tempfile::NamedTempFile::new_in(directory)?;
    let value = match theme {
        Theme::Dark => b"dark\n".as_slice(),
        Theme::Light => b"light\n".as_slice(),
    };
    staged.write_all(value)?;
    staged.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn style(theme: Theme, palette: Palette) -> egui::Style {
    let mut style = egui::Style {
        visuals: match theme {
            Theme::Dark => egui::Visuals::dark(),
            Theme::Light => egui::Visuals::light(),
        },
        ..Default::default()
    };
    let visuals = &mut style.visuals;
    visuals.override_text_color = None;
    visuals.weak_text_color = Some(palette.muted);
    visuals.panel_fill = palette.canvas;
    visuals.window_fill = palette.surface;
    visuals.window_corner_radius = egui::CornerRadius::same(11);
    visuals.window_stroke = Stroke::new(1.0_f32, palette.border);
    visuals.window_shadow = egui::epaint::Shadow::NONE;
    visuals.popup_shadow = egui::epaint::Shadow::NONE;
    visuals.extreme_bg_color = palette.field;
    visuals.faint_bg_color = palette.control;
    visuals.code_bg_color = palette.control;
    visuals.hyperlink_color = palette.accent;
    visuals.error_fg_color = palette.danger;
    visuals.warn_fg_color = palette.danger;
    visuals.text_cursor.stroke.color = palette.accent;
    visuals.selection.bg_fill = palette.accent_fill;
    visuals.selection.stroke = Stroke::new(1.0_f32, ON_ACCENT_FILL);
    visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    visuals.handle_shape = egui::style::HandleShape::Circle;

    for (widget, fill, border, text) in [
        (
            &mut visuals.widgets.noninteractive,
            palette.surface,
            palette.border,
            palette.text,
        ),
        (
            &mut visuals.widgets.inactive,
            palette.control,
            palette.border,
            palette.text,
        ),
        (
            &mut visuals.widgets.hovered,
            palette.hover,
            palette.muted,
            palette.text,
        ),
        (
            &mut visuals.widgets.active,
            palette.selection,
            palette.accent,
            palette.text,
        ),
        (
            &mut visuals.widgets.open,
            palette.control,
            palette.border,
            palette.text,
        ),
    ] {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = Stroke::new(1.0_f32, border);
        widget.fg_stroke = Stroke::new(1.0_f32, text);
        widget.corner_radius = egui::CornerRadius::same(7);
        widget.expansion = 0.0;
    }
    style.spacing.interact_size = egui::vec2(36.0, 30.0);
    style.spacing.item_spacing = egui::vec2(7.0, 6.0);
    style.spacing.scroll.bar_width = 5.0;
    style.interaction.resize_grab_radius_side = 6.0;
    style
}

fn set_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    if let Some(windows) = std::env::var_os("WINDIR") {
        let directory = PathBuf::from(windows).join("Fonts");
        for (name, file) in [("ui-latin", "segoeui.ttf"), ("ui-chinese", "msyh.ttc")] {
            if let Ok(data) = fs::read(directory.join(file)) {
                fonts
                    .font_data
                    .insert(name.into(), Arc::new(egui::FontData::from_owned(data)));
                if let Some(proportional) = fonts.families.get_mut(&egui::FontFamily::Proportional)
                {
                    proportional.insert(0, name.into());
                }
                if name == "ui-chinese"
                    && let Some(monospace) = fonts.families.get_mut(&egui::FontFamily::Monospace)
                {
                    monospace.push(name.into());
                }
            }
        }
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_each_theme_at_the_same_path() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("TakeSeam/theme");
        assert_eq!(read_preference(&path), None);
        write_preference(&path, Theme::Light).unwrap();
        assert_eq!(read_preference(&path), Some(Theme::Light));
        write_preference(&path, Theme::Dark).unwrap();
        assert_eq!(read_preference(&path), Some(Theme::Dark));
    }
}
