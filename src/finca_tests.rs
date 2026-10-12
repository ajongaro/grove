use super::testing::*;
use super::*;

/// Probe/poll tests change process env (GROVE_SSH, FAKE_HOME); run them one
/// at a time.
pub static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

fn script(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
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
    let bin = fake_grove(&dir, &status_json("therver"));
    let ssh = fake_ssh(d.path(), None);
    let r = with_env(&ssh, d.path(), || poll_one(&finca_with_bin(&bin), ANSWERS));
    match r {
        Reach::Reachable(st) => assert_eq!(st.host, "therver"),
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
    let bin = fake_grove(d.path(), &status_json("therver"));
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
    let bin = fake_grove(d.path(), &status_json("therver"));
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
    assert_eq!(
        got,
        vec![
            "-t".to_string(),
            // A dead link (laptop slept, network changed) ends ssh in ~45s
            // instead of leaving a frozen finca session.
            "-o".into(),
            "ServerAliveInterval=15".into(),
            "-o".into(),
            "ServerAliveCountMax=3".into(),
            "me@it's here".into(),
            remote_login_cmd("/opt/my grove/gro've")
        ]
    );
}

#[test]
fn connect_failure_pauses_with_the_exit_code() {
    let d = tempfile::tempdir().unwrap();
    let ssh = script(d.path(), "bad ssh", "echo 'no route' >&2; exit 255");
    let f = Finca {
        name: "pi".into(),
        ssh: "x".into(),
        grove_bin: "/g".into(),
        color: None,
        backup: None,
    };
    let argv = with_env(&ssh, d.path(), || connect_argv(&f, &f.ssh));
    let out = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut c| {
            use std::io::Write;
            c.stdin.take().unwrap().write_all(b"\n")?;
            c.wait_with_output()
        })
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("exited (255)") && text.contains("Enter"),
        "{text}"
    );
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
        name: "therver".into(),
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
                .args(["show-options", "-v", "-t", "=finca-therver:", o])
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
    assert_eq!(
        keys.lines().count(),
        1,
        "only C-q in the finca table: {keys}"
    );

    disconnect(&tmux, "therver");
    assert!(!tmux.has_session("finca-therver"));
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
        name: "therver".into(),
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
    let opt = |o: &str| out(&["show-options", "-v", "-t", "=finca-therver-sh:", o]);
    assert_eq!(opt("key-table").trim(), "grove-shell");
    assert_ne!(opt("prefix").trim(), "None", "the prefix works in a shell");
    // New windows and splits ssh to the finca too, not a local shell.
    assert!(
        opt("default-command").contains("idle ssh"),
        "{}",
        opt("default-command")
    );
    let keys = out(&["list-keys", "-T", "grove-shell"]);
    assert!(
        keys.contains("C-q") && keys.contains("switch-client -t =grove"),
        "{keys}"
    );

    with_env(&ssh, d.path(), || ensure_finca_session(&tmux, &f, &f.ssh)).unwrap();
    disconnect(&tmux, "therver");
    assert!(!tmux.has_session("finca-therver"));
    assert!(
        !tmux.has_session("finca-therver-sh"),
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
