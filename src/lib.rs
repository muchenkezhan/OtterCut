pub const PRODUCT_NAME: &str = "OtterCut";

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
pub mod i18n;

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
pub fn config_directory() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;

    let environment_path = |name| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    #[cfg(windows)]
    return environment_path("LOCALAPPDATA").map(|path| path.join("TakeSeam"));
    #[cfg(target_os = "macos")]
    return environment_path("HOME").map(|path| path.join("Library/Application Support/OtterCut"));
    #[cfg(target_os = "linux")]
    return environment_path("XDG_CONFIG_HOME")
        .or_else(|| environment_path("HOME").map(|path| path.join(".config")))
        .map(|path| path.join("ottercut"));
}

#[cfg(windows)]
pub mod camera;
#[cfg(windows)]
mod click_follow;
pub mod editor_timeline;
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
pub mod microphone;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod platform;
#[cfg(windows)]
pub mod windows_preview;
#[cfg(windows)]
pub mod windows_recorder;
