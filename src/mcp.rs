use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use crate::editor_timeline::{Clip, Timeline, Track, TrackKind};

#[derive(Debug, Deserialize)]
struct ProjectDocument {
    version: u32,
    playhead_us: u64,
    #[serde(default)]
    clips: Option<Vec<Clip>>,
    #[serde(default)]
    tracks: Option<Vec<Track>>,
}

#[derive(Debug, Serialize)]
pub struct TimelineSnapshot {
    pub version: u32,
    pub playhead_us: u64,
    pub duration_us: u64,
    pub tracks: Vec<TrackSnapshot>,
}

#[derive(Debug, Serialize)]
pub struct TrackSnapshot {
    pub id: u64,
    pub kind: TrackKind,
    pub enabled: bool,
    pub clips: Vec<ClipSnapshot>,
}

#[derive(Debug, Serialize)]
pub struct ClipSnapshot {
    pub id: u64,
    pub path: String,
    pub start_us: u64,
    pub duration_us: u64,
    pub source_start_us: u64,
    pub source_end_us: u64,
    pub text: Option<String>,
}

pub fn read_timeline(path: &Path) -> Result<TimelineSnapshot> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("无法读取工程文件（{}）", path.display()))?;
    let document: ProjectDocument =
        serde_json::from_slice(&bytes).context("工程文件格式无效")?;
    let timeline = match document.version {
        1 => Timeline::from_clips(document.clips.context("工程缺少片段")?)
            .context("工程时间线无效")?,
        2 | 3 => Timeline::from_tracks(document.tracks.context("工程缺少轨道")?)
            .context("工程时间线无效")?,
        version => bail!("不支持的工程版本 {version}"),
    };
    Ok(snapshot(document.version, document.playhead_us, &timeline))
}

fn snapshot(version: u32, playhead_us: u64, timeline: &Timeline) -> TimelineSnapshot {
    TimelineSnapshot {
        version,
        playhead_us: playhead_us.min(timeline.duration()),
        duration_us: timeline.duration(),
        tracks: timeline
            .tracks()
            .iter()
            .map(|track| TrackSnapshot {
                id: track.id,
                kind: track.kind,
                enabled: track.enabled,
                clips: track
                    .clips
                    .iter()
                    .map(|clip| ClipSnapshot {
                        id: clip.id,
                        path: clip.path.to_string_lossy().into_owned(),
                        start_us: clip.start,
                        duration_us: clip.duration(),
                        source_start_us: clip.source_start,
                        source_end_us: clip.source_end,
                        text: clip.text.as_ref().map(|text| text.content.clone()),
                    })
                    .collect(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn reads_v2_project_timeline() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("project.takeproj");
        fs::write(
            &path,
            r#"{
              "version": 2,
              "playhead_us": 250,
              "tracks": [{
                "id": 7,
                "kind": "Video",
                "enabled": true,
                "clips": [{
                  "id": 9,
                  "path": "clip.mp4",
                  "media_duration": 1000,
                  "source_start": 100,
                  "source_end": 700,
                  "start": 200
                }]
              }]
            }"#,
        )?;
        let snapshot = read_timeline(&path)?;
        assert_eq!(snapshot.duration_us, 800);
        assert_eq!(snapshot.playhead_us, 250);
        assert_eq!(snapshot.tracks[0].clips[0].duration_us, 600);
        Ok(())
    }
}
