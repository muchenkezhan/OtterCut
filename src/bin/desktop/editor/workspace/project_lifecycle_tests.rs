use super::*;
use crate::editor::media::MediaInfo;
use takeseam::editor_timeline::TrackKind;

#[test]
fn creating_projects_starts_blank_and_saves_each_project_locally() {
    let ctx = egui::Context::default();
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("previous.mp4");
    std::fs::write(&source, b"old source").unwrap();
    let logo = ctx.load_texture(
        "project-lifecycle-logo",
        egui::ColorImage::filled([1, 1], egui::Color32::WHITE),
        egui::TextureOptions::NEAREST,
    );
    let mut workspace = Workspace::new(&ctx, logo, directory.path().to_owned());
    workspace.timeline = Timeline::default();
    let previous_clip = workspace
        .timeline
        .append(source.clone(), 3_000_000)
        .unwrap();
    workspace.library.add_fixture(
        source.clone(),
        MediaInfo {
            duration_us: 3_000_000,
            width: 640,
            height: 360,
            has_video: true,
            has_audio: false,
        },
    );
    workspace.library.selected = Some(0);
    workspace.selected_clip = Some(previous_clip);
    workspace.position = 2_000_000;
    workspace.markers.push(1_000_000);
    workspace.saved_markers.push(1_000_000);
    workspace.pending_additions.push_back(MaterialAddition {
        path: source.clone(),
        destination: TrackDestination::New {
            kind: TrackKind::Video,
            before: None,
        },
        at: Some(1_000_000),
    });

    workspace.new_project(&ctx);
    let first_project = workspace.project.clone().unwrap();

    assert!(first_project.is_file());
    assert!(workspace.timeline.is_empty());
    assert_eq!(workspace.selected_clip, None);
    assert_eq!(workspace.library.selected_path(), None);
    assert!(workspace.library.paths.contains(&source));
    assert_eq!(workspace.position, 0);
    assert_eq!(workspace.duration(), 0);
    assert!(workspace.markers.is_empty());
    assert!(workspace.saved_markers.is_empty());
    assert!(workspace.pending_additions.is_empty());
    assert!(!workspace.player.playing);
    assert!(
        project::load_with_cursor(&first_project)
            .unwrap()
            .0
            .is_empty()
    );

    workspace.timeline = Timeline::default();
    workspace.timeline.append(source, 3_000_000).unwrap();
    workspace.new_project(&ctx);
    let second_project = workspace.project.as_ref().unwrap();

    assert_ne!(first_project, *second_project);
    assert!(first_project.is_file());
    assert!(second_project.is_file());
    assert!(workspace.timeline.is_empty());
}

#[test]
fn startup_project_cards_wrap_and_open_the_clicked_project() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let directory = tempfile::tempdir().unwrap();
    let logo = ctx.load_texture(
        "startup-projects-logo",
        egui::ColorImage::filled([1, 1], egui::Color32::WHITE),
        egui::TextureOptions::NEAREST,
    );
    for index in 1..=6 {
        project::save(
            &directory.path().join(format!("Project {index}.takeproj")),
            &Timeline::default(),
            0,
            CanvasFormat::Original,
            &BackgroundSettings::default(),
        )
        .unwrap();
    }
    let mut workspace = Workspace::new(&ctx, logo, directory.path().to_owned());
    workspace.home = true;
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 700.0));
    let mut frame = |events| {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            },
            |ui| workspace.draw(ui),
        )
    };
    let tree = frame(Vec::new()).platform_output.accesskit_update.unwrap();
    let cards: Vec<_> = tree
        .nodes
        .iter()
        .filter_map(|(_, node)| {
            let label = node.label()?;
            (node.role() == egui::accesskit::Role::Button && label.starts_with("Project "))
                .then(|| (label, node.bounds().unwrap()))
        })
        .collect();
    assert_eq!(cards.len(), 6);
    let first_row_y = cards
        .iter()
        .map(|(_, bounds)| bounds.y0)
        .min_by(f64::total_cmp)
        .unwrap();
    let (label, bounds) = cards
        .iter()
        .find(|(_, bounds)| bounds.y0 > first_row_y)
        .expect("the project cards should wrap onto a second row");
    let point = egui::pos2(
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );
    assert!(screen.contains(point));
    let button = |pressed| egui::Event::PointerButton {
        pos: point,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(vec![egui::Event::PointerMoved(point), button(true)]);
    frame(vec![button(false)]);
    assert!(!workspace.home);
    assert_eq!(
        workspace.project,
        Some(directory.path().join(format!("{label}.takeproj")))
    );
}
