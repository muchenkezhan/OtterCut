pub const PRODUCT_NAME: &str = "OtterCut";

#[cfg(windows)]
pub mod camera;
#[cfg(windows)]
mod click_follow;
pub mod editor_timeline;
#[cfg(windows)]
pub mod microphone;
#[cfg(windows)]
pub mod windows_preview;
#[cfg(windows)]
pub mod windows_recorder;
#[cfg(windows)]
mod zoom_follow;
#[cfg(windows)]
mod zoom_frames;
