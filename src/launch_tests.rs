use super::*;
use crate::model::State;

struct Env {
    _dir: tempfile::TempDir,
    claude_dir: PathBuf,
    proj_dir: PathBuf,
    paths: Paths,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let claude_dir = dir.path().join("claude play");
    let proj_dir = dir.path().join("my proj");
    std::fs::create_dir_all(&claude_dir).unwrap();
    std::fs::create_dir_all(&proj_dir).unwrap();
    let paths = Paths::new(dir.path().join("grove home"));
    paths.ensure().unwrap();
    Env {
        _dir: dir,
        claude_dir,
        proj_dir,
        paths,
    }
}

fn cfg(e: &Env) -> Config {
    Config {
        claude_config_dir: e.claude_dir.to_string_lossy().into(),
        claude_args: vec!["--model".into(), "opus".into()],
        ..Config::default()
    }
}

fn project(e: &Env) -> Project {
    Project {
        name: "My Proj".into(),
        path: e.proj_dir.clone(),
        color: Some("#2e8b57".into()),
        summary: None,
    }
}

fn env_val<'a>(spec: &'a LaunchSpec, k: &str) -> Option<&'a str> {
    spec.env
        .iter()
        .find(|(key, _)| key == k)
        .map(|(_, v)| v.as_str())
}

#[test]
fn build_sets_account_project_and_session_env() {
    let e = env();
    let spec = build_with_bin(
        &cfg(&e),
        &e.paths,
        &project(&e),
        "uuid-1",
        "s1",
        None,
        "/usr/local/bin/claude",
    )
    .unwrap();
    assert_eq!(
        env_val(&spec, "CLAUDE_CONFIG_DIR"),
        Some(e.claude_dir.to_str().unwrap())
    );
    assert_eq!(env_val(&spec, "GROVE_PROJECT"), Some("my-proj"));
    assert_eq!(env_val(&spec, "GROVE_ACCOUNT"), Some("claude play"));
    assert_eq!(env_val(&spec, "GROVE_SESSION"), Some("uuid-1"));
    assert_eq!(
        env_val(&spec, "GROVE_HOME"),
        Some(e.paths.home.to_str().unwrap())
    );
    assert_eq!(spec.cwd, e.proj_dir);
    assert_eq!(spec.window_name, "my-proj:s1");
}

#[test]
fn build_passes_session_hooks_and_mcp_flags() {
    let e = env();
    let spec = build_with_bin(
        &cfg(&e),
        &e.paths,
        &project(&e),
        "uuid-1",
        "s1",
        None,
        "/usr/local/bin/claude",
    )
    .unwrap();
    let a = &spec.argv;
    assert!(a[0].ends_with("claude"), "{a:?}");
    let pair = |flag: &str| a.iter().position(|x| x == flag).map(|i| a[i + 1].clone());
    assert_eq!(pair("--session-id").as_deref(), Some("uuid-1"));
    assert_eq!(
        pair("--settings"),
        Some(
            e.paths
                .run
                .join("hooks.json")
                .to_string_lossy()
                .into_owned()
        )
    );
    assert_eq!(
        pair("--mcp-config"),
        Some(e.paths.run.join("mcp.json").to_string_lossy().into_owned())
    );
    assert_eq!(pair("-n").as_deref(), Some("my-proj:s1"));
    assert_eq!(
        &a[a.len() - 2..],
        &["--model".to_string(), "opus".to_string()]
    );
}

#[test]
fn build_refuses_missing_claude_dir() {
    let e = env();
    let c = Config {
        claude_config_dir: "/no/such/claude".into(),
        ..Config::default()
    };
    let err = build_with_bin(&c, &e.paths, &project(&e), "u", "s1", None, "claude")
        .unwrap_err()
        .to_string();
    assert!(err.contains("/no/such/claude"), "{err}");
}

#[test]
fn build_refuses_missing_project_dir() {
    let e = env();
    let mut p = project(&e);
    p.path = "/no/such/project".into();
    let err = build_with_bin(&cfg(&e), &e.paths, &p, "u", "s1", None, "claude")
        .unwrap_err()
        .to_string();
    assert!(err.contains("/no/such/project"), "{err}");
}

#[test]
fn next_session_name_skips_live_names() {
    let e = env();
    let store = Store::new(e.paths.clone());
    assert_eq!(next_session_name(&store, "p"), "s1");
    let mk = |id: &str, name: &str, state| Session {
        id: id.into(),
        project: "p".into(),
        name: name.into(),
        host: "local".into(),
        tmux_window: None,
        state,
        message: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    store.save_session(&mk("a", "s1", State::Working)).unwrap();
    store.save_session(&mk("b", "s2", State::Ended)).unwrap();
    store.save_session(&mk("c", "s3", State::YourTurn)).unwrap();
    assert_eq!(next_session_name(&store, "p"), "s2");
    assert_eq!(next_session_name(&store, "other"), "s1");
}

/// `cfg` plus a `work` account (dir exists) for projects under `<tmp>/Work`.
fn cfg_with_work(e: &Env) -> (Config, PathBuf) {
    let tmp = e.proj_dir.parent().unwrap();
    let work = tmp.join("claude work");
    std::fs::create_dir_all(&work).unwrap();
    let mut c = cfg(e);
    c.accounts.insert(
        "work".into(),
        crate::config::AccountConfig {
            dir: work.to_string_lossy().into(),
            color: Some("#e8a33d".into()),
            folder: Some(tmp.join("Work").to_string_lossy().into()),
        },
    );
    (c, work)
}

/// A project in `<tmp>/Work/acme`.
fn work_project(e: &Env) -> Project {
    let dir = e.proj_dir.parent().unwrap().join("Work").join("acme");
    std::fs::create_dir_all(&dir).unwrap();
    Project {
        name: "acme".into(),
        path: dir,
        ..project(e)
    }
}

#[test]
fn build_picks_the_account_from_the_project_folder() {
    let e = env();
    let (c, work) = cfg_with_work(&e);
    let spec = build_with_bin(
        &c,
        &e.paths,
        &work_project(&e),
        "u",
        "s1",
        None,
        "/b/claude",
    )
    .unwrap();
    assert_eq!(
        env_val(&spec, "CLAUDE_CONFIG_DIR"),
        Some(work.to_str().unwrap())
    );
    assert_eq!(spec.account.name, "work");
    // Outside the work folder: the default.
    let spec = build_with_bin(&c, &e.paths, &project(&e), "u", "s1", None, "/b/claude").unwrap();
    assert_eq!(
        env_val(&spec, "CLAUDE_CONFIG_DIR"),
        Some(e.claude_dir.to_str().unwrap())
    );
}

#[test]
fn build_refuses_a_missing_account_dir() {
    let e = env();
    let (mut c, _) = cfg_with_work(&e);
    c.accounts.get_mut("work").unwrap().dir = "/definitely/not/here".into();
    let err = build_with_bin(
        &c,
        &e.paths,
        &work_project(&e),
        "u",
        "s1",
        None,
        "/b/claude",
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("/definitely/not/here"), "{err}");
}

#[test]
fn start_opens_window_and_records_session() {
    let e = env();
    let store = Store::new(e.paths.clone());
    let tmux = Tmux::new(format!("grove-test-{}-launch", std::process::id()));
    tmux.new_session_detached("grove", "dash", &e.proj_dir, &["sleep".into(), "30".into()])
        .unwrap();
    // Stand-in for `claude` (ignores claude's flags) so no real session starts.
    let stub = e.proj_dir.join("fake claude");
    let args_file = e.proj_dir.join("args");
    std::fs::write(
        &stub,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nsleep 30\n",
            args_file.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let s = start_with_bin(
        &tmux,
        &store,
        &cfg(&e),
        &project(&e),
        None,
        Some("fix the build".into()),
        stub.to_str().unwrap(),
    )
    .unwrap();
    for _ in 0..100 {
        if args_file.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let args = std::fs::read_to_string(&args_file).unwrap_or_default();
    let windows = tmux.list_windows("grove").unwrap();
    let w = s.tmux_window.clone().unwrap();
    let account = tmux.window_option(&w, "@grove_account");
    let account_color = tmux.window_option(&w, "@grove_account_color");
    tmux.kill_server();
    // The default account, named after its dir ("claude play" → itself).
    assert_eq!(account.as_deref(), Some("claude play"));
    assert_eq!(account_color.as_deref(), Some("default"));
    assert_eq!(s.state, State::Starting);
    assert_eq!(s.name, "s1");
    assert_eq!(s.host, "local");
    assert!(windows.contains(s.tmux_window.as_ref().unwrap()));
    assert_eq!(store.get_session(&s.id).unwrap(), s);
    assert_eq!(args.lines().last(), Some("fix the build"), "{args}");
}

#[test]
fn start_with_missing_project_dir_creates_nothing() {
    let e = env();
    let store = Store::new(e.paths.clone());
    let tmux = Tmux::new(format!("grove-test-{}-nodir", std::process::id()));
    let mut p = project(&e);
    p.path = "/no/such/project".into();
    assert!(start(&tmux, &store, &cfg(&e), &p, None, None).is_err());
    assert!(store.list_sessions(None).is_empty());
}

#[test]
fn ensure_dashboard_creates_session_then_restores_a_missing_dashboard() {
    let e = env();
    let conf = e.paths.run.join("tmux.conf");
    std::fs::write(&conf, crate::runfiles::tmux_conf("grove", &e.paths, None)).unwrap();
    let tmux = Tmux {
        config: Some(conf),
        ..Tmux::new(format!("grove-test-{}-dash", std::process::id()))
    };
    let argv = vec!["sleep".to_string(), "30".to_string()];
    let index0 = |t: &Tmux| {
        let out = t
            .cmd()
            .args(["list-windows", "-t", "=grove", "-F", "#{window_name}"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .any(|l| l == "dashboard")
    };
    assert!(
        ensure_dashboard(&tmux, "grove", &e.proj_dir, &argv).unwrap(),
        "created"
    );
    assert!(index0(&tmux));
    assert!(
        !ensure_dashboard(&tmux, "grove", &e.proj_dir, &argv).unwrap(),
        "already there"
    );

    // A session window keeps the tmux session alive while window 0 dies.
    tmux.new_window("grove", "p:s1", &e.proj_dir, &[], &argv)
        .unwrap();
    tmux.kill_window("=grove:=dashboard").unwrap();
    assert!(!index0(&tmux));
    assert!(
        ensure_dashboard(&tmux, "grove", &e.proj_dir, &argv).unwrap(),
        "restored"
    );
    assert!(index0(&tmux));
    tmux.kill_server();
}

#[test]
fn stale_dashboard_is_restarted_and_fresh_one_left_alone() {
    let e = env();
    let conf = e.paths.run.join("tmux.conf");
    std::fs::write(&conf, crate::runfiles::tmux_conf("grove", &e.paths, None)).unwrap();
    let tmux = Tmux {
        config: Some(conf),
        ..Tmux::new(format!("grove-test-{}-fresh", std::process::id()))
    };
    let argv = vec!["sleep".to_string(), "30".to_string()];
    let pane_pid = |t: &Tmux| {
        let out = t
            .cmd()
            .args([
                "display-message",
                "-p",
                "-t",
                "=grove:=dashboard",
                "#{pane_pid}",
            ])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    ensure_dashboard(&tmux, "grove", &e.proj_dir, &argv).unwrap();
    let first = pane_pid(&tmux);

    // An unstamped dashboard (started by an older grove) counts as stale.
    assert!(ensure_fresh_dashboard(&tmux, "grove", "build-1").unwrap());
    let second = pane_pid(&tmux);
    assert_ne!(first, second, "window 0 should have been respawned");

    // Same build: nothing happens.
    assert!(!ensure_fresh_dashboard(&tmux, "grove", "build-1").unwrap());
    assert_eq!(pane_pid(&tmux), second);

    // Newer build installed: restarted again.
    assert!(ensure_fresh_dashboard(&tmux, "grove", "build-2").unwrap());
    assert_ne!(pane_pid(&tmux), second);
    tmux.kill_server();
}

#[test]
fn build_stamp_changes_when_the_binary_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("grove");
    std::fs::write(&bin, "v1").unwrap();
    let a = build_stamp(&bin);
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&bin, "v2 longer").unwrap();
    assert_ne!(a, build_stamp(&bin));
    assert_eq!(build_stamp(&dir.path().join("missing")), "unknown");
}

#[test]
fn a_prompt_that_could_be_read_as_an_option_or_command_is_refused() {
    let e = env();
    let long = "word ".repeat(2000);
    for bad in [
        "--dangerously-skip-permissions",
        "-p",
        "  --x y",
        "update",
        "!ls -la ~",
        "/review this please",
        long.as_str(),
    ] {
        let err = build_with_bin(
            &cfg(&e),
            &e.paths,
            &project(&e),
            "u",
            "s1",
            Some(bad),
            "/b/claude",
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("prompt"), "{bad}: {err:#}");
    }
    let ok = build_with_bin(
        &cfg(&e),
        &e.paths,
        &project(&e),
        "u",
        "s1",
        Some("fix the - flag"),
        "/b/claude",
    )
    .unwrap();
    assert_eq!(ok.argv.last().map(String::as_str), Some("fix the - flag"));
}

#[test]
fn start_refuses_a_name_a_live_session_has() {
    let e = env();
    let store = Store::new(e.paths.clone());
    let tmux = Tmux::new(format!("grove-test-{}-dupname", std::process::id()));
    tmux.new_session_detached("grove", "dash", &e.proj_dir, &["sleep".into(), "30".into()])
        .unwrap();
    let stub = e.proj_dir.join("fake claude");
    std::fs::write(&stub, "#!/bin/sh\nsleep 30\n").unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let start = |name: &str| {
        start_with_bin(
            &tmux,
            &store,
            &cfg(&e),
            &project(&e),
            Some(name.into()),
            None,
            stub.to_str().unwrap(),
        )
    };
    let first = start("fix");
    let second = start("fix");
    let window = first.as_ref().ok().and_then(|s| s.tmux_window.clone());
    let marked = window.and_then(|w| tmux.window_option(&w, "@grove_session"));
    tmux.kill_server();
    let first = first.unwrap();
    assert!(format!("{:#}", second.unwrap_err()).contains("fix"));
    assert_eq!(marked.as_deref(), Some(first.id.as_str()));
}
