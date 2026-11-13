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
    run_hook_as(home, input, None)
}

/// `run_hook` from the window run `launch` (`GROVE_LAUNCH`), if given.
fn run_hook_as(home: &Path, input: &str, launch: Option<&str>) -> (i32, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_grove"));
    if let Some(l) = launch {
        cmd.env("GROVE_LAUNCH", l);
    }
    let mut child = cmd
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
fn session_start_prints_project_context() {
    let dir = setup("starting");
    let (code, out) = run_hook(&home(&dir), &fixture("session_start"));
    assert_eq!(code, 0);
    assert!(out.contains("Grove project: Demo (/tmp/demo)"), "{out}");
    assert!(out.contains("Summary: A demo project."), "{out}");
    assert!(!out.contains("Wiki"), "the wiki is gone: {out}");
    assert_eq!(session(&dir)["state"], "starting");
}

#[test]
fn session_start_without_a_summary_says_how_to_set_one() {
    let dir = setup("starting");
    std::fs::write(
        home(&dir).join("projects/demo.toml"),
        "name = \"Demo\"\npath = \"/tmp/demo\"\n",
    )
    .unwrap();
    let (_, out) = run_hook(&home(&dir), &fixture("session_start"));
    assert!(
        out.contains("Summary: none yet (set it with the grove update_project tool)"),
        "{out}"
    );
}

/// Runs `grove hook` as it runs inside a Grove window: with GROVE_SESSION set.
fn run_hook_in_window(home: &Path, input: &str, grove_session: &str) -> i32 {
    let mut child = Command::new(env!("CARGO_BIN_EXE_grove"))
        .arg("hook")
        .env("GROVE_HOME", home)
        .env("GROVE_SESSION", grove_session)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait().unwrap().code().unwrap_or(-1)
}

#[test]
fn after_clear_new_conversation_id_still_updates_grove_session() {
    let dir = setup("working");
    let new_id = "99999999-aaaa-bbbb-cccc-dddddddddddd";
    run_hook_in_window(&home(&dir), &fixture("session_end_clear"), SID);
    assert_eq!(session(&dir)["state"], "your_turn");
    let input = fixture("user_prompt_submit").replace(SID, new_id);
    assert_eq!(run_hook_in_window(&home(&dir), &input, SID), 0);
    assert_eq!(session(&dir)["state"], "working");
    assert!(!home(&dir).join(format!("sessions/{new_id}.json")).exists());
}

#[test]
fn unknown_grove_session_env_falls_back_to_stdin_id() {
    let dir = setup("working");
    run_hook_in_window(&home(&dir), &fixture("stop"), "not-a-real-session");
    assert_eq!(session(&dir)["state"], "your_turn");
}

#[test]
fn hooks_follow_the_conversation_through_clear() {
    let dir = setup("your_turn");
    let ev =
        |id: &str, name: &str| format!(r#"{{"session_id":"{id}","hook_event_name":"{name}"}}"#);
    // /clear: the new conversation's SessionStart, then its first prompt.
    run_hook_in_window(&home(&dir), &ev("conv-2", "SessionStart"), SID);
    assert_eq!(session(&dir)["conversation"], "conv-2");
    run_hook_in_window(&home(&dir), &ev("conv-3", "UserPromptSubmit"), SID);
    let s = session(&dir);
    assert_eq!(s["conversation"], "conv-3");
    assert_eq!(s["state"], "working");
}

#[test]
fn an_earlier_runs_hooks_are_ignored() {
    let dir = setup("starting");
    let home = dir.path().join("grove home");
    let file = home.join(format!("sessions/{SID}.json"));
    let mut s: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    s["launch"] = "new-run".into();
    std::fs::write(&file, s.to_string()).unwrap();
    let state = || -> String {
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        v["state"].as_str().unwrap().to_string()
    };
    // The old Claude of a restart, exiting: no effect.
    run_hook_as(&home, &fixture("session_end"), Some("old-run"));
    assert_eq!(state(), "starting");
    // So is one from a window older than tokens.
    run_hook_as(&home, &fixture("session_end"), None);
    assert_eq!(state(), "starting");
    // The new run's events land.
    run_hook_as(&home, &fixture("stop"), Some("new-run"));
    assert_eq!(state(), "your_turn");
}

#[test]
fn session_start_shares_the_todo_list_and_how_to_keep_it() {
    let dir = setup("starting");
    let todos = home(&dir).join("todos");
    std::fs::create_dir_all(&todos).unwrap();
    std::fs::write(
        todos.join("demo.md"),
        "- [ ] fix the flaky test\n- [x] ship it\n- [ ] write the docs\n",
    )
    .unwrap();
    let (_, out) = run_hook(&home(&dir), &fixture("session_start"));
    assert!(out.contains("TODO list is kept in Grove"), "{out}");
    assert!(out.contains("edit_todos"), "{out}");
    assert!(out.contains("Open TODOs (2):"), "{out}");
    assert!(
        out.contains("- fix the flaky test\n- write the docs\n"),
        "{out}"
    );
    assert!(!out.contains("ship it"), "done items aren't listed: {out}");
}

#[test]
fn session_start_lists_ten_open_todos_and_counts_the_rest() {
    let dir = setup("starting");
    let todos = home(&dir).join("todos");
    std::fs::create_dir_all(&todos).unwrap();
    let list: String = (1..=12).map(|i| format!("- [ ] item {i}\n")).collect();
    std::fs::write(todos.join("demo.md"), list).unwrap();
    let (_, out) = run_hook(&home(&dir), &fixture("session_start"));
    assert!(out.contains("Open TODOs (12):"), "{out}");
    assert!(
        out.contains("- item 10\n") && !out.contains("- item 11\n"),
        "{out}"
    );
    assert!(out.contains("and 2 more"), "{out}");
}

#[test]
fn claude_todos_off_leaves_the_todo_list_out() {
    let dir = setup("starting");
    std::fs::write(home(&dir).join("config.toml"), "claude_todos = false\n").unwrap();
    let (_, out) = run_hook(&home(&dir), &fixture("session_start"));
    assert!(out.contains("Grove project: Demo"), "{out}");
    assert!(!out.contains("TODO"), "{out}");
}
