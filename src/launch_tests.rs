use super::*;
use crate::model::State;

/// A Claude launch (the glob import's 8-argument one takes the agent).
fn build_with_bin(
    cfg: &Config,
    paths: &Paths,
    project: &Project,
    session_id: &str,
    name: &str,
    prompt: Option<&str>,
    bin: &str,
) -> Result<LaunchSpec> {
    super::build_with_bin(
        cfg,
        paths,
        project,
        session_id,
        name,
        prompt,
        Agent::Claude,
        bin,
    )
}

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
        id: None,
        name: "My Proj".into(),
        path: e.proj_dir.clone(),
        color: Some("#2e8b57".into()),
        summary: None,
        agent: None,
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
        conversation: None,
        launch: None,
        agent: Default::default(),
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
    std::fs::write(
        &conf,
        crate::runfiles::tmux_conf("grove", &e.paths, None, &crate::ui::theme::GROVE),
    )
    .unwrap();
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
    std::fs::write(
        &conf,
        crate::runfiles::tmux_conf("grove", &e.paths, None, &crate::ui::theme::GROVE),
    )
    .unwrap();
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
fn an_update_reloads_a_running_grove_and_leaves_a_stopped_one_alone() {
    let e = env();
    let conf = e.paths.run.join("tmux.conf");
    std::fs::write(&conf, "set -g @grove_conf old\n").unwrap();
    let tmux = Tmux::new(format!("grove-test-{}-reload", std::process::id()));
    // Not running: nothing started.
    assert!(!reload_running(&tmux, "grove", &conf, "build-1").unwrap());
    assert!(!tmux.has_session("grove"));
    let argv = vec!["sleep".to_string(), "30".to_string()];
    ensure_dashboard(&tmux, "grove", &e.proj_dir, &argv).unwrap();
    std::fs::write(&conf, "set -g @grove_conf new\n").unwrap();
    assert!(reload_running(&tmux, "grove", &conf, "build-1").unwrap());
    let out = tmux
        .cmd()
        .args(["show", "-gv", "@grove_conf"])
        .output()
        .unwrap();
    let dash = tmux.window_option(&dashboard_target("grove"), "@grove_build");
    tmux.kill_server();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "new");
    assert_eq!(
        dash.as_deref(),
        Some("build-1"),
        "the dashboard runs the new build"
    );
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

/// An ended session `name` whose last conversation was `conversation`, with
/// that conversation's transcript saved when `saved`.
fn ended(e: &Env, store: &Store, id: &str, name: &str, conversation: &str, saved: bool) {
    let now = chrono::Utc::now();
    store
        .save_session(&Session {
            id: id.into(),
            project: "my-proj".into(),
            name: name.into(),
            host: "local".into(),
            tmux_window: Some("@99".into()),
            state: State::Ended,
            message: None,
            conversation: Some(conversation.into()),
            launch: None,
            agent: Default::default(),
            created_at: now,
            updated_at: now,
        })
        .unwrap();
    if saved {
        let dir = e.claude_dir.join("projects").join("-my-proj");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{conversation}.jsonl")), "{}\n").unwrap();
    }
}

/// A stand-in `claude` that writes its arguments to the returned file.
fn recording_stub(e: &Env) -> (PathBuf, PathBuf) {
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
    (stub, args_file)
}

fn read_args(args_file: &std::path::Path) -> String {
    for _ in 0..100 {
        if args_file.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    std::fs::read_to_string(args_file).unwrap_or_default()
}

#[test]
fn resume_reopens_the_last_conversation() {
    let e = env();
    let store = Store::new(e.paths.clone());
    let tmux = Tmux::new(format!("grove-test-{}-resume", std::process::id()));
    tmux.new_session_detached("grove", "dash", &e.proj_dir, &["sleep".into(), "30".into()])
        .unwrap();
    ended(&e, &store, "grove-id", "s1", "conv-2", true);
    let (stub, args_file) = recording_stub(&e);
    let s = resume_with_bin(
        &tmux,
        &store,
        &cfg(&e),
        &project(&e),
        "grove-id",
        stub.to_str().unwrap(),
    );
    let args = read_args(&args_file);
    let windows = tmux.list_windows("grove").unwrap_or_default();
    tmux.kill_server();
    let s = s.unwrap();
    let args: Vec<&str> = args.lines().collect();
    let at = args
        .iter()
        .position(|a| *a == "--resume")
        .expect("--resume");
    assert_eq!(args[at + 1], "conv-2", "{args:?}");
    assert!(!args.contains(&"--session-id"), "{args:?}");
    assert_eq!((s.id.as_str(), s.name.as_str()), ("grove-id", "s1"));
    assert_eq!(s.state, State::Starting);
    assert_ne!(s.tmux_window.as_deref(), Some("@99"));
    assert!(windows.contains(s.tmux_window.as_ref().unwrap()));
    assert_eq!(store.get_session("grove-id").unwrap(), s);
}

#[test]
fn resume_takes_a_free_name_when_its_own_is_in_use() {
    let e = env();
    let store = Store::new(e.paths.clone());
    let tmux = Tmux::new(format!("grove-test-{}-resumename", std::process::id()));
    tmux.new_session_detached("grove", "dash", &e.proj_dir, &["sleep".into(), "30".into()])
        .unwrap();
    ended(&e, &store, "old", "s1", "conv-old", true);
    let (stub, _) = recording_stub(&e);
    let live = start_with_bin(
        &tmux,
        &store,
        &cfg(&e),
        &project(&e),
        None,
        None,
        stub.to_str().unwrap(),
    );
    let s = resume_with_bin(
        &tmux,
        &store,
        &cfg(&e),
        &project(&e),
        "old",
        stub.to_str().unwrap(),
    );
    tmux.kill_server();
    assert_eq!(live.unwrap().name, "s1");
    assert_eq!(s.unwrap().name, "s2");
}

#[test]
fn resume_needs_a_saved_conversation() {
    let e = env();
    let store = Store::new(e.paths.clone());
    let tmux = Tmux::new(format!("grove-test-{}-resumenone", std::process::id()));
    ended(&e, &store, "grove-id", "s1", "conv-2", false);
    let err = resume(&tmux, &store, &cfg(&e), &project(&e), "grove-id").unwrap_err();
    assert!(err.to_string().contains("nothing to resume"), "{err}");
    assert_eq!(store.get_session("grove-id").unwrap().state, State::Ended);
}

#[test]
fn resume_leaves_live_sessions_alone() {
    let e = env();
    let store = Store::new(e.paths.clone());
    let tmux = Tmux::new(format!("grove-test-{}-resumelive", std::process::id()));
    ended(&e, &store, "grove-id", "s1", "conv-2", true);
    let mut s = store.get_session("grove-id").unwrap();
    s.state = State::Working;
    store.save_session(&s).unwrap();
    assert!(resume(&tmux, &store, &cfg(&e), &project(&e), "grove-id").is_err());
    assert_eq!(store.get_session("grove-id").unwrap(), s);
}

#[test]
fn restart_reopens_a_live_session_in_a_new_window_as_a_new_run() {
    let e = env();
    let store = Store::new(e.paths.clone());
    let tmux = Tmux::new(format!("grove-test-{}-restart", std::process::id()));
    tmux.new_session_detached("grove", "dash", &e.proj_dir, &["sleep".into(), "30".into()])
        .unwrap();
    ended(&e, &store, "grove-id", "s1", "conv-2", true);
    let (stub, args_file) = recording_stub(&e);
    let bin = stub.to_str().unwrap();
    let first = resume_with_bin(&tmux, &store, &cfg(&e), &project(&e), "grove-id", bin);
    let mut live = store.get_session("grove-id").unwrap();
    live.state = State::YourTurn;
    store.save_session(&live).unwrap();
    let _ = read_args(&args_file);
    std::fs::remove_file(&args_file).unwrap();
    let again = restart_with_bin(&tmux, &store, &cfg(&e), &project(&e), "grove-id", bin);
    let args = read_args(&args_file);
    let windows = tmux.list_windows("grove").unwrap_or_default();
    tmux.kill_server();
    let (first, again) = (first.unwrap(), again.unwrap());
    assert_eq!((again.id.as_str(), again.name.as_str()), ("grove-id", "s1"));
    assert_eq!(again.state, State::Starting);
    assert_ne!(again.tmux_window, first.tmux_window);
    assert!(!windows.contains(first.tmux_window.as_ref().unwrap()));
    assert!(windows.contains(again.tmux_window.as_ref().unwrap()));
    assert!(first.launch.is_some() && again.launch.is_some());
    assert_ne!(again.launch, first.launch, "each run has its own token");
    assert!(args.lines().any(|a| a == "conv-2"), "{args}");
}

#[test]
fn restart_without_a_saved_conversation_leaves_the_session_running() {
    let e = env();
    let store = Store::new(e.paths.clone());
    let tmux = Tmux::new(format!("grove-test-{}-restartnone", std::process::id()));
    ended(&e, &store, "grove-id", "s1", "conv-2", false);
    let mut s = store.get_session("grove-id").unwrap();
    s.state = State::YourTurn;
    store.save_session(&s).unwrap();
    let err = restart(&tmux, &store, &cfg(&e), &project(&e), "grove-id").unwrap_err();
    assert!(err.to_string().contains("can't be restarted"), "{err}");
    assert_eq!(store.get_session("grove-id").unwrap(), s);
}

#[test]
fn orphans_are_live_sessions_from_before_the_server_started() {
    let t = chrono::Utc::now();
    let mk = |id: &str, state, ago: i64| Session {
        id: id.into(),
        project: "p".into(),
        name: id.into(),
        host: "local".into(),
        tmux_window: Some("@1".into()),
        state,
        message: None,
        conversation: None,
        launch: None,
        agent: Default::default(),
        created_at: t,
        updated_at: t - chrono::Duration::seconds(ago),
    };
    let sessions = vec![
        mk("old-live", State::YourTurn, 60),
        mk("old-ended", State::Ended, 60),
        mk("new-live", State::Working, -5),
    ];
    let ids: Vec<String> = orphans(&sessions, t).into_iter().map(|s| s.id).collect();
    assert_eq!(ids, vec!["old-live"]);
}

#[test]
fn revive_resumes_sessions_the_last_server_took_down() {
    let e = env();
    let store = Store::new(e.paths.clone());
    store.save_project(&project(&e)).unwrap();
    let tmux = Tmux::new(format!("grove-test-{}-revive", std::process::id()));
    tmux.new_session_detached("grove", "dash", &e.proj_dir, &["sleep".into(), "30".into()])
        .unwrap();
    let started = tmux.server_started().expect("server start time");
    // Live when the old server went down: one with a conversation to
    // resume, one without.
    for (id, name, saved) in [("back", "s1", true), ("lost", "s2", false)] {
        ended(&e, &store, id, name, &format!("conv-{id}"), saved);
        let mut s = store.get_session(id).unwrap();
        s.state = State::YourTurn;
        s.updated_at = started - chrono::Duration::minutes(5);
        store.save_session(&s).unwrap();
    }
    let (stub, args_file) = recording_stub(&e);
    let r = revive_with_bin(&tmux, &store, &cfg(&e), started, stub.to_str().unwrap());
    let args = read_args(&args_file);
    let windows = tmux.list_windows("grove").unwrap_or_default();
    // Run again: nothing left to bring back.
    let again = revive_with_bin(&tmux, &store, &cfg(&e), started, stub.to_str().unwrap());
    tmux.kill_server();
    assert_eq!(
        r,
        Revived {
            back: 1,
            failed: vec!["my-proj:s2".into()]
        }
    );
    let back = store.get_session("back").unwrap();
    assert_eq!(back.state, State::Starting);
    assert!(windows.contains(back.tmux_window.as_ref().unwrap()));
    assert!(args.lines().any(|a| a == "conv-back"), "{args}");
    assert_eq!(store.get_session("lost").unwrap().state, State::Ended);
    assert_eq!(again, Revived::default());
}

#[test]
fn an_opencode_launch_runs_standalone_with_groves_plugin_mcp_and_context() {
    let e = env();
    let spec = super::build_with_bin(
        &cfg(&e),
        &e.paths,
        &project(&e),
        "1111-2222",
        "s1",
        // One word is fine: it's `--prompt`'s value, not a subcommand.
        Some("hello"),
        Agent::OpenCode,
        "/b/opencode",
    )
    .unwrap();
    assert_eq!(
        spec.argv,
        [
            "/b/opencode",
            "--standalone",
            "-s",
            "ses_11112222",
            "--prompt",
            "hello",
            e.proj_dir.to_str().unwrap(),
        ]
    );
    assert_eq!(env_val(&spec, "CLAUDE_CONFIG_DIR"), None);
    assert_eq!(env_val(&spec, "GROVE_SESSION"), Some("1111-2222"));
    let config: serde_json::Value =
        serde_json::from_str(env_val(&spec, "OPENCODE_CONFIG_CONTENT").unwrap()).unwrap();
    assert_eq!(
        config["plugins"][0],
        e.paths.run.join("opencode-plugin").to_str().unwrap()
    );
    assert_eq!(config["mcp"]["servers"]["grove"]["type"], "local");
    assert_eq!(config["mcp"]["servers"]["grove"]["command"][1], "mcp");
    assert_eq!(
        config["instructions"][0],
        opencode_context(&e.paths, "my-proj").to_str().unwrap()
    );
}

#[test]
fn an_opencode_first_prompt_still_cant_be_a_command() {
    let e = env();
    for p in ["/init", "!rm -rf x", "--auto", "-c"] {
        let err = super::build_with_bin(
            &cfg(&e),
            &e.paths,
            &project(&e),
            "u",
            "s1",
            Some(p),
            Agent::OpenCode,
            "opencode",
        )
        .unwrap_err();
        assert!(err.to_string().contains("can't start with"), "{p}: {err}");
    }
}

#[test]
fn an_opencode_project_starts_opencode_and_resumes_its_session_without_a_transcript() {
    let e = env();
    let store = Store::new(e.paths.clone());
    let tmux = Tmux::new(format!("grove-test-{}-opencode", std::process::id()));
    tmux.new_session_detached("grove", "dash", &e.proj_dir, &["sleep".into(), "30".into()])
        .unwrap();
    let mut p = project(&e);
    p.agent = Some(Agent::OpenCode);
    let (stub, args_file) = recording_stub(&e);
    let bin = stub.to_str().unwrap();
    let s = start_with_bin(&tmux, &store, &cfg(&e), &p, None, None, bin).unwrap();
    let first = read_args(&args_file);
    let context = std::fs::read_to_string(opencode_context(&e.paths, "my-proj"));
    std::fs::remove_file(&args_file).unwrap();
    let again = restart_with_bin(&tmux, &store, &cfg(&e), &p, &s.id, bin);
    let second = read_args(&args_file);
    tmux.kill_server();
    let id = crate::model::opencode_session(&s.id);
    assert_eq!(s.agent, Agent::OpenCode);
    assert_eq!(s.conversation.as_deref(), Some(id.as_str()));
    assert!(first.lines().any(|a| a == id), "{first}");
    assert!(context.unwrap().starts_with("Grove project: my-proj"));
    let again = again.unwrap();
    assert_eq!(
        (again.id.as_str(), again.agent),
        (s.id.as_str(), Agent::OpenCode)
    );
    assert!(second.lines().any(|a| a == id), "{second}");
}

#[test]
fn the_config_names_the_agent_unless_the_project_does() {
    let mut c = Config::default();
    let e = env();
    let mut p = project(&e);
    assert_eq!(c.agent_for(&p), Agent::Claude);
    c.agent = Agent::OpenCode;
    assert_eq!(c.agent_for(&p), Agent::OpenCode);
    p.agent = Some(Agent::Claude);
    assert_eq!(c.agent_for(&p), Agent::Claude);
    let parsed: Project =
        toml::from_str("name = \"x\"\npath = \"/x\"\nagent = \"opencode\"\n").unwrap();
    assert_eq!(parsed.agent, Some(Agent::OpenCode));
}
