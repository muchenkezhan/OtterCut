use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    let source = "src/platform/macos/capture.m";
    let info_plist = "src/platform/macos/Info.plist";
    println!("cargo:rerun-if-changed={source}");
    println!("cargo:rerun-if-changed={info_plist}");

    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        other => panic!("Unsupported macOS architecture: {other:?}"),
    };
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"))
        .join("ottercut-macos-capture");
    let status = Command::new("xcrun")
        .args([
            "--sdk",
            "macosx",
            "clang",
            "-arch",
            arch,
            "-fobjc-arc",
            "-fblocks",
            "-mmacosx-version-min=15.0",
            "-framework",
            "AppKit",
            "-framework",
            "AVFoundation",
            "-framework",
            "CoreMedia",
            "-framework",
            "ScreenCaptureKit",
            source,
            "-o",
        ])
        .arg(output)
        .status()
        .expect("Building the macOS recorder requires Xcode or Command Line Tools with macOS 15 SDK or later");
    assert!(
        status.success(),
        "Could not build the macOS recording helper"
    );
}
