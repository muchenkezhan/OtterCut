use std::path::PathBuf;

use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use takeseam::editor_timeline::{OverlayText, PRIMARY_VIDEO, TextStyle, Timeline, TrackKind};

use super::canvas::{RULER_HEIGHT, TRACK_HEIGHT};
use super::edge_drop::{BAND_HEIGHT, TrackGap};
use super::track_rows::Rows;
use super::{Mode, TRACK_HEADER, TimelineInput, TimelineView};

fn clipboard_frame(
    ctx: &egui::Context,
    view: &mut TimelineView,
    timeline: &mut Timeline,
    playhead: &mut u64,
    selected: &mut Option<u64>,
    event: Event,
) -> TimelineInput {
    let mut result = None;
    let _ = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 350.0))),
            events: vec![event],
            ..Default::default()
        },
        |ui| result = Some(view.draw(ui, timeline, playhead, selected, false)),
    );
    result.unwrap()
}

#[test]
fn native_copy_and_paste_redirect_from_a_video_track_without_changing_existing_clips() {
    let ctx = egui::Context::default();
    let mut view = TimelineView::default();
    let mut timeline = Timeline::default();
    let video = timeline.append("screen.mp4".into(), 8_000_000).unwrap();
    let text = OverlayText {
        content: "新字幕".into(),
        style: TextStyle::default(),
    };
    let (caption_track, caption) = timeline
        .insert_text(TrackKind::Caption, 0, 3_000_000, text.clone())
        .unwrap();
    let original = timeline.tracks().to_vec();
    let mut selected = Some(caption);
    let mut playhead = 0;
    let result = clipboard_frame(
        &ctx,
        &mut view,
        &mut timeline,
        &mut playhead,
        &mut selected,
        Event::Copy,
    );
    assert_eq!(result.clipboard_feedback, Some("已复制片段"));
    assert_eq!(timeline.tracks(), original);

    selected = Some(video);
    playhead = 5_000_000;
    let result = clipboard_frame(
        &ctx,
        &mut view,
        &mut timeline,
        &mut playhead,
        &mut selected,
        Event::Paste(String::new()),
    );
    assert!(result.error.is_none());
    assert_eq!(result.clipboard_feedback, Some("已粘贴片段"));
    let pasted = timeline.clip(selected.unwrap()).unwrap();
    assert_eq!(pasted.start, 5_000_000);
    assert_eq!(pasted.text.as_ref(), Some(&text));
    assert_eq!(
        timeline.track_for_clip(pasted.id).unwrap().id,
        caption_track
    );

    selected = Some(video);
    let result = clipboard_frame(
        &ctx,
        &mut view,
        &mut timeline,
        &mut playhead,
        &mut selected,
        Event::Paste(String::new()),
    );
    assert!(result.error.is_none());
    let second_track = timeline.track_for_clip(selected.unwrap()).unwrap();
    assert_eq!(second_track.kind, TrackKind::Caption);
    assert_ne!(second_track.id, caption_track);
    assert_eq!(timeline.track(caption_track).unwrap().clips.len(), 2);
    assert!(timeline.undo());
    assert!(timeline.undo());
    assert_eq!(timeline.tracks(), original);
}

#[test]
fn native_cut_and_paste_move_audio_and_each_edit_can_be_undone() {
    use takeseam::editor_timeline::PRIMARY_AUDIO;

    let ctx = egui::Context::default();
    let mut view = TimelineView::default();
    let mut timeline = Timeline::default();
    let video = timeline.append("screen.mp4".into(), 8_000_000).unwrap();
    let audio = timeline
        .insert_media_on_track(PRIMARY_AUDIO, "voice.wav".into(), 2_000_000, 1_000_000)
        .unwrap();
    let original = timeline.tracks().to_vec();
    let mut selected = Some(audio);
    let mut playhead = 1_000_000;
    let result = clipboard_frame(
        &ctx,
        &mut view,
        &mut timeline,
        &mut playhead,
        &mut selected,
        Event::Cut,
    );
    assert_eq!(result.clipboard_feedback, Some("已剪切片段"));
    assert!(selected.is_none());
    assert!(timeline.clip(audio).is_none());

    selected = Some(video);
    playhead = 4_000_000;
    let result = clipboard_frame(
        &ctx,
        &mut view,
        &mut timeline,
        &mut playhead,
        &mut selected,
        Event::Paste(String::new()),
    );
    assert!(result.error.is_none());
    let pasted = timeline.clip(selected.unwrap()).unwrap();
    assert_eq!(pasted.start, 4_000_000);
    assert_eq!(pasted.duration(), 2_000_000);
    assert_eq!(
        timeline.track_for_clip(pasted.id).unwrap().id,
        PRIMARY_AUDIO
    );
    assert!(timeline.undo());
    assert!(timeline.undo());
    assert_eq!(timeline.tracks(), original);
}

#[test]
fn a_focused_text_field_keeps_native_text_paste_off_the_timeline() {
    let ctx = egui::Context::default();
    let mut view = TimelineView::default();
    let mut timeline = Timeline::default();
    let mut selected = Some(timeline.append("screen.mp4".into(), 8_000_000).unwrap());
    let mut playhead = 2_000_000;
    clipboard_frame(
        &ctx,
        &mut view,
        &mut timeline,
        &mut playhead,
        &mut selected,
        Event::Copy,
    );
    let editor_id = egui::Id::new("clipboard-text-field");
    let mut text = String::new();
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        ui.add(egui::TextEdit::singleline(&mut text).id(editor_id))
            .request_focus();
    });
    assert!(ctx.text_edit_focused());
    let revision = timeline.revision();
    let _ = ctx.run_ui(
        egui::RawInput {
            events: vec![Event::Paste("字幕文本".into())],
            ..Default::default()
        },
        |ui| {
            let input = view.draw(ui, &mut timeline, &mut playhead, &mut selected, false);
            assert!(input.clipboard_feedback.is_none());
            ui.add(egui::TextEdit::singleline(&mut text).id(editor_id));
        },
    );
    assert_eq!(text, "字幕文本");
    assert_eq!(timeline.revision(), revision);
}

#[test]
fn right_click_splits_the_hit_clip_without_editing_when_the_menu_opens() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut timeline = Timeline::default();
    let first = timeline.append("selected.mp4".into(), 2_000_000).unwrap();
    let hit = timeline.append("hit.mp4".into(), 2_000_000).unwrap();
    let mut playhead = 300_000;
    let mut selected = Some(first);
    let mut view = TimelineView {
        mode: Mode::Blade,
        ..TimelineView::default()
    };
    let mut frame = |events| {
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 470.0))),
                events,
                ..Default::default()
            },
            |ui| {
                assert!(
                    view.draw(ui, &mut timeline, &mut playhead, &mut selected, true)
                        .error
                        .is_none()
                );
            },
        );
        (output, playhead, selected, timeline.clips().len())
    };
    let center = |output: egui::FullOutput, label| {
        let bounds = output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .into_iter()
            .filter_map(|(_, node)| {
                (node.label() == Some(label))
                    .then(|| node.bounds())
                    .flatten()
            })
            .reduce(|first, second| first.union(second))
            .unwrap();
        pos2(
            ((bounds.x0 + bounds.x1) * 0.5) as f32,
            ((bounds.y0 + bounds.y1) * 0.5) as f32,
        )
    };
    let button = |pos, button, pressed| Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let point = center(frame(Vec::new()).0, "hit.mp4");
    frame(vec![
        Event::PointerMoved(point),
        button(point, PointerButton::Secondary, true),
    ]);
    let (_, actual_playhead, actual_selected, count) =
        frame(vec![button(point, PointerButton::Secondary, false)]);
    assert_eq!(actual_playhead, 300_000);
    assert_eq!(actual_selected, Some(first));
    assert_eq!(count, 2);

    let item = center(frame(Vec::new()).0, "在此分割");
    frame(vec![
        Event::PointerMoved(item),
        button(item, PointerButton::Primary, true),
    ]);
    frame(vec![button(item, PointerButton::Primary, false)]);

    let clips = timeline.clips();
    assert_eq!(clips.len(), 3);
    assert_eq!(clips[0].id, first);
    assert_eq!(clips[0].end(), 2_000_000);
    assert_eq!(clips[1].id, hit);
    assert_eq!(clips[1].end(), 3_000_000);
    assert_eq!(clips[2].start, 3_000_000);
    assert_eq!(playhead, 3_000_000);
    assert!(timeline.undo());
    assert_eq!(timeline.clips().len(), 2);
}

#[test]
fn track_header_menu_deletes_only_the_clicked_track_and_undo_restores_it() {
    use takeseam::editor_timeline::PRIMARY_AUDIO;

    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut timeline = Timeline::default();
    let video = timeline.append("video.mp4".into(), 8_000_000).unwrap();
    timeline
        .insert_media_on_track(PRIMARY_AUDIO, "voice.wav".into(), 3_000_000, 0)
        .unwrap();
    let original = timeline.tracks().to_vec();
    let mut view = TimelineView::default();
    let mut playhead = 0;
    let mut selected = Some(video);
    let mut frame = |events| {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 490.0))),
                events,
                ..Default::default()
            },
            |ui| {
                assert!(
                    view.draw(ui, &mut timeline, &mut playhead, &mut selected, false)
                        .error
                        .is_none()
                )
            },
        )
    };
    let center = |output: egui::FullOutput, label| {
        let bounds = output
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
        pos2(
            ((bounds.x0 + bounds.x1) * 0.5) as f32,
            ((bounds.y0 + bounds.y1) * 0.5) as f32,
        )
    };
    let header = center(frame(Vec::new()), "选中第1条音频轨");
    let button = |pos, button, pressed| Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: Modifiers::NONE,
    };
    frame(vec![
        Event::PointerMoved(header),
        button(header, PointerButton::Secondary, true),
    ]);
    frame(vec![button(header, PointerButton::Secondary, false)]);
    let delete = center(frame(Vec::new()), "删除轨道");
    frame(vec![
        Event::PointerMoved(delete),
        button(delete, PointerButton::Primary, true),
    ]);
    frame(vec![button(delete, PointerButton::Primary, false)]);
    assert!(timeline.track(PRIMARY_AUDIO).is_none());
    assert_eq!(selected, Some(video));
    assert!(timeline.undo());
    assert_eq!(timeline.tracks(), original);
}

#[test]
fn blade_button_changes_the_cursor_cuts_at_the_pointer_and_can_toggle_back() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut timeline = Timeline::default();
    timeline.append("screen.mp4".into(), 2_000_000).unwrap();
    let original = timeline.clips().to_vec();
    let mut view = TimelineView::default();
    let mut playhead = 0;
    let mut selected = None;
    let mut frame = |events| {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 330.0))),
                events,
                ..Default::default()
            },
            |ui| {
                assert!(
                    view.draw(ui, &mut timeline, &mut playhead, &mut selected, true)
                        .error
                        .is_none()
                );
            },
        )
    };
    let access = frame(Vec::new()).platform_output.accesskit_update.unwrap();
    let bounds = |label| {
        access.nodes.iter().filter_map(move |(_, node)| {
            (node.label() == Some(label))
                .then(|| node.bounds())
                .flatten()
        })
    };
    let blade = bounds("刀片 · C").next().unwrap();
    let blade = pos2(
        ((blade.x0 + blade.x1) * 0.5) as f32,
        ((blade.y0 + blade.y1) * 0.5) as f32,
    );
    let clip_bounds = bounds("screen.mp4")
        .reduce(|first, second| first.union(second))
        .unwrap();
    let cut = pos2(
        ((clip_bounds.x0 + clip_bounds.x1) * 0.5) as f32,
        ((clip_bounds.y0 + clip_bounds.y1) * 0.5) as f32,
    );
    let top_row = bounds("选中第2条视频轨").next().unwrap();
    let ruler = pos2(cut.x, top_row.y0 as f32 - BAND_HEIGHT - RULER_HEIGHT * 0.5);
    let click = |frame: &mut dyn FnMut(Vec<Event>) -> egui::FullOutput, pos| {
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(vec![Event::PointerMoved(pos), button(true)]);
        frame(vec![button(false)]);
        frame(Vec::new());
    };
    let cursor_at = |frame: &mut dyn FnMut(Vec<Event>) -> egui::FullOutput, pos| {
        frame(vec![Event::PointerMoved(pos)])
            .platform_output
            .cursor_icon
    };
    let assert_native_cursor = |icon| {
        assert!(!matches!(
            icon,
            egui::CursorIcon::None | egui::CursorIcon::Crosshair
        ));
    };

    click(&mut frame, blade);
    let blade_frame = frame(vec![Event::PointerMoved(cut)]);
    assert_eq!(
        blade_frame.platform_output.cursor_icon,
        egui::CursorIcon::None
    );
    assert!(blade_frame.shapes.iter().any(|clipped| {
        matches!(&clipped.shape, egui::Shape::Path(path)
            if path.closed
                && path.fill.a() > 0
                && path.points.first().is_some_and(|tip| tip.distance(cut) < 0.01))
    }));
    assert_native_cursor(cursor_at(&mut frame, ruler));
    assert_native_cursor(cursor_at(&mut frame, blade));
    assert_eq!(cursor_at(&mut frame, cut), egui::CursorIcon::None);
    assert_native_cursor(frame(vec![Event::PointerGone]).platform_output.cursor_icon);
    assert_eq!(cursor_at(&mut frame, cut), egui::CursorIcon::None);
    for pressed in [true, false] {
        frame(vec![Event::Key {
            key: egui::Key::V,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
    }
    assert_native_cursor(frame(Vec::new()).platform_output.cursor_icon);
    click(&mut frame, cut + vec2(20.0, 0.0));
    click(&mut frame, blade);
    click(&mut frame, cut);
    click(&mut frame, blade);
    click(&mut frame, cut + vec2(20.0, 0.0));

    assert_eq!(timeline.clips().len(), 2);
    assert_eq!(timeline.clips()[0].source_end, 1_000_000);
    assert_eq!(timeline.clips()[1].start, 1_000_000);
    assert_eq!(timeline.clips()[1].source_start, 1_000_000);
    assert!(view.mode == Mode::Select);
    assert!(timeline.undo());
    assert_eq!(timeline.clips(), original);
}

#[test]
fn dragging_a_clip_between_video_tracks_handles_a_batched_egui_frame() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut timeline = Timeline::default();
    let upper = timeline
        .tracks()
        .iter()
        .find(|track| track.kind == TrackKind::Video && track.id != PRIMARY_VIDEO)
        .unwrap()
        .id;
    let id = timeline
        .append(PathBuf::from("screen.mp4"), 2_000_000)
        .unwrap();
    let mut timeline_view = TimelineView::default();
    let mut playhead = 0;
    let mut selected = None;
    let mut frame = |events| {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 330.0))),
                events,
                ..Default::default()
            },
            |ui| {
                timeline_view.draw(ui, &mut timeline, &mut playhead, &mut selected, true);
            },
        )
    };
    let access = frame(Vec::new()).platform_output.accesskit_update.unwrap();
    let track_center = |label| {
        let bounds = access
            .nodes
            .iter()
            .find_map(|(_, node)| {
                (node.label() == Some(label))
                    .then(|| node.bounds())
                    .flatten()
            })
            .unwrap_or_else(|| panic!("track header was not laid out: {label}"));
        pos2(TRACK_HEADER + 38.0, ((bounds.y0 + bounds.y1) * 0.5) as f32)
    };
    let origin = track_center("选中第1条视频轨");
    let destination = track_center("选中第2条视频轨");
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    frame(vec![
        button(origin, true),
        Event::PointerMoved(destination),
        button(destination, false),
        Event::PointerGone,
    ]);
    assert_eq!(timeline.track_for_clip(id).unwrap().id, upper);
    assert_eq!(timeline.clip(id).unwrap().start, 0);
    assert_eq!(selected, Some(id));
    assert!(timeline.undo());
    assert_eq!(timeline.track_for_clip(id).unwrap().id, PRIMARY_VIDEO);
}

#[test]
fn dropping_a_clip_on_the_upper_band_creates_a_track_even_when_rows_overflow() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut timeline = Timeline::default();
    for _ in 0..5 {
        timeline.add_track(TrackKind::Audio).unwrap();
    }
    let id = timeline
        .append(PathBuf::from("screen.mp4"), 2_000_000)
        .unwrap();
    let track_count = timeline.tracks().len();
    let mut timeline_view = TimelineView::default();
    let mut playhead = 0;
    let mut selected = None;
    let mut frame = |events| {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 245.0))),
                events,
                ..Default::default()
            },
            |ui| {
                timeline_view.draw(ui, &mut timeline, &mut playhead, &mut selected, true);
            },
        )
    };
    let access = frame(Vec::new()).platform_output.accesskit_update.unwrap();
    assert!(
        access
            .nodes
            .iter()
            .all(|(_, node)| node.label() != Some("拖入素材建立音频轨"))
    );
    let position = |label| {
        let bounds = access
            .nodes
            .iter()
            .find_map(|(_, node)| {
                (node.label() == Some(label) || node.value() == Some(label))
                    .then(|| node.bounds())
                    .flatten()
            })
            .unwrap_or_else(|| panic!("timeline label was not laid out: {label}"));
        pos2(TRACK_HEADER + 38.0, ((bounds.y0 + bounds.y1) * 0.5) as f32)
    };
    let origin = position("选中第1条视频轨");
    let to = position("拖入素材建立视频轨");
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    frame(vec![
        button(origin, true),
        Event::PointerMoved(to),
        button(to, false),
        Event::PointerGone,
    ]);

    assert_eq!(timeline.tracks().len(), track_count + 1);
    assert_eq!(
        timeline.track_for_clip(id).unwrap().id,
        timeline.tracks()[0].id
    );
    assert_eq!(timeline.clip(id).unwrap().start, 0);
    assert_eq!(selected, Some(id));
    assert!(timeline.undo());
    assert_eq!(timeline.tracks().len(), track_count);
    assert_eq!(timeline.track_for_clip(id).unwrap().id, PRIMARY_VIDEO);
}

#[test]
fn dropping_an_existing_clip_between_scrolled_rows_inserts_a_track() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut timeline = Timeline::default();
    for _ in 0..4 {
        timeline.add_track(TrackKind::Audio).unwrap();
    }
    let id = timeline.append("screen.mp4".into(), 2_000_000).unwrap();
    let original_tracks = timeline.tracks().to_vec();
    let mut timeline_view = TimelineView {
        vertical_scroll: 18.0,
        ..TimelineView::default()
    };
    let mut playhead = 0;
    let mut selected = None;
    let mut frame = |events| {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 300.0))),
                events,
                ..Default::default()
            },
            |ui| {
                timeline_view.draw(ui, &mut timeline, &mut playhead, &mut selected, true);
            },
        )
    };
    let access = frame(Vec::new()).platform_output.accesskit_update.unwrap();
    let row = access
        .nodes
        .iter()
        .find_map(|(_, node)| {
            (node.label() == Some("选中第1条视频轨"))
                .then(|| node.bounds())
                .flatten()
        })
        .unwrap();
    let x = TRACK_HEADER + 38.0;
    let origin = pos2(x, ((row.y0 + row.y1) * 0.5) as f32);
    let to = pos2(x, row.y0 as f32);
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    frame(vec![
        button(origin, true),
        Event::PointerMoved(to),
        button(to, false),
        Event::PointerGone,
    ]);

    let tracks = timeline.tracks();
    assert_eq!(tracks.len(), original_tracks.len() + 1);
    assert_eq!(tracks[0], original_tracks[0]);
    assert_eq!(tracks[2].id, PRIMARY_VIDEO);
    assert!(tracks[2].clips.is_empty());
    assert_eq!(tracks[1].kind, TrackKind::Video);
    assert_eq!(tracks[1].clips[0].id, id);
    assert_eq!(tracks[1].clips[0].start, 0);
    assert_eq!(selected, Some(id));
    assert!(timeline.undo());
    assert_eq!(timeline.tracks(), original_tracks);
}

#[test]
fn trailing_blank_space_targets_a_new_audio_track() {
    let timeline = Timeline::default();
    let boundaries = Rows::measure(&timeline);
    let rows = Rows {
        area: Rect::from_min_size(pos2(0.0, 60.0), vec2(900.0, TRACK_HEIGHT * 5.0)),
        edge_band: BAND_HEIGHT,
        offset: 0.0,
        boundaries: &boundaries,
    };
    let last_row_bottom = rows.boundary(timeline.tracks().len());
    assert!(TrackGap::at(rows, &timeline, pos2(200.0, last_row_bottom - 5.0)).is_none());
    let gap = TrackGap::at(rows, &timeline, pos2(200.0, last_row_bottom + 8.0)).unwrap();
    assert!(gap.allows(TrackKind::Audio));
    assert!(gap.allows(TrackKind::Video));
    assert!(gap.before.is_none());
    assert!(gap.y > last_row_bottom);
}

#[test]
fn track_end_drop_targets_scroll_with_the_rows() {
    let mut timeline = Timeline::default();
    for _ in 0..3 {
        timeline.add_track(TrackKind::Audio).unwrap();
    }
    let boundaries = Rows::measure(&timeline);
    let area = Rect::from_min_size(pos2(0.0, 60.0), vec2(900.0, 132.0));
    let mut rows = Rows {
        area,
        edge_band: BAND_HEIGHT,
        offset: 0.0,
        boundaries: &boundaries,
    };
    let top = pos2(200.0, area.top() + 13.0);
    let bottom = pos2(200.0, area.bottom() - 13.0);

    assert!(
        TrackGap::at(rows, &timeline, top)
            .unwrap()
            .allows(TrackKind::Video)
    );
    assert!(TrackGap::at(rows, &timeline, bottom).is_none());
    assert!(rows.at(&timeline, bottom.y).is_some());

    rows.offset = TRACK_HEIGHT + 6.0;
    for point in [top, bottom] {
        assert!(rows.at(&timeline, point.y).is_some());
        assert!(TrackGap::at(rows, &timeline, point).is_none());
    }

    rows.offset = rows.max_offset();
    assert!(TrackGap::at(rows, &timeline, top).is_none());
    assert!(rows.at(&timeline, top.y).is_some());
    assert!(
        TrackGap::at(rows, &timeline, bottom)
            .unwrap()
            .allows(TrackKind::Audio)
    );
}

#[test]
fn track_gaps_allow_any_media_kind_in_the_existing_order() {
    let mut timeline = Timeline::default();
    let _caption = timeline.add_track(TrackKind::Caption).unwrap();
    let _title = timeline.add_track(TrackKind::Text).unwrap();
    let boundaries = Rows::measure(&timeline);
    let rows = Rows {
        area: Rect::from_min_size(pos2(0.0, 60.0), vec2(900.0, TRACK_HEIGHT * 5.0)),
        edge_band: BAND_HEIGHT,
        offset: 10.0,
        boundaries: &boundaries,
    };
    assert_eq!(
        rows.at(&timeline, rows.boundary(1) - 6.0).unwrap().id,
        timeline.tracks()[0].id
    );
    assert_eq!(
        rows.at(&timeline, rows.boundary(1) + 6.0).unwrap().id,
        timeline.tracks()[1].id
    );
    let gap = TrackGap::at(rows, &timeline, pos2(200.0, rows.boundary(1))).unwrap();
    assert!(gap.allows(TrackKind::Video));
    assert!(gap.allows(TrackKind::Audio));
    assert_eq!(gap.before, Some(timeline.tracks()[1].id));
}

#[test]
fn dragging_a_caption_edge_can_extend_its_duration() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut timeline = Timeline::default();
    timeline.add_track(TrackKind::Text).unwrap();
    let (_, id) = timeline
        .insert_text(
            TrackKind::Caption,
            600_000,
            2_000_000,
            OverlayText {
                content: "测试字幕".into(),
                style: TextStyle::default(),
            },
        )
        .unwrap();
    let mut view = TimelineView {
        disable_snap: true,
        ..TimelineView::default()
    };
    let mut playhead = 0;
    let mut selected = None;
    let mut frame = |events| {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 420.0))),
                events,
                ..Default::default()
            },
            |ui| {
                let input = view.draw(ui, &mut timeline, &mut playhead, &mut selected, false);
                assert!(input.error.is_none(), "{:?}", input.error);
            },
        )
    };
    let access = frame(Vec::new()).platform_output.accesskit_update.unwrap();
    let edge = access
        .nodes
        .iter()
        .filter_map(|(_, node)| {
            (node.label() == Some("测试字幕"))
                .then(|| node.bounds())
                .flatten()
        })
        .max_by(|a, b| a.x1.total_cmp(&b.x1))
        .unwrap();
    let start = pos2((edge.x1 - 2.0) as f32, ((edge.y0 + edge.y1) * 0.5) as f32);
    let end = start + vec2(56.0, 0.0);
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    frame(vec![
        button(start, true),
        Event::PointerMoved(end),
        button(end, false),
    ]);

    assert_eq!(selected, Some(id));
    assert_eq!(timeline.clip(id).unwrap().duration(), 3_000_000);
    assert!(timeline.undo());
    assert_eq!(timeline.clip(id).unwrap().duration(), 2_000_000);
}

#[test]
fn audio_volume_line_adjusts_gain_without_moving_the_clip() {
    let (mut timeline, clip) = drag_audio_from_volume_line(vec2(0.0, 10.0));
    assert_eq!(timeline.clip(clip).unwrap().start, 1_000_000);
    assert!(timeline.clip(clip).unwrap().volume_db_tenths < 0);
    assert!(timeline.undo());
    assert_eq!(timeline.clip(clip).unwrap().volume_db_tenths, 0);
}

#[test]
fn horizontal_drag_on_an_audio_clip_moves_it_on_its_own_track() {
    use takeseam::editor_timeline::PRIMARY_AUDIO;

    let (mut timeline, clip) = drag_audio_from_volume_line(vec2(56.0, 0.0));
    assert_eq!(timeline.track_for_clip(clip).unwrap().id, PRIMARY_AUDIO);
    assert_eq!(timeline.clip(clip).unwrap().start, 2_000_000);
    assert_eq!(timeline.clip(clip).unwrap().volume_db_tenths, 0);
    assert!(timeline.undo());
    assert_eq!(timeline.clip(clip).unwrap().start, 1_000_000);
}

fn drag_audio_from_volume_line(delta: egui::Vec2) -> (Timeline, u64) {
    use takeseam::editor_timeline::PRIMARY_AUDIO;

    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut timeline = Timeline::default();
    let clip = timeline
        .insert_media_on_track(PRIMARY_AUDIO, "voice.wav".into(), 3_000_000, 1_000_000)
        .unwrap();
    let mut view = TimelineView {
        disable_snap: true,
        ..Default::default()
    };
    let mut playhead = 0;
    let mut selected = None;
    let mut frame = |events| {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 380.0))),
                events,
                ..Default::default()
            },
            |ui| {
                let result = view.draw(ui, &mut timeline, &mut playhead, &mut selected, false);
                assert!(result.error.is_none());
            },
        )
    };
    let accessibility = frame(Vec::new()).platform_output.accesskit_update.unwrap();
    let line = accessibility
        .nodes
        .iter()
        .find_map(|(_, node)| {
            (node.label() == Some("片段音量"))
                .then(|| node.bounds())
                .flatten()
        })
        .unwrap();
    let origin = pos2(
        ((line.x0 + line.x1) * 0.5) as f32,
        ((line.y0 + line.y1) * 0.5) as f32,
    );
    let destination = origin + delta;
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    frame(vec![
        button(origin, true),
        Event::PointerMoved(destination),
        button(destination, false),
    ]);
    assert_eq!(selected, Some(clip));
    (timeline, clip)
}
