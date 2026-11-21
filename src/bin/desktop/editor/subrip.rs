use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{Context as _, Result, bail, ensure};

const MAX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CUES: usize = 10_000;

pub(super) struct Cue {
    pub start_us: u64,
    pub duration_us: u64,
    pub text: String,
}

pub(super) fn read(path: &Path) -> Result<Vec<Cue>> {
    let file = File::open(path).context("无法打开 SRT 字幕文件")?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("无法读取 SRT 字幕文件")?;
    ensure!(bytes.len() as u64 <= MAX_BYTES, "SRT 文件超过 4 MB");
    parse(&decode(&bytes)?)
}

fn decode(bytes: &[u8]) -> Result<String> {
    if let Some(utf16) = bytes
        .strip_prefix(&[0xFF, 0xFE])
        .or_else(|| bytes.strip_prefix(&[0xFE, 0xFF]))
    {
        ensure!(utf16.len().is_multiple_of(2), "SRT 的 UTF-16 编码不完整");
        let little_endian = bytes.starts_with(&[0xFF, 0xFE]);
        let units = utf16
            .chunks_exact(2)
            .map(|chunk| {
                let pair = [chunk[0], chunk[1]];
                if little_endian {
                    u16::from_le_bytes(pair)
                } else {
                    u16::from_be_bytes(pair)
                }
            })
            .collect::<Vec<_>>();
        return String::from_utf16(&units).context("SRT 的 UTF-16 编码无效");
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8(bytes.to_vec()).context("请将 SRT 文件保存为 UTF-8 或带 BOM 的 UTF-16")
}

fn parse(text: &str) -> Result<Vec<Cue>> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = text.lines().enumerate().peekable();
    let mut cues = Vec::new();
    while let Some((line_index, line)) = lines.next() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        ensure!(cues.len() < MAX_CUES, "SRT 超过 {MAX_CUES} 条字幕");
        let timing_line = if line.bytes().all(|byte| byte.is_ascii_digit()) {
            lines.next().map(|(_, line)| line.trim()).unwrap_or("")
        } else {
            line
        };
        let error = || format!("SRT 第 {} 行附近的时间码无效", line_index + 1);
        let (from, to) = timing_line.split_once("-->").with_context(error)?;
        let start_us = timestamp(from.trim()).with_context(error)?;
        // Some exporters append screen coordinates after the end time.
        let end_us = to
            .split_whitespace()
            .next()
            .and_then(timestamp)
            .with_context(error)?;
        ensure!(
            end_us > start_us,
            "SRT 第 {} 行附近的结束时间须晚于开始",
            line_index + 1
        );

        let mut content = String::new();
        while let Some((_, line)) = lines.next_if(|(_, line)| !line.trim().is_empty()) {
            if !content.is_empty() {
                content.push('\n');
            }
            content.push_str(line.trim());
        }
        if content.is_empty() {
            bail!("SRT 第 {} 行附近的字幕为空", line_index + 1);
        }
        cues.push(Cue {
            start_us,
            duration_us: end_us - start_us,
            text: content,
        });
    }
    ensure!(!cues.is_empty(), "SRT 文件没有字幕");
    Ok(cues)
}

fn timestamp(text: &str) -> Option<u64> {
    let (whole, millis) = text.split_once([',', '.'])?;
    let (hours, rest) = whole.split_once(':')?;
    let (minutes, seconds) = rest.split_once(':')?;
    if hours.len() < 2
        || minutes.len() != 2
        || seconds.len() != 2
        || millis.len() != 3
        || [hours, minutes, seconds, millis]
            .iter()
            .any(|part| !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    let hours: u64 = hours.parse().ok()?;
    let minutes: u64 = minutes.parse().ok()?;
    let seconds: u64 = seconds.parse().ok()?;
    let millis: u64 = millis.parse().ok()?;
    if minutes >= 60 || seconds >= 60 {
        return None;
    }
    hours
        .checked_mul(3_600_000_000)?
        .checked_add(minutes * 60_000_000 + seconds * 1_000_000 + millis * 1_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_windows_utf16_srt_with_millisecond_precision_and_multiple_lines() {
        let source = "1\r\n00:00:01,250 --> 00:00:03,045 X1:0\r\n第一行\r\nSecond line\r\n \r\n00:00:03.045 --> 00:00:05.000\r\n下一段";
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(source.encode_utf16().flat_map(u16::to_le_bytes));
        let cues = parse(&decode(&bytes).unwrap()).unwrap();
        assert_eq!(cues[0].start_us, 1_250_000);
        assert_eq!(cues[0].duration_us, 1_795_000);
        assert_eq!(cues[0].text, "第一行\nSecond line");
        assert_eq!(cues[1].start_us, 3_045_000);
        assert_eq!(cues[1].text, "下一段");
    }

    #[test]
    fn reports_invalid_timing_instead_of_returning_a_partial_import() {
        let source =
            "1\n00:00:00,100 --> 00:00:00,500\nValid\n\n2\n00:00:61,000 --> 00:01:10,000\nInvalid";
        let error = parse(source).err().unwrap();
        assert!(error.to_string().contains("第 5 行"));
    }
}
