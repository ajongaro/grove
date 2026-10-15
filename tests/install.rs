//! `install.sh` against a fake release folder (file:// URLs, so no network).

use std::path::Path;
use std::process::{Command, Output};

/// The release asset this machine should get, as install.sh names it.
fn target() -> String {
    let cpu = std::env::consts::ARCH;
    match std::env::consts::OS {
        "macos" => format!("{cpu}-apple-darwin"),
        _ => format!("{cpu}-unknown-linux-musl"),
    }
}

/// A release folder holding a fake grove for this machine and its checksum.
fn fake_release(dir: &Path, checksum_ok: bool) {
    let stage = dir.join("stage");
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(stage.join("grove"), "#!/bin/sh\necho grove 9.9.9\n").unwrap();
    let asset = format!("grove-{}.tar.gz", target());
    let st = Command::new("tar")
        .args(["-czf"])
        .arg(dir.join(&asset))
        .args(["-C"])
        .arg(&stage)
        .arg("grove")
        .status()
        .unwrap();
    assert!(st.success());
    let sum = Command::new("shasum")
        .args(["-a", "256"])
        .arg(&asset)
        .current_dir(dir)
        .output()
        .unwrap();
    let mut line = String::from_utf8(sum.stdout).unwrap();
    if !checksum_ok {
        line.replace_range(0..8, "00000000");
    }
    std::fs::write(dir.join("SHA256SUMS"), line).unwrap();
}

fn install(release: &Path, bin: &Path) -> Output {
    Command::new("sh")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh"))
        .env(
            "GROVE_DOWNLOAD_URL",
            format!("file://{}", release.display()),
        )
        .env("GROVE_INSTALL_DIR", bin)
        .output()
        .unwrap()
}

#[test]
fn installs_the_binary_for_this_machine() {
    let dir = tempfile::tempdir().unwrap();
    fake_release(dir.path(), true);
    let bin = dir.path().join("my bin");
    let out = install(dir.path(), &bin);
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{text}");
    let run = Command::new(bin.join("grove")).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "grove 9.9.9\n");
    assert!(text.contains("grove 9.9.9"), "{text}");
}

#[test]
fn a_bad_checksum_installs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    fake_release(dir.path(), false);
    let bin = dir.path().join("bin");
    let out = install(dir.path(), &bin);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("checksum"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!bin.join("grove").exists());
}
