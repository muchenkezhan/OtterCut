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
pub use macos::{
    CameraDevice, CameraMode, CameraPreviewCommand, CameraPreviewShape, DeviceList,
    MicrophoneDevice, capture_devices, select_source, start_camera_preview,
};
#[cfg(target_os = "macos")]
pub use macos_session::{RecordingEvent, record, record_with_camera_preview};

pub struct RecordingRequest {
    pub output: PathBuf,
    pub microphone: bool,
    pub duration: Option<Duration>,
    #[cfg(target_os = "macos")]
    pub source: Option<String>,
    #[cfg(target_os = "macos")]
    pub microphone_device: Option<String>,
    #[cfg(target_os = "macos")]
    pub camera_device: Option<String>,
    #[cfg(target_os = "macos")]
    pub camera_mode: Option<CameraMode>,
    #[cfg(target_os = "macos")]
    pub click_follow: bool,
}

pub struct SavedRecording {
    pub path: PathBuf,
    pub camera_path: Option<PathBuf>,
    pub warning: Option<String>,
}
