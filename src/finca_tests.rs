use super::testing::*;
use super::*;

/// Probe/poll tests change process env (GROVE_SSH, FAKE_HOME); run them one
/// at a time.
pub static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn with_env<T>(ssh: &std::path::Path, fake_home: &std::path::Path, f: impl FnOnce() -> T) -> T {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::set_var("GROVE_SSH", ssh);
        std::env::set_var("FAKE_HOME", fake_home);
    }
    let out = f();
    unsafe {
        std::env::remove_var("GROVE_SSH");
        std::env::remove_var("FAKE_HOME");
    }
    out
}

#[test]
fn quoting_survives_spaces_and_quotes() {
    assert_eq!(sh_quote("plain"), "'plain'");
    assert_eq!(sh_quote("a b"), "'a b'");
    assert_eq!(sh_quote("it's"), "'it'\\''s'");
    let out = std::process::Command::new("sh").arg("-c").arg(format!("printf %s {}", sh_quote("x'y $HOME `z`"))).output().unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "x'y $HOME `z`");
}

#[test]
fn probe_finds_grove_in_local_bin() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("remote home");
    std::fs::create_dir_all(home.join(".local/bin")).unwrap();
    fake_grove(&home.join(".local/bin"), "{}");
    let ssh = fake_ssh(d.path(), None);
    let found = with_env(&ssh, &home, || probe("me@box", None)).unwrap();
    assert_eq!(found, home.join(".local/bin/grove").to_string_lossy());
}

#[test]
fn probe_prefers_explicit_bin() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("h");
    let custom = d.path().join("custom dir");
    std::fs::create_dir_all(&custom).unwrap();
    std::fs::create_dir_all(home.join(".local/bin")).unwrap();
    fake_grove(&home.join(".local/bin"), "{}");
    let bin = fake_grove(&custom, "{}");
    let ssh = fake_ssh(d.path(), None);
    let found = with_env(&ssh, &home, || probe("me@box", Some(bin.to_str().unwrap()))).unwrap();
    assert_eq!(found, bin.to_string_lossy());
}

#[test]
fn probe_reports_missing_grove() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("empty home");
    std::fs::create_dir_all(&home).unwrap();
    let ssh = fake_ssh(d.path(), None);
    let err = with_env(&ssh, &home, || probe("me@box", None)).unwrap_err().to_string();
    assert!(err.contains("no grove") && err.contains("me@box"), "{err}");
}

#[test]
fn probe_reports_ssh_failure_with_key_hint() {
    let d = tempfile::tempdir().unwrap();
    let ssh = fake_ssh(d.path(), Some(255));
    let err = with_env(&ssh, d.path(), || probe("me@box", None)).unwrap_err().to_string();
    assert!(err.contains("Connection refused"), "{err}");
    assert!(err.contains("ssh-copy-id"), "{err}");
}
