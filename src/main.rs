#[cfg(windows)]
fn main() -> std::process::ExitCode {
    use clap::Parser;
    use takeseam::windows_recorder;

    match windows_recorder::run(windows_recorder::Cli::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("录制失败（{error:#}）");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    eprintln!("{} 的录制原型需要 Windows 11。", takeseam::PRODUCT_NAME);
    std::process::ExitCode::FAILURE
}
