use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use anyhow::{Context as _, Result, ensure};
use serde_json::Value;
use takeseam::editor_timeline::{Clip, Track, TrackId, TrackKind};

use super::media::MediaTools;
use super::subrip;

pub(super) struct SpeechJob {
    updates: Receiver<Result<Vec<SpeechCue>>>,
    pub(super) track_id: TrackId,
}

pub(super) struct SpeechCue {
    pub start_us: u64,
    pub duration_us: u64,
    pub text: String,
}

#[derive(Clone, Copy)]
enum WhisperBackend {
    WhisperCpp,
    OpenAiCli,
}

#[derive(Clone)]
struct WhisperConfig {
    executable: PathBuf,
    model: PathBuf,
    backend: WhisperBackend,
}

impl SpeechJob {
    pub(super) fn start(tools: MediaTools, track: Track) -> Result<Self> {
        let config = WhisperConfig::resolve()?;
        let track_id = track.id;
        let (updates, incoming) = mpsc::channel();
        thread::Builder::new()
            .name("OtterCut speech recognition".into())
            .spawn(move || {
                let result = recognize_track(&tools, &track, &config);
                let _ = updates.send(result);
            })
            .context("无法启动语音识别任务")?;
        Ok(Self {
            updates: incoming,
            track_id,
        })
    }

    pub(super) fn poll(&mut self) -> Option<(TrackId, Result<Vec<SpeechCue>>)> {
        let track_id = self.track_id;
        match self.updates.try_recv() {
            Ok(result) => Some((track_id, result)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some((track_id, Err(anyhow::anyhow!("语音识别任务已中断"))))
            }
        }
    }
}

impl WhisperConfig {
    fn resolve() -> Result<Self> {
        let executable = std::env::var_os("OTTERCUT_WHISPER_BIN")
            .map(PathBuf::from)
            .or_else(default_executable_path)
            .or_else(|| find_command("whisper-cli"))
            .or_else(|| find_command("whisper"))
            .context("未找到本地 Whisper 命令，请设置 OTTERCUT_WHISPER_BIN")?;
        let model = std::env::var_os("OTTERCUT_WHISPER_MODEL")
            .map(PathBuf::from)
            .or_else(default_model_path)
            .context("未配置本地 Whisper 模型，请设置 OTTERCUT_WHISPER_MODEL")?;
        ensure!(model.is_file(), "Whisper 模型文件不存在（{}）", model.display());
        let name = executable
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        Ok(Self {
            executable,
            model,
            backend: if name.contains("whisper-cli") {
                WhisperBackend::WhisperCpp
            } else {
                WhisperBackend::OpenAiCli
            },
        })
    }
}

fn find_command(name: &str) -> Option<PathBuf> {
    Command::new(name)
        .arg("--help")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?
        .success()
        .then(|| PathBuf::from(name))
}

fn default_model_path() -> Option<PathBuf> {
    let candidates = [
        takeseam::config_directory().map(|path| path.join("whisper/models/ggml-base.bin")),
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .map(|path| path.join(".cache/whisper/ggml-base.bin")),
    ];
    candidates.into_iter().flatten().find(|path| path.is_file())
}

fn default_executable_path() -> Option<PathBuf> {
    let executable = takeseam::config_directory()?.join("whisper/whisper-cli");
    let executable = if cfg!(windows) {
        executable.with_extension("exe")
    } else {
        executable
    };
    executable.is_file().then_some(executable)
}

fn recognize_track(
    tools: &MediaTools,
    track: &Track,
    config: &WhisperConfig,
) -> Result<Vec<SpeechCue>> {
    ensure!(
        matches!(track.kind, TrackKind::Audio | TrackKind::Video),
        "请选择包含音频的媒体轨道"
    );
    let stage = tempfile::tempdir().context("无法创建语音识别临时目录")?;
    let mut cues = Vec::new();
    for (index, clip) in track.clips.iter().enumerate() {
        let wav = stage.path().join(format!("clip-{index:04}.wav"));
        extract_audio(tools, clip, &wav)?;
        let local = run_whisper(config, &wav, stage.path(), index)?;
        for cue in local {
            let start = clip
                .start
                .saturating_add(cue.start_us)
                .min(clip.end().saturating_sub(1));
            let duration = cue.duration_us.max(1);
            cues.push(SpeechCue {
                start_us: start,
                duration_us: duration,
                text: cue.text,
            });
        }
    }
    cues.sort_by_key(|cue| cue.start_us);
    for index in 0..cues.len() {
        let end = cues
            .get(index + 1)
            .map(|cue| cue.start_us)
            .unwrap_or_else(|| track.duration())
            .max(cues[index].start_us.saturating_add(cues[index].duration_us));
        cues[index].duration_us = end.saturating_sub(cues[index].start_us).max(1);
    }
    ensure!(!cues.is_empty(), "没有识别到语音内容");
    Ok(cues)
}

fn extract_audio(tools: &MediaTools, clip: &Clip, output: &Path) -> Result<()> {
    let source = super::media::local_media(&clip.path)?;
    let status = super::media::hidden_command(&tools.ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-ss",
            &seconds(clip.source_start),
            "-t",
            &seconds(clip.duration()),
            "-i",
        ])
        .arg(source)
        .args(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
        .arg(output)
        .status()
        .context("无法启动 FFmpeg 提取音频")?;
    ensure!(status.success(), "无法提取音频用于语音识别");
    Ok(())
}

fn run_whisper(
    config: &WhisperConfig,
    wav: &Path,
    stage: &Path,
    index: usize,
) -> Result<Vec<subrip::Cue>> {
    let output_stem = stage.join(format!("result-{index:04}"));
    let output_srt = output_stem.with_extension("srt");
    let output_json = output_stem.with_extension("json");
    let status = if matches!(config.backend, WhisperBackend::WhisperCpp) {
        super::media::hidden_command(&config.executable)
            .args(["-m"])
            .arg(&config.model)
            .args(["-f"])
            .arg(wav)
            .args(["-l", "auto", "-oj", "-of"])
            .arg(&output_stem)
            .status()
            .context("无法启动 whisper-cli")?
    } else {
        super::media::hidden_command(&config.executable)
            .args(["-m", "whisper", wav.to_string_lossy().as_ref()])
            .args(["--model", "base"])
            .args(["--output_format", "srt", "--output_dir"])
            .arg(stage)
            .status()
            .context("无法启动 OpenAI Whisper")?
    };
    ensure!(status.success(), "Whisper 语音识别失败");
    if output_json.is_file() {
        return parse_whisper_json(&output_json);
    }
    ensure!(output_srt.is_file(), "Whisper 没有生成字幕结果");
    subrip::read(&output_srt)
}
fn parse_whisper_json(path: &Path) -> Result<Vec<subrip::Cue>> {
    let bytes = std::fs::read(path).context("无法读取 Whisper JSON 结果")?;
    let value: Value = serde_json::from_slice(&bytes).context("Whisper JSON 格式无效")?;
    let entries = value
        .get("transcription")
        .or_else(|| value.get("segments"))
        .and_then(Value::as_array)
        .context("Whisper JSON 缺少时间片段")?;
    let mut cues = Vec::new();
    for entry in entries {
        let text = entry
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .context("Whisper 返回了空字幕片段")?;
        let start = entry
            .get("start")
            .or_else(|| entry.get("from"))
            .or_else(|| entry.get("timestamps").and_then(|value| value.get("from")))
            .and_then(json_time_us)
            .context("Whisper 缺少字幕开始时间")?;
        let end = entry
            .get("end")
            .or_else(|| entry.get("to"))
            .or_else(|| entry.get("timestamps").and_then(|value| value.get("to")))
            .and_then(json_time_us)
            .filter(|end| *end > start)
            .context("Whisper 缺少字幕结束时间")?;
        cues.push(subrip::Cue {
            start_us: start,
            duration_us: end - start,
            text: text.to_owned(),
        });
    }
    ensure!(!cues.is_empty(), "Whisper 没有识别到字幕");
    Ok(cues)
}

fn json_time_us(value: &Value) -> Option<u64> {
    if let Some(seconds) = value.as_f64() {
        return (seconds.is_finite() && seconds >= 0.0).then_some((seconds * 1_000_000.0) as u64);
    }
    let text = value.as_str()?.trim();
    let text = text.strip_prefix('[').unwrap_or(text).strip_suffix(']').unwrap_or(text);
    let parts = text.split(':').collect::<Vec<_>>();
    let parse_number = |part: &str| part.replace(',', ".").parse::<f64>().ok();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => (0.0, parse_number(minutes)?, parse_number(seconds)?),
        [hours, minutes, seconds] => (
            parse_number(hours)?,
            parse_number(minutes)?,
            parse_number(seconds)?,
        ),
        _ => return None,
    };
    let total = hours * 3_600.0 + minutes * 60.0 + seconds;
    (total >= 0.0).then_some((total * 1_000_000.0) as u64)
}

fn seconds(usecs: u64) -> String {
    format!("{}.{:06}", usecs / 1_000_000, usecs % 1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_whisper_segment_times_from_json() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("result.json");
        std::fs::write(
            &path,
            r#"{"segments":[{"start":1.25,"end":3.5,"text":"你好 世界"}]}"#,
        )?;
        let cues = parse_whisper_json(&path)?;
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].start_us, 1_250_000);
        assert_eq!(cues[0].duration_us, 2_250_000);
        assert_eq!(cues[0].text, "你好 世界");
        Ok(())
    }

    #[test]
    fn parses_timestamp_strings_with_milliseconds() {
        assert_eq!(json_time_us(&Value::String("00:01:02.500".into())), Some(62_500_000));
        assert_eq!(json_time_us(&Value::String("00:00:02,580".into())), Some(2_580_000));
    }
}

