use std::path::PathBuf;
use std::time::Duration;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_session;

#[cfg(target_os = "linux")]
pub use linux::record;
#[cfg(target_os = "macos")]
pub use macos_session::record;

pub struct RecordingRequest {
    pub output: PathBuf,
    pub microphone: bool,
    pub duration: Option<Duration>,
}

pub struct SavedRecording {
    pub path: PathBuf,
    pub warning: Option<String>,
}
