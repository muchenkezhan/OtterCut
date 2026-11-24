use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use anyhow::{Context as _, Result, bail, ensure};
use serde::Deserialize;

const CAPTURE_HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ottercut-macos-capture"));
const INFO_PLIST: &[u8] = include_bytes!("macos/Info.plist");

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct CameraDevice {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub modes: Vec<CameraMode>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct CameraMode {
    pub width: u32,
    pub height: u32,
    pub fps_milli: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CameraPreviewShape {
    #[default]
    Circle,
    Square,
}

impl CameraPreviewShape {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Circle => "circle",
            Self::Square => "square",
        }
    }
}

#[derive(Clone, Copy)]
pub enum CameraPreviewCommand {
    Visibility(bool),
    Shape(CameraPreviewShape),
}

impl CameraPreviewCommand {
    pub fn write_to(self, output: &mut impl std::io::Write) -> std::io::Result<()> {
        let line: &[u8] = match self {
            Self::Visibility(true) => b"show-camera-preview\n",
            Self::Visibility(false) => b"hide-camera-preview\n",
            Self::Shape(CameraPreviewShape::Circle) => b"camera-preview-circle\n",
            Self::Shape(CameraPreviewShape::Square) => b"camera-preview-square\n",
        };
        output.write_all(line)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct MicrophoneDevice {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub meter_id: Option<String>,
}

#[derive(Deserialize)]
pub struct DeviceList {
    #[serde(default)]
    pub microphones: Vec<MicrophoneDevice>,
    #[serde(default)]
    pub cameras: Vec<CameraDevice>,
}

pub fn select_source(mode: &str) -> Result<Option<String>> {
    let output = Command::new(capture_helper()?)
        .arg("select")
        .arg(mode)
        .output()
        .context("无法打开 macOS 录制来源选择器")?;
    if output.status.code() == Some(2) {
        return Ok(None);
    }
    if !output.status.success() {
        bail!(
            "macOS 来源选择失败（{}）",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let source = String::from_utf8(output.stdout)
        .context("macOS 来源选择器返回了无效编码")?
        .trim()
        .to_owned();
    ensure!(!source.is_empty(), "macOS 来源选择器没有返回来源");
    let value: serde_json::Value =
        serde_json::from_str(&source).context("macOS 来源信息格式无效")?;
    ensure!(value.get("style").is_some(), "macOS 来源信息缺少类型");
    Ok(Some(source))
}

pub fn capture_devices() -> Result<DeviceList> {
    let output = Command::new(capture_helper()?)
        .arg("devices")
        .output()
        .context("无法读取 macOS 设备列表")?;
    if !output.status.success() {
        bail!(
            "无法读取 macOS 设备列表（{}）",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout).context("macOS 设备列表格式无效")
}

pub fn start_camera_preview(device: &str, shape: CameraPreviewShape) -> Result<Child> {
    Command::new(capture_helper()?)
        .arg("preview-camera")
        .arg(device)
        .arg(shape.as_str())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("无法打开 macOS 摄像头预览")
}

pub fn start_recording(
    output: &Path,
    microphone: bool,
    microphone_device: Option<&str>,
    source: Option<&str>,
    camera_device: Option<&str>,
    camera_mode: Option<&CameraMode>,
    camera_output: Option<&Path>,
    click_follow: bool,
) -> Result<Child> {
    let helper = capture_helper()?;
    let microphone_device = if microphone {
        microphone_device
            .map(|device| device.split_once(':').map_or(device, |(_, uid)| uid))
            .unwrap_or("default")
    } else {
        "silent"
    };
    Command::new(helper)
        .process_group(0)
        .arg("record")
        .arg(output)
        .arg(microphone_device)
        .arg(source.unwrap_or("picker"))
        .arg(if click_follow {
            "click-follow"
        } else {
            "no-click-follow"
        })
        .arg(camera_device.unwrap_or(""))
        .arg(camera_output.unwrap_or_else(|| Path::new("")))
        .arg(camera_mode.map_or_else(|| "0".to_owned(), |mode| mode.width.to_string()))
        .arg(camera_mode.map_or_else(|| "0".to_owned(), |mode| mode.height.to_string()))
        .arg(camera_mode.map_or_else(|| "0".to_owned(), |mode| mode.fps_milli.to_string()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("无法启动 macOS 录制组件，请确认系统为 macOS 15 或更新版本")
}

fn capture_helper() -> Result<PathBuf> {
    let home = std::env::home_dir().context("找不到 macOS 用户目录")?;
    let contents = home.join("Library/Application Support/OtterCut/OtterCut Capture.app/Contents");
    let binaries = contents.join("MacOS");
    fs::create_dir_all(&binaries).context("无法创建 macOS 录制组件目录")?;
    install_file(&contents.join("Info.plist"), INFO_PLIST, 0o644)?;
    let helper = binaries.join("ottercut-macos-capture");
    install_file(&helper, CAPTURE_HELPER, 0o755)?;
    Ok(helper)
}

fn install_file(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let installed = fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.is_file()
            && metadata.mode() & 0o777 == mode
            && fs::read(path).is_ok_and(|contents| contents == bytes)
    });
    if installed {
        return Ok(());
    }
    let directory = path.parent().context("macOS 录制组件缺少父目录")?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(directory).context("无法暂存 macOS 录制组件")?;
    temporary
        .write_all(bytes)
        .context("无法写入 macOS 录制组件")?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(mode))
        .context("无法设置 macOS 录制组件权限")?;
    temporary.persist(path).context("无法安装 macOS 录制组件")?;
    Ok(())
}
