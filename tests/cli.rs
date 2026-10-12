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
fn status_json_reports_only_sessions_with_a_live_window() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join("projects")).unwrap();
    std::fs::create_dir_all(home.join("sessions")).unwrap();
    std::fs::write(
        home.join("projects/demo.toml"),
        "name = \"demo\"\npath = \"/tmp\"\n",
    )
    .unwrap();
    let socket = format!("grove-test-{}-status", std::process::id());
    let tmux = |args: &[&str]| {
        Command::new("tmux")
            .args(["-L", &socket, "-f", "/dev/null"])
            .args(args)
            .output()
            .unwrap()
    };
    let status = || {
        let out = grove(&home)
            .args(["status", "--json"])
            .env("GROVE_TMUX_SOCKET", &socket)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
    };
    let write_session = |window: &str| {
        std::fs::write(
            home.join("sessions/a.json"),
            format!(r#"{{"id":"a","project":"demo","name":"s1","host":"local","tmux_window":"{window}","state":"needs_you","message":"m","created_at":"2026-10-07T10:00:00Z","updated_at":"2026-10-07T10:00:00Z"}}"#),
        )
        .unwrap();
    };

    // No Grove tmux server (e.g. after a reboot): the stale file isn't live.
    write_session("@1");
    let v = status();
    assert_eq!(
        (v["version"].as_u64(), v["needs_you"].as_u64()),
        (Some(1), Some(0))
    );
    assert!(v["host"].as_str().is_some_and(|h| !h.is_empty()));

    // With the session's window running, it is.
    tmux(&["new-session", "-d", "-s", "grove", "sleep", "30"]);
    let w =
        String::from_utf8(tmux(&["display-message", "-p", "-t", "grove:0", "#{window_id}"]).stdout)
            .unwrap();
    write_session(w.trim());
    let v = status();
    let sock =
        String::from_utf8(tmux(&["display-message", "-p", "#{socket_path}"]).stdout).unwrap();
    tmux(&["kill-server"]);
    let _ = std::fs::remove_file(sock.trim());
    assert_eq!(v["needs_you"], 1, "{v}");
    assert_eq!(v["projects"][0]["sessions"][0]["name"], "s1");
}

/// Fake ssh + a fake grove in the "remote" home's ~/.local/bin.
fn fake_remote(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let ssh = dir.join("fake ssh");
    std::fs::write(&ssh, "#!/bin/sh\nwhile [ $# -gt 0 ]; do case \"$1\" in -o) shift 2;; -t|-T) shift;; *) break;; esac; done\nshift\nHOME=\"$FAKE_HOME\" exec sh -c \"$*\"\n").unwrap();
    let remote_home = dir.join("remote");
    let bin = remote_home.join(".local/bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("grove"), "#!/bin/sh\necho grove 0.1.0\n").unwrap();
    for f in [ssh.clone(), bin.join("grove")] {
        std::fs::set_permissions(&f, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    }
    (ssh, remote_home)
}

#[test]
fn finca_add_ls_rm() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let (ssh, remote_home) = fake_remote(dir.path());
    let run = |args: &[&str]| {
        grove(&home)
            .args(args)
            .env("GROVE_SSH", &ssh)
            .env("FAKE_HOME", &remote_home)
            .output()
            .unwrap()
    };

    let out = run(&["finca", "add", "homebox", "me@homebox.local"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let toml = std::fs::read_to_string(home.join("fincas/homebox.toml")).unwrap();
    assert!(toml.contains("ssh = \"me@homebox.local\""), "{toml}");
    assert!(
        toml.contains(&format!(
            "grove_bin = \"{}\"",
            remote_home.join(".local/bin/grove").display()
        )),
        "{toml}"
    );

    let out = run(&["finca", "ls"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("homebox"));

    let out = run(&["finca", "rm", "homebox"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!home.join("fincas/homebox.toml").exists());
}

#[test]
fn finca_add_failure_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let ssh = dir.path().join("failing ssh");
    std::fs::write(
        &ssh,
        "#!/bin/sh\necho 'Permission denied (publickey)' >&2\nexit 255\n",
    )
    .unwrap();
    std::fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let out = grove(&home)
        .args(["finca", "add", "pi", "me@pi"])
        .env("GROVE_SSH", &ssh)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Permission denied"));
    assert!(!home.join("fincas/pi.toml").exists());
}

#[test]
fn finca_rm_closes_an_open_connection_session() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let (ssh, remote_home) = fake_remote(dir.path());
    let socket = format!("grove-test-{}-clirm", std::process::id());
    let tmux = |args: &[&str]| {
        Command::new("tmux")
            .args(["-L", &socket, "-f", "/dev/null"])
            .args(args)
            .output()
            .unwrap()
    };
    let run = |args: &[&str]| {
        grove(&home)
            .args(args)
            .env("GROVE_SSH", &ssh)
            .env("FAKE_HOME", &remote_home)
            .env("GROVE_TMUX_SOCKET", &socket)
            .output()
            .unwrap()
    };
    assert!(run(&["finca", "add", "box", "me@box"]).status.success());
    // Keeps the server alive after rm closes finca-box, so cleanup can find it.
    tmux(&["new-session", "-d", "-s", "keepalive", "sleep", "30"]);
    tmux(&["new-session", "-d", "-s", "finca-box", "sleep", "30"]);
    let out = run(&["finca", "rm", "box"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let still = tmux(&["has-session", "-t", "=finca-box"]).status.success();
    let sock =
        String::from_utf8(tmux(&["display-message", "-p", "#{socket_path}"]).stdout).unwrap();
    tmux(&["kill-server"]);
    let _ = std::fs::remove_file(sock.trim());
    assert!(!still, "finca session should be closed");
}

#[test]
fn add_new_creates_folder_with_git() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let proj = dir.path().join("Play").join("fresh");
    let out = grove(&home)
        .args(["add", "--new"])
        .arg(&proj)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(proj.join(".git").is_dir());
    assert!(home.join("projects/fresh.toml").is_file());
}

#[test]
fn add_missing_folder_without_a_terminal_suggests_new() {
    let dir = tempfile::tempdir().unwrap();
    let out = grove(&dir.path().join("home"))
        .args(["add"])
        .arg(dir.path().join("nope"))
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("--new"), "{err}");
    assert!(!dir.path().join("nope").exists());
}

#[test]
fn add_says_which_account_the_folder_picks() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let (play, work) = (dir.path().join("play"), dir.path().join("work"));
    let (acme, side) = (dir.path().join("Work/acme"), dir.path().join("Play/side"));
    for d in [&home, &play, &work, &acme, &side] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(
        home.join("config.toml"),
        format!(
            "claude_config_dir = \"{}\"\n[accounts.work]\ndir = \"{}\"\nfolder = \"{}\"\n",
            play.display(),
            work.display(),
            dir.path().join("Work").display()
        ),
    )
    .unwrap();
    let add = |p: &std::path::Path| {
        let o = grove(&home)
            .args(["add", p.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(o.status.success(), "{o:?}");
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    let out = add(&acme);
    assert!(out.contains("work account"), "{out}");
    let out = add(&side);
    assert!(out.contains("play account"), "{out}");
    // The folder decides; nothing is stored.
    let toml = std::fs::read_to_string(home.join("projects/acme.toml")).unwrap();
    assert!(!toml.contains("account"), "{toml}");
}

#[test]
fn rm_forgets_a_project_but_not_its_folder() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let proj = dir.path().join("old");
    std::fs::create_dir_all(&proj).unwrap();
    // A tmux server with no sessions, so nothing counts as running.
    let socket = format!("grove-cli-rm-{}", std::process::id());
    let run = |args: &[&str]| {
        grove(&home)
            .env("GROVE_TMUX_SOCKET", &socket)
            .args(args)
            .output()
            .unwrap()
    };
    assert!(run(&["add", proj.to_str().unwrap()]).status.success());
    let out = run(&["rm", "old"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("Removed old"));
    assert!(!home.join("projects/old.toml").exists());
    assert!(proj.is_dir());
    assert!(!run(&["rm", "old"]).status.success(), "already gone");
}
