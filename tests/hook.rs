//! Runs the real `grove hook` binary against a temp GROVE_HOME.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const SID: &str = "11111111-2222-3333-4444-555555555556";

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/hooks/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// Runs `grove hook` with `input` on stdin; returns (exit code, stdout).
fn run_hook(home: &Path, input: &str) -> (i32, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_grove"))
        .arg("hook")
        .env("GROVE_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn setup(state: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("grove home");
    std::fs::create_dir_all(home.join("sessions")).unwrap();
    std::fs::create_dir_all(home.join("projects")).unwrap();
    std::fs::write(
        home.join("projects/demo.toml"),
        "name = \"Demo\"\npath = \"/tmp/demo\"\nsummary = \"A demo project.\"\n",
    )
    .unwrap();
    std::fs::write(
        home.join(format!("sessions/{SID}.json")),
        format!(
            r#"{{"id":"{SID}","project":"demo","name":"s1","host":"local","tmux_window":"@4",
               "state":"{state}","message":null,
               "created_at":"2026-10-06T10:00:00Z","updated_at":"2026-10-06T10:00:00Z"}}"#
        ),
    )
    .unwrap();
    dir
}

fn home(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("grove home")
}

fn session(dir: &tempfile::TempDir) -> serde_json::Value {
    serde_json::from_str(
        &std::fs::read_to_string(home(dir).join(format!("sessions/{SID}.json"))).unwrap(),
    )
    .unwrap()
}

#[test]
fn permission_prompt_sets_needs_you_with_message() {
    let dir = setup("working");
    let (code, _) = run_hook(&home(&dir), &fixture("notification_permission"));
    assert_eq!(code, 0);
    let s = session(&dir);
    assert_eq!(s["state"], "needs_you");
    assert_eq!(s["message"], "Claude needs your permission to use Bash");
    assert_ne!(s["updated_at"], "2026-10-06T10:00:00Z");
}

#[test]
fn stop_sets_your_turn_and_clears_message() {
    let dir = setup("needs_you");
    run_hook(&home(&dir), &fixture("stop"));
    let s = session(&dir);
    assert_eq!(s["state"], "your_turn");
    assert!(s["message"].is_null());
}

#[test]
fn garbage_input_exits_zero_and_changes_nothing() {
    let dir = setup("working");
    let before = session(&dir);
    for input in ["", "not json", "{\"session_id\":1}"] {
        let (code, out) = run_hook(&home(&dir), input);
        assert_eq!(code, 0, "{input:?}");
        assert!(out.is_empty(), "{input:?}");
    }
    assert_eq!(session(&dir), before);
    let log = std::fs::read_to_string(home(&dir).join("grove.log")).unwrap();
    assert!(log.contains("hook"), "{log}");
}

#[test]
fn unknown_session_is_ignored_silently() {
    let dir = setup("working");
    let input = fixture("stop").replace(SID, "99999999-0000-0000-0000-000000000000");
    let (code, out) = run_hook(&home(&dir), &input);
    assert_eq!(code, 0);
    assert!(out.is_empty());
    assert!(
        !home(&dir)
            .join("sessions/99999999-0000-0000-0000-000000000000.json")
            .exists()
    );
    assert!(!home(&dir).join("grove.log").exists());
}

#[test]
fn missing_grove_home_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let (code, _) = run_hook(&dir.path().join("nope"), &fixture("stop"));
    assert_eq!(code, 0);
}

#[test]
fn session_start_prints_project_context_with_wiki_titles() {
    let dir = setup("starting");
    let wiki = home(&dir).join("wiki/demo");
    std::fs::create_dir_all(&wiki).unwrap();
    std::fs::write(wiki.join("commands.md"), "# Commands\n\nx").unwrap();
    std::fs::write(wiki.join("architecture.md"), "# Architecture\n\ny").unwrap();
    let (code, out) = run_hook(&home(&dir), &fixture("session_start"));
    assert_eq!(code, 0);
    assert!(out.contains("Grove project: Demo (/tmp/demo)"), "{out}");
    assert!(out.contains("Summary: A demo project."), "{out}");
    assert!(
        out.contains("Wiki sections: Architecture, Commands"),
        "{out}"
    );
    assert!(
        !out.contains("x\n"),
        "wiki bodies must not be printed: {out}"
    );
    assert_eq!(session(&dir)["state"], "starting");
}

#[test]
fn session_start_without_wiki_says_none() {
    let dir = setup("starting");
    let (_, out) = run_hook(&home(&dir), &fixture("session_start"));
    assert!(out.contains("Wiki sections: none yet"), "{out}");
}
