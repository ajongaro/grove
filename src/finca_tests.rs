use super::testing::*;
use super::*;

/// Probe/poll tests change process env (GROVE_SSH, FAKE_HOME); run them one
/// at a time.
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Poll timeout for fakes that exit on their own (answer or fail). macOS
/// checks each freshly written executable on its first run (~1s, much more
/// under a parallel test run), so this is a ceiling, not an expectation.
const ANSWERS: std::time::Duration = std::time::Duration::from_secs(30);

/// Polls `cond` until it holds or `within` passes; returns whether it held.
fn eventually(within: std::time::Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + within;
    while std::time::Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    cond()
}

pub(crate) fn with_env<T>(
    ssh: &std::path::Path,
    fake_home: &std::path::Path,
    f: impl FnOnce() -> T,
) -> T {
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
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("printf %s {}", sh_quote("x'y $HOME `z`")))
        .output()
        .unwrap();
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
    let err = with_env(&ssh, &home, || probe("me@box", None))
        .unwrap_err()
        .to_string();
    assert!(err.contains("no grove") && err.contains("me@box"), "{err}");
}

#[test]
fn probe_reports_ssh_failure_with_key_hint() {
    let d = tempfile::tempdir().unwrap();
    let ssh = fake_ssh(d.path(), Some(255));
    let err = with_env(&ssh, d.path(), || probe("me@box", None))
        .unwrap_err()
        .to_string();
    assert!(err.contains("Connection refused"), "{err}");
    assert!(err.contains("ssh-copy-id"), "{err}");
}

fn status_json(host: &str) -> String {
    let d = tempfile::tempdir().unwrap();
    let st = crate::status::build(
        &crate::store::Store::new(crate::config::Paths::new(d.path().join("h"))),
        host,
        &[],
    );
    serde_json::to_string(&st).unwrap()
}

pub(crate) fn script(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    p
}

fn finca_with_bin(bin: &std::path::Path) -> Finca {
    Finca {
        name: "box".into(),
        ssh: "me@box".into(),
        grove_bin: bin.to_string_lossy().into(),
        color: None,
        backup: None,
    }
}

#[test]
fn poll_reachable_parses_status() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join("bin dir");
    std::fs::create_dir_all(&dir).unwrap();
    let bin = fake_grove(&dir, &status_json("homebox"));
    let ssh = fake_ssh(d.path(), None);
    let r = with_env(&ssh, d.path(), || poll_one(&finca_with_bin(&bin), ANSWERS));
    match r {
        Reach::Reachable(st) => assert_eq!(st.host, "homebox"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn poll_runs_grove_under_the_login_shell() {
    // Like Homebrew's tmux on macOS: only the login profile puts it on PATH,
    // and without it grove can't see which sessions are live.
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("remote home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join(".profile"),
        "echo 'welcome to the box'\nexport FROM_PROFILE=1\n",
    )
    .unwrap();
    let json = status_json("homebox");
    let bin = script(
        d.path(),
        "grove",
        &format!("[ \"$FROM_PROFILE\" = 1 ] || exit 1\ncat <<'JSON'\n{json}\nJSON"),
    );
    let ssh = fake_ssh(d.path(), None);
    let shell = std::env::var_os("SHELL");
    let r = with_env(&ssh, &home, || {
        unsafe { std::env::set_var("SHELL", "/bin/sh") };
        let r = poll_one(&finca_with_bin(&bin), ANSWERS);
        match &shell {
            Some(s) => unsafe { std::env::set_var("SHELL", s) },
            None => unsafe { std::env::remove_var("SHELL") },
        }
        r
    });
    match r {
        Reach::Reachable(st) => assert_eq!(st.host, "homebox"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn poll_ssh_failure_is_unreachable() {
    let d = tempfile::tempdir().unwrap();
    let ssh = fake_ssh(d.path(), Some(255));
    let r = with_env(&ssh, d.path(), || {
        poll_one(&finca_with_bin(std::path::Path::new("/g")), ANSWERS)
    });
    assert!(
        matches!(&r, Reach::Unreachable { error } if error.contains("Connection refused")),
        "{r:?}"
    );
}

#[test]
fn poll_old_grove_needs_update() {
    let d = tempfile::tempdir().unwrap();
    let bin = script(
        d.path(),
        "old-grove",
        "echo \"error: unrecognized subcommand 'status'\" >&2; exit 2",
    );
    let ssh = fake_ssh(d.path(), None);
    let r = with_env(&ssh, d.path(), || poll_one(&finca_with_bin(&bin), ANSWERS));
    assert_eq!(r, Reach::NeedsUpdate);
}

#[test]
fn poll_newer_status_version_needs_update() {
    let d = tempfile::tempdir().unwrap();
    let json = status_json("h").replace("\"version\":1", "\"version\":9");
    let bin = fake_grove(d.path(), &json);
    let ssh = fake_ssh(d.path(), None);
    let r = with_env(&ssh, d.path(), || poll_one(&finca_with_bin(&bin), ANSWERS));
    assert_eq!(r, Reach::NeedsUpdate);
}

#[test]
fn poll_hanging_finca_times_out() {
    let d = tempfile::tempdir().unwrap();
    let bin = script(d.path(), "slow-grove", "sleep 30");
    let ssh = fake_ssh(d.path(), None);
    // Timed inside with_env: waiting for ENV_LOCK behind other tests isn't polling.
    let (r, took) = with_env(&ssh, d.path(), || {
        let start = std::time::Instant::now();
        let r = poll_one(&finca_with_bin(&bin), std::time::Duration::from_millis(500));
        (r, start.elapsed())
    });
    assert!(took < std::time::Duration::from_secs(5), "took {took:?}");
    assert!(
        matches!(&r, Reach::Unreachable { error } if error.contains("timed out")),
        "{r:?}"
    );
}

#[test]
fn poller_reports_every_finca_and_responds_to_kick() {
    let d = tempfile::tempdir().unwrap();
    let store = crate::store::Store::new(crate::config::Paths::new(d.path().join("home")));
    let bin = fake_grove(d.path(), &status_json("a"));
    store
        .add_finca(&Finca {
            name: "alpha".into(),
            ssh: "x".into(),
            grove_bin: bin.to_string_lossy().into(),
            color: None,
            backup: None,
        })
        .unwrap();
    store
        .add_finca(&Finca {
            name: "beta".into(),
            ssh: "y".into(),
            grove_bin: "/nope".into(),
            color: None,
            backup: None,
        })
        .unwrap();
    let ssh = fake_ssh(d.path(), None);
    with_env(&ssh, d.path(), || {
        let poller = Poller::start(store.clone(), std::time::Duration::from_secs(3600), ANSWERS);
        let mut seen = std::collections::HashMap::new();
        while seen.len() < 2 {
            let r = poller
                .results
                .recv_timeout(2 * ANSWERS)
                .expect("first round");
            seen.insert(r.slug.clone(), r.reach);
        }
        assert!(matches!(seen["alpha"], Reach::Reachable(_)));
        assert!(!matches!(seen["beta"], Reach::Reachable(_)));
        poller.kick();
        let again = poller
            .results
            .recv_timeout(2 * ANSWERS)
            .expect("kicked round");
        assert!(["alpha", "beta"].contains(&again.slug.as_str()));
    });
}

// ---------- backup ----------

/// Like `fake_ssh`, but `me@home` is unreachable.
fn ssh_without_home(dir: &std::path::Path) -> std::path::PathBuf {
    script(
        dir,
        "ssh no home",
        "while [ $# -gt 0 ]; do case \"$1\" in -o) shift 2;; -t|-T) shift;; *) break;; esac; done\n\
         if [ \"$1\" = me@home ]; then echo 'ssh: Could not resolve hostname home' >&2; exit 255; fi\n\
         shift\nHOME=\"$FAKE_HOME\" exec sh -c \"$*\"",
    )
}

fn away_finca(bin: &std::path::Path, backup: Option<&str>) -> Finca {
    Finca {
        name: "box".into(),
        ssh: "me@home".into(),
        grove_bin: bin.to_string_lossy().into(),
        color: None,
        backup: backup.map(Into::into),
    }
}

#[test]
fn poll_falls_back_to_backup() {
    let d = tempfile::tempdir().unwrap();
    let bin = fake_grove(d.path(), &status_json("homebox"));
    let ssh = ssh_without_home(d.path());
    let (reach, via) = with_env(&ssh, d.path(), || {
        poll_via(&away_finca(&bin, Some("me@box.ts.net")), ANSWERS)
    });
    assert!(matches!(reach, Reach::Reachable(_)), "{reach:?}");
    assert_eq!(via.as_deref(), Some("me@box.ts.net"));
}

#[test]
fn poll_prefers_primary_when_it_answers() {
    let d = tempfile::tempdir().unwrap();
    let bin = fake_grove(d.path(), &status_json("homebox"));
    let ssh = fake_ssh(d.path(), None);
    let mut f = away_finca(&bin, Some("me@box.ts.net"));
    f.ssh = "me@box.local".into();
    let (_, via) = with_env(&ssh, d.path(), || poll_via(&f, ANSWERS));
    assert_eq!(via.as_deref(), Some("me@box.local"));
}

#[test]
fn poll_with_both_down_names_each_destination() {
    let d = tempfile::tempdir().unwrap();
    let ssh = fake_ssh(d.path(), Some(255));
    let (reach, via) = with_env(&ssh, d.path(), || {
        poll_via(
            &away_finca(std::path::Path::new("/g"), Some("me@box.ts.net")),
            ANSWERS,
        )
    });
    assert_eq!(via, None);
    assert!(
        matches!(&reach, Reach::Unreachable { error }
            if error.contains("me@home:") && error.contains("me@box.ts.net:")),
        "{reach:?}"
    );
}

#[test]
fn finca_file_without_backup_still_parses() {
    let f: Finca =
        toml::from_str("name = \"pi\"\nssh = \"a@pi.local\"\ngrove_bin = \"/g\"\n").unwrap();
    assert_eq!(f.backup, None);
    assert_eq!(f.destinations().collect::<Vec<_>>(), ["a@pi.local"]);
    let with = toml::to_string(&Finca {
        backup: Some("a@pi.ts.net".into()),
        ..f
    })
    .unwrap();
    assert!(with.contains("backup = \"a@pi.ts.net\""), "{with}");
}

#[test]
fn connect_argv_uses_the_given_destination() {
    let d = tempfile::tempdir().unwrap();
    let args_out = d.path().join("args.txt");
    let ssh = script(
        d.path(),
        "echo ssh",
        &format!(
            "for a in \"$@\"; do echo \"$a\"; done > '{}'",
            args_out.display()
        ),
    );
    let f = away_finca(std::path::Path::new("/g"), Some("me@box.ts.net"));
    let argv = with_env(&ssh, d.path(), || connect_argv(&f, "me@box.ts.net"));
    assert!(
        std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .status()
            .unwrap()
            .success()
    );
    let got = std::fs::read_to_string(&args_out).unwrap();
    assert!(got.lines().any(|l| l == "me@box.ts.net"), "{got}");
    assert!(!got.lines().any(|l| l == "me@home"), "{got}");
}

#[test]
fn shell_argv_opens_a_plain_login_shell_at_the_destination() {
    let d = tempfile::tempdir().unwrap();
    let args_out = d.path().join("args.txt");
    let ssh = script(
        d.path(),
        "echo ssh",
        &format!(
            "for a in \"$@\"; do echo \"$a\"; done > '{}'",
            args_out.display()
        ),
    );
    let f = away_finca(std::path::Path::new("/g"), Some("me@box.ts.net"));
    let argv = with_env(&ssh, d.path(), || shell_argv(&f, "me@box.ts.net"));
    assert!(
        std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .status()
            .unwrap()
            .success()
    );
    let got = std::fs::read_to_string(&args_out).unwrap();
    let args: Vec<_> = got.lines().collect();
    assert!(args.contains(&"-t"), "{got}");
    // No remote command: ssh starts the user's login shell.
    assert_eq!(args.last(), Some(&"me@box.ts.net"), "{got}");
    assert!(!got.contains("/g"), "{got}");
}

// ---------- connect ----------

#[test]
fn connect_argv_quotes_dest_and_bin_for_both_shells() {
    let d = tempfile::tempdir().unwrap();
    let args_out = d.path().join("args.txt");
    let ssh = script(
        d.path(),
        "echo ssh",
        &format!(
            "for a in \"$@\"; do echo \"$a\"; done > '{}'",
            args_out.display()
        ),
    );
    let f = Finca {
        name: "my box".into(),
        ssh: "me@it's here".into(),
        grove_bin: "/opt/my grove/gro've".into(),
        color: None,
        backup: None,
    };
    let argv = with_env(&ssh, d.path(), || connect_argv(&f, &f.ssh));
    let st = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .status()
        .unwrap();
    assert!(st.success());
    let got: Vec<String> = std::fs::read_to_string(&args_out)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    // The remote shell gets one string that runs the quoted bin via a login shell.
    // Sharing options (with keep-alives), then the destination and one
    // string that runs the quoted bin via a login shell.
    let mut want = vec!["-t".to_string()];
    want.extend(share_opts());
    assert!(want.contains(&"ServerAliveInterval=15".to_string()));
    want.push("me@it's here".into());
    want.push(remote_login_cmd("/opt/my grove/gro've"));
    assert_eq!(got, want);
}

/// Runs an ssh_script (with the fake ssh `body`) as the window would,
/// pressing Enter at the pause, and returns what it printed.
fn run_failed_ssh(body: &str, remote: Option<&str>, grace: u64) -> String {
    let d = tempfile::tempdir().unwrap();
    let ssh = script(d.path(), "bad ssh", body);
    let f = Finca {
        name: "pi".into(),
        ssh: "x".into(),
        grove_bin: "/g".into(),
        color: None,
        backup: None,
    };
    let sh = with_env(&ssh, d.path(), || {
        ssh_script_with(&f, &f.ssh, remote, grace)
    });
    let out = std::process::Command::new("sh")
        .args(["-c", &sh])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut c| {
            use std::io::Write;
            c.stdin.take().unwrap().write_all(b"\n")?;
            c.wait_with_output()
        })
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn connect_failure_says_it_couldnt_connect_and_why() {
    let text = run_failed_ssh(
        "echo 'ssh: connect to host x port 22: No route to host' >&2; exit 255",
        Some("grove"),
        10,
    );
    assert!(text.contains("Couldn't connect to pi"), "{text}");
    assert!(text.contains("No route to host"), "{text}");
    assert!(text.contains("Press Enter to close"), "{text}");
}

#[test]
fn connect_failure_resets_the_screen_first() {
    let text = run_failed_ssh("exit 255", Some("grove"), 10);
    // Leaves the remote's full-screen view and stops focus and theme reports
    // before anything is printed.
    let reset = text
        .find("\x1b[?1049l")
        .expect("leaves the alternate screen");
    assert!(text.contains("\x1b[?1004l"), "focus reports off");
    assert!(text.contains("\x1b[?2031l"), "theme reports off");
    assert!(reset < text.find("Couldn't").unwrap(), "{text:?}");
}

#[test]
fn a_dropped_connection_says_it_was_lost() {
    let text = run_failed_ssh(
        "echo 'Shared connection to x closed.' >&2; exit 255",
        Some("grove"),
        0,
    );
    assert!(text.contains("Lost the connection to pi"), "{text}");
    assert!(text.contains("Shared connection to x closed."), "{text}");
}

#[test]
fn a_remote_grove_failure_names_its_exit_code() {
    let text = run_failed_ssh("exit 3", Some("grove"), 10);
    assert!(text.contains("Grove on pi exited (code 3)"), "{text}");
}

#[test]
fn a_remote_shell_failure_names_its_exit_code() {
    let text = run_failed_ssh("exit 3", None, 10);
    assert!(text.contains("The shell on pi exited (code 3)"), "{text}");
}

#[test]
fn a_clean_exit_does_not_pause() {
    let text = run_failed_ssh(
        "echo 'Shared connection to x closed.' >&2; exit 0",
        Some("grove"),
        10,
    );
    assert!(!text.contains("Enter"), "{text}");
}

#[test]
fn a_failed_connection_leaves_a_clean_pane() {
    let d = tempfile::tempdir().unwrap();
    let tmux = crate::tmux::Tmux::new(format!("grove-test-{}-sshfail", std::process::id()));
    struct Kill<'a>(&'a crate::tmux::Tmux);
    impl Drop for Kill<'_> {
        fn drop(&mut self) {
            self.0.kill_server();
        }
    }
    let _k = Kill(&tmux);
    // The remote dashboard takes over the screen, asks for mouse and focus
    // reports, then the link dies.
    let ssh = script(
        d.path(),
        "dying ssh",
        "printf '\\033[?1049h\\033[?1000h\\033[?1004hREMOTE DASHBOARD'; sleep 0.3; exit 255",
    );
    let f = Finca {
        name: "pi".into(),
        ssh: "x".into(),
        grove_bin: "/g".into(),
        color: None,
        backup: None,
    };
    let argv = with_env(&ssh, d.path(), || connect_argv(&f, &f.ssh));
    tmux.new_session_detached("finca-pi", "pi", d.path(), &argv)
        .unwrap();
    let show =
        |what: &[&str]| String::from_utf8(tmux.cmd().args(what).output().unwrap().stdout).unwrap();
    let mut screen = String::new();
    for _ in 0..50 {
        screen = show(&["capture-pane", "-p", "-t", "=finca-pi:"]);
        if screen.contains("Press Enter") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(screen.contains("Couldn't connect to pi"), "{screen}");
    assert!(!screen.contains("REMOTE DASHBOARD"), "{screen}");
    let flags = show(&[
        "display-message",
        "-p",
        "-t",
        "=finca-pi:",
        "#{alternate_on} #{mouse_any_flag}",
    ]);
    assert_eq!(flags.trim(), "0 0");
}

#[test]
fn finca_session_has_its_own_key_table_and_no_prefix() {
    let d = tempfile::tempdir().unwrap();
    let conf = d.path().join("tmux.conf");
    std::fs::write(
        &conf,
        crate::runfiles::tmux_conf(
            "grove",
            &crate::config::Paths::new(d.path().join("conf-home")),
            None,
        ),
    )
    .unwrap();
    let tmux = crate::tmux::Tmux {
        config: Some(conf),
        ..crate::tmux::Tmux::new(format!("grove-test-{}-finca", std::process::id()))
    };
    struct Kill<'a>(&'a crate::tmux::Tmux);
    impl Drop for Kill<'_> {
        fn drop(&mut self) {
            self.0.kill_server();
        }
    }
    let _k = Kill(&tmux);
    tmux.new_session_detached(
        "grove",
        "dashboard",
        d.path(),
        &["sleep".into(), "30".into()],
    )
    .unwrap();
    let ssh = script(d.path(), "idle ssh", "sleep 30");
    let f = Finca {
        name: "homebox".into(),
        ssh: "x".into(),
        grove_bin: "/g".into(),
        color: Some("#5c6bc0".into()),
        backup: None,
    };

    assert!(
        with_env(&ssh, d.path(), || ensure_finca_session(&tmux, &f, &f.ssh)).unwrap(),
        "created"
    );
    assert!(
        !with_env(&ssh, d.path(), || ensure_finca_session(&tmux, &f, &f.ssh)).unwrap(),
        "reused, not duplicated"
    );
    let opt = |o: &str| {
        String::from_utf8(
            tmux.cmd()
                .args(["show-options", "-v", "-t", "=finca-homebox:", o])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
    };
    assert_eq!(opt("key-table").trim(), "grove-finca");
    assert_eq!(opt("prefix").trim(), "None");
    let keys = String::from_utf8(
        tmux.cmd()
            .args(["list-keys", "-T", "grove-finca"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(
        keys.contains("C-q") && keys.contains("switch-client -t =grove"),
        "{keys}"
    );
    assert!(
        keys.contains("C-v") && keys.contains("paste --session"),
        "{keys}"
    );
    assert_eq!(
        keys.lines().count(),
        2,
        "only C-q and C-v in the finca table: {keys}"
    );
    // C-v sends through the address this connection uses.
    assert_eq!(opt("@grove_dest").trim(), "x");

    disconnect(&tmux, "homebox");
    assert!(!tmux.has_session("finca-homebox"));
}

#[test]
fn shell_session_keeps_the_prefix_opens_windows_on_the_finca_and_c_q_returns() {
    let d = tempfile::tempdir().unwrap();
    let conf = d.path().join("tmux.conf");
    std::fs::write(
        &conf,
        crate::runfiles::tmux_conf(
            "grove",
            &crate::config::Paths::new(d.path().join("conf-home")),
            None,
        ),
    )
    .unwrap();
    let tmux = crate::tmux::Tmux {
        config: Some(conf),
        ..crate::tmux::Tmux::new(format!("grove-test-{}-shell", std::process::id()))
    };
    struct Kill<'a>(&'a crate::tmux::Tmux);
    impl Drop for Kill<'_> {
        fn drop(&mut self) {
            self.0.kill_server();
        }
    }
    let _k = Kill(&tmux);
    tmux.new_session_detached(
        "grove",
        "dashboard",
        d.path(),
        &["sleep".into(), "30".into()],
    )
    .unwrap();
    let ssh = script(d.path(), "idle ssh", "sleep 30");
    let f = Finca {
        name: "homebox".into(),
        ssh: "x".into(),
        grove_bin: "/g".into(),
        color: None,
        backup: None,
    };
    assert!(with_env(&ssh, d.path(), || ensure_shell_session(&tmux, &f, &f.ssh)).unwrap());
    assert!(
        !with_env(&ssh, d.path(), || ensure_shell_session(&tmux, &f, &f.ssh)).unwrap(),
        "reused, not duplicated"
    );
    let out =
        |args: &[&str]| String::from_utf8(tmux.cmd().args(args).output().unwrap().stdout).unwrap();
    let opt = |o: &str| out(&["show-options", "-v", "-t", "=finca-homebox-sh:", o]);
    assert_eq!(opt("key-table").trim(), "grove-shell");
    assert_ne!(opt("prefix").trim(), "None", "the prefix works in a shell");
    // New windows and splits ssh to the finca too, not a local shell.
    assert!(
        opt("default-command").contains("idle ssh"),
        "{}",
        opt("default-command")
    );
    // The dashboard lists the shell's windows to jump back into.
    assert_eq!(shell_windows(&tmux, "homebox").len(), 1);
    out(&["new-window", "-d", "-t", "=finca-homebox-sh:"]);
    let wins = shell_windows(&tmux, "homebox");
    assert_eq!(wins.len(), 2, "{wins:?}");
    assert!(wins.iter().all(|w| w.id.starts_with('@')), "{wins:?}");
    assert!(shell_windows(&tmux, "nowhere").is_empty());
    let keys = out(&["list-keys", "-T", "grove-shell"]);
    assert!(
        keys.contains("C-q") && keys.contains("switch-client -t =grove"),
        "{keys}"
    );

    with_env(&ssh, d.path(), || ensure_finca_session(&tmux, &f, &f.ssh)).unwrap();
    disconnect(&tmux, "homebox");
    assert!(!tmux.has_session("finca-homebox"));
    assert!(
        !tmux.has_session("finca-homebox-sh"),
        "rm closes the shell too"
    );
}

#[test]
fn connect_runs_remote_grove_through_a_login_shell_with_awkward_paths() {
    let d = tempfile::tempdir().unwrap();
    let marker = d.path().join("ran.txt");
    let dir = d.path().join("my grove");
    std::fs::create_dir_all(&dir).unwrap();
    // Records that it ran, and whether it saw the login shell's PATH entry.
    let bin = script(
        &dir,
        "gro've",
        &format!(
            "echo \"ran login=${{GROVE_LOGIN_MARK:-no}}\" > '{}'",
            marker.display()
        ),
    );
    let remote_home = d.path().join("remote home");
    std::fs::create_dir_all(&remote_home).unwrap();
    // A login shell reads ~/.profile; a plain `sh -c` would not.
    std::fs::write(
        remote_home.join(".profile"),
        "export GROVE_LOGIN_MARK=yes\n",
    )
    .unwrap();
    let ssh = fake_ssh(d.path(), None);
    let f = Finca {
        name: "box".into(),
        ssh: "me@it's here".into(),
        grove_bin: bin.to_string_lossy().into(),
        color: None,
        backup: None,
    };
    let argv = with_env(&ssh, &remote_home, || connect_argv(&f, &f.ssh));
    let st = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .env("SHELL", "/bin/sh")
        .env("FAKE_HOME", &remote_home)
        .status()
        .unwrap();
    assert!(st.success());
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap().trim(),
        "ran login=yes"
    );
}

#[test]
fn when_the_finca_connection_ends_the_client_returns_to_grove() {
    let d = tempfile::tempdir().unwrap();
    let conf = d.path().join("tmux.conf");
    std::fs::write(
        &conf,
        crate::runfiles::tmux_conf(
            "grove",
            &crate::config::Paths::new(d.path().join("conf-home")),
            None,
        ),
    )
    .unwrap();
    let inner = format!("grove-test-{}-ret", std::process::id());
    let host = format!("grove-test-{}-rethost", std::process::id());
    let tmux = crate::tmux::Tmux {
        config: Some(conf.clone()),
        ..crate::tmux::Tmux::new(inner.clone())
    };
    let host_tmux = crate::tmux::Tmux::new(host.clone());
    struct Kill<'a>(&'a crate::tmux::Tmux, &'a crate::tmux::Tmux);
    impl Drop for Kill<'_> {
        fn drop(&mut self) {
            self.0.kill_server();
            self.1.kill_server();
        }
    }
    let _k = Kill(&tmux, &host_tmux);
    host_tmux
        .new_session_detached("keepalive", "k", d.path(), &["sleep".into(), "30".into()])
        .unwrap();
    tmux.new_session_detached(
        "grove",
        "dashboard",
        d.path(),
        &["sleep".into(), "30".into()],
    )
    .unwrap();
    // The "remote grove" quits cleanly after a moment, like pressing q there.
    let ssh = script(d.path(), "quitting ssh", "sleep 2; exit 0");
    let f = Finca {
        name: "box".into(),
        ssh: "x".into(),
        grove_bin: "/g".into(),
        color: None,
        backup: None,
    };
    with_env(&ssh, d.path(), || ensure_finca_session(&tmux, &f, &f.ssh)).unwrap();
    // A real client (in a pane of another server) attached to the finca.
    host_tmux
        .new_session_detached(
            "h",
            "h",
            d.path(),
            &[
                "tmux".into(),
                "-L".into(),
                inner.clone(),
                "attach".into(),
                "-t".into(),
                "=finca-box".into(),
            ],
        )
        .unwrap();
    let client_session = || {
        let out = tmux
            .cmd()
            .args(["list-clients", "-F", "#{client_session}"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    std::thread::sleep(std::time::Duration::from_millis(800));
    assert_eq!(client_session(), "finca-box");
    assert!(
        eventually(ANSWERS, || client_session() == "grove"),
        "client should land back on the dashboard, not be detached (on {:?})",
        client_session()
    );
}

// ---------- connection sharing ----------

#[test]
fn share_opts_reuse_one_connection_per_host_with_keepalives() {
    let opts = share_opts_in(std::path::Path::new("/h/.grove/run"));
    let joined = opts.join(" ");
    assert_eq!(
        joined,
        "-o ControlMaster=auto -o ControlPath=/h/.grove/run/ssh-%C -o ControlPersist=10m \
         -o ServerAliveInterval=15 -o ServerAliveCountMax=3"
    );
}

#[test]
fn share_opts_are_skipped_when_the_socket_path_would_be_too_long() {
    // macOS caps socket paths at 104 bytes; ssh adds a 40-char hash and a
    // temporary 17-char suffix.
    let deep = format!("/{}", "d".repeat(60));
    assert!(share_opts_in(std::path::Path::new(&deep)).is_empty());
}

#[test]
fn every_ssh_grove_runs_shares_connections() {
    let d = tempfile::tempdir().unwrap();
    let args_out = d.path().join("args.txt");
    let ssh = script(
        d.path(),
        "record ssh",
        &format!("echo \"$*\" >> '{}'", args_out.display()),
    );
    let f = finca_with_bin(std::path::Path::new("/g"));
    with_env(&ssh, d.path(), || {
        let _ = probe("me@box", None);
        let _ = poll_one(&f, ANSWERS);
        for argv in [connect_argv(&f, "me@box"), shell_argv(&f, "me@box")] {
            std::process::Command::new(&argv[0])
                .args(&argv[1..])
                .status()
                .unwrap();
        }
    });
    let calls = std::fs::read_to_string(&args_out).unwrap();
    assert_eq!(calls.lines().count(), 4, "{calls}");
    for call in calls.lines() {
        assert!(call.contains("ControlMaster=auto"), "{call}");
    }
}

#[test]
fn shell_window_titles_skip_tmuxs_default() {
    // tmux titles a pane with this machine's host name until the remote
    // shell sets one.
    let out = "@4\t1\tlaptop\tlaptop\n@7\t2\tme@homebox: ~/stacks\tlaptop\n";
    assert_eq!(
        parse_shell_windows(out),
        [
            ShellWindow {
                id: "@4".into(),
                index: 1,
                title: None
            },
            ShellWindow {
                id: "@7".into(),
                index: 2,
                title: Some("me@homebox: ~/stacks".into())
            },
        ]
    );
}

#[test]
fn focus_asks_the_finca_to_select_the_window() {
    let d = tempfile::tempdir().unwrap();
    let log = d.path().join("args.txt");
    let bin = script(
        d.path(),
        "grove",
        &format!("echo \"$@\" > {}", sh_quote(&log.to_string_lossy())),
    );
    let ssh = fake_ssh(d.path(), None);
    with_env(&ssh, d.path(), || {
        focus(&finca_with_bin(&bin), "me@box", "@3")
    })
    .unwrap();
    assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), "focus @3");

    let failing = script(d.path(), "old grove", "echo 'no such command' >&2; exit 2");
    let err = with_env(&ssh, d.path(), || {
        focus(&finca_with_bin(&failing), "me@box", "@3")
    })
    .unwrap_err();
    assert!(format!("{err:#}").contains("no such command"), "{err:#}");
}

#[test]
fn facts_script_reports_the_machine() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("remote home");
    std::fs::create_dir_all(home.join(".local/bin")).unwrap();
    fake_grove(&home.join(".local/bin"), "{}");
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    script(&bin, "tmux", "exit 0");
    let ssh = fake_ssh(d.path(), None);
    let out = with_env(&ssh, &home, || {
        unsafe { std::env::set_var("FAKE_PATH", format!("{}:/usr/bin:/bin", bin.display())) };
        let r = run_remote("me@box", &facts_script());
        unsafe { std::env::remove_var("FAKE_PATH") };
        r
    })
    .unwrap();
    let f = crate::setup::parse_facts(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(f.home, home.to_string_lossy());
    assert!(f.has("tmux") && !f.has("claude"));
    assert_eq!(
        f.grove,
        Some((
            home.join(".local/bin/grove").to_string_lossy().into(),
            format!("grove {}", env!("CARGO_PKG_VERSION"))
        ))
    );
}

#[test]
fn remote_stdin_feeds_the_command() {
    let d = tempfile::tempdir().unwrap();
    let ssh = fake_ssh(d.path(), None);
    let src = d.path().join("in.txt");
    std::fs::write(&src, "hello").unwrap();
    let out = with_env(&ssh, d.path(), || {
        remote_stdin("me@box", "cat", std::fs::File::open(&src).unwrap().into())
    })
    .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hello");
}

#[test]
fn interactive_ssh_has_a_tty_and_the_command() {
    let argv = interactive_ssh("me@box", Some("claude auth login"), &[]);
    assert!(argv.contains(&"-t".to_string()), "{argv:?}");
    assert_eq!(argv.last().unwrap(), &login_shell_cmd("claude auth login"));
    let accept = interactive_ssh(
        "me@box",
        Some("true"),
        &["-o", "StrictHostKeyChecking=accept-new"],
    );
    assert!(
        accept
            .windows(2)
            .any(|w| w == ["-o", "StrictHostKeyChecking=accept-new"])
    );
}

fn plain(c: &str) -> String {
    c.to_string()
}

fn facts_for(home: &std::path::Path, same_platform: bool) -> crate::setup::Facts {
    crate::setup::Facts {
        os: if same_platform {
            std::env::consts::OS.into()
        } else {
            "plan9".into()
        },
        arch: std::env::consts::ARCH.into(),
        host: "homebox".into(),
        home: home.to_string_lossy().into(),
        tools: ["cargo".to_string()].into(),
        grove: None,
    }
}

#[test]
fn same_platform_install_copies_this_binary_via_a_swap() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("remote home");
    std::fs::create_dir_all(&home).unwrap();
    let exe = fake_grove(d.path(), "{}");
    let bin = home.join(".local/bin/grove");
    let ssh = fake_ssh(d.path(), None);
    let v = with_env(&ssh, &home, || {
        install_grove(
            "me@box",
            &facts_for(&home, true),
            &bin.to_string_lossy(),
            How::Copy(&exe),
            plain,
        )
    })
    .unwrap();
    assert_eq!(v, format!("grove {}", env!("CARGO_PKG_VERSION")));
    assert_eq!(std::fs::read(&bin).unwrap(), std::fs::read(&exe).unwrap());
    assert!(
        !home.join(".local/bin/grove.new").exists(),
        "swapped into place"
    );
}

#[test]
fn other_platform_ships_source_without_target_or_git() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("remote home");
    let src = d.path().join("src");
    for p in ["target/release", ".git", "src"] {
        std::fs::create_dir_all(src.join(p)).unwrap();
    }
    std::fs::write(src.join("Cargo.toml"), "[package]").unwrap();
    std::fs::write(src.join("src/main.rs"), "fn main(){}").unwrap();
    std::fs::write(src.join("target/release/big"), "x").unwrap();
    std::fs::create_dir_all(home.join("src/grove")).unwrap();
    std::fs::write(home.join("src/grove/stale.rs"), "old").unwrap();
    std::fs::create_dir_all(home.join("src/grove/.git")).unwrap();
    std::fs::write(home.join("src/grove/.git/HEAD"), "ref").unwrap();
    // A fake cargo "build" that installs a grove into target/release.
    let bin_dir = home.join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let grove = fake_grove(d.path(), "{}");
    script(
        &bin_dir,
        "cargo",
        &format!(
            "mkdir -p target/release && cp {} target/release/grove",
            sh_quote(&grove.to_string_lossy())
        ),
    );
    let bin = home.join(".local/bin/grove");
    let ssh = fake_ssh(d.path(), None);
    let v = with_env(&ssh, &home, || {
        unsafe { std::env::set_var("FAKE_PATH", format!("{}:/usr/bin:/bin", bin_dir.display())) };
        let r = install_grove(
            "me@box",
            &facts_for(&home, false),
            &bin.to_string_lossy(),
            How::Build(&src),
            plain,
        );
        unsafe { std::env::remove_var("FAKE_PATH") };
        r
    })
    .unwrap();
    assert_eq!(v, format!("grove {}", env!("CARGO_PKG_VERSION")));
    let shipped = home.join("src/grove");
    assert!(shipped.join("src/main.rs").exists());
    assert_eq!(
        std::fs::read_to_string(shipped.join(".git/HEAD")).unwrap(),
        "ref",
        "a clone there keeps its .git"
    );
    assert!(!shipped.join("stale.rs").exists(), "old files go");
    assert!(
        !shipped.join("target/release/big").exists(),
        "local target isn't shipped"
    );
}

#[test]
fn source_build_without_source_folder_fails_clearly() {
    let d = tempfile::tempdir().unwrap();
    let gone = d.path().join("deleted checkout");
    let err = install_grove(
        "me@box",
        &facts_for(d.path(), false),
        "/x/grove",
        How::Build(&gone),
        plain,
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("source"), "{err:#}");
}

#[test]
fn source_build_needs_cargo_there() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("Cargo.toml"), "").unwrap();
    let mut f = facts_for(d.path(), false);
    f.tools.clear();
    let err = install_grove("me@box", &f, "/x/grove", How::Build(d.path()), plain).unwrap_err();
    assert!(format!("{err:#}").contains("cargo"), "{err:#}");
}

#[test]
fn a_copy_that_wont_run_there_keeps_the_working_grove() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("remote home");
    let bin = home.join(".local/bin/grove");
    std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
    std::fs::write(&bin, "working").unwrap();
    let broken = script(d.path(), "broken", "exit 1");
    let ssh = fake_ssh(d.path(), None);
    let r = with_env(&ssh, &home, || {
        install_grove(
            "me@box",
            &facts_for(&home, true),
            &bin.to_string_lossy(),
            How::Copy(&broken),
            plain,
        )
    });
    assert!(r.is_err());
    assert_eq!(std::fs::read_to_string(&bin).unwrap(), "working");
}

#[cfg(target_os = "macos")]
#[test]
fn shipped_source_has_no_macos_metadata_files() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("remote home");
    let src = d.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("Cargo.toml"), "[package]").unwrap();
    // An extended attribute is what makes macOS tar add a ._ file.
    std::process::Command::new("xattr")
        .args(["-w", "com.example.test", "x"])
        .arg(src.join("Cargo.toml"))
        .status()
        .unwrap();
    // The finca extracts like Linux tar: ._ entries become files.
    let bin = d.path().join("finca bin");
    std::fs::create_dir_all(&bin).unwrap();
    script(
        &bin,
        "tar",
        "exec /usr/bin/python3 -c \"import sys, tarfile; tarfile.open(fileobj=sys.stdin.buffer, mode='r|').extractall('.')\"",
    );
    let ssh = fake_ssh(d.path(), None);
    with_env(&ssh, &home, || {
        unsafe { std::env::set_var("FAKE_PATH", format!("{}:/usr/bin:/bin", bin.display())) };
        let r = ship_source("me@box", &src);
        unsafe { std::env::remove_var("FAKE_PATH") };
        r
    })
    .unwrap();
    let names: Vec<String> = std::fs::read_dir(home.join("src/grove"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["Cargo.toml"]);
}

fn platform_facts(os: &str, arch: &str) -> crate::setup::Facts {
    crate::setup::Facts {
        os: os.into(),
        arch: arch.into(),
        host: "pi".into(),
        home: "/home/me".into(),
        tools: Default::default(),
        grove: None,
    }
}

/// Another platform than this machine's.
fn elsewhere() -> crate::setup::Facts {
    if std::env::consts::OS == "linux" {
        platform_facts("macos", "aarch64")
    } else {
        platform_facts("linux", "aarch64")
    }
}

#[test]
fn install_plan_copies_downloads_or_builds() {
    let here = platform_facts(std::env::consts::OS, std::env::consts::ARCH);
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("Cargo.toml"), "").unwrap();
    let src = Some(d.path());
    assert_eq!(install_plan(&here, true, src), Plan::Copy);
    assert_eq!(install_plan(&here, false, None), Plan::Copy);
    // A release build fetches the matching release, checkout or not.
    assert_eq!(install_plan(&elsewhere(), true, src), Plan::Download);
    // A source build builds there, so the finca gets the same code...
    assert_eq!(install_plan(&elsewhere(), false, src), Plan::Build);
    // ...unless its checkout is gone.
    assert_eq!(install_plan(&elsewhere(), false, None), Plan::Download);
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(
        install_plan(&elsewhere(), false, Some(empty.path())),
        Plan::Download
    );
}

#[test]
fn release_targets_per_platform() {
    assert_eq!(
        release_target("linux", "aarch64").unwrap(),
        "aarch64-unknown-linux-musl"
    );
    assert_eq!(
        release_target("linux", "x86_64").unwrap(),
        "x86_64-unknown-linux-musl"
    );
    assert_eq!(
        release_target("macos", "aarch64").unwrap(),
        "aarch64-apple-darwin"
    );
    assert_eq!(
        release_target("macos", "x86_64").unwrap(),
        "x86_64-apple-darwin"
    );
    assert!(release_target("freebsd", "x86_64").is_err());
    assert!(release_target("linux", "armv7l").is_err());
}

/// A release folder with a grove for `target` and its SHA256SUMS.
pub(crate) fn fake_release(dir: &std::path::Path, target: &str, checksum_ok: bool) {
    let stage = dir.join("stage");
    std::fs::create_dir_all(&stage).unwrap();
    script(&stage, "grove", "echo 'grove 0.1.0'");
    let asset = format!("grove-{target}.tar.gz");
    assert!(
        std::process::Command::new("tar")
            .arg("-czf")
            .arg(dir.join(&asset))
            .arg("-C")
            .arg(&stage)
            .arg("grove")
            .status()
            .unwrap()
            .success()
    );
    let sum = std::process::Command::new("shasum")
        .args(["-a", "256", &asset])
        .current_dir(dir)
        .output()
        .unwrap();
    let mut line = String::from_utf8(sum.stdout).unwrap();
    if !checksum_ok {
        line.replace_range(0..8, "00000000");
    }
    std::fs::write(dir.join("SHA256SUMS"), line).unwrap();
}

#[test]
fn fetch_release_downloads_checks_and_caches() {
    let d = tempfile::tempdir().unwrap();
    let rel = d.path().join("release");
    std::fs::create_dir_all(&rel).unwrap();
    fake_release(&rel, "aarch64-unknown-linux-musl", true);
    let run = d.path().join("run");
    let base = format!("file://{}", rel.display());
    let got = fetch_release(&run, &base, "0.1.0", "aarch64-unknown-linux-musl").unwrap();
    assert!(got.starts_with(&run), "{}", got.display());
    assert!(
        std::fs::read_to_string(&got)
            .unwrap()
            .contains("grove 0.1.0")
    );
    // Cached: works with the release gone.
    std::fs::remove_dir_all(&rel).unwrap();
    assert_eq!(
        fetch_release(&run, &base, "0.1.0", "aarch64-unknown-linux-musl").unwrap(),
        got
    );
}

#[test]
fn fetch_release_refuses_a_bad_checksum() {
    let d = tempfile::tempdir().unwrap();
    fake_release(d.path(), "aarch64-unknown-linux-musl", false);
    let run = d.path().join("run");
    let base = format!("file://{}", d.path().display());
    let e = fetch_release(&run, &base, "0.1.0", "aarch64-unknown-linux-musl").unwrap_err();
    assert!(format!("{e:#}").contains("checksum"), "{e:#}");
    // Nothing cached, so the next try downloads again.
    let cached: Vec<_> = walk(&run);
    assert!(!cached.iter().any(|p| p.ends_with("grove")), "{cached:?}");
}

#[test]
fn fetch_release_missing_names_the_version_and_target() {
    let d = tempfile::tempdir().unwrap();
    let base = format!("file://{}", d.path().display());
    let e = fetch_release(
        &d.path().join("run"),
        &base,
        "9.9.9",
        "x86_64-unknown-linux-musl",
    )
    .unwrap_err();
    let e = format!("{e:#}");
    assert!(
        e.contains("9.9.9") && e.contains("x86_64-unknown-linux-musl"),
        "{e}"
    );
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.flatten()
        .flat_map(|e| {
            let p = e.path();
            if p.is_dir() { walk(&p) } else { vec![p] }
        })
        .collect()
}
