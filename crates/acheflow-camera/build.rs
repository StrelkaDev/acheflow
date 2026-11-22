use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=native/camera.m");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        _ => panic!("Unsupported macOS camera architecture"),
    };
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let object = out.join("camera.o");
    let archive = out.join("libacheflow_camera.a");
    run(Command::new("xcrun")
        .args([
            "--sdk",
            "macosx",
            "clang",
            "-arch",
            arch,
            "-O2",
            "-fobjc-arc",
            "-fblocks",
            "-Werror",
            "-Wno-deprecated-declarations",
            "-c",
            "native/camera.m",
            "-o",
        ])
        .arg(&object));
    run(Command::new("xcrun")
        .args(["libtool", "-static", "-o"])
        .arg(&archive)
        .arg(object));
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=acheflow_camera");
    for framework in [
        "AVFoundation",
        "Foundation",
        "CoreFoundation",
        "CoreMedia",
        "CoreVideo",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}

fn run(command: &mut Command) {
    assert!(
        command
            .status()
            .expect("run the macOS camera compiler")
            .success(),
        "{command:?} failed"
    );
}
