//! Top-level `grove` commands that don't need a terminal.

use std::process::Command;

fn grove(home: &std::path::Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_grove"));
    c.env("GROVE_HOME", home);
    c
}

#[test]
fn add_registers_folder_as_project() {
    let dir = tempfile::tempdir().unwrap();
    let proj = dir.path().join("Side Project");
    std::fs::create_dir_all(&proj).unwrap();
    let home = dir.path().join("home");
    let out = grove(&home).args(["add"]).arg(&proj).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("Side Project"));
    let toml = std::fs::read_to_string(home.join("projects/side-project.toml")).unwrap();
    assert!(toml.contains("name = \"Side Project\""), "{toml}");
}

#[test]
fn add_without_path_uses_current_dir() {
    let dir = tempfile::tempdir().unwrap();
    let proj = dir.path().join("here");
    std::fs::create_dir_all(&proj).unwrap();
    let home = dir.path().join("home");
    let out = grove(&home).arg("add").current_dir(&proj).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(home.join("projects/here.toml").is_file());
}

#[test]
fn add_missing_dir_fails_with_message() {
    let dir = tempfile::tempdir().unwrap();
    let out = grove(&dir.path().join("home"))
        .args(["add", "/no/such/dir"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("/no/such/dir"));
}

#[test]
fn start_refuses_when_claude_account_dir_missing() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        "claude_config_dir = \"/no/such/claude-play\"\ntmux_session = \"grove-cli-test\"\n",
    )
    .unwrap();
    let out = grove(&home).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("/no/such/claude-play"), "{err}");
}

#[test]
fn start_reports_bad_config() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), "tmux_session = [").unwrap();
    let out = grove(&home).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("config.toml"));
}

#[test]
fn status_json_reports_local_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join("projects")).unwrap();
    std::fs::create_dir_all(home.join("sessions")).unwrap();
    std::fs::write(home.join("projects/demo.toml"), "name = \"demo\"\npath = \"/tmp\"\n").unwrap();
    std::fs::write(
        home.join("sessions/a.json"),
        r#"{"id":"a","project":"demo","name":"s1","host":"local","tmux_window":"@1","state":"needs_you","message":"m","created_at":"2026-10-07T10:00:00Z","updated_at":"2026-10-07T10:00:00Z"}"#,
    )
    .unwrap();
    let out = grove(&home).args(["status", "--json"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["version"], 1);
    assert_eq!(v["needs_you"], 1);
    assert_eq!(v["projects"][0]["sessions"][0]["name"], "s1");
    assert!(v["host"].as_str().is_some_and(|h| !h.is_empty()));
}
