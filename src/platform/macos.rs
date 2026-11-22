use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use anyhow::{Context as _, Result};

const CAPTURE_HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ottercut-macos-capture"));
const INFO_PLIST: &[u8] = include_bytes!("macos/Info.plist");

pub fn start_recording(output: &Path, microphone: bool) -> Result<Child> {
    let helper = capture_helper()?;
    Command::new(helper)
        .process_group(0)
        .arg(output)
        .arg(if microphone { "mic" } else { "silent" })
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
