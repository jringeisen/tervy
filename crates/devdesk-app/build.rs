use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../../scripts/meeting_audio.swift");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let target = root.join("target");
    let cache = target.join("meeting-helper-cache");
    std::fs::create_dir_all(&cache).expect("meeting helper module cache");
    let result = Command::new("swiftc")
        .args(["-target", "arm64-apple-macosx26.0"])
        .arg("-module-cache-path")
        .arg(&cache)
        .arg("-parse-as-library")
        .arg(root.join("scripts/meeting_audio.swift"))
        .arg("-o")
        .arg(target.join("meeting-audio"))
        .status()
        .expect("swiftc for meeting audio");
    assert!(result.success(), "could not compile meeting audio helper");
}
