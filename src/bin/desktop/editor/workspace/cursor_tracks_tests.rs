use super::*;
use crate::editor::cursor_effect::{CursorSample, CursorTelemetry};

#[test]
fn cursor_telemetry_is_mapped_to_its_own_video_clip() {
    let directory = tempfile::tempdir().unwrap();
    let first_path = directory.path().join("first.mp4");
    let second_path = directory.path().join("second.mp4");
    let mut timeline = Timeline::default();
    let first_clip = timeline
        .insert_media(first_path.clone(), 1_000_000, 0)
        .unwrap();
    let second_clip = timeline
        .insert_media(second_path.clone(), 1_000_000, 1_000_000)
        .unwrap();
    let telemetry = CursorTelemetry {
        version: 1,
        width: 640,
        height: 360,
        samples: vec![CursorSample {
            at_us: 500_000,
            x: 0.5,
            y: 0.5,
            pressed: true,
        }],
    };
    let telemetry_path = PathBuf::from(format!("{}.cursor.json", second_path.display()));
    std::fs::write(&telemetry_path, serde_json::to_vec(&telemetry).unwrap()).unwrap();
    let mut cache = CursorMarkerCache::default();

    let tracks = cursor_tracks(&timeline, &mut cache);

    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].clip_id, second_clip);
    assert_eq!(
        tracks[0].markers,
        [CursorClickMarker {
            timeline_us: 1_500_000,
            source_us: 500_000,
        }]
    );
    assert!(timeline.clip(first_clip).is_some());

    let repeat = timeline
        .insert_media(second_path, 1_000_000, 2_000_000)
        .unwrap();
    timeline.delete_cursor_click(second_clip, 500_000).unwrap();
    let tracks = cursor_tracks(&timeline, &mut cache);
    assert_eq!(tracks[0].clip_id, second_clip);
    assert!(tracks[0].markers.is_empty());
    assert_eq!(tracks[1].clip_id, repeat);
    assert_eq!(tracks[1].markers[0].timeline_us, 2_500_000);
    assert!(timeline.undo());
    assert_eq!(cursor_tracks(&timeline, &mut cache)[0].markers.len(), 1);
}
