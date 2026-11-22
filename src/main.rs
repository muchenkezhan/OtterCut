#[cfg(windows)]
fn main() -> std::process::ExitCode {
    use clap::Parser;
    use takeseam::windows_recorder;

    match windows_recorder::run(windows_recorder::Cli::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "{}",
                takeseam::i18n::Locale::preferred().recording_failed(&format!("{error:#}"))
            );
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[path = "platform/cli.rs"]
mod portable_cli;

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn main() -> std::process::ExitCode {
    use clap::Parser as _;

    match portable_cli::run(portable_cli::Cli::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "{}",
                takeseam::i18n::Locale::preferred().recording_failed(&format!("{error:#}"))
            );
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn main() -> std::process::ExitCode {
    eprintln!(
        "{} 支持 Windows、macOS 和 Linux 桌面。",
        takeseam::PRODUCT_NAME
    );
    std::process::ExitCode::FAILURE
}
