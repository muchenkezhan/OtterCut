use super::*;

#[test]
fn removing_a_track_keeps_other_tracks_and_undo_restores_its_clips() {
    let mut timeline = Timeline::default();
    timeline.append("screen.mp4".into(), 20).unwrap();
    let track = timeline.add_track(TrackKind::Audio).unwrap();
    let clip = timeline
        .insert_media_on_track(track, "voice.wav".into(), 30, 5)
        .unwrap();
    timeline.set_clip_volume(clip, -75).unwrap();
    timeline.set_track_enabled(track, false).unwrap();
    let original = timeline.tracks().to_vec();
    let revision = timeline.revision();

    timeline.remove_track(track).unwrap();
    assert!(timeline.track(track).is_none());
    assert!(timeline.clip(clip).is_none());
    assert_eq!(timeline.tracks(), &original[..original.len() - 1]);
    assert_eq!(timeline.revision(), revision + 1);
    assert_eq!(
        timeline.remove_track(track),
        Err(TimelineError::TrackNotFound(track))
    );
    assert_eq!(timeline.revision(), revision + 1);
    assert!(timeline.undo());
    assert_eq!(timeline.tracks(), original);
    assert!(timeline.redo());
    assert!(timeline.track(track).is_none());
    assert!(timeline.add_track(TrackKind::Audio).unwrap() > track);
}

#[test]
fn removing_the_last_track_leaves_an_empty_sequence_that_can_add_new_tracks() {
    let mut timeline =
        Timeline::from_tracks(vec![Track::new(PRIMARY_AUDIO, TrackKind::Audio)]).unwrap();
    timeline.remove_track(PRIMARY_AUDIO).unwrap();
    assert!(timeline.tracks().is_empty());
    assert_eq!(timeline.duration(), 0);
    assert!(timeline.undo());
    assert_eq!(
        timeline.tracks(),
        &[Track::new(PRIMARY_AUDIO, TrackKind::Audio)]
    );
    assert!(timeline.redo());

    let mut reopened = Timeline::from_tracks(timeline.tracks().to_vec()).unwrap();
    assert!(reopened.tracks().is_empty());
    let audio = reopened.add_track(TrackKind::Audio).unwrap();
    assert!(audio > DEFAULT_TRACK_ID_MAX);
    assert_eq!(reopened.track(audio).unwrap().kind, TrackKind::Audio);
}

#[test]
fn changing_audio_volume_is_undoable_and_survives_trimming_splitting_and_pasting() {
    let mut timeline = Timeline::default();
    let video = timeline.append("screen.mp4".into(), 20).unwrap();
    let audio = timeline
        .insert_media_on_track(PRIMARY_AUDIO, "voice.wav".into(), 30, 5)
        .unwrap();
    let baseline = timeline.revision();
    assert_eq!(
        timeline.set_clip_volume(video, -50),
        Err(TimelineError::IncompatibleTrack)
    );
    assert_eq!(
        timeline.set_clip_volume(audio, Clip::MAX_VOLUME_DB_TENTHS + 1),
        Err(TimelineError::InvalidVolume)
    );
    assert_eq!(timeline.revision(), baseline);

    timeline.set_clip_volume(audio, -125).unwrap();
    assert_eq!(timeline.revision(), baseline + 1);
    timeline.set_clip_volume(audio, -125).unwrap();
    assert_eq!(timeline.revision(), baseline + 1);
    assert!(timeline.undo());
    assert_eq!(timeline.clip(audio).unwrap().volume_db_tenths, 0);
    assert!(timeline.redo());
    timeline.trim_start(audio, 10, false).unwrap();
    let right = timeline.split(audio, 20).unwrap();
    assert_eq!(timeline.clip(audio).unwrap().volume_db_tenths, -125);
    let trimmed = timeline.clip(right).unwrap().clone();
    assert_eq!(trimmed.volume_db_tenths, -125);
    let (_, pasted) = timeline
        .paste_on_new_track_before(TrackKind::Audio, None, &trimmed, 42)
        .unwrap();
    assert_eq!(
        timeline.clip(pasted),
        Some(&Clip {
            id: pasted,
            start: 42,
            ..trimmed
        })
    );
}

#[cfg(windows)]
#[test]
fn legacy_clips_load_at_unity_and_adjusted_volume_roundtrips() {
    let old = r#"{"id":1,"path":"voice.wav","media_duration":30,"source_start":0,"source_end":30,"start":0}"#;
    let mut clip = serde_json::from_str::<Clip>(old).unwrap();
    assert_eq!(clip.volume_db_tenths, 0);
    assert!(
        serde_json::to_value(&clip)
            .unwrap()
            .get("volume_db_tenths")
            .is_none()
    );
    clip.volume_db_tenths = 96;
    let serialized = serde_json::to_value(&clip).unwrap();
    assert_eq!(serialized["volume_db_tenths"], 96);
    assert_eq!(serde_json::from_value::<Clip>(serialized).unwrap(), clip);
}

#[test]
fn moving_selected_clips_keeps_offsets_and_tracks_as_one_edit() {
    let mut timeline = Timeline::default();
    let first = timeline
        .insert_media_on_track(PRIMARY_VIDEO, "first.mp4".into(), 4, 2)
        .unwrap();
    let second = timeline
        .insert_media_on_track(PRIMARY_VIDEO, "second.mp4".into(), 6, 14)
        .unwrap();
    let stationary = timeline
        .insert_media_on_track(PRIMARY_VIDEO, "fixed.mp4".into(), 6, 31)
        .unwrap();
    let audio = timeline
        .insert_media_on_track(PRIMARY_AUDIO, "voice.wav".into(), 10, 4)
        .unwrap();
    let original = timeline.tracks().to_vec();
    let revision = timeline.revision();
    let selected = [first, second, audio, audio];

    for (ids, delta, error) in [
        (selected.as_slice(), 18, TimelineError::Overlap),
        (selected.as_slice(), -4, TimelineError::TimeOutOfBounds),
        (&[first, 999], 3, TimelineError::ClipNotFound(999)),
    ] {
        assert_eq!(timeline.move_clips(ids, delta), Err(error));
        assert_eq!(timeline.tracks(), original);
        assert_eq!(timeline.revision(), revision);
    }
    timeline.move_clips(&[], 10).unwrap();
    timeline.move_clips(&[first, 999], 0).unwrap();
    assert_eq!(timeline.revision(), revision);

    timeline.move_clips(&selected, 7).unwrap();
    for (id, track, start) in [
        (first, PRIMARY_VIDEO, 9),
        (second, PRIMARY_VIDEO, 21),
        (audio, PRIMARY_AUDIO, 11),
        (stationary, PRIMARY_VIDEO, 31),
    ] {
        assert_eq!(timeline.clip(id).unwrap().start, start);
        assert_eq!(timeline.track_for_clip(id).unwrap().id, track);
    }
    assert_eq!(timeline.revision(), revision + 1);
    assert!(timeline.undo());
    assert_eq!(timeline.tracks(), original);
    assert!(timeline.redo());
    assert_eq!(timeline.clip(audio).unwrap().start, 11);
}

#[test]
fn removing_selected_clips_preserves_gaps_and_is_atomic() {
    let mut timeline = Timeline::default();
    let video = timeline.append("video.mp4".into(), 8).unwrap();
    let keep = timeline.append("keep.mp4".into(), 16).unwrap();
    let audio = timeline
        .insert_media_on_track(PRIMARY_AUDIO, "voice.wav".into(), 12, 5)
        .unwrap();
    let original = timeline.tracks().to_vec();
    let revision = timeline.revision();

    assert_eq!(
        timeline.remove_clips(&[video, 999, audio]),
        Err(TimelineError::ClipNotFound(999))
    );
    timeline.remove_clips(&[]).unwrap();
    assert_eq!(timeline.tracks(), original);
    assert_eq!(timeline.revision(), revision);

    timeline.remove_clips(&[audio, video, audio]).unwrap();
    assert!(timeline.clip(video).is_none());
    assert!(timeline.clip(audio).is_none());
    assert_eq!(timeline.clip(keep).unwrap().start, 8);
    assert_eq!(timeline.revision(), revision + 1);
    assert!(timeline.undo());
    assert_eq!(timeline.tracks(), original);
    assert!(timeline.redo());
    assert!(timeline.clip(audio).is_none());
}

#[test]
fn ripple_edits_only_the_selected_track() {
    let mut timeline = Timeline::default();
    let overlay = timeline.tracks()[0].id;
    let main = timeline.append("main.mp4".into(), 30).unwrap();
    let music = timeline
        .insert_media_on_track(PRIMARY_AUDIO, "music.wav".into(), 60, 0)
        .unwrap();
    let foreground = timeline
        .insert_media_on_track(overlay, "foreground.mp4".into(), 20, 5)
        .unwrap();
    let after = timeline
        .insert_media_on_track(overlay, "after.mp4".into(), 7, 26)
        .unwrap();

    assert_eq!(timeline.duration(), 60);
    assert_eq!(timeline.clip_at(10).unwrap().id, main);
    assert_eq!(
        timeline.track(overlay).unwrap().clip_at(10).unwrap().id,
        foreground
    );
    assert_eq!(
        timeline
            .track(PRIMARY_AUDIO)
            .unwrap()
            .clip_at(10)
            .unwrap()
            .id,
        music
    );

    timeline.remove(foreground, true).unwrap();
    assert_eq!(timeline.clip(after).unwrap().start, 6);
    assert_eq!(timeline.clip(main).unwrap().end(), 30);
    assert_eq!(timeline.clip(music).unwrap().end(), 60);
    assert!(timeline.undo());
    assert_eq!(timeline.clip(foreground).unwrap().start, 5);
    assert_eq!(timeline.clip(after).unwrap().start, 26);
    assert!(timeline.redo());
    assert_eq!(timeline.clip(after).unwrap().start, 6);
}

#[test]
fn reassignment_and_track_output_controls_have_atomic_history() {
    let mut timeline = Timeline::default();
    let overlay = timeline.tracks()[0].id;
    let base = timeline.append("base.mp4".into(), 20).unwrap();
    let other = timeline
        .insert_media_on_track(overlay, "overlay.mp4".into(), 10, 0)
        .unwrap();

    let revision = timeline.revision();
    assert_eq!(
        timeline.move_to_track(other, PRIMARY_AUDIO, 0),
        Err(TimelineError::IncompatibleTrack)
    );
    assert_eq!(
        timeline.move_to_track(other, PRIMARY_VIDEO, 15),
        Err(TimelineError::Overlap)
    );
    assert_eq!(timeline.revision(), revision);
    assert_eq!(timeline.track_for_clip(other).unwrap().id, overlay);

    timeline.move_to_track(other, PRIMARY_VIDEO, 20).unwrap();
    assert_eq!(timeline.clips()[1].id, other);
    assert_eq!(timeline.clips()[0].id, base);
    assert!(timeline.track(overlay).unwrap().clips.is_empty());
    assert!(timeline.undo());
    assert_eq!(timeline.track_for_clip(other).unwrap().id, overlay);
    assert!(timeline.redo());

    let new_video = timeline.add_track(TrackKind::Video).unwrap();
    let new_audio = timeline.add_track(TrackKind::Audio).unwrap();
    assert_eq!(timeline.tracks().first().unwrap().id, new_video);
    assert_eq!(timeline.tracks().last().unwrap().id, new_audio);
    timeline.set_track_enabled(PRIMARY_VIDEO, false).unwrap();
    assert!(!timeline.track(PRIMARY_VIDEO).unwrap().enabled);
    assert_eq!(timeline.duration(), 30);
    assert!(timeline.undo());
    assert!(timeline.track(PRIMARY_VIDEO).unwrap().enabled);
    assert!(timeline.undo());
    assert!(timeline.track(new_audio).is_none());
    assert!(timeline.redo());
    assert_eq!(timeline.tracks().last().unwrap().id, new_audio);
}

#[test]
fn adding_an_empty_track_before_an_anchor_is_undoable() {
    let mut timeline = Timeline::default();
    timeline.append("main.mp4".into(), 30).unwrap();
    let original = timeline.tracks().to_vec();
    let revision = timeline.revision();

    assert_eq!(
        timeline.add_track_before(TrackKind::Audio, Some(PRIMARY_VIDEO)),
        Err(TimelineError::IncompatibleTrack)
    );
    assert_eq!(timeline.tracks(), original);
    assert_eq!(timeline.revision(), revision);

    let id = timeline
        .add_track_before(TrackKind::Video, Some(PRIMARY_VIDEO))
        .unwrap();
    assert_eq!(timeline.tracks()[0], original[0]);
    assert_eq!(timeline.tracks()[1], Track::new(id, TrackKind::Video));
    assert_eq!(timeline.tracks()[2..], original[1..]);
    assert!(timeline.undo());
    assert_eq!(timeline.tracks(), original);
    assert!(timeline.redo());
    assert_eq!(timeline.tracks()[1].id, id);
}

#[test]
fn adding_below_the_last_video_track_stays_below_in_a_video_only_project() {
    let original = vec![
        Track::new(3, TrackKind::Video),
        Track::new(PRIMARY_VIDEO, TrackKind::Video),
    ];
    let mut timeline = Timeline::from_tracks(original.clone()).unwrap();
    let id = timeline
        .add_track_after(TrackKind::Video, PRIMARY_VIDEO)
        .unwrap();

    assert_eq!(timeline.tracks()[..original.len()], original);
    assert_eq!(
        timeline.tracks().last().unwrap(),
        &Track::new(id, TrackKind::Video)
    );
    assert!(timeline.undo());
    assert_eq!(timeline.tracks(), original);
}

#[test]
fn saved_tracks_allow_overlaps_but_require_unique_clip_ids() {
    let mut timeline = Timeline::default();
    let overlay = timeline.tracks()[0].id;
    let main = timeline.append("main.mp4".into(), 20).unwrap();
    let other = timeline
        .insert_media_on_track(overlay, "overlay.mp4".into(), 10, 0)
        .unwrap();
    let new = Timeline::from_tracks(timeline.tracks().to_vec()).unwrap();
    assert_eq!(new.tracks(), timeline.tracks());
    assert_eq!(new.duration(), 20);

    let mut tracks = new.tracks().to_vec();
    tracks[0].clips[0].id = main;
    assert!(
        matches!(Timeline::from_tracks(tracks), Err(TimelineError::DuplicateClipId(id)) if id == main)
    );
    let mut tracks = new.tracks().to_vec();
    tracks[0].id = PRIMARY_VIDEO;
    assert!(matches!(
        Timeline::from_tracks(tracks),
        Err(TimelineError::DuplicateTrackId(PRIMARY_VIDEO))
    ));
    assert_ne!(main, other);
}

#[test]
fn dropping_a_source_creates_and_fills_each_outer_track_in_one_edit() {
    for (kind, name) in [
        (TrackKind::Video, "overlay.mp4"),
        (TrackKind::Audio, "music.wav"),
    ] {
        let mut timeline = Timeline::default();
        let before = timeline.tracks().to_vec();
        let (track, clip) = timeline
            .insert_media_on_new_track(kind, name.into(), 30, 10)
            .unwrap();
        let outer = match kind {
            TrackKind::Video => timeline.tracks().first().unwrap(),
            TrackKind::Audio => timeline.tracks().last().unwrap(),
            _ => unreachable!("test covers imported media"),
        };
        assert_eq!((outer.id, outer.kind), (track, kind));
        assert_eq!(outer.clips, [Clip::full_source(clip, name.into(), 30, 10)]);
        assert_eq!(timeline.revision(), 1);
        assert!(timeline.undo());
        assert_eq!(timeline.tracks(), before);
        assert!(!timeline.can_undo());
        assert!(timeline.redo());
        assert_eq!(timeline.track_for_clip(clip).unwrap().id, track);
    }
}

#[test]
fn pasting_to_a_new_track_preserves_trimmed_media_and_caption_style() {
    let mut timeline = Timeline::default();
    let audio = timeline
        .insert_media_on_track(PRIMARY_AUDIO, "trimmed.wav".into(), 40, 6)
        .unwrap();
    timeline.trim_start(audio, 10, false).unwrap();
    timeline.trim_end(audio, 30, false).unwrap();
    let (caption_track, caption) = timeline
        .insert_text(
            TrackKind::Caption,
            3,
            20,
            OverlayText {
                content: "片段字幕".into(),
                style: TextStyle {
                    size: 38,
                    placement: TextPlacement::Top,
                    bold: true,
                    ..TextStyle::default()
                },
            },
        )
        .unwrap();

    for (kind, anchor, source_id) in [
        (TrackKind::Audio, None, audio),
        (TrackKind::Caption, Some(caption_track), caption),
    ] {
        let before = timeline.tracks().to_vec();
        let revision = timeline.revision();
        let source = timeline.clip(source_id).unwrap().clone();
        let (track, id) = timeline
            .paste_on_new_track_before(kind, anchor, &source, 13)
            .unwrap();
        let expected = Clip {
            id,
            start: 13,
            ..source.clone()
        };

        assert_ne!(id, source_id);
        assert_eq!(timeline.clip(source_id), Some(&source));
        assert_eq!(timeline.clip(id), Some(&expected));
        assert_eq!(timeline.track(track).unwrap().kind, kind);
        assert_eq!(timeline.track_for_clip(id).unwrap().id, track);
        if let Some(anchor) = anchor {
            assert!(
                timeline
                    .tracks()
                    .windows(2)
                    .any(|pair| { pair[0].id == track && pair[1].id == anchor })
            );
        }
        assert_eq!(timeline.revision(), revision + 1);
        assert!(timeline.undo());
        assert_eq!(timeline.tracks(), before);
        assert!(timeline.redo());
        assert_eq!(timeline.clip(id), Some(&expected));
        assert!(timeline.undo());
    }
}

#[test]
fn invalid_paste_on_new_track_does_not_consume_ids_or_history() {
    let mut timeline = Timeline::default();
    let clip = timeline.append("source.mp4".into(), 30).unwrap();
    let undone = timeline.add_track(TrackKind::Audio).unwrap();
    assert!(timeline.undo());
    let source = timeline.clip(clip).unwrap().clone();
    let before = timeline.tracks().to_vec();
    let revision = timeline.revision();

    assert_eq!(
        timeline.paste_on_new_track_before(TrackKind::Caption, None, &source, 0),
        Err(TimelineError::IncompatibleTrack)
    );
    assert_eq!(
        timeline.paste_on_new_track_before(TrackKind::Audio, Some(PRIMARY_VIDEO), &source, 0),
        Err(TimelineError::IncompatibleTrack)
    );
    assert_eq!(
        timeline.paste_on_new_track_before(TrackKind::Video, None, &source, u64::MAX),
        Err(TimelineError::TimeOutOfBounds)
    );
    assert_eq!(timeline.revision(), revision);
    assert_eq!(timeline.tracks(), before);
    assert!(timeline.redo());
    assert_eq!(timeline.tracks().last().unwrap().id, undone);
    assert_eq!(
        timeline.paste_on_new_track_before(TrackKind::Video, None, &source, 0),
        Ok((undone + 1, clip + 1))
    );
}

#[test]
fn moving_to_an_outer_track_preserves_the_slice_and_track_history() {
    let mut timeline = Timeline::default();
    let video = timeline.append("video.mp4".into(), 30).unwrap();
    let audio = timeline
        .insert_media_on_track(PRIMARY_AUDIO, "music.wav".into(), 45, 5)
        .unwrap();
    timeline.trim_start(audio, 12, false).unwrap();

    for (kind, clip) in [(TrackKind::Video, video), (TrackKind::Audio, audio)] {
        let before = timeline.tracks().to_vec();
        let companion = if clip == video { audio } else { video };
        let other_clip = timeline.clip(companion).unwrap().clone();
        let mut moved_clip = timeline.clip(clip).unwrap().clone();
        moved_clip.start = 19;
        let track = timeline.move_to_new_track(clip, 19).unwrap();
        let outer = match kind {
            TrackKind::Video => timeline.tracks().first().unwrap(),
            TrackKind::Audio => timeline.tracks().last().unwrap(),
            _ => unreachable!("test covers imported media"),
        };
        assert_eq!((outer.id, outer.kind), (track, kind));
        assert_eq!(timeline.clip(clip), Some(&moved_clip));
        assert_eq!(timeline.clip(companion), Some(&other_clip));
        assert!(timeline.undo());
        assert_eq!(timeline.tracks(), before);
        assert!(timeline.redo());
        assert_eq!(timeline.track_for_clip(clip).unwrap().id, track);
    }
}

#[test]
fn invalid_edge_drops_do_not_create_a_track_or_disturb_history() {
    let mut timeline = Timeline::default();
    let clip = timeline.append("video.mp4".into(), 30).unwrap();
    let track = timeline.add_track(TrackKind::Audio).unwrap();
    assert!(timeline.undo());
    let before = timeline.tracks().to_vec();
    let revision = timeline.revision();

    assert_eq!(
        timeline.insert_media_on_new_track(TrackKind::Video, "empty.mp4".into(), 0, 15),
        Err(TimelineError::InvalidSourceRange)
    );
    assert_eq!(
        timeline.insert_media_on_new_track(TrackKind::Audio, "music.wav".into(), 10, u64::MAX),
        Err(TimelineError::TimeOutOfBounds)
    );
    assert_eq!(
        timeline.move_to_new_track(clip, u64::MAX),
        Err(TimelineError::TimeOutOfBounds)
    );
    assert_eq!(timeline.tracks(), before);
    assert_eq!(timeline.revision(), revision);
    assert!(timeline.redo());
    assert_eq!(timeline.tracks().last().unwrap().id, track);
}

#[test]
fn sources_can_create_tracks_between_tracks_of_the_same_kind() {
    let mut timeline = Timeline::default();
    let first_video = timeline.tracks()[0].id;
    let last_audio = timeline.add_track(TrackKind::Audio).unwrap();
    let (video_track, video) = timeline
        .insert_media_on_new_track_before(
            TrackKind::Video,
            Some(PRIMARY_VIDEO),
            "inserted.mp4".into(),
            40,
            9,
        )
        .unwrap();
    let (audio_track, audio) = timeline
        .insert_media_on_new_track_before(
            TrackKind::Audio,
            Some(last_audio),
            "inserted.wav".into(),
            25,
            6,
        )
        .unwrap();

    assert_eq!(
        timeline
            .tracks()
            .iter()
            .map(|track| track.id)
            .collect::<Vec<_>>(),
        [
            first_video,
            video_track,
            PRIMARY_VIDEO,
            PRIMARY_AUDIO,
            audio_track,
            last_audio
        ]
    );
    assert_eq!(timeline.track_for_clip(video).unwrap().id, video_track);
    assert_eq!(timeline.track_for_clip(audio).unwrap().id, audio_track);
    assert_eq!(timeline.clip(video).unwrap().start, 9);
    assert_eq!(timeline.clip(audio).unwrap().start, 6);
}

#[test]
fn either_kind_can_be_created_at_the_video_audio_boundary() {
    for (kind, path) in [
        (TrackKind::Video, "video.mp4"),
        (TrackKind::Audio, "audio.wav"),
    ] {
        let mut timeline = Timeline::default();
        let before = timeline.tracks().to_vec();
        let (track, clip) = timeline
            .insert_media_on_new_track_before(kind, Some(PRIMARY_AUDIO), path.into(), 20, 3)
            .unwrap();
        assert_eq!(timeline.tracks()[1].id, PRIMARY_VIDEO);
        assert_eq!(timeline.tracks()[2].id, track);
        assert_eq!(timeline.tracks()[2].kind, kind);
        assert_eq!(timeline.tracks()[3].id, PRIMARY_AUDIO);
        assert!(timeline.undo());
        assert_eq!(timeline.tracks(), before);
        assert!(!timeline.can_undo());
        assert!(timeline.redo());
        assert_eq!(timeline.track_for_clip(clip).unwrap().id, track);
    }
}

#[test]
fn moving_to_a_middle_track_preserves_the_slice_and_is_atomic() {
    let mut timeline = Timeline::default();
    let lower_audio = timeline.add_track(TrackKind::Audio).unwrap();
    let clip = timeline
        .insert_media_on_track(lower_audio, "trimmed.wav".into(), 40, 6)
        .unwrap();
    timeline.trim_start(clip, 10, false).unwrap();
    let before = timeline.tracks().to_vec();
    let mut expected_clip = timeline.clip(clip).unwrap().clone();
    expected_clip.start = 19;

    let inserted = timeline
        .move_to_new_track_before(clip, Some(lower_audio), 19)
        .unwrap();
    assert_eq!(timeline.tracks()[2].id, PRIMARY_AUDIO);
    assert_eq!(timeline.tracks()[3].id, inserted);
    assert_eq!(timeline.tracks()[4].id, lower_audio);
    assert!(timeline.track(lower_audio).unwrap().clips.is_empty());
    assert_eq!(timeline.track(inserted).unwrap().clips, [expected_clip]);
    assert!(timeline.undo());
    assert_eq!(timeline.tracks(), before);
    assert!(timeline.redo());
    assert_eq!(timeline.track_for_clip(clip).unwrap().id, inserted);
}

#[test]
fn invalid_middle_track_anchors_leave_history_and_clips_intact() {
    let mut timeline = Timeline::default();
    let video = timeline.append("video.mp4".into(), 20).unwrap();
    let lower_audio = timeline.add_track(TrackKind::Audio).unwrap();
    let removed = timeline.add_track(TrackKind::Video).unwrap();
    assert!(timeline.undo());
    let before = timeline.tracks().to_vec();
    let revision = timeline.revision();

    assert_eq!(
        timeline.insert_media_on_new_track_before(
            TrackKind::Audio,
            Some(PRIMARY_VIDEO),
            "audio.wav".into(),
            10,
            0,
        ),
        Err(TimelineError::IncompatibleTrack)
    );
    assert_eq!(
        timeline.move_to_new_track_before(video, Some(lower_audio), 0),
        Err(TimelineError::IncompatibleTrack)
    );
    assert_eq!(
        timeline.move_to_new_track_before(video, Some(removed), 0),
        Err(TimelineError::TrackNotFound(removed))
    );
    assert_eq!(timeline.tracks(), before);
    assert_eq!(timeline.revision(), revision);
    assert!(timeline.redo());
    assert_eq!(timeline.tracks()[0].id, removed);
}
