use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use eframe::egui;
use takeseam::editor_timeline::{Clip, Track, TrackKind};

use super::background::BackgroundSettings;
use super::canvas_format::CanvasFormat;
use super::media::background;
use super::media::overlays::{Overlays, SCRIPT_NAME, sequence_size};
use super::media::{
    MediaInfo, MediaTools, append_segment_audio_filters, append_video_fades, hidden_command,
    local_media, sequence_filter,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExportQuality {
    High,
    Standard,
    Small,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExportResolution {
    Canvas,
    Hd,
    FullHd,
    UltraHd,
}

impl Default for ExportResolution {
    fn default() -> Self {
        Self::Canvas
    }
}

impl ExportResolution {
    fn dimensions(self, source: (u32, u32)) -> (u32, u32) {
        let (target_width, target_height) = match self {
            Self::Canvas => return source,
            Self::Hd => (1280, 720),
            Self::FullHd => (1920, 1080),
            Self::UltraHd => (3840, 2160),
        };
        let (target_width, target_height) = if source.1 > source.0 {
            (target_height, target_width)
        } else {
            (target_width, target_height)
        };
        let scale = f64::min(
            f64::from(target_width) / f64::from(source.0),
            f64::from(target_height) / f64::from(source.1),
        );
        let width = (f64::from(source.0) * scale).round() as u32;
        let height = (f64::from(source.1) * scale).round() as u32;
        (width.max(2) & !1, height.max(2) & !1)
    }
}

impl ExportQuality {
    pub(super) fn crf(self) -> u8 {
        match self {
            Self::High => 16,
            Self::Standard => 19,
            Self::Small => 23,
        }
    }

    pub(super) fn preset(self) -> &'static str {
        match self {
            Self::High => "slow",
            Self::Standard => "veryfast",
            Self::Small => "faster",
        }
    }

    pub(super) fn bitrate(self) -> &'static str {
        match self {
            Self::High => "12M",
            Self::Standard => "8M",
            Self::Small => "5M",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct ExportSettings {
    pub title: String,
    pub output: PathBuf,
    pub output_text: String,
    pub fps: u32,
    pub quality: ExportQuality,
    pub resolution: ExportResolution,
    pub audio: bool,
    pub audio_bitrate: u32,
    pub include_background: bool,
    pub include_subtitles: bool,
}

impl ExportSettings {
    pub(super) fn new(directory: &Path) -> Self {
        let title = format!("{}-剪辑", takeseam::PRODUCT_NAME);
        let output = directory.join(format!("{title}.mp4"));
        Self {
            title,
            output_text: output.to_string_lossy().into_owned(),
            output,
            fps: 30,
            quality: ExportQuality::Standard,
            resolution: ExportResolution::Canvas,
            audio: true,
            audio_bitrate: 192,
            include_background: true,
            include_subtitles: true,
        }
    }

    pub(super) fn resolve_output(&mut self, directory: &Path) -> Result<()> {
        let text = self.output_text.trim();
        ensure!(!text.is_empty(), "请输入导出文件路径");
        let mut output = PathBuf::from(text);
        if !output.is_absolute() {
            output = directory.join(output);
        }
        if output.extension().is_none() {
            output.set_extension("mp4");
        }
        ensure!(
            output
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4")),
            "当前格式仅支持 .mp4"
        );
        self.output = output;
        self.output_text = self.output.to_string_lossy().into_owned();
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct ExportJob {
    active: Option<Worker>,
    progress: f32,
}

struct Worker {
    cancel: Arc<AtomicBool>,
    updates: Receiver<Update>,
}

enum Update {
    Progress(f32),
    Finished(Result<PathBuf>),
}

impl ExportJob {
    pub(super) fn start(
        &mut self,
        ctx: &egui::Context,
        tools: MediaTools,
        tracks: Vec<Track>,
        settings: ExportSettings,
        canvas_format: CanvasFormat,
        backdrop: BackgroundSettings,
    ) -> Result<()> {
        ensure!(!self.running(), "已有导出任务");
        ensure!(
            tracks.iter().any(|track| !track.clips.is_empty()),
            "请先在时间线上添加片段"
        );
        let (tx, updates) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let wake = ctx.clone();
        thread::Builder::new()
            .name(format!("{} export", takeseam::PRODUCT_NAME))
            .spawn(move || {
                let output = settings.output.clone();
                let result = export_with_settings(
                    &tools,
                    &tracks,
                    &settings,
                    canvas_format,
                    &backdrop,
                    &worker_cancel,
                    |completion| {
                        let _ = tx.send(Update::Progress(completion));
                        wake.request_repaint();
                    },
                );
                let _ = tx.send(Update::Finished(result.map(|()| output)));
                wake.request_repaint();
            })
            .context("无法启动导出")?;
        self.active = Some(Worker { cancel, updates });
        self.progress = 0.0;
        Ok(())
    }

    pub(super) fn running(&self) -> bool {
        self.active.is_some()
    }

    pub(super) fn progress(&self) -> f32 {
        self.progress
    }

    pub(super) fn cancel(&self) {
        if let Some(worker) = &self.active {
            worker.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub(super) fn poll(&mut self) -> Option<Result<PathBuf>> {
        while let Some(worker) = &self.active {
            match worker.updates.try_recv() {
                Ok(Update::Progress(progress)) => self.progress = progress,
                Ok(Update::Finished(result)) => {
                    self.active = None;
                    return Some(result);
                }
                Err(mpsc::TryRecvError::Empty) => return None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.active = None;
                    return Some(Err(anyhow::anyhow!("导出任务意外停止")));
                }
            }
        }
        None
    }
}

impl Drop for ExportJob {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
fn export(
    tools: &MediaTools,
    tracks: &[Track],
    output: &Path,
    canvas_format: CanvasFormat,
    backdrop: &BackgroundSettings,
    cancel: &AtomicBool,
    progress: impl FnMut(f32),
) -> Result<()> {
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let mut settings = ExportSettings::new(parent);
    settings.output = output.to_path_buf();
    settings.output_text = output.to_string_lossy().into_owned();
    export_with_settings(
        tools,
        tracks,
        &settings,
        canvas_format,
        backdrop,
        cancel,
        progress,
    )
}

fn export_with_settings(
    tools: &MediaTools,
    tracks: &[Track],
    settings: &ExportSettings,
    canvas_format: CanvasFormat,
    backdrop: &BackgroundSettings,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
) -> Result<()> {
    ensure!(
        settings
            .output
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4")),
        "导出文件扩展名必须是 .mp4",
    );
    ensure!(
        !settings.output.try_exists()?,
        "输出文件已存在，请换个文件名"
    );
    ensure!(backdrop.is_valid(), "背景设置无效");
    let parent = settings
        .output
        .parent()
        .filter(|path| path.is_dir())
        .context("输出目录不可用")?;
    let stage = tempfile::Builder::new()
        .prefix(".takeseam-export-")
        .tempdir_in(parent)?;
    let stage_dir = std::path::absolute(stage.path())?;
    let log = stage_dir.join("ffmpeg.log");
    let length = tracks
        .iter()
        .flat_map(|track| &track.clips)
        .map(Clip::end)
        .max()
        .filter(|length| *length > 0)
        .context("时间线为空")?;
    let media = probe_tracks(tools, tracks, cancel)?;
    ensure!(canvas_format.is_valid(), "画幅尺寸无效");
    let canvas_dimensions = canvas_format
        .dimensions()
        .unwrap_or_else(|| canvas_size(tracks, &media));
    let (width, height) = settings.resolution.dimensions(canvas_dimensions);
    if settings.include_background && backdrop.enabled() {
        background::stage(backdrop, &stage_dir, (width, height))?;
    }
    let profile = Profile {
        width,
        height,
        encoder: encoder(tools)?,
        fps: settings.fps,
        quality: settings.quality,
        crf: settings.quality.crf(),
        audio: settings.audio,
    };
    let disabled_background = BackgroundSettings::default();
    let process = RenderContext {
        tools,
        stage: &stage_dir,
        log: &log,
        cancel,
        backdrop: if settings.include_background {
            backdrop
        } else {
            &disabled_background
        },
        include_subtitles: settings.include_subtitles,
    };
    let mut boundaries = vec![0, length];
    boundaries.extend(
        tracks
            .iter()
            .filter(|track| track.enabled && !track.kind.is_overlay())
            .flat_map(|track| &track.clips)
            .flat_map(|clip| [clip.start, clip.end()]),
    );
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut manifest_text = String::from("ffconcat version 1.0\n");
    for (index, interval) in boundaries.windows(2).enumerate() {
        check_cancel(cancel)?;
        let [start, end] = [interval[0], interval[1]];
        let segment = Segment::at(tracks, &media, start, end);
        let filename = format!("part-{index:06}.mov");
        render_segment(&process, &profile, &segment, &filename)
            .with_context(|| format!("片段 {} 导出失败", index + 1))?;
        let duration = seconds(end - start);
        let _ = write!(manifest_text, "file '{filename}'\nduration {duration}\n");
        progress((end as f64 / length as f64 * 0.93) as f32);
    }
    check_cancel(cancel)?;
    let manifest = stage.path().join("segments.ffconcat");
    fs::write(&manifest, manifest_text)?;
    let final_file = tempfile::Builder::new()
        .prefix(".takeseam-")
        .suffix(".mp4")
        .tempfile_in(parent)?
        .into_temp_path();
    let mut final_command = command(tools);
    final_command
        .args([
            "-f",
            "concat",
            "-safe",
            "1",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
        ])
        .arg(manifest)
        .args(["-map", "0:v:0", "-c:v", "copy", "-movflags", "+faststart"]);
    if settings.audio {
        final_command.args([
            "-map",
            "0:a:0",
            "-c:a",
            "aac",
            "-b:a",
            &format!("{}k", settings.audio_bitrate),
        ]);
    } else {
        final_command.arg("-an");
    }
    final_command.arg(final_file.as_os_str());
    run(&mut final_command, &log, cancel).context("无法合并导出片段")?;
    check_cancel(cancel)?;
    ensure!(fs::metadata(&final_file)?.len() > 0, "导出文件为空");
    final_file
        .persist_noclobber(&settings.output)
        .map_err(|error| error.error)
        .context("无法保存导出视频")?;
    progress(1.0);
    Ok(())
}

fn probe_tracks<'a>(
    tools: &MediaTools,
    tracks: &'a [Track],
    cancel: &AtomicBool,
) -> Result<HashMap<&'a Path, MediaInfo>> {
    let mut media = HashMap::new();
    for track in tracks
        .iter()
        .filter(|track| track.enabled && !track.kind.is_overlay())
    {
        for clip in &track.clips {
            check_cancel(cancel)?;
            let info = match media.entry(clip.path.as_path()) {
                std::collections::hash_map::Entry::Occupied(cached) => cached.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => entry.insert(
                    tools
                        .probe(&clip.path)
                        .with_context(|| format!("无法读取素材 {}", clip.path.display()))?,
                ),
            };
            ensure!(
                clip.source_end <= info.duration_us.saturating_add(1_000),
                "素材 {} 的长度已变化，请重新导入",
                clip.path.display(),
            );
            match track.kind {
                TrackKind::Video => ensure!(
                    info.has_video,
                    "视频轨上的素材 {} 没有视频流",
                    clip.path.display()
                ),
                TrackKind::Audio => ensure!(
                    info.has_audio,
                    "音频轨上的素材 {} 没有音频流",
                    clip.path.display()
                ),
                TrackKind::Caption | TrackKind::Text => {
                    unreachable!("only media tracks are probed")
                }
            }
        }
    }
    Ok(media)
}

fn canvas_size(tracks: &[Track], media: &HashMap<&Path, MediaInfo>) -> (u32, u32) {
    let source = tracks
        .iter()
        .rev()
        .filter(|track| track.enabled && track.kind == TrackKind::Video)
        .find_map(|track| track.clips.first())
        .map(|clip| media[clip.path.as_path()]);
    sequence_size(source)
}

struct Segment<'a> {
    start: u64,
    end: u64,
    video: Option<&'a Clip>,
    audio: Vec<&'a Clip>,
    overlays: Overlays,
}

impl<'a> Segment<'a> {
    fn at(tracks: &'a [Track], media: &HashMap<&Path, MediaInfo>, start: u64, end: u64) -> Self {
        let mut video = None;
        let mut audio = Vec::new();
        for track in tracks
            .iter()
            .filter(|track| track.enabled && !track.kind.is_overlay())
        {
            let Some(clip) = track.clip_at(start) else {
                continue;
            };
            if track.kind == TrackKind::Video && video.is_none() {
                video = Some(clip);
            }
            if media[clip.path.as_path()].has_audio {
                audio.push(clip);
            }
        }
        Self {
            start,
            end,
            video,
            audio,
            overlays: Overlays::in_range(tracks, start..end),
        }
    }

    fn source_position(&self, clip: &Clip) -> u64 {
        clip.source_time_at(self.start)
    }
}

struct Profile {
    width: u32,
    height: u32,
    encoder: &'static str,
    fps: u32,
    quality: ExportQuality,
    crf: u8,
    audio: bool,
}

struct RenderContext<'a> {
    tools: &'a MediaTools,
    stage: &'a Path,
    log: &'a Path,
    cancel: &'a AtomicBool,
    backdrop: &'a BackgroundSettings,
    include_subtitles: bool,
}

fn render_segment(
    process: &RenderContext<'_>,
    profile: &Profile,
    segment: &Segment<'_>,
    filename: &str,
) -> Result<()> {
    let mut ffmpeg = command(process.tools);
    let duration = seconds(segment.end - segment.start);
    let width = profile.width;
    let height = profile.height;
    if let Some(clip) = segment.video {
        input_clip(&mut ffmpeg, segment, clip, false)?;
    } else {
        ffmpeg.args(["-f", "lavfi", "-i"]).arg(format!(
            "color=c=black:s={width}x{height}:r={}",
            profile.fps
        ));
    }
    let audio_map = profile
        .audio
        .then(|| audio_mix(&mut ffmpeg, segment, &duration))
        .transpose()?;
    let mut input_filter = format!("fps={}:start_time=0", profile.fps);
    if let Some(clip) = segment.video {
        if clip.reverse {
            input_filter.push_str(",reverse");
        }
        if clip.speed_milli != 1000 {
            let _ = write!(
                input_filter,
                ",setpts=PTS/{:.3}",
                f32::from(clip.speed_milli) / 1000.0
            );
        }
        append_video_fades(
            &mut input_filter,
            segment.start,
            clip.start,
            clip.duration(),
            clip.fade_in_us,
            clip.fade_out_us,
        );
    }
    let _ = write!(
        input_filter,
        ",tpad=stop_mode=clone:stop_duration={duration}"
    );
    let mut output_filter = String::from("format=yuv420p");
    if process.include_subtitles
        && segment.overlays.write_for_playback(
            process.stage,
            (width, height),
            segment.start..segment.end,
        )?
    {
        ffmpeg.current_dir(process.stage);
        let _ = write!(output_filter, ",ass=filename={SCRIPT_NAME}");
    }
    let video = if process.backdrop.enabled() {
        ffmpeg
            .current_dir(process.stage)
            .args(["-filter_threads", "1"]);
        background::filter(
            process.backdrop,
            (width, height),
            (width, height),
            segment.start,
            segment.video.is_some(),
            &input_filter,
            &output_filter,
        )?
    } else {
        format!(
            "{},{input_filter},{output_filter}",
            sequence_filter(width, height)
        )
    };
    ffmpeg.args(["-t", &duration, "-map", "0:V:0"]);
    if let Some(audio_map) = audio_map {
        ffmpeg.args(["-map", audio_map]);
    }
    ffmpeg.args(["-vf", &video, "-c:v", profile.encoder]);
    if profile.encoder == "libx264" {
        ffmpeg.args([
            "-preset",
            profile.quality.preset(),
            "-crf",
            &profile.crf.to_string(),
        ]);
    } else {
        ffmpeg.args(["-b:v", profile.quality.bitrate()]);
    }
    ffmpeg.args(["-pix_fmt", "yuv420p", "-bf", "0"]);
    if profile.audio {
        ffmpeg.args(["-c:a", "pcm_s16le", "-ar", "48000", "-ac", "2"]);
    } else {
        ffmpeg.arg("-an");
    }
    ffmpeg.args(["-video_track_timescale", "90000"]);
    ffmpeg.arg(process.stage.join(filename));
    run(&mut ffmpeg, process.log, process.cancel)
}

fn input_clip(
    command: &mut Command,
    segment: &Segment<'_>,
    clip: &Clip,
    audio_only: bool,
) -> Result<()> {
    command.args(["-protocol_whitelist", "file,pipe", "-sn", "-dn"]);
    if audio_only {
        command.arg("-vn");
    }
    command
        .arg("-ss")
        .arg(seconds(segment.source_position(clip)))
        .arg("-i")
        .arg(local_media(&clip.path)?);
    Ok(())
}

fn audio_mix(command: &mut Command, segment: &Segment<'_>, duration: &str) -> Result<&'static str> {
    if segment.audio.is_empty() {
        command.args([
            "-f",
            "lavfi",
            "-i",
            "anullsrc=channel_layout=stereo:sample_rate=48000",
        ]);
        return Ok("1:a:0");
    }
    let mut graph = String::new();
    let mut input = 1;
    let count = segment.audio.len();
    for (index, clip) in segment.audio.iter().enumerate() {
        let stream = if segment.video.is_some_and(|video| video.id == clip.id) {
            0
        } else {
            input_clip(command, segment, clip, true)?;
            input += 1;
            input - 1
        };
        let _ = write!(
            graph,
            "[{stream}:a:0]aresample=48000:async=1:first_pts=0,aformat=sample_fmts=fltp:channel_layouts=stereo,apad=whole_dur={duration}"
        );
        append_segment_audio_filters(
            &mut graph,
            clip.volume_db_tenths,
            clip.fade_in_us,
            clip.fade_out_us,
            segment.start.saturating_sub(clip.start),
            clip.duration(),
            segment.end.saturating_sub(segment.start),
            clip.speed_milli,
            clip.reverse,
        );
        if count == 1 {
            graph.push_str("[mix]");
        } else {
            let _ = write!(graph, "[a{index}];");
        }
    }
    if count > 1 {
        for index in 0..count {
            let _ = write!(graph, "[a{index}]");
        }
        let _ = write!(
            graph,
            "amix=inputs={count}:normalize=0,alimiter=limit=0.95:level=0:latency=1[mix]"
        );
    }
    command.arg("-filter_complex").arg(graph);
    Ok("[mix]")
}

fn encoder(tools: &MediaTools) -> Result<&'static str> {
    let result = hidden_command(&tools.ffmpeg)
        .args(["-hide_banner", "-encoders"])
        .output()
        .context("无法读取 MP4 编码器")?;
    let names = String::from_utf8_lossy(&result.stdout);
    for name in [
        "libx264",
        #[cfg(windows)]
        "h264_mf",
        #[cfg(target_os = "macos")]
        "h264_videotoolbox",
    ] {
        if names
            .lines()
            .any(|line| line.split_whitespace().nth(1) == Some(name))
        {
            return Ok(name);
        }
    }
    bail!("本机 FFmpeg 缺少 H.264 编码器")
}

fn command(tools: &MediaTools) -> Command {
    let mut ffmpeg = hidden_command(&tools.ffmpeg);
    ffmpeg
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .stdout(Stdio::null());
    ffmpeg
}

fn run(command: &mut Command, log: &Path, cancel: &AtomicBool) -> Result<()> {
    check_cancel(cancel)?;
    let mut process = command
        .stderr(Stdio::from(File::create(log)?))
        .spawn()
        .context("无法启动 FFmpeg")?;
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = process.kill();
            let _ = process.wait();
            bail!("已取消导出");
        }
        if let Some(status) = process.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_millis(35));
    };
    if !status.success() {
        let text = fs::read_to_string(log).unwrap_or_default();
        let reason: String = text.trim().chars().take(260).collect();
        bail!("FFmpeg 导出失败（{reason}）");
    }
    Ok(())
}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Relaxed), "已取消导出");
    Ok(())
}

fn seconds(microseconds: u64) -> String {
    format!(
        "{}.{:06}",
        microseconds / 1_000_000,
        microseconds % 1_000_000
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::background::{BackdropPreset, BackgroundMotion, BackgroundSource};
    use crate::editor::canvas_format::AspectRatio;
    use crate::editor::dynamic_recipe::DynamicRecipe;
    use takeseam::editor_timeline::{
        OverlayText, PRIMARY_AUDIO, TextAppearance, TextPlacement, TextPosition, TextStyle,
        Timeline,
    };

    #[test]
    fn export_settings_resolve_relative_mp4_and_keep_quality_options() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = ExportSettings::new(directory.path());
        settings.output_text = "final.mp4".into();
        settings.resolve_output(directory.path()).unwrap();
        assert_eq!(settings.output, directory.path().join("final.mp4"));
        assert_eq!(ExportQuality::High.crf(), 16);
        assert_eq!(ExportQuality::Standard.crf(), 19);
        assert_eq!(ExportQuality::Small.crf(), 23);
    }

    #[test]
    fn export_settings_reject_non_mp4_output() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = ExportSettings::new(directory.path());
        settings.output_text = "final.webm".into();
        assert!(settings.resolve_output(directory.path()).is_err());
    }

    #[test]
    fn export_resolution_preserves_landscape_and_portrait_aspect() {
        assert_eq!(
            ExportResolution::FullHd.dimensions((1920, 1080)),
            (1920, 1080)
        );
        assert_eq!(
            ExportResolution::FullHd.dimensions((1080, 1920)),
            (1080, 1920)
        );
        assert_eq!(ExportResolution::Hd.dimensions((1920, 1080)), (1280, 720));
    }

    #[test]
    #[ignore = "requires local FFmpeg and FFprobe"]
    fn export_animates_the_background_behind_the_video_across_a_cut() {
        let tools = MediaTools::discover().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("recording.mp4");
        let cancel = AtomicBool::new(false);
        run(
            command(&tools)
                .args(["-f", "lavfi", "-i", "color=red:s=320x180:r=30"])
                .args(["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"])
                .args([
                    "-t",
                    "1.2",
                    "-c:v",
                    encoder(&tools).unwrap(),
                    "-c:a",
                    "aac",
                    "-pix_fmt",
                    "yuv420p",
                ])
                .arg(&source),
            &directory.path().join("test.log"),
            &cancel,
        )
        .unwrap();

        let mut timeline = Timeline::default();
        let clip = timeline.append(source, 1_200_000).unwrap();
        timeline.split(clip, 600_000).unwrap();
        let meteor = DynamicRecipe::modern_presets()[4].clone();
        for (index, (source, motion)) in [
            (
                BackgroundSource::Wallpaper(BackdropPreset::Tide),
                BackgroundMotion::Wave,
            ),
            (BackgroundSource::Dynamic(meteor.clone()), meteor.motion),
        ]
        .into_iter()
        .enumerate()
        {
            let output = directory.path().join(format!("backdrop-{index}.mp4"));
            let background = BackgroundSettings {
                source,
                motion,
                inset: 18,
                ..BackgroundSettings::default()
            };
            export(
                &tools,
                timeline.tracks(),
                &output,
                CanvasFormat::Original,
                &background,
                &cancel,
                |_| {},
            )
            .unwrap();
            assert!(tools.probe(&output).unwrap().has_audio);
            let pixels_at = |at| {
                let frame = command(&tools)
                    .args(["-ss", at, "-i"])
                    .arg(&output)
                    .args([
                        "-frames:v",
                        "1",
                        "-pix_fmt",
                        "rgb24",
                        "-f",
                        "rawvideo",
                        "pipe:1",
                    ])
                    .stdout(Stdio::piped())
                    .output()
                    .unwrap();
                assert!(frame.status.success());
                assert_eq!(frame.stdout.len(), 320 * 180 * 3);
                let center = (320 * 90 + 160) * 3;
                assert!(frame.stdout[center] > 200 && frame.stdout[center + 2] < 30);
                frame.stdout
            };
            let early = pixels_at("0.10");
            let late = pixels_at("1.00");
            assert!(
                early
                    .iter()
                    .zip(late)
                    .filter(|(a, b)| a.abs_diff(*b) > 4)
                    .count()
                    > 20
            );
        }
    }

    #[test]
    #[ignore = "requires local FFmpeg and FFprobe"]
    fn portrait_export_fits_landscape_video_inside_the_selected_canvas() {
        let tools = MediaTools::discover().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("landscape.mp4");
        let output = directory.path().join("portrait.mp4");
        let log = directory.path().join("test.log");
        let cancel = AtomicBool::new(false);
        run(
            command(&tools)
                .args([
                    "-f",
                    "lavfi",
                    "-i",
                    "color=red:s=320x180:r=30",
                    "-t",
                    "0.30",
                ])
                .args(["-c:v", encoder(&tools).unwrap(), "-pix_fmt", "yuv420p"])
                .arg(&source),
            &log,
            &cancel,
        )
        .unwrap();
        let mut timeline = Timeline::default();
        timeline.append(source, 300_000).unwrap();
        export(
            &tools,
            timeline.tracks(),
            &output,
            CanvasFormat::Preset(AspectRatio::Portrait),
            &BackgroundSettings::default(),
            &cancel,
            |_| {},
        )
        .unwrap();

        let info = tools.probe(&output).unwrap();
        assert_eq!((info.width, info.height), (1080, 1920));
        let frame = command(&tools)
            .arg("-i")
            .arg(output)
            .args([
                "-vf",
                "scale=108:192",
                "-frames:v",
                "1",
                "-pix_fmt",
                "rgb24",
                "-f",
                "rawvideo",
                "pipe:1",
            ])
            .stdout(Stdio::piped())
            .output()
            .unwrap();
        assert!(frame.status.success());
        assert_eq!(frame.stdout.len(), 108 * 192 * 3);
        let center = (96 * 108 + 54) * 3;
        assert!(frame.stdout[center] > 200);
        assert!(frame.stdout[0..3].iter().all(|&channel| channel < 16));
    }

    #[test]
    #[ignore = "requires local FFmpeg with libass and FFprobe"]
    fn export_burns_in_timed_subtitles_and_titles() {
        let tools = MediaTools::discover().unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("video.mp4");
        let log = temporary.path().join("test.log");
        let output = temporary.path().join("成片文字.mp4");
        let cancel = AtomicBool::new(false);
        run(
            command(&tools)
                .args(["-f", "lavfi", "-i", "color=red:s=320x180:r=30", "-t", "1.2"])
                .args(["-c:v", encoder(&tools).unwrap()])
                .arg(&source),
            &log,
            &cancel,
        )
        .unwrap();

        let mut timeline = Timeline::default();
        timeline.append(source, 1_200_000).unwrap();
        for (kind, start, duration, placement, content) in [
            (
                TrackKind::Caption,
                200_000,
                500_000,
                TextPlacement::Bottom,
                "中文字幕",
            ),
            (
                TrackKind::Text,
                400_000,
                500_000,
                TextPlacement::Top,
                "标题\nTitle",
            ),
        ] {
            timeline
                .insert_text(
                    kind,
                    start,
                    duration,
                    OverlayText {
                        content: content.into(),
                        style: TextStyle {
                            size: 100,
                            placement,
                            bold: true,
                            ..TextStyle::default()
                        },
                    },
                )
                .unwrap();
        }
        timeline
            .insert_text(
                TrackKind::Caption,
                500_000,
                250_000,
                OverlayText {
                    content: "金色".into(),
                    style: TextStyle {
                        size: 110,
                        position: Some(TextPosition { x: 2000, y: 4200 }),
                        appearance: TextAppearance::Gold,
                        ..TextStyle::default()
                    },
                },
            )
            .unwrap();
        export(
            &tools,
            timeline.tracks(),
            &output,
            CanvasFormat::Original,
            &BackgroundSettings::default(),
            &cancel,
            |_| {},
        )
        .unwrap();
        for (position, should_have_top, should_have_bottom, should_have_gold) in [
            ("0.10", false, false, false),
            ("0.32", false, true, false),
            ("0.52", true, true, true),
            ("0.82", true, false, false),
            ("1.05", false, false, false),
        ] {
            let frame = command(&tools)
                .args(["-ss", position, "-i"])
                .arg(&output)
                .args([
                    "-frames:v",
                    "1",
                    "-pix_fmt",
                    "rgb24",
                    "-f",
                    "rawvideo",
                    "pipe:1",
                ])
                .stdout(Stdio::piped())
                .output()
                .unwrap();
            assert!(
                frame.status.success(),
                "{}",
                String::from_utf8_lossy(&frame.stderr)
            );
            let row_bytes = 320 * 3;
            assert_eq!(frame.stdout.len(), row_bytes * 180);
            let has_white = |rows: std::ops::Range<usize>| {
                frame.stdout[rows.start * row_bytes..rows.end * row_bytes]
                    .chunks_exact(3)
                    .filter(|pixel| pixel.iter().all(|channel| *channel > 190))
                    .count()
                    > 15
            };
            assert_eq!(has_white(0..90), should_have_top, "{position} seconds");
            assert_eq!(has_white(90..180), should_have_bottom, "{position} seconds");
            let gold_positions = frame
                .stdout
                .chunks_exact(3)
                .enumerate()
                .filter(|(_, pixel)| {
                    pixel[0] > 190 && pixel[1] > 150 && pixel[1].saturating_sub(pixel[2]) > 45
                })
                .map(|(index, _)| (index % 320, index / 320))
                .collect::<Vec<_>>();
            assert_eq!(
                gold_positions.len() > 15,
                should_have_gold,
                "{position} seconds"
            );
            assert!(
                gold_positions
                    .iter()
                    .all(|(x, y)| (32..96).contains(x) && (62..88).contains(y))
            );
        }
    }

    #[test]
    #[ignore = "requires local FFmpeg and FFprobe"]
    fn export_preserves_clip_order_gaps_and_audio() {
        let tools = MediaTools::discover().unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let red = temporary.path().join("red.mp4");
        let green = temporary.path().join("green.mp4");
        let log = temporary.path().join("test.log");
        let cancel = AtomicBool::new(false);
        let codec = encoder(&tools).unwrap();
        for (path, color, audio) in [(&red, "red", true), (&green, "lime", false)] {
            let mut input = command(&tools);
            input
                .args(["-f", "lavfi", "-i"])
                .arg(format!("color=c={color}:s=320x180:r=30"));
            if audio {
                input.args([
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:sample_rate=48000",
                    "-c:a",
                    "aac",
                ]);
            }
            input
                .args(["-t", "1.2", "-c:v", codec, "-pix_fmt", "yuv420p"])
                .arg(path);
            run(&mut input, &log, &cancel).unwrap();
        }
        let tracks = vec![video_track(
            1,
            true,
            vec![
                Clip {
                    id: 1,
                    path: red,
                    media_duration: 1_200_000,
                    source_start: 200_000,
                    source_end: 800_000,
                    start: 0,
                    text: None,
                    volume_db_tenths: 0,
                    transition_in: None,
            fade_in_us: 0,
            fade_out_us: 0,
            speed_milli: 1000,
            reverse: false,
            timeline_duration_us: 0,
            group_id: None,
                },
                Clip {
                    id: 2,
                    path: green,
                    media_duration: 1_200_000,
                    source_start: 100_000,
                    source_end: 700_000,
                    start: 900_000,
                    text: None,
                    volume_db_tenths: 0,
                    transition_in: None,
            fade_in_us: 0,
            fade_out_us: 0,
            speed_milli: 1000,
            reverse: false,
            timeline_duration_us: 0,
            group_id: None,
                },
            ],
        )];
        let output = temporary.path().join("测试成片.mp4");
        export(
            &tools,
            &tracks,
            &output,
            CanvasFormat::Original,
            &BackgroundSettings::default(),
            &cancel,
            |_| {},
        )
        .unwrap();
        let info = tools.probe(&output).unwrap();
        assert!(info.has_audio);
        assert!(
            info.duration_us.abs_diff(1_500_000) <= 34_000,
            "duration {}",
            info.duration_us
        );
        assert_eq!((info.width, info.height), (320, 180));
        assert_frames(
            &tools,
            &output,
            &[
                ("0.25", [250_u8, 0, 0]),
                ("0.75", [0, 0, 0]),
                ("1.21", [0, 250, 0]),
            ],
        );
        assert!(
            export(
                &tools,
                &tracks,
                &output,
                CanvasFormat::Original,
                &BackgroundSettings::default(),
                &cancel,
                |_| {}
            )
            .is_err()
        );
    }

    #[test]
    #[ignore = "requires local FFmpeg and FFprobe"]
    fn multitrack_export_uses_top_video_and_mixes_enabled_audio() {
        let tools = MediaTools::discover().unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let log = temporary.path().join("test.log");
        let cancel = AtomicBool::new(false);
        let red = temporary.path().join("base-red.mp4");
        let green = temporary.path().join("overlay-green.mp4");
        let music = temporary.path().join("standalone.wav");
        let muted = temporary.path().join("muted.wav");
        let codec = encoder(&tools).unwrap();
        for (path, color, frequency) in [(&red, "red", Some(440)), (&green, "lime", None)] {
            let mut input = command(&tools);
            input
                .args(["-f", "lavfi", "-i"])
                .arg(format!("color=c={color}:s=320x180:r=30"));
            if let Some(hz) = frequency {
                input
                    .args(["-f", "lavfi", "-i"])
                    .arg(format!("sine=frequency={hz}:sample_rate=48000"))
                    .args(["-c:a", "aac"]);
            }
            input
                .args(["-t", "1.2", "-c:v", codec, "-pix_fmt", "yuv420p"])
                .arg(path);
            run(&mut input, &log, &cancel).unwrap();
        }
        for (path, hz) in [(&music, 880), (&muted, 1760)] {
            run(
                command(&tools)
                    .args(["-f", "lavfi", "-i"])
                    .arg(format!("sine=frequency={hz}:sample_rate=48000"))
                    .args(["-t", "1.2"])
                    .arg(path),
                &log,
                &cancel,
            )
            .unwrap();
        }

        let clip = |id, path, start, source_start, source_end| Clip {
            id,
            path,
            start,
            media_duration: 1_200_000,
            source_start,
            source_end,
            text: None,
            volume_db_tenths: 0,
            transition_in: None,
            fade_in_us: 0,
            fade_out_us: 0,
            speed_milli: 1000,
            reverse: false,
            timeline_duration_us: 0,
            group_id: None,
        };
        let tracks = vec![
            video_track(
                4,
                false,
                vec![clip(
                    4,
                    temporary.path().join("unavailable.mp4"),
                    1_200_000,
                    0,
                    200_000,
                )],
            ),
            video_track(3, true, vec![clip(3, green, 400_000, 150_000, 550_000)]),
            video_track(1, true, vec![clip(1, red, 0, 0, 1_200_000)]),
            Track {
                id: 2,
                kind: TrackKind::Audio,
                enabled: true,
                clips: vec![clip(2, music, 350_000, 200_000, 800_000)],
            },
            Track {
                id: 5,
                kind: TrackKind::Audio,
                enabled: false,
                clips: vec![clip(5, muted, 0, 0, 1_200_000)],
            },
        ];
        let output = temporary.path().join("多轨测试成片.mp4");
        export(
            &tools,
            &tracks,
            &output,
            CanvasFormat::Original,
            &BackgroundSettings::default(),
            &cancel,
            |_| {},
        )
        .unwrap();
        let info = tools.probe(&output).unwrap();
        assert!(
            info.duration_us.abs_diff(1_400_000) <= 34_000,
            "duration {}",
            info.duration_us
        );
        assert_frames(
            &tools,
            &output,
            &[
                ("0.20", [250, 0, 0]),
                ("0.55", [0, 250, 0]),
                ("1.04", [250, 0, 0]),
                ("1.28", [0, 0, 0]),
            ],
        );
        let before_overlap = audio_samples(&tools, &output, "0.08");
        let overlap = audio_samples(&tools, &output, "0.52");
        assert!(tone(&before_overlap, 440.0) > 0.04);
        assert!(tone(&before_overlap, 880.0) < 0.008);
        assert!(
            tone(&overlap, 440.0) > 0.04,
            "covered video's audio is retained"
        );
        assert!(
            tone(&overlap, 880.0) > 0.04,
            "independent audio overlaps the original audio"
        );
        assert!(tone(&overlap, 1760.0) < 0.008, "muted track is silent");

        let audio_only = temporary.path().join("仅音频轨.mp4");
        export(
            &tools,
            &tracks[3..],
            &audio_only,
            CanvasFormat::Original,
            &BackgroundSettings::default(),
            &cancel,
            |_| {},
        )
        .unwrap();
        assert_frames(&tools, &audio_only, &[("0.55", [0, 0, 0])]);
        assert!(tone(&audio_samples(&tools, &audio_only, "0.52"), 880.0) > 0.04);
    }

    #[test]
    #[ignore = "requires local FFmpeg and FFprobe"]
    fn export_applies_volume_to_only_the_adjusted_clip() {
        let tools = MediaTools::discover().unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let log = temporary.path().join("test.log");
        let cancel = AtomicBool::new(false);
        let video = temporary.path().join("video.mp4");
        let audio = temporary.path().join("tone.wav");
        let output = temporary.path().join("gain.mp4");
        run(
            command(&tools)
                .args([
                    "-f",
                    "lavfi",
                    "-i",
                    "color=black:s=320x180:r=30",
                    "-t",
                    "1.2",
                ])
                .args(["-c:v", encoder(&tools).unwrap()])
                .arg(&video),
            &log,
            &cancel,
        )
        .unwrap();
        run(
            command(&tools)
                .args([
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:sample_rate=48000:duration=0.6",
                ])
                .arg(&audio),
            &log,
            &cancel,
        )
        .unwrap();

        let mut timeline = Timeline::default();
        timeline.append(video, 1_200_000).unwrap();
        let adjusted = timeline
            .insert_media_on_track(PRIMARY_AUDIO, audio.clone(), 600_000, 0)
            .unwrap();
        timeline
            .insert_media_on_track(PRIMARY_AUDIO, audio, 600_000, 600_000)
            .unwrap();
        timeline.set_clip_volume(adjusted, -120).unwrap();
        export(
            &tools,
            timeline.tracks(),
            &output,
            CanvasFormat::Original,
            &BackgroundSettings::default(),
            &cancel,
            |_| {},
        )
        .unwrap();

        let quiet = tone(&audio_samples(&tools, &output, "0.15"), 440.0);
        let original = tone(&audio_samples(&tools, &output, "0.75"), 440.0);
        assert!(original > 0.04, "unadjusted tone amplitude {original}");
        let ratio = quiet / original;
        assert!((ratio - 0.251).abs() < 0.05, "exported tone ratio {ratio}");
    }

    fn video_track(id: u64, enabled: bool, clips: Vec<Clip>) -> Track {
        Track {
            id,
            kind: TrackKind::Video,
            enabled,
            clips,
        }
    }

    fn assert_frames(tools: &MediaTools, output: &Path, frames: &[(&str, [u8; 3])]) {
        for &(position, color) in frames {
            let frame = hidden_command(&tools.ffmpeg)
                .args(["-hide_banner", "-loglevel", "error", "-ss", position, "-i"])
                .arg(output)
                .args([
                    "-frames:v",
                    "1",
                    "-vf",
                    "scale=2:2",
                    "-pix_fmt",
                    "rgb24",
                    "-f",
                    "rawvideo",
                    "pipe:1",
                ])
                .output()
                .unwrap();
            assert!(frame.status.success());
            assert!(frame.stdout.len() >= 3);
            assert!(
                frame.stdout[..3]
                    .iter()
                    .zip(color)
                    .all(|(actual, expected)| actual.abs_diff(expected) < 30),
                "unexpected frame at {position} seconds: {:?}",
                &frame.stdout[..3]
            );
        }
    }

    fn audio_samples(tools: &MediaTools, output: &Path, position: &str) -> Vec<f32> {
        let decoded = hidden_command(&tools.ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-ss", position, "-i"])
            .arg(output)
            .args([
                "-t", "0.2", "-map", "0:a:0", "-ac", "1", "-ar", "8000", "-f", "f32le", "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(decoded.status.success());
        assert!(decoded.stdout.len() >= 1_500 * 4);
        decoded
            .stdout
            .chunks_exact(4)
            .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
            .collect()
    }

    fn tone(samples: &[f32], frequency: f64) -> f64 {
        let (mut cosine, mut sine) = (0.0, 0.0);
        for (index, &sample) in samples.iter().enumerate() {
            let (y, x) = (std::f64::consts::TAU * frequency * index as f64 / 8000.0).sin_cos();
            cosine += x * f64::from(sample);
            sine += y * f64::from(sample);
        }
        2.0 * cosine.hypot(sine) / samples.len() as f64
    }
}
