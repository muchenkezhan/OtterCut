use std::thread;
use std::time::Instant;

use eframe::egui::ImageData;
use takeseam::editor_timeline::{
    OverlayText, PRIMARY_AUDIO, PRIMARY_VIDEO, TextAppearance, TextPlacement, TextPosition,
    TextStyle,
};

use super::super::media::hidden_command;
use super::*;

#[test]
fn clip_volume_changes_the_preview_span_without_changing_its_source() {
    let mut timeline = Timeline::default();
    let path = PathBuf::from("voice.wav");
    let clip = timeline
        .insert_media_on_track(PRIMARY_AUDIO, path.clone(), 800_000, 0)
        .unwrap();
    let original = Span::at(&timeline, 200_000).unwrap().key;
    timeline.set_clip_volume(clip, -120).unwrap();
    let adjusted = Span::at(&timeline, 200_000).unwrap().key;
    assert!(original != adjusted);
    assert_eq!(original.start, adjusted.start);
    assert_eq!(original.end, adjusted.end);
    assert_eq!(original.audio[0].path, adjusted.audio[0].path);
    assert_eq!(adjusted.audio[0].volume_db_tenths, -120);

    let asset = Span::for_asset(
        &path,
        MediaInfo {
            duration_us: 800_000,
            width: 0,
            height: 0,
            has_video: false,
            has_audio: true,
        },
    );
    assert_eq!(asset.key.audio[0].volume_db_tenths, 0);
}

#[test]
fn volume_changes_are_classified_separately_from_media_changes() {
    let mut timeline = Timeline::default();
    let path = PathBuf::from("voice.wav");
    let clip = timeline
        .insert_media_on_track(PRIMARY_AUDIO, path, 800_000, 0)
        .unwrap();
    let original = Span::at(&timeline, 200_000).unwrap().key;
    timeline.set_clip_volume(clip, -120).unwrap();
    let adjusted = Span::at(&timeline, 200_000).unwrap().key;

    assert!(original.differs_only_in_audio_volume(&adjusted));
    assert!(adjusted.differs_only_in_audio_volume(&original));

    timeline
        .move_clip(clip, 100_000)
        .expect("moving a clip should remain a media change");
    let moved = Span::at(&timeline, 200_000).unwrap().key;
    assert!(!adjusted.differs_only_in_audio_volume(&moved));
}

#[test]
#[ignore = "requires local FFmpeg with libass and FFprobe"]
fn paused_preview_renders_captions_on_a_black_canvas_at_the_playhead() {
    let tools = MediaTools::discover().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut library = Library::new(fixture.path().to_owned(), None, &ctx);
    let mut timeline = Timeline::default();
    let (track, _) = timeline
        .insert_text(
            TrackKind::Caption,
            300_000,
            300_000,
            OverlayText {
                content: "中文 {\\b1} \\N".into(),
                style: TextStyle {
                    size: 100,
                    placement: TextPlacement::Bottom,
                    bold: false,
                    ..TextStyle::default()
                },
            },
        )
        .unwrap();
    let mut player = Player::default();
    for (show, requested, visible) in [
        (true, 300_000_u64, true),
        (true, 200_000, false),
        (false, 450_000, false),
    ] {
        timeline.set_track_enabled(track, show).unwrap();
        let mut position = requested;
        player.reset();
        assert_eq!(
            player.sync(
                &ctx,
                Some(&tools),
                Span::at(&timeline, position),
                &mut library,
                position
            ),
            None
        );
        let image = wait_for_preview(&ctx, &mut player, &mut position, timeline.duration());
        let ImageData::Color(image) = image;
        assert_eq!(image.size, [960, 540]);
        assert!(
            image.pixels[0].to_array()[..3]
                .iter()
                .all(|channel| *channel < 4)
        );
        let white = image.pixels[960 * 405..]
            .iter()
            .filter(|pixel| pixel.r() > 180)
            .count();
        assert_eq!(
            white > 500,
            visible,
            "{position} microseconds, track enabled={show}"
        );
    }
}

#[test]
#[ignore = "requires local FFmpeg with libass and FFprobe"]
fn preview_replaces_only_the_dragged_caption_and_stamps_the_displayed_frame() {
    let tools = MediaTools::discover().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut library = Library::new(fixture.path().to_owned(), None, &ctx);
    let mut timeline = Timeline::default();
    let (_, moved) = timeline
        .insert_text(
            TrackKind::Caption,
            0,
            600_000,
            OverlayText {
                content: "金色".into(),
                style: TextStyle {
                    size: 100,
                    position: Some(TextPosition { x: 2000, y: 4200 }),
                    appearance: TextAppearance::Gold,
                    ..TextStyle::default()
                },
            },
        )
        .unwrap();
    timeline
        .insert_text(
            TrackKind::Caption,
            0,
            600_000,
            OverlayText {
                content: "底部".into(),
                style: TextStyle {
                    size: 100,
                    ..TextStyle::default()
                },
            },
        )
        .unwrap();
    let mut player = Player::default();
    let mut position = 300_000;
    for (exclude, previous_excludes) in [(None, false), (Some(moved), false), (None, true)] {
        let span = Span::at(&timeline, position).map(|span| span.with_excluded_overlay(exclude));
        assert_eq!(
            player.sync(&ctx, Some(&tools), span, &mut library, position),
            None
        );
        if exclude.is_some() || previous_excludes {
            assert!(
                player.texture().is_some(),
                "the old frame should still be visible"
            );
        }
        assert_eq!(player.texture_excludes(moved), previous_excludes);
        let ImageData::Color(image) =
            wait_for_preview(&ctx, &mut player, &mut position, timeline.duration());
        assert_eq!(image.size, [960, 540]);
        assert_eq!(player.texture_excludes(moved), exclude.is_some());
        let gold_pixels = image
            .pixels
            .iter()
            .enumerate()
            .filter(|(_, pixel)| {
                pixel.r() > 190 && pixel.g() > 150 && pixel.g().saturating_sub(pixel.b()) > 45
            })
            .map(|(index, _)| (index % 960, index / 960))
            .collect::<Vec<_>>();
        assert_eq!(gold_pixels.len() > 500, exclude.is_none());
        assert!(
            gold_pixels
                .iter()
                .all(|(x, y)| (120..263).contains(x) && (191..263).contains(y))
        );
        assert!(
            image.pixels[960 * 405..]
                .iter()
                .filter(|pixel| pixel.r() > 180)
                .count()
                > 500
        );
    }
}

#[test]
#[ignore = "requires local FFmpeg with libass and FFprobe"]
fn video_frame_survives_quality_and_subtitle_preview_changes() {
    let tools = MediaTools::discover().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut library = Library::new(fixture.path().to_owned(), None, &ctx);
    let mut timeline = Timeline::default();
    let path = fixture.path().join("blue.mp4");
    let info = color_fixture(&tools, &path, "blue", "640x360");
    timeline
        .insert_media_on_track(PRIMARY_VIDEO, path.clone(), info.duration_us, 0)
        .unwrap();
    library.add_fixture(path, info);
    let mut caption = OverlayText {
        content: "画面上方的文字".into(),
        style: TextStyle::default(),
    };
    let (_, moved) = timeline
        .insert_text(TrackKind::Caption, 0, 500_000, caption.clone())
        .unwrap();
    let mut player = Player::default();
    let mut position = 100_000;
    let sync =
        |player: &mut Player, timeline: &Timeline, library: &mut Library, position, exclude| {
            let span = Span::at(timeline, position).map(|span| span.with_excluded_overlay(exclude));
            assert_eq!(
                player.sync(&ctx, Some(&tools), span, library, position),
                None
            );
        };
    sync(&mut player, &timeline, &mut library, position, None);
    let ImageData::Color(original) =
        wait_for_preview(&ctx, &mut player, &mut position, timeline.duration());
    assert_eq!(original.size, [320, 180]);
    assert_eq!(player.texture_composition_size(), Some((640, 360)));
    let texture = player.texture().unwrap().id();

    player.set_quality(PreviewQuality::Sixteenth);
    sync(&mut player, &timeline, &mut library, position, Some(moved));
    assert_eq!(player.texture().unwrap().id(), texture);
    assert_eq!(player.texture().unwrap().size(), [320, 180]);
    assert!(!player.texture_excludes(moved));
    let ImageData::Color(replacement) =
        wait_for_preview(&ctx, &mut player, &mut position, timeline.duration());
    assert_eq!(replacement.size, [40, 22]);
    assert!(replacement.pixels[0].b() > 200);
    assert!(replacement.pixels.last().unwrap().b() > 200);
    assert_eq!(player.texture_composition_size(), Some((640, 360)));
    assert!(player.texture_excludes(moved));

    caption.style.position = Some(TextPosition { x: 3000, y: 4000 });
    timeline.update_text(moved, caption).unwrap();
    sync(&mut player, &timeline, &mut library, position, Some(moved));
    assert_eq!(player.texture().unwrap().id(), texture);
    assert!(player.texture_excludes(moved));

    player.toggle(&mut position, timeline.duration());
    sync(&mut player, &timeline, &mut library, position, None);
    wait_for_preview(&ctx, &mut player, &mut position, timeline.duration());
    assert!(!player.texture_excludes(moved));
    position += 40_000;
    player.playing = false;
    sync(&mut player, &timeline, &mut library, position, Some(moved));
    assert_eq!(player.texture().unwrap().id(), texture);
    assert!(!player.texture_excludes(moved));
    let ImageData::Color(paused) =
        wait_for_preview(&ctx, &mut player, &mut position, timeline.duration());
    assert!(paused.pixels[0].b() > 200);
    assert!(player.texture_excludes(moved));
}

#[test]
#[ignore = "requires local FFmpeg and FFprobe"]
fn changing_canvas_ratio_previews_portrait_with_letterboxed_video() {
    use crate::editor::canvas_format::AspectRatio;

    let tools = MediaTools::discover().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let path = directory.path().join("landscape.mp4");
    let info = color_fixture(&tools, &path, "blue", "640x360");
    let mut library = Library::new(directory.path().to_owned(), None, &ctx);
    library.add_fixture(path.clone(), info);
    let mut timeline = Timeline::default();
    timeline.append(path, info.duration_us).unwrap();
    let mut player = Player::default();
    let mut position = 100_000;
    let sync = |player: &mut Player, library: &mut Library, format, position| {
        let span = Span::at(&timeline, position).map(|span| span.with_canvas_format(format));
        assert_eq!(
            player.sync(&ctx, Some(&tools), span, library, position),
            None
        );
    };
    sync(&mut player, &mut library, CanvasFormat::Original, position);
    let ImageData::Color(original) =
        wait_for_preview(&ctx, &mut player, &mut position, timeline.duration());
    assert_eq!(original.size, [320, 180]);

    sync(
        &mut player,
        &mut library,
        CanvasFormat::Preset(AspectRatio::Portrait),
        position,
    );
    assert_eq!(player.texture_composition_size(), Some((640, 360)));
    let ImageData::Color(portrait) =
        wait_for_preview(&ctx, &mut player, &mut position, timeline.duration());
    assert_eq!(player.texture_composition_size(), Some((1080, 1920)));
    assert_eq!(portrait.size, [540, 960]);
    assert!(portrait.pixels[480 * 540 + 270].b() > 200);
    assert!(portrait.pixels[0].b() < 16);
}

#[test]
#[ignore = "requires local FFmpeg and FFprobe"]
fn video_preview_composites_behind_the_inset_source() {
    use crate::editor::background::{BackgroundSettings, BackgroundShadow, BackgroundSource};

    let tools = MediaTools::discover().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.mp4");
    let info = color_fixture(&tools, &path, "blue", "640x360");
    let ctx = egui::Context::default();
    let mut library = Library::new(directory.path().to_owned(), None, &ctx);
    library.add_fixture(path.clone(), info);
    let mut timeline = Timeline::default();
    timeline.append(path, info.duration_us).unwrap();
    let settings = BackgroundSettings {
        source: BackgroundSource::Solid([245, 190, 25]),
        inset: 20,
        radius: 0,
        shadow: BackgroundShadow::None,
        ..BackgroundSettings::default()
    };
    let mut player = Player::default();
    let mut position = 100_000;
    let span = Span::at(&timeline, position).map(|span| span.with_background(&settings));
    assert_eq!(
        player.sync(&ctx, Some(&tools), span, &mut library, position),
        None
    );
    let ImageData::Color(frame) =
        wait_for_preview(&ctx, &mut player, &mut position, timeline.duration());
    assert_eq!(frame.size, [320, 180]);
    assert!(frame.pixels[0].r() > 200 && frame.pixels[0].g() > 150);
    assert!(frame.pixels[frame.size[0] * 90 + 160].b() > 200);
}

#[test]
#[ignore = "requires local FFmpeg"]
fn dynamic_recipe_previews_different_frames_with_no_media() {
    use crate::editor::background::{BackgroundSettings, BackgroundSource};
    use crate::editor::dynamic_recipe::DynamicRecipe;

    let tools = MediaTools::discover().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut library = Library::new(directory.path().to_owned(), None, &ctx);
    for recipe in [
        DynamicRecipe::presets()[0].clone(),
        DynamicRecipe::modern_presets()[4].clone(),
    ] {
        let background = BackgroundSettings {
            motion: recipe.motion,
            source: BackgroundSource::Dynamic(recipe),
            ..Default::default()
        };
        let mut player = Player::default();
        let mut frames = Vec::new();
        for mut position in [0, 2_000_000, 0] {
            let span = Span::for_background(8_000_000)
                .with_canvas_format(CanvasFormat::Custom {
                    width: 320,
                    height: 180,
                })
                .with_background(&background);
            assert_eq!(
                player.sync(&ctx, Some(&tools), Some(span), &mut library, position),
                None
            );
            let ImageData::Color(frame) =
                wait_for_preview(&ctx, &mut player, &mut position, 8_000_000);
            assert_eq!(frame.size, [160, 90]);
            frames.push(frame);
        }
        assert_ne!(frames[0].pixels, frames[1].pixels);
        assert_eq!(frames[0].pixels, frames[2].pixels);
    }
}

fn wait_for_preview(
    ctx: &egui::Context,
    player: &mut Player,
    position: &mut u64,
    duration: u64,
) -> ImageData {
    let started = Instant::now();
    loop {
        let output = ctx.run_ui(egui::RawInput::default(), |_| {
            player.advance(ctx, position, duration);
        });
        if let Some(texture) = player.texture()
            && let Some((_, image)) = output
                .textures_delta
                .set
                .into_iter()
                .find(|(id, _)| *id == texture.id())
        {
            return image.image;
        }
        assert!(player.error().is_none(), "{:?}", player.error());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "preview timed out"
        );
        thread::sleep(Duration::from_millis(8));
    }
}

fn color_fixture(tools: &MediaTools, path: &Path, color: &str, size: &str) -> MediaInfo {
    let generated = hidden_command(&tools.ffmpeg)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("color={color}:s={size}:r=30"),
            "-t",
            "0.5",
            "-c:v",
            "mpeg4",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    tools.probe(path).unwrap()
}

#[test]
#[ignore = "requires local FFmpeg and FFprobe"]
fn top_video_preview_uses_the_base_canvas_and_honors_track_output() {
    let tools = MediaTools::discover().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut library = Library::new(fixture.path().to_owned(), None, &ctx);
    let mut timeline = Timeline::default();
    let top = timeline.tracks().first().unwrap().id;
    for (track, name, color, size) in [
        (PRIMARY_VIDEO, "base.mp4", "red", "160x90"),
        (top, "overlay.mp4", "lime", "90x160"),
    ] {
        let path = fixture.path().join(name);
        let info = color_fixture(&tools, &path, color, size);
        timeline
            .insert_media_on_track(track, path.clone(), info.duration_us, 0)
            .unwrap();
        library.add_fixture(path, info);
    }

    let mut player = Player::default();
    player.set_quality(PreviewQuality::Full);
    let mut position = 100_000;
    for show_top in [true, false] {
        timeline.set_track_enabled(top, show_top).unwrap();
        let span = Span::at(&timeline, position);
        assert_eq!(
            player.sync(&ctx, Some(&tools), span, &mut library, position),
            None
        );
        let started = Instant::now();
        let frame = loop {
            let output = ctx.run_ui(egui::RawInput::default(), |_| {
                player.advance(&ctx, &mut position, timeline.duration());
            });
            if let Some(texture) = player.texture()
                && let Some((_, frame)) = output
                    .textures_delta
                    .set
                    .into_iter()
                    .find(|(id, _)| *id == texture.id())
            {
                break frame.image;
            }
            assert!(player.error().is_none(), "{:?}", player.error());
            assert!(
                started.elapsed() < Duration::from_secs(4),
                "preview timed out"
            );
            thread::sleep(Duration::from_millis(8));
        };
        let ImageData::Color(frame) = frame;
        assert_eq!(frame.size, [160, 90]);
        let center = frame.pixels[45 * 160 + 80];
        let corner = frame.pixels[0];
        if show_top {
            assert!(center.g() > 200 && center.r() < 30);
            assert!(corner.r() < 20 && corner.g() < 20);
        } else {
            assert!(center.r() > 200 && center.g() < 30);
        }
    }
}
