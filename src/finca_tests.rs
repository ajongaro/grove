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

fn status_json(host: &str) -> String {
    let d = tempfile::tempdir().unwrap();
    let st = crate::status::build(&crate::store::Store::new(crate::config::Paths::new(d.path().join("h"))), host);
    serde_json::to_string(&st).unwrap()
}

fn script(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    p
}

fn finca_with_bin(bin: &std::path::Path) -> Finca {
    Finca { name: "box".into(), ssh: "me@box".into(), grove_bin: bin.to_string_lossy().into(), color: None }
}

#[test]
fn poll_reachable_parses_status() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join("bin dir");
    std::fs::create_dir_all(&dir).unwrap();
    let bin = fake_grove(&dir, &status_json("therver"));
    let ssh = fake_ssh(d.path(), None);
    let r = with_env(&ssh, d.path(), || poll_one(&finca_with_bin(&bin), std::time::Duration::from_secs(5)));
    match r {
        Reach::Reachable(st) => assert_eq!(st.host, "therver"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn poll_ssh_failure_is_unreachable() {
    let d = tempfile::tempdir().unwrap();
    let ssh = fake_ssh(d.path(), Some(255));
    let r = with_env(&ssh, d.path(), || poll_one(&finca_with_bin(std::path::Path::new("/g")), std::time::Duration::from_secs(5)));
    assert!(matches!(&r, Reach::Unreachable { error } if error.contains("Connection refused")), "{r:?}");
}

#[test]
fn poll_old_grove_needs_update() {
    let d = tempfile::tempdir().unwrap();
    let bin = script(d.path(), "old-grove", "echo \"error: unrecognized subcommand 'status'\" >&2; exit 2");
    let ssh = fake_ssh(d.path(), None);
    let r = with_env(&ssh, d.path(), || poll_one(&finca_with_bin(&bin), std::time::Duration::from_secs(5)));
    assert_eq!(r, Reach::NeedsUpdate);
}

#[test]
fn poll_newer_status_version_needs_update() {
    let d = tempfile::tempdir().unwrap();
    let json = status_json("h").replace("\"version\":1", "\"version\":9");
    let bin = fake_grove(d.path(), &json);
    let ssh = fake_ssh(d.path(), None);
    let r = with_env(&ssh, d.path(), || poll_one(&finca_with_bin(&bin), std::time::Duration::from_secs(5)));
    assert_eq!(r, Reach::NeedsUpdate);
}

#[test]
fn poll_hanging_finca_times_out() {
    let d = tempfile::tempdir().unwrap();
    let bin = script(d.path(), "slow-grove", "sleep 30");
    let ssh = fake_ssh(d.path(), None);
    let start = std::time::Instant::now();
    let r = with_env(&ssh, d.path(), || poll_one(&finca_with_bin(&bin), std::time::Duration::from_millis(500)));
    assert!(start.elapsed() < std::time::Duration::from_secs(5), "took {:?}", start.elapsed());
    assert!(matches!(&r, Reach::Unreachable { error } if error.contains("timed out")), "{r:?}");
}

#[test]
fn poller_reports_every_finca_and_responds_to_kick() {
    let d = tempfile::tempdir().unwrap();
    let store = crate::store::Store::new(crate::config::Paths::new(d.path().join("home")));
    let bin = fake_grove(d.path(), &status_json("a"));
    store.add_finca(&Finca { name: "alpha".into(), ssh: "x".into(), grove_bin: bin.to_string_lossy().into(), color: None }).unwrap();
    store.add_finca(&Finca { name: "beta".into(), ssh: "y".into(), grove_bin: "/nope".into(), color: None }).unwrap();
    let ssh = fake_ssh(d.path(), None);
    with_env(&ssh, d.path(), || {
        let poller = Poller::start(store.clone(), std::time::Duration::from_secs(3600), std::time::Duration::from_secs(5));
        let mut seen = std::collections::HashMap::new();
        while seen.len() < 2 {
            let r = poller.results.recv_timeout(std::time::Duration::from_secs(10)).expect("first round");
            seen.insert(r.slug.clone(), r.reach);
        }
        assert!(matches!(seen["alpha"], Reach::Reachable(_)));
        assert!(!matches!(seen["beta"], Reach::Reachable(_)));
        poller.kick();
        let again = poller.results.recv_timeout(std::time::Duration::from_secs(10)).expect("kicked round");
        assert!(["alpha", "beta"].contains(&again.slug.as_str()));
    });
}
