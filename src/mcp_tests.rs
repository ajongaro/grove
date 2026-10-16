use super::*;
use crate::config::{AccountConfig, Config, Paths};
use crate::model::{Project, Session, State};

/// Projects live in <tmp>/code/<name>; <tmp>/work is the work account's folder.
struct T {
    dir: tempfile::TempDir,
    m: GroveMcp,
}

impl std::ops::Deref for T {
    type Target = GroveMcp;
    fn deref(&self) -> &GroveMcp {
        &self.m
    }
}

fn two_accounts(dir: &std::path::Path) -> Config {
    let mut accounts = std::collections::BTreeMap::new();
    accounts.insert(
        "work".to_string(),
        AccountConfig {
            dir: dir.join("claude-work").to_string_lossy().into(),
            color: None,
            folder: Some(dir.join("work").to_string_lossy().into()),
        },
    );
    Config {
        claude_config_dir: dir.join("claude-play").to_string_lossy().into(),
        accounts,
        tmux_session: format!("grove-test-{}-mcp", std::process::id()),
        ..Config::default()
    }
}

/// A server scoped to `account` ("play" or "work").
fn scoped(account: &str, default: Option<&str>) -> T {
    // Each test gets its own tmux server: tests run in parallel.
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let socket = format!("grove-test-{}-mcp{n}", std::process::id());
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        tmux_session: socket.clone(),
        ..two_accounts(dir.path())
    };
    let store = Store::new(Paths::new(dir.path().join("home")));
    let m = GroveMcp::fixed(
        store,
        cfg,
        crate::tmux::Tmux::new(socket),
        Ok(account.to_string()),
        default.map(str::to_string),
    );
    T { dir, m }
}

/// The play account, as the pre-scope tests assume.
fn mcp(default: Option<&str>) -> T {
    scoped("play", default)
}

/// Adds a project at <tmp>/code/<name> (play) or <tmp>/work/<name> (work).
fn add_in(t: &T, name: &str, work: bool) {
    let base = if work { "work" } else { "code" };
    let path = t.dir.path().join(base).join(name);
    std::fs::create_dir_all(&path).unwrap();
    t.store
        .add_project(&Project {
            name: name.into(),
            path,
            color: None,
            summary: None,
        })
        .unwrap();
}

fn add(t: &T, name: &str) {
    add_in(t, name, false);
}

#[test]
fn scope_flag_beats_grove_account() {
    let d = tempfile::tempdir().unwrap();
    let cfg = two_accounts(d.path());
    assert_eq!(
        scope_for(&cfg, Some("work"), Some("play"), None),
        Ok("work".into())
    );
    assert_eq!(scope_for(&cfg, None, Some("work"), None), Ok("work".into()));
}

#[test]
fn scope_without_an_account_says_how_to_register() {
    let d = tempfile::tempdir().unwrap();
    let e = scope_for(&two_accounts(d.path()), None, None, None).unwrap_err();
    assert!(e.contains("grove mcp setup"), "{e}");
}

#[test]
fn scope_unknown_account_names_the_real_ones() {
    let d = tempfile::tempdir().unwrap();
    let e = scope_for(&two_accounts(d.path()), Some("nope"), None, None).unwrap_err();
    assert!(e.contains("nope") && e.contains("work"), "{e}");
}

#[test]
fn scope_refuses_another_accounts_claude_config() {
    let d = tempfile::tempdir().unwrap();
    let cfg = two_accounts(d.path());
    let play_dir = d.path().join("claude-play");
    std::fs::create_dir_all(&play_dir).unwrap();
    let e = scope_for(&cfg, Some("work"), None, play_dir.to_str()).unwrap_err();
    assert!(e.contains("work") && e.contains("claude-play"), "{e}");
    // Its own config dir, or an unrelated one, is fine.
    let work_dir = d.path().join("claude-work");
    assert_eq!(
        scope_for(&cfg, Some("work"), None, work_dir.to_str()),
        Ok("work".into())
    );
    assert_eq!(
        scope_for(&cfg, Some("work"), None, Some("/elsewhere")),
        Ok("work".into())
    );
}

#[test]
fn scope_error_is_returned_by_every_tool() {
    let t = mcp(None);
    add(&t, "alpha");
    t.set_scope(Err("no account".into()));
    assert_eq!(t.do_list_projects(), Err("no account".into()));
    assert_eq!(
        t.do_get_project(Some("alpha".into())),
        Err("no account".into())
    );
}

#[test]
fn work_scope_sees_only_work_projects() {
    let t = scoped("work", None);
    add_in(&t, "alpha", true);
    add_in(&t, "beta", false);
    let list = t.do_list_projects().unwrap();
    assert!(list.contains("alpha") && !list.contains("beta"), "{list}");
    assert!(t.do_get_project(Some("alpha".into())).is_ok());
}

#[test]
fn out_of_scope_project_looks_like_a_missing_one() {
    let t = scoped("work", None);
    add_in(&t, "alpha", true);
    add_in(&t, "beta", false);
    let hidden = t.do_get_project(Some("beta".into())).unwrap_err();
    let missing = t.do_get_project(Some("gamma".into())).unwrap_err();
    assert_eq!(hidden.replace("beta", "X"), missing.replace("gamma", "X"));
    assert!(!missing.contains("beta"), "{missing}");
    let upd = t
        .do_update_project(Some("beta".into()), Some("pwned".into()), None)
        .unwrap_err();
    assert!(!upd.contains("pwned"));
    assert_eq!(t.store.get_project("beta").unwrap().summary, None);
}

#[test]
fn symlinked_path_into_the_work_folder_counts_as_work() {
    let t = scoped("work", None);
    let real = t.dir.path().join("work").join("real");
    std::fs::create_dir_all(&real).unwrap();
    let link = t.dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    t.store
        .add_project(&Project {
            name: "linked".into(),
            path: link,
            color: None,
            summary: None,
        })
        .unwrap();
    assert!(t.do_list_projects().unwrap().contains("linked"));
}

#[test]
fn list_projects_empty_home() {
    let m = mcp(None);
    let out = m.do_list_projects().unwrap();
    assert!(out.contains("No projects"), "{out}");
}

#[test]
fn list_projects_shows_name_path_summary() {
    let m = mcp(None);
    add(&m, "alpha");
    m.do_update_project(Some("alpha".into()), Some("Alpha app.".into()), None)
        .unwrap();
    let out = m.do_list_projects().unwrap();
    assert!(
        out.contains("alpha") && out.contains("code/alpha") && out.contains("Alpha app."),
        "{out}"
    );
}

#[test]
fn get_project_defaults_to_session_project() {
    let m = mcp(Some("beta"));
    add(&m, "alpha");
    add(&m, "beta");
    let out = m.do_get_project(None).unwrap();
    assert!(out.contains("beta") && !out.contains("alpha"), "{out}");
    assert!(!out.contains("Wiki"), "{out}");
}

#[test]
fn no_project_and_no_default_is_an_error() {
    let m = mcp(None);
    add(&m, "alpha");
    let err = m.do_get_project(None).unwrap_err();
    assert!(err.contains("project") && err.contains("alpha"), "{err}");
}

#[test]
fn unknown_project_lists_valid_names() {
    let m = mcp(None);
    add(&m, "alpha");
    let err = m.do_get_project(Some("nope".into())).unwrap_err();
    assert!(err.contains("alpha"), "{err}");
}

#[test]
fn update_project_persists_summary_and_color() {
    let m = mcp(Some("alpha"));
    add(&m, "alpha");
    m.do_update_project(None, Some("New summary".into()), Some("#AABBCC".into()))
        .unwrap();
    let p = m.store.get_project("alpha").unwrap();
    assert_eq!(p.summary.as_deref(), Some("New summary"));
    assert_eq!(p.color.as_deref(), Some("#AABBCC"));
}

#[test]
fn update_project_rejects_bad_color_and_changes_nothing() {
    let m = mcp(Some("alpha"));
    add(&m, "alpha");
    for bad in ["red", "#12345", "#GGGGGG", "123456"] {
        let err = m
            .do_update_project(None, Some("x".into()), Some(bad.into()))
            .unwrap_err();
        assert!(err.contains("#RRGGBB"), "{bad}: {err}");
    }
    assert_eq!(m.store.get_project("alpha").unwrap().summary, None);
}

#[test]
fn update_project_with_nothing_to_change_errors() {
    let m = mcp(Some("alpha"));
    add(&m, "alpha");
    assert!(m.do_update_project(None, None, None).is_err());
}

#[test]
fn setup_prints_one_command_per_account() {
    let d = tempfile::tempdir().unwrap();
    let cfg = two_accounts(d.path());
    let out = setup_commands(
        &cfg,
        std::path::Path::new("/opt/homebrew/opt/grove/bin/grove"),
        d.path(),
    );
    let work = d.path().join("claude-work");
    assert!(
        out.contains(&format!(
            "CLAUDE_CONFIG_DIR='{}' claude mcp add --scope user grove -- '/opt/homebrew/opt/grove/bin/grove' mcp --account 'work'",
            work.display()
        )),
        "{out}"
    );
    assert!(out.contains("--account 'play'"), "{out}");
}

#[test]
fn setup_for_the_default_claude_dir_has_no_prefix() {
    let d = tempfile::tempdir().unwrap();
    let cfg = Config {
        claude_config_dir: d.path().join(".claude").to_string_lossy().into(),
        ..Config::default()
    };
    let out = setup_commands(&cfg, std::path::Path::new("/b/grove"), d.path());
    assert!(
        out.contains("\nclaude mcp add --scope user grove -- '/b/grove' mcp --account 'default'"),
        "{out}"
    );
    assert!(!out.contains("CLAUDE_CONFIG_DIR="), "{out}");
}

#[test]
fn claude_json_location() {
    let home = std::path::Path::new("/home/me");
    assert_eq!(
        claude_json(&home.join(".claude"), home),
        home.join(".claude.json")
    );
    assert_eq!(
        claude_json(&home.join(".claude-work"), home),
        home.join(".claude-work/.claude.json")
    );
}

#[test]
fn a_config_error_reaches_every_tool() {
    let d = tempfile::tempdir().unwrap();
    let paths = Paths::new(d.path().join("home"));
    paths.ensure().unwrap();
    std::fs::write(paths.config_file(), "this is = = not toml").unwrap();
    let m = GroveMcp::new(
        Store::new(paths),
        crate::tmux::Tmux::new("x"),
        Some("work".into()),
        &|_| None,
    );
    let e = m.do_list_projects().unwrap_err();
    assert!(e.contains("config"), "{e}");
}

#[test]
fn new_reads_account_project_and_session_from_env() {
    let d = tempfile::tempdir().unwrap();
    let paths = Paths::new(d.path().join("home"));
    paths.ensure().unwrap();
    let env = |k: &str| match k {
        "GROVE_ACCOUNT" => Some("default".to_string()),
        "GROVE_PROJECT" => Some("alpha".to_string()),
        "GROVE_SESSION" => Some("sid".to_string()),
        _ => None,
    };
    let m = GroveMcp::new(Store::new(paths), crate::tmux::Tmux::new("x"), None, &env);
    assert_eq!(m.scope(), Ok("default".into()));
    assert_eq!(m.default_project.as_deref(), Some("alpha"));
    assert_eq!(m.own_session.as_deref(), Some("sid"));
}

#[test]
fn get_project_shows_readme_sessions_and_todos() {
    let t = mcp(None);
    add(&t, "alpha");
    let dir = t.dir.path().join("code/alpha");
    std::fs::write(dir.join("README.md"), "# Alpha\nDoes alpha things.\n").unwrap();
    std::fs::write(dir.join("CLAUDE.md"), "Use tabs.\n").unwrap();
    t.store
        .write_todos("alpha", "- [ ] ship it\n- [x] start\n")
        .unwrap();
    let out = t.do_get_project(Some("alpha".into())).unwrap();
    for want in [
        "Does alpha things.",
        "Use tabs.",
        "git: none",
        "1. [ ] ship it",
        "2. [x] start",
        "no sessions",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
}

#[test]
fn list_projects_shows_counts_and_open_todos() {
    let t = mcp(None);
    add(&t, "alpha");
    t.store
        .write_todos("alpha", "- [ ] a\n- [ ] b\n- [x] c\n")
        .unwrap();
    let out = t.do_list_projects().unwrap();
    assert!(
        out.contains("no sessions") && out.contains("2 open to-dos"),
        "{out}"
    );
}

fn op(op: &str, n: Option<usize>, text: Option<&str>) -> TodoOpArg {
    TodoOpArg {
        op: op.into(),
        n,
        text: text.map(str::to_string),
    }
}

#[test]
fn edit_todos_saves_and_returns_the_list() {
    let t = mcp(None);
    add(&t, "alpha");
    t.store.write_todos("alpha", "- [ ] a\n").unwrap();
    let out = t
        .do_edit_todos(
            Some("alpha".into()),
            vec![op("check", Some(1), None), op("add", None, Some("b"))],
        )
        .unwrap();
    assert!(
        out.contains("1. [x] a") && out.contains("2. [ ] b"),
        "{out}"
    );
    assert_eq!(t.store.read_todos("alpha"), "- [x] a\n- [ ] b\n");
}

#[test]
fn a_bad_op_saves_nothing() {
    let t = mcp(None);
    add(&t, "alpha");
    t.store.write_todos("alpha", "- [ ] a\n").unwrap();
    for bad in [
        op("check", Some(5), None),
        op("frob", Some(1), None),
        op("reword", Some(1), None),
    ] {
        assert!(
            t.do_edit_todos(Some("alpha".into()), vec![op("add", None, Some("x")), bad])
                .is_err()
        );
    }
    assert_eq!(t.store.read_todos("alpha"), "- [ ] a\n");
}

#[test]
fn todos_of_another_account_are_hidden() {
    let t = scoped("work", None);
    add_in(&t, "beta", false);
    t.store.write_todos("beta", "- [ ] secret\n").unwrap();
    assert!(t.do_get_todos(Some("beta".into())).is_err());
    assert!(
        t.do_edit_todos(Some("beta".into()), vec![op("add", None, Some("x"))])
            .is_err()
    );
    assert_eq!(t.store.read_todos("beta"), "- [ ] secret\n");
}

fn add_session(t: &T, project: &str, name: &str, state: State, window: Option<&str>) -> Session {
    let now = chrono::Utc::now();
    let s = Session {
        id: format!("id-{project}-{name}"),
        project: project.into(),
        name: name.into(),
        host: "local".into(),
        tmux_window: window.map(str::to_string),
        state,
        message: None,
        created_at: now,
        updated_at: now,
    };
    t.store.save_session(&s).unwrap();
    // Grove marks each session's window with its id (launch.rs).
    if let Some(w) = window {
        t.tmux
            .set_window_option(w, "@grove_session", &s.id)
            .unwrap();
    }
    s
}

#[test]
fn list_sessions_is_scoped_and_skips_ended() {
    let t = scoped("work", None);
    add_in(&t, "alpha", true);
    add_in(&t, "beta", false);
    add_session(&t, "alpha", "s1", State::Working, None);
    add_session(&t, "alpha", "old", State::Ended, None);
    add_session(&t, "beta", "s1", State::YourTurn, None);
    let out = t.do_list_sessions(None).unwrap();
    assert!(out.contains("alpha:s1") && out.contains("working"), "{out}");
    assert!(!out.contains("beta") && !out.contains("old"), "{out}");
}

#[test]
fn session_names_resolve_within_the_account() {
    let t = mcp(None);
    add(&t, "alpha");
    add(&t, "gamma");
    add_session(&t, "alpha", "s1", State::Working, None);
    add_session(&t, "gamma", "s1", State::Working, None);
    add_session(&t, "alpha", "fix", State::Working, None);
    assert_eq!(t.session("fix").unwrap().0.project, "alpha");
    assert_eq!(t.session("gamma:s1").unwrap().0.project, "gamma");
    assert_eq!(t.session("id-alpha-s1").unwrap().0.project, "alpha");
    let e = t.session("s1").unwrap_err();
    assert!(e.contains("alpha:s1") && e.contains("gamma:s1"), "{e}");
}

#[test]
fn another_accounts_session_is_unknown() {
    let t = scoped("work", None);
    add_in(&t, "alpha", true);
    add_in(&t, "beta", false);
    add_session(&t, "alpha", "s1", State::Working, None);
    add_session(&t, "beta", "secret", State::Working, None);
    let hidden = t.session("secret").unwrap_err();
    let missing = t.session("nope").unwrap_err();
    assert_eq!(hidden.replace("secret", "X"), missing.replace("nope", "X"));
    assert!(t.session("beta:secret").is_err());
    assert!(t.session("id-beta-secret").is_err());
    // A bare name shared with another account's session isn't ambiguous.
    add_session(&t, "beta", "s1", State::Working, None);
    assert_eq!(t.session("s1").unwrap().0.project, "alpha");
}

#[test]
fn read_session_shows_the_screen() {
    let t = mcp(None);
    add(&t, "alpha");
    let session = t.cfg().tmux_session;
    t.tmux
        .new_session_detached(
            &session,
            "w",
            t.dir.path(),
            &[
                "sh".into(),
                "-c".into(),
                "printf 'hello\\nfrom alpha\\n'; sleep 30".into(),
            ],
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let window = t.tmux.list_windows_as(&session, "#{window_id}").unwrap()[0].clone();
    add_session(&t, "alpha", "s1", State::YourTurn, Some(&window));
    let out = t.do_read_session("s1".into(), None);
    add_session(&t, "alpha", "s2", State::YourTurn, None);
    let no_window = t.do_read_session("s2".into(), None);
    t.tmux.kill_server();
    let gone = t.do_read_session("s1".into(), None);
    let out = out.unwrap();
    assert!(out.contains("hello") && out.contains("from alpha"), "{out}");
    assert!(!out.ends_with('\n'), "{out:?}");
    assert!(no_window.unwrap_err().contains("window"));
    assert!(gone.unwrap_err().contains("gone"));
}

#[test]
fn start_session_needs_grove_running() {
    let t = mcp(None);
    add(&t, "alpha");
    let e = t
        .do_start_session(Some("alpha".into()), None, None)
        .unwrap_err();
    assert!(e.contains("Grove isn't running"), "{e}");
}

#[test]
fn start_session_of_another_account_is_refused() {
    let t = scoped("work", None);
    add_in(&t, "beta", false);
    let e = t
        .do_start_session(Some("beta".into()), None, None)
        .unwrap_err();
    assert!(e.contains("no project"), "{e}");
}

#[test]
fn start_session_opens_a_window_with_the_prompt() {
    let t = scoped("work", None);
    add_in(&t, "alpha", true);
    std::fs::create_dir_all(t.dir.path().join("claude-work")).unwrap();
    let session = t.cfg().tmux_session;
    t.tmux
        .new_session_detached(
            &session,
            "dash",
            t.dir.path(),
            &["sleep".into(), "30".into()],
        )
        .unwrap();
    let args_file = t.dir.path().join("args");
    let stub = t.dir.path().join("fake-claude");
    std::fs::write(
        &stub,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nsleep 30\n",
            args_file.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let out = t.do_start_session_with_bin(
        Some("alpha".into()),
        Some("look at the tests".into()),
        Some("tests".into()),
        stub.to_str().unwrap(),
    );
    for _ in 0..100 {
        if args_file.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    t.tmux.kill_server();
    let out = out.unwrap();
    assert!(out.contains("alpha:tests") && out.contains("work"), "{out}");
    let args = std::fs::read_to_string(&args_file).unwrap();
    assert_eq!(args.lines().last(), Some("look at the tests"), "{args}");
}

#[test]
fn send_prompt_types_into_a_waiting_session_only() {
    let mut t = mcp(None);
    add(&t, "alpha");
    let session = t.cfg().tmux_session;
    // `cat` echoes what's typed, standing in for Claude's input box.
    t.tmux
        .new_session_detached(&session, "w", t.dir.path(), &["cat".into()])
        .unwrap();
    let window = t.tmux.list_windows_as(&session, "#{window_id}").unwrap()[0].clone();
    add_session(&t, "alpha", "busy", State::Working, Some(&window));
    add_session(&t, "alpha", "ask", State::NeedsYou, Some(&window));
    let me = add_session(&t, "alpha", "me", State::YourTurn, Some(&window));
    // Last, so the window is marked as this one's (the others never reach it).
    add_session(&t, "alpha", "wait", State::YourTurn, Some(&window));
    let sent = t.do_send_prompt("wait".into(), "hello there".into());
    let busy = t.do_send_prompt("busy".into(), "x".into());
    let ask = t.do_send_prompt("ask".into(), "x".into());
    t.m.own_session = Some(me.id.clone());
    let own = t.do_send_prompt("me".into(), "x".into());
    std::thread::sleep(std::time::Duration::from_millis(300));
    let screen = t.tmux.capture(&window, 20).unwrap();
    t.tmux.kill_server();
    sent.unwrap();
    assert!(screen.contains("hello there"), "{screen}");
    assert!(busy.unwrap_err().contains("working"));
    assert!(ask.unwrap_err().contains("needs you"));
    assert!(own.unwrap_err().contains("own"));
}

#[test]
fn send_prompt_strips_control_characters() {
    let t = mcp(None);
    add(&t, "alpha");
    let session = t.cfg().tmux_session;
    let out = t.dir.path().join("typed");
    // Records exactly what arrives on the pane's input.
    t.tmux
        .new_session_detached(
            &session,
            "w",
            t.dir.path(),
            &[
                "sh".into(),
                "-c".into(),
                format!("stty raw -echo; head -c 200 > '{}'", out.display()),
            ],
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let window = t.tmux.list_windows_as(&session, "#{window_id}").unwrap()[0].clone();
    add_session(&t, "alpha", "wait", State::YourTurn, Some(&window));
    // An end-of-paste marker plus a fake "approve" keystroke, and a bell.
    let sent = t.do_send_prompt("wait".into(), "hi\u{1b}[201~1\r\u{7}there".into());
    std::thread::sleep(std::time::Duration::from_millis(400));
    t.tmux.kill_server();
    sent.unwrap();
    let got = std::fs::read(&out).unwrap_or_default();
    assert!(!got.contains(&0x07), "{got:?}");
    let text = String::from_utf8_lossy(&got);
    assert!(text.contains("hi[201~1there"), "{text:?}");
    assert_eq!(text.matches('\u{1b}').count(), 0, "{text:?}");
}

/// A window running a recorder that writes what it's sent to `<tmp>/typed`.
fn recorder(t: &T) -> (String, std::path::PathBuf) {
    let session = t.cfg().tmux_session;
    let out = t.dir.path().join("typed");
    t.tmux
        .new_session_detached(
            &session,
            "w",
            t.dir.path(),
            &[
                "sh".into(),
                "-c".into(),
                format!("stty raw -echo; cat > '{}'", out.display()),
            ],
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let window = t.tmux.list_windows_as(&session, "#{window_id}").unwrap()[0].clone();
    (window, out)
}

#[test]
fn send_prompt_leaves_other_paste_buffers_alone() {
    let t = mcp(None);
    add(&t, "alpha");
    let (window, _) = recorder(&t);
    add_session(&t, "alpha", "wait", State::YourTurn, Some(&window));
    t.tmux
        .cmd()
        .args(["set-buffer", "-b", "grove-paste", "keep"])
        .status()
        .unwrap();
    let sent = t.do_send_prompt("wait".into(), "hello there".into());
    let kept = t
        .tmux
        .cmd()
        .args(["show-buffer", "-b", "grove-paste"])
        .output()
        .unwrap();
    t.tmux.kill_server();
    sent.unwrap();
    assert_eq!(String::from_utf8_lossy(&kept.stdout), "keep");
}

#[test]
fn send_prompt_handles_long_text() {
    let t = mcp(None);
    add(&t, "alpha");
    let (window, out) = recorder(&t);
    add_session(&t, "alpha", "wait", State::YourTurn, Some(&window));
    let text = format!("{} END", "word ".repeat(5000));
    let sent = t.do_send_prompt("wait".into(), text);
    std::thread::sleep(std::time::Duration::from_millis(800));
    t.tmux.kill_server();
    sent.unwrap();
    let got = std::fs::read_to_string(&out).unwrap_or_default();
    assert!(got.contains("END"), "got {} bytes", got.len());
}

#[test]
fn a_window_that_isnt_the_sessions_is_refused() {
    let t = mcp(None);
    add(&t, "alpha");
    let (window, out) = recorder(&t);
    add_session(&t, "alpha", "wait", State::YourTurn, Some(&window));
    // A new tmux server reused the id for someone else's window.
    t.tmux
        .set_window_option(&window, "@grove_session", "another-session")
        .unwrap();
    let sent = t.do_send_prompt("wait".into(), "hello there".into());
    let read = t.do_read_session("wait".into(), None);
    std::thread::sleep(std::time::Duration::from_millis(200));
    t.tmux.kill_server();
    assert!(sent.unwrap_err().contains("gone"));
    assert!(read.unwrap_err().contains("gone"));
    assert_eq!(std::fs::read_to_string(&out).unwrap_or_default(), "");
}

#[test]
fn send_prompt_refuses_shell_and_slash_commands() {
    let t = mcp(None);
    add(&t, "alpha");
    let (window, out) = recorder(&t);
    add_session(&t, "alpha", "wait", State::YourTurn, Some(&window));
    let bang = t.do_send_prompt("wait".into(), "  !rm -rf ~".into());
    let slash = t.do_send_prompt("wait".into(), "/permissions".into());
    // Each line counts: without bracketed paste, a newline submits.
    let later = t.do_send_prompt("wait".into(), "hello\n  !rm -rf ~".into());
    let later_slash = t.do_send_prompt("wait".into(), "ok\r\n/permissions".into());
    let sep = t.do_send_prompt("wait".into(), "ok\u{2028}!rm -rf ~".into());
    std::thread::sleep(std::time::Duration::from_millis(200));
    t.tmux.kill_server();
    assert!(bang.unwrap_err().contains('!'));
    assert!(slash.unwrap_err().contains('/'));
    assert!(later.unwrap_err().contains('!'));
    assert!(later_slash.unwrap_err().contains('/'));
    assert!(sep.unwrap_err().contains('!'));
    assert_eq!(std::fs::read_to_string(&out).unwrap_or_default(), "");
}

#[test]
fn a_full_handle_naming_two_sessions_is_ambiguous() {
    let t = mcp(None);
    add(&t, "alpha");
    add_session(&t, "alpha", "s1", State::Working, None);
    let mut dup = add_session(&t, "alpha", "s1", State::Working, None);
    dup.id = "id-other".into();
    t.store.save_session(&dup).unwrap();
    let e = t.session("alpha:s1").unwrap_err();
    assert!(e.contains("id-alpha-s1") && e.contains("id-other"), "{e}");
}

#[test]
fn scope_follows_config_edits_without_a_restart() {
    let d = tempfile::tempdir().unwrap();
    let paths = Paths::new(d.path().join("home"));
    paths.ensure().unwrap();
    let work = d.path().join("work");
    std::fs::create_dir_all(work.join("alpha")).unwrap();
    let store = Store::new(paths.clone());
    store
        .add_project(&Project {
            name: "alpha".into(),
            path: work.join("alpha"),
            color: None,
            summary: None,
        })
        .unwrap();
    let env = |k: &str| (k == "GROVE_ACCOUNT").then(|| "default".to_string());
    let m = GroveMcp::new(store, crate::tmux::Tmux::new("x"), None, &env);
    assert!(m.do_list_projects().unwrap().contains("alpha"));
    std::fs::write(
        paths.config_file(),
        format!(
            "[accounts.work]\ndir = \"{}\"\nfolder = \"{}\"\n",
            d.path().join("claude-work").display(),
            work.display()
        ),
    )
    .unwrap();
    let after = m.do_list_projects().unwrap();
    assert!(!after.contains("alpha"), "{after}");
}
