use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use takeseam::i18n::{Locale, key};
use takeseam::platform::{self, RecordingRequest};

#[derive(Parser)]
#[command(version, about = "OtterCut 桌面录制")]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 通过系统对话框选择屏幕或窗口，按 Ctrl+C 停止。
    Record {
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        mic: bool,
        /// 从实际开始录制时起计时。
        #[arg(long)]
        seconds: Option<NonZeroU64>,
    },
}

pub fn run(cli: Cli) -> Result<()> {
    let Command::Record {
        output,
        mic,
        seconds,
    } = cli.command;
    let stopped = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&stopped);
    ctrlc::set_handler(move || signal.store(true, Ordering::Relaxed)).context("无法监听 Ctrl+C")?;
    let request = RecordingRequest {
        output,
        microphone: mic,
        duration: seconds.map(|seconds| Duration::from_secs(seconds.get())),
    };
    let result = platform::record(request, &stopped, || {
        eprintln!("{}", Locale::preferred().text(key::CLI_RECORDING_STARTED));
    })?;
    if let Some(saved) = result {
        println!("{}", saved.path.display());
        if let Some(warning) = saved.warning {
            eprintln!("{warning}");
        }
    }
    Ok(())
}
