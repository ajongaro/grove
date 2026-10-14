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
    // So the laptop can jump straight into it.
    assert_eq!(v["projects"][0]["sessions"][0]["window"], w.trim());
}

#[test]
fn focus_makes_a_window_current_for_the_next_attach() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let socket = format!("grove-test-{}-focus", std::process::id());
    let tmux = |args: &[&str]| {
        let out = Command::new("tmux")
            .args(["-L", &socket, "-f", "/dev/null"])
            .args(args)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    tmux(&["new-session", "-d", "-s", "grove", "sleep", "30"]);
    let second = tmux(&[
        "new-window",
        "-d",
        "-P",
        "-F",
        "#{window_id}",
        "-t",
        "grove",
        "sleep",
        "30",
    ]);
    let focus = |window: &str| {
        grove(&home)
            .args(["focus", window])
            .env("GROVE_TMUX_SOCKET", &socket)
            .output()
            .unwrap()
    };
    let ok = focus(&second);
    let current = tmux(&["display-message", "-p", "-t", "grove", "#{window_id}"]);
    let missing = focus("@999");
    let sock = tmux(&["display-message", "-p", "#{socket_path}"]);
    tmux(&["kill-server"]);
    let _ = std::fs::remove_file(sock);
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    assert_eq!(current, second);
    assert!(
        !missing.status.success(),
        "a window that's gone is an error"
    );
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

#[test]
fn finca_setup_without_a_terminal_refuses_instead_of_saying_yes() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let out = grove(&home)
        .args(["finca", "setup", "me@homebox.invalid"])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("terminal"), "{err}");
    assert!(
        !home
            .join("fincas")
            .read_dir()
            .is_ok_and(|mut d| d.next().is_some())
    );
}

/// A test tmux server with session `finca-homebox` (a pane running
/// `cat -v` in raw-ish mode), a finca file, a fake ssh and a fake osascript.
struct PasteRig {
    _d: tempfile::TempDir,
    home: std::path::PathBuf,
    remote: std::path::PathBuf,
    socket: String,
    env: Vec<(String, std::path::PathBuf)>,
}

impl PasteRig {
    fn new(tag: &str, ssh_exit: Option<i32>) -> Self {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        std::fs::create_dir_all(home.join("fincas")).unwrap();
        std::fs::create_dir_all(home.join("run")).unwrap();
        std::fs::write(
            home.join("fincas/homebox.toml"),
            "name = \"homebox\"\nssh = \"me@homebox.local\"\ngrove_bin = \"/g\"\n",
        )
        .unwrap();
        let remote = d.path().join("remote home");
        std::fs::create_dir_all(&remote).unwrap();
        let ssh = d.path().join("fake ssh");
        let body = match ssh_exit {
            Some(c) => format!("#!/bin/sh\necho 'connection refused' >&2\nexit {c}\n"),
            None => "#!/bin/sh\nwhile [ $# -gt 0 ]; do case \"$1\" in -o) shift 2;; -t|-T) shift;; *) break;; esac; done\nshift\nHOME=\"$FAKE_HOME\" exec sh -c \"$*\"\n".into(),
        };
        std::fs::write(&ssh, body).unwrap();
        let osa = d.path().join("osascript");
        std::fs::write(&osa, "#!/bin/sh\nfor a; do last=$a; done\ncase \"$*\" in\n*furl*) [ -n \"$FAKE_CLIP_FILE\" ] || exit 1; echo \"$FAKE_CLIP_FILE\";;\n*PNGf*) : > \"$last\"; [ -n \"$FAKE_CLIP_PNG\" ] || exit 1; cat \"$FAKE_CLIP_PNG\" > \"$last\";;\n*) exit 1;;\nesac\n").unwrap();
        for p in [&ssh, &osa] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let socket = format!("grove-test-{}-{tag}", std::process::id());
        let rig = Self {
            env: vec![
                ("GROVE_SSH".into(), ssh),
                ("FAKE_HOME".into(), remote.clone()),
                ("GROVE_OSASCRIPT".into(), osa),
            ],
            home,
            remote,
            socket,
            _d: d,
        };
        rig.tmux(&[
            "new-session",
            "-d",
            "-s",
            "finca-homebox",
            "-x",
            "120",
            "-y",
            "10",
            "sh",
            "-c",
            "stty -icanon -iexten -echo; cat -v",
        ]);
        rig.tmux(&[
            "set-option",
            "-t",
            "finca-homebox",
            "@grove_dest",
            "me@homebox.local",
        ]);
        rig
    }

    fn tmux(&self, args: &[&str]) -> String {
        let out = Command::new("tmux")
            .args(["-L", &self.socket, "-f", "/dev/null"])
            .args(args)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn paste(&self, clip_png: Option<&std::path::Path>) -> std::process::Output {
        let pane = self.tmux(&["display-message", "-p", "-t", "finca-homebox", "#{pane_id}"]);
        let mut c = grove(&self.home);
        c.args(["paste", "--session", "finca-homebox", "--pane", &pane])
            .env("GROVE_TMUX_SOCKET", &self.socket)
            .env_remove("FAKE_CLIP_FILE");
        for (k, v) in &self.env {
            c.env(k, v);
        }
        match clip_png {
            Some(p) => c.env("FAKE_CLIP_PNG", p),
            None => c.env_remove("FAKE_CLIP_PNG"),
        };
        c.output().unwrap()
    }

    /// The pane's text, after giving cat a moment to echo.
    fn screen(&self) -> String {
        for _ in 0..50 {
            let s = self.tmux(&["capture-pane", "-p", "-t", "finca-homebox"]);
            if !s.trim().is_empty() {
                return s;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        String::new()
    }
}

impl Drop for PasteRig {
    fn drop(&mut self) {
        let sock = self.tmux(&["display-message", "-p", "#{socket_path}"]);
        self.tmux(&["kill-server"]);
        let _ = std::fs::remove_file(sock);
    }
}

#[test]
fn paste_sends_a_clipboard_image_and_types_its_path() {
    let rig = PasteRig::new("paste-img", None);
    let png = rig.home.join("shot.png");
    std::fs::write(&png, "pixels").unwrap();
    let out = rig.paste(Some(&png));
    assert!(
        out.status.success() && out.stdout.is_empty() && out.stderr.is_empty(),
        "{out:?}"
    );
    let inbox = rig.remote.join(".grove/inbox");
    let sent: Vec<_> = std::fs::read_dir(&inbox)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].to_string_lossy().ends_with("-clipboard.png"));
    assert_eq!(std::fs::read_to_string(&sent[0]).unwrap(), "pixels");
    assert!(
        rig.screen().contains(&*sent[0].to_string_lossy()),
        "{}",
        rig.screen()
    );
    assert_eq!(
        std::fs::read_dir(rig.home.join("run")).unwrap().count(),
        0,
        "temp png removed"
    );
}

#[test]
fn paste_without_an_image_passes_c_v_through() {
    let rig = PasteRig::new("paste-text", None);
    let out = rig.paste(None);
    assert!(out.status.success(), "{out:?}");
    assert!(rig.screen().contains("^V"), "{}", rig.screen());
}

#[test]
fn paste_reports_a_failed_send_and_pastes_nothing() {
    let rig = PasteRig::new("paste-down", Some(255));
    let png = rig.home.join("shot.png");
    std::fs::write(&png, "pixels").unwrap();
    let out = rig.paste(Some(&png));
    assert!(
        out.status.success() && out.stdout.is_empty() && out.stderr.is_empty(),
        "{out:?}"
    );
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(
        rig.tmux(&["capture-pane", "-p", "-t", "finca-homebox"])
            .trim()
            .is_empty()
    );
}
