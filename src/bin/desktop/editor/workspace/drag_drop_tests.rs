use eframe::egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};

use super::*;
use crate::editor::media::MediaInfo;
use takeseam::editor_timeline::{PRIMARY_AUDIO, PRIMARY_VIDEO};

#[test]
fn removing_a_library_card_keeps_the_source_used_by_the_timeline() {
    let ctx = egui::Context::default();
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("clip.mp4");
    std::fs::write(&source, b"sample").unwrap();
    let logo = ctx.load_texture(
        "test-logo",
        egui::ColorImage::filled([1, 1], egui::Color32::WHITE),
        egui::TextureOptions::NEAREST,
    );
    let mut workspace = Workspace::new(&ctx, logo, directory.path().to_owned());
    workspace.timeline = Timeline::default();
    workspace.library.insert(source.clone(), true);
    let clip = workspace
        .timeline
        .append(source.clone(), 2_000_000)
        .unwrap();
    let revision = workspace.timeline.revision();

    workspace.material_action(
        &ctx,
        MaterialAction::Remove(source.clone()),
        MaterialScope::All,
    );

    assert!(workspace.library.paths.is_empty());
    assert_eq!(workspace.timeline.revision(), revision);
    assert_eq!(workspace.timeline.clip(clip).unwrap().path, source);
    assert_eq!(std::fs::read(source).unwrap().as_slice(), b"sample");
}

#[test]
fn adding_from_the_audio_panel_prefers_an_audio_track_over_the_active_video_track() {
    let timeline = Timeline::default();
    let target = Workspace::media_track(&timeline, PRIMARY_VIDEO, None, MaterialScope::Audio);
    assert_eq!(target, Some(PRIMARY_AUDIO));
}

#[derive(Clone, Copy)]
enum DropLocation {
    Track,
    EdgeWithOverflow,
    BetweenTracks,
    AudioAtBoundary,
}

#[test]
fn dragging_asset_cards_inserts_at_the_landing_point_and_can_undo() {
    verify_material_drop(DropLocation::Track);
}

#[test]
fn overflowing_timeline_keeps_both_track_creation_drop_zones_reachable() {
    verify_material_drop(DropLocation::EdgeWithOverflow);
}

#[test]
fn dragging_a_material_between_video_tracks_creates_a_track_in_that_gap() {
    verify_material_drop(DropLocation::BetweenTracks);
}

#[test]
fn audio_probed_after_the_drop_still_fits_at_the_video_audio_boundary() {
    verify_material_drop(DropLocation::AudioAtBoundary);
}

fn verify_material_drop(location: DropLocation) {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let directory = tempfile::tempdir().unwrap();
    let logo = ctx.load_texture(
        "test-logo",
        egui::ColorImage::filled([1, 1], egui::Color32::WHITE),
        egui::TextureOptions::NEAREST,
    );
    let mut workspace = Workspace::new(&ctx, logo, directory.path().to_owned());
    workspace.timeline = Timeline::default();
    // Real card/track widgets with cached metadata, independent of the FFmpeg installation.
    workspace.tools = Some(MediaTools {
        ffmpeg: directory.path().join("ffmpeg.exe"),
        ffprobe: directory.path().join("ffprobe.exe"),
    });
    workspace.thumbnails = Thumbnails::new(None, &ctx);
    // Enough cards to make dragging compete with scrolling in the source panel.
    for (name, duration_us) in [
        ("first.mp4", 2_000_000),
        ("second.mp4", 3_000_000),
        ("third.mp4", 2_000_000),
        ("fourth.mp4", 2_000_000),
        ("fifth.mp4", 2_000_000),
        ("sixth.mp4", 2_000_000),
    ] {
        workspace.library.add_fixture(
            directory.path().join(name),
            MediaInfo {
                duration_us,
                width: 320,
                height: 180,
                has_video: true,
                has_audio: name == "second.mp4",
            },
        );
    }
    if matches!(location, DropLocation::EdgeWithOverflow) {
        for _ in 0..6 {
            for (kind, name) in [
                (TrackKind::Video, "first.mp4"),
                (TrackKind::Audio, "second.mp4"),
            ] {
                let track = workspace.timeline.add_track(kind).unwrap();
                workspace
                    .timeline
                    .insert_media_on_track(track, directory.path().join(name), 2_000_000, 0)
                    .unwrap();
            }
        }
    }
    if matches!(location, DropLocation::BetweenTracks) {
        let upper = workspace.timeline.tracks()[0].id;
        for track in [upper, PRIMARY_VIDEO] {
            workspace
                .timeline
                .insert_media_on_track(track, directory.path().join("second.mp4"), 3_000_000, 0)
                .unwrap();
        }
    }
    let original = workspace.timeline.tracks().to_vec();

    if matches!(location, DropLocation::AudioAtBoundary) {
        let path = directory.path().join("sound.wav");
        workspace.queue_material(
            &ctx,
            path.clone(),
            TrackDestination::New {
                kind: TrackKind::Video,
                before: Some(PRIMARY_AUDIO),
            },
            Some(125_000),
        );
        workspace.library.add_fixture(
            path,
            MediaInfo {
                duration_us: 2_000_000,
                width: 0,
                height: 0,
                has_video: false,
                has_audio: true,
            },
        );
        workspace.add_materials_when_ready();
        let tracks = workspace.timeline.tracks();
        assert_eq!(tracks.len(), original.len() + 1);
        assert_eq!(tracks[2].kind, TrackKind::Audio);
        assert_eq!(tracks[2].clips[0].start, 125_000);
        assert_eq!(tracks[3].id, PRIMARY_AUDIO);
        assert!(workspace.timeline.undo());
        assert_eq!(workspace.timeline.tracks(), original);
        return;
    }

    let mut frame = |events| {
        workspace.add_materials_when_ready();
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1240.0, 760.0))),
            events,
            ..Default::default()
        };
        eframe::App::raw_input_hook(&mut workspace, &ctx, &mut input);
        ctx.run_ui(input, |ui| workspace.draw(ui))
    };
    let first_frame = frame(Vec::new());
    let accessibility = first_frame.platform_output.accesskit_update.unwrap();
    let find_bounds = |accessibility: &egui::accesskit::TreeUpdate, label: &str| {
        accessibility
            .nodes
            .iter()
            .filter_map(|(_, node)| {
                (node.label() == Some(label)
                    || (!label.ends_with(".mp4") && node.value() == Some(label)))
                .then(|| node.bounds())
                .flatten()
            })
            .min_by(|a, b| a.y0.total_cmp(&b.y0))
    };
    let bounds = |label| {
        find_bounds(&accessibility, label).unwrap_or_else(|| panic!("widget not found: {label}"))
    };
    let center = |label| {
        let bounds = bounds(label);
        pos2(
            ((bounds.x0 + bounds.x1) * 0.5) as f32,
            ((bounds.y0 + bounds.y1) * 0.5) as f32,
        )
    };
    let first = center("first.mp4");
    let second = center("second.mp4");
    let toolbar = bounds("添加到时间线");
    let drag = |frame: &mut dyn FnMut(Vec<Event>) -> egui::FullOutput,
                from: Pos2,
                to: Pos2,
                batched: bool| {
        let button = |pos, pressed| Event::PointerButton {
            pos,
            pressed,
            button: PointerButton::Primary,
            modifiers: Modifiers::NONE,
        };
        frame(vec![Event::PointerMoved(from)]);
        if batched {
            frame(vec![
                button(from, true),
                Event::PointerMoved(to),
                button(to, false),
            ]);
        } else {
            frame(vec![button(from, true)]);
            frame(vec![Event::PointerMoved(from + vec2(0.0, 12.0))]);
            frame(vec![Event::PointerMoved(to)]);
            frame(vec![button(to, false)]);
        }
        frame(Vec::new());
    };

    if matches!(location, DropLocation::EdgeWithOverflow) {
        let top = center("拖入素材建立视频轨");
        assert!(find_bounds(&accessibility, "拖入素材建立音频轨").is_none());
        drag(&mut frame, first, pos2(345.0, top.y), true);

        let scroll_to_bottom = |frame: &mut dyn FnMut(Vec<Event>) -> egui::FullOutput| {
            let update = frame(vec![
                Event::PointerMoved(top + vec2(100.0, 60.0)),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, -5000.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: Modifiers::NONE,
                },
            ])
            .platform_output
            .accesskit_update
            .unwrap();
            assert!(find_bounds(&update, "拖入素材建立视频轨").is_none());
            let band = find_bounds(&update, "拖入素材建立音频轨").expect("lower band is visible");
            pos2(259.0, ((band.y0 + band.y1) * 0.5) as f32)
        };
        let bottom = scroll_to_bottom(&mut frame);
        drag(&mut frame, second, bottom, false);
        let bottom = scroll_to_bottom(&mut frame);
        drag(&mut frame, first, bottom, true);

        let tracks = workspace.timeline.tracks();
        assert_eq!(tracks.len(), original.len() + 3);
        let video = tracks
            .iter()
            .find(|track| {
                track.kind == TrackKind::Video
                    && track
                        .clips
                        .iter()
                        .any(|clip| filename(&clip.path) == "first.mp4")
            })
            .unwrap();
        let audio = tracks
            .iter()
            .find(|track| {
                track.kind == TrackKind::Audio
                    && track
                        .clips
                        .iter()
                        .any(|clip| filename(&clip.path) == "second.mp4")
            })
            .unwrap();
        assert_eq!(video.kind, TrackKind::Video);
        assert_eq!(audio.kind, TrackKind::Audio);
        assert_eq!(filename(&video.clips[0].path), "first.mp4");
        assert_eq!(filename(&audio.clips[0].path), "second.mp4");
        assert!(video.clips[0].start <= workspace.timeline.duration());
        assert!(audio.clips[0].start <= workspace.timeline.duration());
        for original_track in &original {
            assert!(tracks.iter().any(|track| track.id == original_track.id));
        }
        for _ in 0..3 {
            assert!(workspace.timeline.undo());
        }
        assert_eq!(workspace.timeline.tracks(), original);
        return;
    }

    if matches!(location, DropLocation::BetweenTracks) {
        let above = bounds("选中第2条视频轨");
        let below = bounds("选中第1条视频轨");
        let gap_y = ((above.y1 + below.y0) * 0.5) as f32;
        drag(&mut frame, first, pos2(345.0, gap_y), true);

        let tracks = workspace.timeline.tracks();
        assert_eq!(tracks.len(), original.len() + 1);
        assert_eq!(tracks[0], original[0]);
        assert_eq!(&tracks[2..], &original[1..]);
        assert_eq!(tracks[1].kind, TrackKind::Video);
        assert_eq!(filename(&tracks[1].clips[0].path), "first.mp4");
        assert!(tracks[1].clips[0].start > 0);
        assert!(workspace.timeline.undo());
        assert_eq!(workspace.timeline.tracks(), original);
        return;
    }

    let track_y = center("选中第1条视频轨").y;
    let audio_y = center("选中第1条音频轨").y;
    // Outside the timeline cancels. The track label and blank canvas both accept a
    // drop, and dropping onto an occupied clip inserts without splitting it.
    drag(&mut frame, first, pos2(510.0, 200.0), false);
    drag(&mut frame, first, pos2(36.0, track_y), true);
    drag(
        &mut frame,
        second,
        pos2(toolbar.x0 as f32 + 24.0, track_y),
        false,
    );
    drag(&mut frame, second, pos2(565.0, track_y), true);
    drag(&mut frame, second, pos2(36.0, audio_y), false);

    let clips: Vec<_> = workspace
        .timeline
        .tracks()
        .iter()
        .flat_map(|track| &track.clips)
        .collect();
    assert_eq!(clips.len(), 4);
    let first_clip = clips
        .iter()
        .find(|clip| filename(&clip.path) == "first.mp4")
        .unwrap();
    assert_eq!(first_clip.duration(), 2_000_000);
    let audio = workspace
        .timeline
        .tracks()
        .iter()
        .find(|track| track.kind == TrackKind::Audio)
        .map(|track| &track.clips)
        .unwrap();
    assert_eq!(audio.len(), 1);
    assert_eq!(filename(&audio[0].path), "second.mp4");
    assert!(audio[0].start <= workspace.timeline.duration());
    for _ in 0..4 {
        assert!(workspace.timeline.undo());
    }
    assert_eq!(workspace.timeline.all_clips().count(), 0);
    assert!(!workspace.timeline.can_undo());
}
