use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use takeseam::editor_timeline::{Clip, Timeline, Track};

use super::background::{BackgroundSettings, BackgroundSource};
use super::canvas_format::CanvasFormat;
use super::cursor_effect::CursorEffect;

const MEDIA_VERSION: u32 = 2;
const TEXT_VERSION: u32 = 3;
const MAX_PROJECT_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Document {
    version: u32,
    playhead_us: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    clips: Option<Vec<Clip>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tracks: Option<Vec<Track>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    canvas: Option<CanvasFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    background: Option<BackgroundSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cursor_effect: Option<CursorEffect>,
}

#[cfg(test)]
pub(super) fn load(path: &Path) -> Result<(Timeline, u64, CanvasFormat, BackgroundSettings)> {
    let (timeline, position, canvas, background, _) = load_with_cursor(path)?;
    Ok((timeline, position, canvas, background))
}

pub(super) fn load_with_cursor(
    path: &Path,
) -> Result<(Timeline, u64, CanvasFormat, BackgroundSettings, CursorEffect)> {
    let file = File::open(path).context("无法打开剪辑工程")?;
    ensure!(
        file.metadata()?.len() <= MAX_PROJECT_BYTES,
        "剪辑工程文件过大"
    );
    let mut document: Document =
        serde_json::from_reader(BufReader::new(file)).context("剪辑工程格式无效")?;
    let canvas = document.canvas.unwrap_or_default();
    ensure!(canvas.is_valid(), "工程画幅尺寸无效");
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let mut background = document.background.take().unwrap_or_default();
    ensure!(background.is_valid(), "工程背景设置无效");
    if let BackgroundSource::Image(image) = &mut background.source {
        resolve_media_path(parent, image);
    }
    let resolve = |clip: &mut Clip| {
        if clip.text.is_none() {
            resolve_media_path(parent, &mut clip.path);
        }
    };
    let timeline = match document.version {
        1 => {
            let mut clips = document.clips.take().context("工程缺少片段")?;
            clips.iter_mut().for_each(resolve);
            Timeline::from_clips(clips)
        }
        MEDIA_VERSION | TEXT_VERSION => {
            let mut tracks = document.tracks.take().context("工程缺少轨道")?;
            tracks
                .iter_mut()
                .flat_map(|track| &mut track.clips)
                .for_each(resolve);
            Timeline::from_tracks(tracks)
        }
        _ => anyhow::bail!("剪辑工程版本不受支持"),
    }
    .context("剪辑工程的时间线无效")?;
    let playhead = document.playhead_us.min(timeline.duration());
    let cursor_effect = document
        .cursor_effect
        .unwrap_or_else(|| CursorEffect::default_for(timeline.duration()));
    Ok((timeline, playhead, canvas, background, cursor_effect))
}

#[cfg(test)]
pub(super) fn save(
    path: &Path,
    timeline: &Timeline,
    playhead: u64,
    canvas: CanvasFormat,
    background: &BackgroundSettings,
) -> Result<()> {
    save_with_cursor(
        path,
        timeline,
        playhead,
        canvas,
        background,
        &CursorEffect::default_for(timeline.duration()),
    )
}

pub(super) fn save_with_cursor(
    path: &Path,
    timeline: &Timeline,
    playhead: u64,
    canvas: CanvasFormat,
    background: &BackgroundSettings,
    cursor_effect: &CursorEffect,
) -> Result<()> {
    ensure!(
        path.extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("takeproj")),
        "工程文件扩展名必须是 .takeproj",
    );
    ensure!(canvas.is_valid(), "工程画幅尺寸无效");
    ensure!(background.is_valid(), "工程背景设置无效");
    cursor_effect.validate()?;
    let parent = path
        .parent()
        .filter(|path| path.is_dir())
        .context("工程目录不可用")?;
    let mut tracks = timeline.tracks().to_vec();
    for clip in tracks.iter_mut().flat_map(|track| &mut track.clips) {
        if clip.text.is_none() {
            store_media_path(parent, &mut clip.path);
        }
    }
    let backdrop = (*background != BackgroundSettings::default()).then(|| {
        let mut settings = background.clone();
        if let BackgroundSource::Image(image) = &mut settings.source {
            store_media_path(parent, image);
        }
        settings
    });
    let document = Document {
        version: if timeline
            .tracks()
            .iter()
            .any(|track| track.kind.is_overlay())
        {
            TEXT_VERSION
        } else {
            MEDIA_VERSION
        },
        playhead_us: playhead.min(timeline.duration()),
        clips: None,
        tracks: Some(tracks),
        canvas: (!canvas.is_original()).then_some(canvas),
        background: backdrop,
        cursor_effect: Some(cursor_effect.clone()),
    };
    let mut file = tempfile::NamedTempFile::new_in(parent).context("无法创建临时工程")?;
    {
        let mut writer = BufWriter::new(file.as_file_mut());
        serde_json::to_writer_pretty(&mut writer, &document).context("无法写入剪辑工程")?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }
    ensure!(
        file.as_file().metadata()?.len() <= MAX_PROJECT_BYTES,
        "剪辑工程文件超过 16 MB"
    );
    file.as_file().sync_all()?;
    // Never replace a media file if the picker was used to enter an arbitrary filename.
    let backdrop = match &background.source {
        BackgroundSource::Image(image) => Some(image.as_path()),
        _ => None,
    };
    let sources = timeline
        .all_clips()
        .filter(|clip| clip.text.is_none())
        .map(|clip| clip.path.as_path());
    for source in sources.chain(backdrop) {
        if let (Ok(output), Ok(input)) = (fs::canonicalize(path), fs::canonicalize(source)) {
            ensure!(
                !output.as_os_str().eq_ignore_ascii_case(input.as_os_str()),
                "工程文件不能覆盖素材"
            );
        }
    }
    file.persist(path)
        .map_err(|error| error.error)
        .context("无法保存剪辑工程")?;
    Ok(())
}

fn resolve_media_path(parent: &Path, path: &mut PathBuf) {
    if path.has_root() {
        return;
    }
    let resolved = parent.join(&*path);
    #[cfg(not(windows))]
    if let Some(name) = path.to_str()
        && !resolved.is_file()
    {
        // Retain foreign drive/UNC references when reopening a project on another platform.
        let bytes = name.as_bytes();
        if name.starts_with('\\')
            || (bytes.first().is_some_and(u8::is_ascii_alphabetic) && bytes.get(1) == Some(&b':'))
        {
            return;
        }
        // Older Windows projects used backslashes even for relative paths.
        if name.contains('\\') {
            *path = parent.join(name.replace('\\', "/"));
            return;
        }
    }
    *path = resolved;
}

fn store_media_path(parent: &Path, path: &mut PathBuf) {
    if let Ok(relative) = path.strip_prefix(parent)
        && !relative.as_os_str().is_empty()
    {
        *path = relative.to_owned();
    }
    #[cfg(windows)]
    if path.is_relative()
        && let Some(name) = path.to_str().filter(|name| name.contains('\\'))
    {
        *path = name.replace('\\', "/").into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::background::{BackgroundFrame, BackgroundMotion};
    use crate::editor::dynamic_recipe::DynamicRecipe;
    use takeseam::editor_timeline::{
        OverlayText, PRIMARY_AUDIO, PRIMARY_VIDEO, TextAppearance, TextPlacement, TextPosition,
        TextStyle, TrackKind,
    };

    #[test]
    fn project_keeps_the_edited_dynamic_recipe() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("动态配方.takeproj");
        let mut recipe = DynamicRecipe::presets()[2].clone();
        recipe.focus = [75, 22];
        recipe.speed = 145;
        let background = BackgroundSettings {
            motion: recipe.motion,
            source: BackgroundSource::Dynamic(recipe),
            ..BackgroundSettings::default()
        };
        save(
            &project,
            &Timeline::default(),
            0,
            CanvasFormat::Original,
            &background,
        )
        .unwrap();
        assert_eq!(load(&project).unwrap().3, background);
    }

    #[test]
    fn restores_background_settings_and_resolves_the_image_from_the_project() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("背景.takeproj");
        let mut background = BackgroundSettings {
            source: BackgroundSource::Image("墙纸.png".into()),
            motion: BackgroundMotion::Wave,
            frame: BackgroundFrame::Window,
            blur: 23,
            ..BackgroundSettings::default()
        };
        save(
            &project,
            &Timeline::default(),
            0,
            CanvasFormat::Original,
            &background,
        )
        .unwrap();
        background.source = BackgroundSource::Image(directory.path().join("墙纸.png"));
        assert_eq!(load(&project).unwrap().3, background);
    }

    #[test]
    fn roundtrip_keeps_trimmed_segments_and_playhead() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("剪辑测试.takeproj");
        let mut timeline = Timeline::default();
        let source = directory.path().join("recording.mp4");
        let id = timeline.append(source, 5_000_000).unwrap();
        let right = timeline.split(id, 1_000_000).unwrap();
        timeline.trim_end(right, 3_500_000, false).unwrap();
        let audio = timeline
            .insert_media_on_track(
                PRIMARY_AUDIO,
                directory.path().join("music.wav"),
                4_000_000,
                250_000,
            )
            .unwrap();
        timeline.set_clip_volume(audio, -95).unwrap();
        let overlay = timeline.add_track(TrackKind::Video).unwrap();
        timeline.set_track_enabled(overlay, false).unwrap();
        save(
            &project,
            &timeline,
            2_123_456,
            CanvasFormat::Original,
            &BackgroundSettings::default(),
        )
        .unwrap();
        let (loaded, position, canvas, _) = load(&project).unwrap();
        assert_eq!(loaded.tracks(), timeline.tracks());
        assert_eq!(position, 2_123_456);
        assert_eq!(canvas, CanvasFormat::Original);
        let json: serde_json::Value =
            serde_json::from_reader(File::open(&project).unwrap()).unwrap();
        assert!(json.get("canvas").is_none());
    }

    #[test]
    fn opens_a_legacy_single_track_project_on_v1() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("legacy.takeproj");
        let legacy = r#"{"version":1,"playhead_us":20,"clips":[{"id":5,"path":"legacy.mp4","media_duration":100,"source_start":10,"source_end":80,"start":0}]}"#;
        fs::write(&project, legacy).unwrap();
        let (timeline, position, canvas, background) = load(&project).unwrap();
        assert_eq!(background, BackgroundSettings::default());
        assert_eq!(position, 20);
        assert_eq!(canvas, CanvasFormat::Original);
        assert_eq!(timeline.track_for_clip(5).unwrap().id, PRIMARY_VIDEO);
        assert_eq!(
            timeline.clip(5).unwrap().path,
            directory.path().join("legacy.mp4")
        );
        assert!(timeline.track(PRIMARY_AUDIO).unwrap().clips.is_empty());
    }

    #[test]
    fn media_below_the_project_uses_portable_relative_paths() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("portable.takeproj");
        let mut timeline = Timeline::default();
        let id = timeline
            .append(directory.path().join("素材").join("video.mp4"), 1_000_000)
            .unwrap();
        let background = BackgroundSettings {
            source: BackgroundSource::Image(directory.path().join("素材").join("background.png")),
            ..BackgroundSettings::default()
        };
        save(&project, &timeline, 0, CanvasFormat::Original, &background).unwrap();

        let document: Document = serde_json::from_reader(File::open(&project).unwrap()).unwrap();
        let tracks = document.tracks.unwrap();
        let clip = tracks.iter().flat_map(|track| &track.clips).next().unwrap();
        assert_eq!(clip.path.to_str(), Some("素材/video.mp4"));
        assert_eq!(
            document.background.unwrap().source,
            BackgroundSource::Image("素材/background.png".into())
        );

        let relocated = tempfile::tempdir().unwrap();
        let copied = relocated.path().join("portable.takeproj");
        fs::copy(&project, &copied).unwrap();
        let (timeline, _, _, background) = load(&copied).unwrap();
        assert_eq!(
            timeline.clip(id).unwrap().path,
            relocated.path().join("素材/video.mp4")
        );
        assert_eq!(
            background.source,
            BackgroundSource::Image(relocated.path().join("素材/background.png"))
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn resolves_legacy_windows_relative_media_without_rewriting_foreign_absolute_paths() {
        let directory = tempfile::tempdir().unwrap();
        let mut relative = PathBuf::from(r"素材\video.mp4");
        resolve_media_path(directory.path(), &mut relative);
        assert_eq!(relative, directory.path().join("素材/video.mp4"));
        let mut foreign = PathBuf::from(r"C:\素材\video.mp4");
        resolve_media_path(directory.path(), &mut foreign);
        assert_eq!(foreign, PathBuf::from(r"C:\素材\video.mp4"));
    }

    #[test]
    fn reopens_timed_text_without_resolving_it_as_a_media_path() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("字幕.takeproj");
        let mut timeline = Timeline::default();
        let text = OverlayText {
            content: "标题\nSecond line".into(),
            style: TextStyle {
                size: 72,
                placement: TextPlacement::Top,
                bold: true,
                position: Some(TextPosition { x: 6_250, y: 3_400 }),
                appearance: TextAppearance::DarkBox,
            },
        };
        let (_, id) = timeline
            .insert_text(TrackKind::Caption, 1_250_000, 2_500_000, text.clone())
            .unwrap();
        save(
            &project,
            &timeline,
            1_500_000,
            CanvasFormat::Original,
            &BackgroundSettings::default(),
        )
        .unwrap();
        let json: serde_json::Value =
            serde_json::from_reader(File::open(&project).unwrap()).unwrap();
        assert_eq!(json["version"], TEXT_VERSION);
        let (loaded, at, canvas, _) = load(&project).unwrap();
        assert_eq!(loaded.tracks(), timeline.tracks());
        assert_eq!(at, 1_500_000);
        assert_eq!(canvas, CanvasFormat::Original);
        let clip = loaded.clip(id).unwrap();
        assert_eq!(clip.text, Some(text));
        assert!(clip.path.as_os_str().is_empty());
    }

    #[test]
    fn preserves_custom_canvas_and_rejects_invalid_dimensions() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("自定义画幅.takeproj");
        let timeline = Timeline::default();
        let canvas = CanvasFormat::Custom {
            width: 800,
            height: 1280,
        };
        save(
            &project,
            &timeline,
            0,
            canvas,
            &BackgroundSettings::default(),
        )
        .unwrap();
        assert_eq!(load(&project).unwrap().2, canvas);

        let invalid = CanvasFormat::Custom {
            width: 0,
            height: 1280,
        };
        assert!(
            save(
                &project,
                &timeline,
                0,
                invalid,
                &BackgroundSettings::default()
            )
            .is_err()
        );
        assert_eq!(load(&project).unwrap().2, canvas);

        let mut document: Document =
            serde_json::from_reader(File::open(&project).unwrap()).unwrap();
        document.canvas = Some(invalid);
        serde_json::to_writer(File::create(&project).unwrap(), &document).unwrap();
        assert!(load(&project).is_err());
    }
}
