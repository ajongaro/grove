use super::*;
use crate::config::Paths;
use crate::model::{Project, Session};

struct T {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    ctx: Ctx,
}

impl Drop for T {
    fn drop(&mut self) {
        self.ctx.tmux.kill_server();
    }
}

fn setup(tag: &str) -> T {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let paths = Paths::new(root.join("home"));
    paths.ensure().unwrap();
    let cfg = Config {
        tmux_session: "grove".into(),
        ..Config::default()
    };
    let tmux = Tmux::new(format!("grove-test-{}-{tag}", std::process::id()));
    T {
        _dir: dir,
        root,
        ctx: Ctx {
            store: Store::new(paths),
            cfg,
            tmux,
        },
    }
}

fn sess(id: &str, project: &str, state: State, window: Option<&str>) -> Session {
    Session {
        id: id.into(),
        project: project.into(),
        name: id.into(),
        host: "local".into(),
        tmux_window: window.map(str::to_string),
        state,
        message: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

fn sleeper() -> Vec<String> {
    vec!["sleep".into(), "30".into()]
}

#[test]
fn add_project_uses_folder_name_and_canonical_path() {
    let t = setup("add");
    let dir = t.root.join("My Cool App");
    std::fs::create_dir_all(&dir).unwrap();
    let name = add_project_at(&t.ctx.store, &format!("{}/", dir.display())).unwrap();
    assert_eq!(name, "My Cool App");
    let p = t.ctx.store.get_project("my-cool-app").unwrap();
    assert_eq!(p.path, dir.canonicalize().unwrap());
}

#[test]
fn add_project_expands_tilde() {
    let t = setup("tilde");
    let home = std::env::var("HOME").unwrap();
    // HOME itself always exists.
    add_project_at(&t.ctx.store, "~").unwrap();
    let all: Vec<Project> = t
        .ctx
        .store
        .list_projects()
        .into_iter()
        .filter_map(Result::ok)
        .collect();
    assert_eq!(
        all[0].path,
        std::path::PathBuf::from(home).canonicalize().unwrap()
    );
}

#[test]
fn add_project_rejects_missing_dir_and_duplicates() {
    let t = setup("adderr");
    assert!(add_project_at(&t.ctx.store, "/no/such/dir").is_err());
    let dir = t.root.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    add_project_at(&t.ctx.store, dir.to_str().unwrap()).unwrap();
    assert!(add_project_at(&t.ctx.store, dir.to_str().unwrap()).is_err());
}

#[test]
fn perform_add_project_reports_in_footer() {
    let t = setup("addmsg");
    let dir = t.root.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    let out = perform(
        &t.ctx,
        Action::AddProject {
            path: dir.to_string_lossy().into(),
        },
    )
    .unwrap();
    assert!(matches!(out, Outcome::Message(m) if m.contains("app")));
}

#[test]
fn clear_ended_removes_only_ended_sessions_of_that_project() {
    let t = setup("clear");
    let s = &t.ctx.store;
    s.save_session(&sess("a", "p", State::Ended, None)).unwrap();
    s.save_session(&sess("b", "p", State::Working, None))
        .unwrap();
    s.save_session(&sess("c", "q", State::Ended, None)).unwrap();
    perform(&t.ctx, Action::ClearEnded { slug: "p".into() }).unwrap();
    let ids: Vec<_> = s.list_sessions(None).into_iter().map(|x| x.id).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"b".to_string()) && ids.contains(&"c".to_string()));
}

#[test]
fn kill_closes_window_and_marks_ended() {
    let t = setup("kill");
    let tm = &t.ctx.tmux;
    tm.new_session_detached("grove", "dash", &t.root, &sleeper())
        .unwrap();
    let w = tm
        .new_window("grove", "p:s1", &t.root, &[], &sleeper())
        .unwrap();
    t.ctx
        .store
        .save_session(&sess("a", "p", State::Working, Some(&w)))
        .unwrap();
    perform(
        &t.ctx,
        Action::Kill {
            session_id: "a".into(),
        },
    )
    .unwrap();
    assert!(!tm.list_windows("grove").unwrap().contains(&w));
    assert_eq!(t.ctx.store.get_session("a").unwrap().state, State::Ended);
}

#[test]
fn kill_of_already_closed_window_still_marks_ended() {
    let t = setup("kill2");
    t.ctx
        .tmux
        .new_session_detached("grove", "dash", &t.root, &sleeper())
        .unwrap();
    t.ctx
        .store
        .save_session(&sess("a", "p", State::Working, Some("@999")))
        .unwrap();
    perform(
        &t.ctx,
        Action::Kill {
            session_id: "a".into(),
        },
    )
    .unwrap();
    assert_eq!(t.ctx.store.get_session("a").unwrap().state, State::Ended);
}

#[test]
fn new_session_for_missing_project_dir_is_an_error() {
    let t = setup("newerr");
    t.ctx
        .store
        .add_project(&Project {
            name: "gone".into(),
            path: "/no/such".into(),
            color: None,
            summary: None,
        })
        .unwrap();
    assert!(
        perform(
            &t.ctx,
            Action::NewSession {
                slug: "gone".into(),
                name: None
            }
        )
        .is_err()
    );
    assert!(t.ctx.store.list_sessions(None).is_empty());
}

#[test]
fn project_shell_for_missing_project_dir_is_an_error() {
    let t = setup("shellerr");
    t.ctx
        .store
        .add_project(&Project {
            name: "gone".into(),
            path: "/no/such".into(),
            color: None,
            summary: None,
        })
        .unwrap();
    let err = perform(
        &t.ctx,
        Action::ProjectShell {
            slug: "gone".into(),
        },
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("/no/such"), "{err:#}");
}

#[test]
fn project_shell_runs_your_shell_outside_tmux() {
    // A popup can't hold panes: tools that see $TMUX (fzf-tmux) would open
    // in a split behind it, where no key reaches them.
    let env = ["env", "-u", "TMUX", "-u", "TMUX_PANE"];
    let with = |sh: &str| {
        let mut v: Vec<String> = env.iter().map(|s| s.to_string()).collect();
        v.push(sh.to_string());
        v
    };
    assert_eq!(shell_argv(Some("/bin/zsh")), with("/bin/zsh"));
    assert_eq!(shell_argv(Some("")), with("/bin/sh"));
    assert_eq!(shell_argv(None), with("/bin/sh"));
}

#[test]
fn the_project_shell_sees_no_tmux() {
    let out = std::process::Command::new(&shell_argv(None)[0])
        .args(&shell_argv(None)[1..])
        .args(["-c", "echo \"[$TMUX][$TMUX_PANE]\""])
        .env("TMUX", "/tmp/sock,1,0")
        .env("TMUX_PANE", "%3")
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "[][]");
}

#[test]
fn quit_and_none() {
    let t = setup("quit");
    assert_eq!(perform(&t.ctx, Action::Quit).unwrap(), Outcome::Quit);
    assert_eq!(perform(&t.ctx, Action::None).unwrap(), Outcome::Nothing);
}

#[test]
fn reconcile_ends_vanished_and_flags_needs_you() {
    let t = setup("recon");
    let tm = &t.ctx.tmux;
    tm.new_session_detached("grove", "dash", &t.root, &sleeper())
        .unwrap();
    let live = tm
        .new_window("grove", "p:needy", &t.root, &[], &sleeper())
        .unwrap();
    let s = &t.ctx.store;
    s.save_session(&sess("needy", "p", State::NeedsYou, Some(&live)))
        .unwrap();
    s.save_session(&sess("gone", "p", State::Working, Some("@999")))
        .unwrap();
    let mut flagged = HashMap::new();
    reconcile(&t.ctx, &mut flagged).unwrap();
    assert_eq!(s.get_session("gone").unwrap().state, State::Ended);
    assert_eq!(s.get_session("needy").unwrap().state, State::NeedsYou);
    assert_eq!(flagged.get(&live), Some(&true));
    let name = |id: &str| {
        let out = tm
            .cmd()
            .args(["display-message", "-p", "-t", id, "#{window_name}"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    // The marker is in the name so it shows under any status-bar theme.
    assert_eq!(name(&live), "⚑ p:needy");

    // Leaving needs_you clears the flag.
    let mut n = s.get_session("needy").unwrap();
    n.state = State::Working;
    s.save_session(&n).unwrap();
    reconcile(&t.ctx, &mut flagged).unwrap();
    assert_eq!(flagged.get(&live), Some(&false));
    assert_eq!(name(&live), "p:needy");
}

#[test]
fn reconcile_without_tmux_session_is_ok() {
    let t = setup("recon-none");
    t.ctx
        .store
        .save_session(&sess("a", "p", State::Working, Some("@1")))
        .unwrap();
    // No grove tmux session at all: don't mass-end sessions on a tmux hiccup.
    assert!(reconcile(&t.ctx, &mut HashMap::new()).is_err());
    assert_eq!(t.ctx.store.get_session("a").unwrap().state, State::Working);
}

#[test]
fn new_session_uses_current_config_file() {
    let t = setup("freshcfg");
    let dir = t.root.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    add_project_at(&t.ctx.store, dir.to_str().unwrap()).unwrap();
    // Edited after the dashboard started: must win over ctx.cfg.
    std::fs::write(
        t.ctx.store.paths.config_file(),
        "claude_config_dir = \"/no/such/edited-account\"\n",
    )
    .unwrap();
    let err = perform(
        &t.ctx,
        Action::NewSession {
            slug: "app".into(),
            name: None,
        },
    )
    .unwrap_err();
    assert!(
        format!("{err:#}").contains("/no/such/edited-account"),
        "{err:#}"
    );
}

#[test]
fn add_prompt_default_follows_the_folder_grove_was_run_from() {
    let t = setup("cwd");
    let tm = &t.ctx.tmux;
    tm.new_session_detached("grove", "dash", &t.root, &sleeper())
        .unwrap();
    let mut app = crate::ui::app::App::new("/initial".into());
    refresh_cwd(&t.ctx, &mut app);
    assert_eq!(
        app.cwd, "/initial",
        "no GROVE_CWD yet: keep the old default"
    );
    tm.set_environment("grove", "GROVE_CWD", "/Users/me/Play/foo")
        .unwrap();
    refresh_cwd(&t.ctx, &mut app);
    assert_eq!(app.cwd, "/Users/me/Play/foo");
}

#[test]
fn create_project_makes_nested_dirs_with_git_and_registers() {
    let t = setup("create");
    let dir = t.root.join("Play").join("New Thing");
    let name = create_project_at(&t.ctx.store, &dir.to_string_lossy()).unwrap();
    assert_eq!(name, "New Thing");
    assert!(dir.join(".git").is_dir(), "git init ran");
    assert_eq!(
        t.ctx.store.get_project("new-thing").unwrap().path,
        dir.canonicalize().unwrap()
    );
}

#[test]
fn create_project_refuses_existing_paths() {
    let t = setup("create-exists");
    let file = t.root.join("a-file");
    std::fs::write(&file, "x").unwrap();
    let err = create_project_at(&t.ctx.store, &file.to_string_lossy())
        .unwrap_err()
        .to_string();
    assert!(err.contains("already exists"), "{err}");
    let dir = t.root.join("a-dir");
    std::fs::create_dir(&dir).unwrap();
    assert!(create_project_at(&t.ctx.store, &dir.to_string_lossy()).is_err());
    assert!(t.ctx.store.list_projects().is_empty());
}

#[test]
fn perform_create_project_reports_in_footer() {
    let t = setup("create-msg");
    let dir = t.root.join("fresh");
    let out = perform(
        &t.ctx,
        Action::CreateProject {
            path: dir.to_string_lossy().into(),
        },
    )
    .unwrap();
    assert!(
        matches!(&out, Outcome::Message(m) if m.contains("fresh") && m.contains("git")),
        "{out:?}"
    );
}

#[test]
fn remove_project_waits_for_running_sessions() {
    let t = setup("rm");
    let tm = &t.ctx.tmux;
    tm.new_session_detached("grove", "dash", &t.root, &sleeper())
        .unwrap();
    let live = tm
        .new_window("grove", "alpha:s1", &t.root, &[], &sleeper())
        .unwrap();
    let s = &t.ctx.store;
    add_project_at(s, &t.root.to_string_lossy()).unwrap();
    let slug = s.list_projects()[0].as_ref().unwrap().slug();
    s.save_session(&sess("run", &slug, State::Working, Some(&live)))
        .unwrap();
    s.save_session(&sess("old", &slug, State::Ended, None))
        .unwrap();

    let err = remove_project(s, tm, "grove", &slug).unwrap_err();
    assert!(format!("{err}").contains("1 running session"), "{err}");
    assert!(s.get_project(&slug).is_ok());

    tm.kill_window(&live).unwrap();
    remove_project(s, tm, "grove", &slug).unwrap();
    assert!(s.get_project(&slug).is_err());
    assert!(s.list_sessions(Some(&slug)).is_empty());
    assert!(t.root.is_dir(), "the folder is kept");
}

#[test]
fn observe_names_local_and_finca_sessions() {
    let t = setup("observe");
    let mut app = App::new(String::new());
    app.sessions = vec![sess("1", "alpha", State::NeedsYou, Some("@3"))];
    let st = crate::status::Status {
        version: crate::status::STATUS_VERSION,
        host: "homebox".into(),
        generated_at: chrono::Utc::now(),
        needs_you: 1,
        live: 1,
        projects: vec![crate::status::ProjectStatus {
            name: "Beta".into(),
            slug: "beta".into(),
            sessions: vec![crate::status::SessionStatus {
                name: "2".into(),
                state: State::NeedsYou,
                message: None,
                updated_at: chrono::Utc::now(),
                window: None,
            }],
        }],
    };
    let _ = &t;
    app.fincas = vec![super::super::app::FincaRow {
        slug: "homebox".into(),
        name: "homebox".into(),
        color: None,
        error: None,
        reach: None,
        checked_at: None,
        last_status: Some(st),
        last_ok: None,
        via: None,
    }];
    let seen = observe(&app);
    let labels: Vec<_> = seen
        .iter()
        .map(|o| (o.label.as_str(), o.source.as_str()))
        .collect();
    assert_eq!(
        labels,
        vec![("alpha:1", "local"), ("homebox · Beta:2", "homebox")]
    );
    assert_eq!(seen[0].window.as_deref(), Some("@3"));
    assert_eq!(seen[1].key, "homebox/beta/2");
}

#[test]
fn save_todos_writes_the_projects_checklist() {
    let t = setup("todos");
    let out = perform(
        &t.ctx,
        Action::SaveTodos {
            slug: "p".into(),
            text: "- [ ] a\n".into(),
        },
    )
    .unwrap();
    assert_eq!(out, Outcome::Nothing);
    assert_eq!(t.ctx.store.read_todos("p"), "- [ ] a\n");
}

fn window_name(tm: &Tmux, id: &str) -> String {
    let out = tm
        .cmd()
        .args(["display-message", "-p", "-t", id, "#{window_name}"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
fn rename_session_renames_its_window_keeping_the_needs_you_mark() {
    let t = setup("rename");
    let tm = &t.ctx.tmux;
    tm.new_session_detached("grove", "dash", &t.root, &sleeper())
        .unwrap();
    let w = tm
        .new_window("grove", "p:s1", &t.root, &[], &sleeper())
        .unwrap();
    let s = &t.ctx.store;
    s.save_session(&sess("1", "p", State::NeedsYou, Some(&w)))
        .unwrap();
    let out = perform(
        &t.ctx,
        Action::RenameSession {
            session_id: "1".into(),
            name: "  api work ".into(),
        },
    )
    .unwrap();
    assert_eq!(out, Outcome::Message("Renamed 1 to api work".into()));
    assert_eq!(s.get_session("1").unwrap().name, "api work");
    assert_eq!(window_name(tm, &w), "⚑ p:api work");
}

#[test]
fn rename_refuses_a_name_another_live_session_in_the_project_has() {
    let t = setup("rename-taken");
    let s = &t.ctx.store;
    s.save_session(&sess("1", "p", State::Working, None))
        .unwrap();
    s.save_session(&sess("2", "p", State::YourTurn, None))
        .unwrap();
    s.save_session(&sess("old", "p", State::Ended, None))
        .unwrap();
    s.save_session(&sess("elsewhere", "q", State::Working, None))
        .unwrap();
    let rename = |name: &str| {
        perform(
            &t.ctx,
            Action::RenameSession {
                session_id: "1".into(),
                name: name.into(),
            },
        )
    };
    let err = rename("2").unwrap_err().to_string();
    assert!(err.contains("already") && err.contains("2"), "{err}");
    assert_eq!(s.get_session("1").unwrap().name, "1");
    // Ended sessions and other projects don't count.
    rename("old").unwrap();
    rename("elsewhere").unwrap();
    assert_eq!(s.get_session("1").unwrap().name, "elsewhere");
}

#[test]
fn window_title_is_project_and_name_with_the_mark_when_needed() {
    assert_eq!(window_title(&sess("a", "p", State::Working, None)), "p:a");
    assert_eq!(
        window_title(&sess("a", "p", State::NeedsYou, None)),
        "⚑ p:a"
    );
}

#[test]
fn update_runs_the_installed_grove_from_the_add_prompt_folder() {
    // Running `grove` records its cwd as the add-project default, so it must
    // run from the dashboard's current default, not from ~/.grove.
    let c = update_command(std::path::Path::new("/bin/grove"), "/home/me/code");
    assert_eq!(c.get_program(), "/bin/grove");
    assert_eq!(c.get_args().count(), 0);
    assert_eq!(
        c.get_current_dir(),
        Some(std::path::Path::new("/home/me/code"))
    );
}

#[test]
fn grid_reply_types_into_a_local_waiting_session() {
    let t = setup("gridreply");
    let typed = t.root.join("typed");
    t.ctx
        .tmux
        .new_session_detached(
            "grove",
            "w",
            &t.root,
            &[
                "sh".into(),
                "-c".into(),
                format!("stty raw -echo; cat > '{}'", typed.display()),
            ],
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let w = t.ctx.tmux.list_windows_as("grove", "#{window_id}").unwrap()[0].clone();
    for (id, state) in [("s1", State::YourTurn), ("s2", State::Working)] {
        t.ctx
            .store
            .save_session(&sess(id, "alpha", state, Some(&w)))
            .unwrap();
    }
    // The window is s1's.
    t.ctx
        .tmux
        .set_window_option(&w, "@grove_session", "s1")
        .unwrap();
    let reply = |id: &str| {
        perform(
            &t.ctx,
            Action::GridReply {
                finca: None,
                session_id: id.into(),
                handle: format!("alpha:{id}"),
                text: "hello there".into(),
            },
        )
    };
    let sent = reply("s1");
    let busy = reply("s2");
    std::thread::sleep(std::time::Duration::from_millis(400));
    match sent {
        Ok(Outcome::Message(m)) => assert!(m.contains("sent to alpha:s1"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert!(format!("{:#}", busy.unwrap_err()).contains("working"));
    assert!(
        std::fs::read_to_string(&typed)
            .unwrap_or_default()
            .contains("hello there")
    );
}

#[test]
fn grid_kill_closes_a_local_window_and_names_it() {
    let t = setup("gridkill");
    let tm = &t.ctx.tmux;
    tm.new_session_detached("grove", "dash", &t.root, &sleeper())
        .unwrap();
    let w = tm
        .new_window("grove", "p:a", &t.root, &[], &sleeper())
        .unwrap();
    t.ctx
        .store
        .save_session(&sess("a", "p", State::Working, Some(&w)))
        .unwrap();
    let kill = || {
        perform(
            &t.ctx,
            Action::GridKill {
                finca: None,
                session_id: "a".into(),
                handle: "p:a".into(),
            },
        )
    };
    match kill() {
        Ok(Outcome::Message(m)) => assert_eq!(m, "Killed p:a"),
        other => panic!("{other:?}"),
    }
    assert!(!tm.list_windows("grove").unwrap().contains(&w));
    assert!(format!("{:#}", kill().unwrap_err()).contains("ended"));
}
