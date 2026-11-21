use std::path::Path;

use eframe::egui::{self, Event, Pos2, Rect, Theme, vec2};

use super::super::background::{
    BackdropPreset, BackgroundFrame, BackgroundMotion, BackgroundSettings, BackgroundSource,
};
use super::super::dynamic_recipe::DynamicRecipe;
use super::show;

#[test]
fn background_controls_work_in_narrow_light_and_wide_dark_panels() {
    for (width, theme) in [(160.0, Theme::Light), (430.0, Theme::Dark)] {
        let ctx = egui::Context::default();
        ctx.set_theme(theme);
        ctx.enable_accesskit();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(width, 840.0));
        let initial = BackgroundSettings::default();

        let mut expected = BackgroundSettings {
            source: BackgroundSource::Wallpaper(BackdropPreset::ALL[0]),
            ..initial.clone()
        };
        let picture = click_control(&ctx, screen, &initial, "图片");
        assert_eq!(picture, expected, "{theme:?} · {width}px");

        expected.motion = BackgroundMotion::Aurora;
        let aurora = click_control(&ctx, screen, &picture, "极光");
        assert_eq!(aurora, expected, "{theme:?} · {width}px");

        expected.frame = BackgroundFrame::Window;
        let window = click_control(&ctx, screen, &aurora, "窗口");
        assert_eq!(window, expected, "{theme:?} · {width}px");
    }
}

fn click_control(
    ctx: &egui::Context,
    screen: Rect,
    settings: &BackgroundSettings,
    label: &str,
) -> BackgroundSettings {
    click_panel(ctx, screen, settings, label)
        .1
        .unwrap_or_else(|| panic!("点击「{label}」后背景设置没有更新"))
}

#[test]
fn dynamic_preset_can_be_selected_and_copied_as_a_recipe() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(280.0, 850.0));
    let settings = click_control(&ctx, screen, &BackgroundSettings::default(), "深蓝网格");
    let expected = &DynamicRecipe::modern_presets()[0];
    assert_eq!(settings.motion, expected.motion);
    assert_eq!(settings.source, BackgroundSource::Dynamic(expected.clone()));
    let (output, _) = click_panel(&ctx, screen, &settings, "复制配方");
    let clipboard = output
        .platform_output
        .commands
        .into_iter()
        .find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        serde_json::from_str::<DynamicRecipe>(&clipboard).unwrap(),
        *expected
    );
}

fn click_panel(
    ctx: &egui::Context,
    screen: Rect,
    settings: &BackgroundSettings,
    label: &str,
) -> (egui::FullOutput, Option<BackgroundSettings>) {
    let frame = |events| {
        let mut changed = None;
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            },
            |ui| changed = show(ui, settings, Path::new(".")),
        );
        (output, changed)
    };
    let bounds = frame(Vec::new())
        .0
        .platform_output
        .accesskit_update
        .expect("面板没有无障碍节点")
        .nodes
        .into_iter()
        .filter_map(|(_, node)| {
            (node.label() == Some(label))
                .then(|| node.bounds())
                .flatten()
        })
        .max_by(|a, b| ((a.x1 - a.x0) * (a.y1 - a.y0)).total_cmp(&((b.x1 - b.x0) * (b.y1 - b.y0))))
        .unwrap_or_else(|| panic!("没有找到背景控件「{label}」"));
    let bounds = Rect::from_min_max(
        egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
        egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
    );
    assert!(
        bounds.is_positive() && screen.contains_rect(bounds),
        "背景控件「{label}」超出面板。控件 {bounds:?}，面板 {screen:?}"
    );
    let point = bounds.center();
    let button = |pressed| Event::PointerButton {
        pos: point,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(vec![Event::PointerMoved(point), button(true)]);
    frame(vec![button(false)])
}
