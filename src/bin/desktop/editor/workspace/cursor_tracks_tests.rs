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
    assert_eq!(tracks[0].markers, [1_500_000]);
    assert!(timeline.clip(first_clip).is_some());
}
