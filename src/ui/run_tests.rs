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
fn remove_project_waits_for_running_sessions_and_keeps_the_wiki() {
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
    s.write_wiki(&slug, "Notes", "kept").unwrap();

    let err = remove_project(s, tm, "grove", &slug).unwrap_err();
    assert!(format!("{err}").contains("1 running session"), "{err}");
    assert!(s.get_project(&slug).is_ok());

    tm.kill_window(&live).unwrap();
    remove_project(s, tm, "grove", &slug).unwrap();
    assert!(s.get_project(&slug).is_err());
    assert!(s.list_sessions(Some(&slug)).is_empty());
    assert_eq!(s.wiki_sections(&slug).len(), 1);
}

#[test]
fn observe_names_local_and_finca_sessions() {
    let t = setup("observe");
    let mut app = App::new(String::new());
    app.sessions = vec![sess("1", "alpha", State::NeedsYou, Some("@3"))];
    let st = crate::status::Status {
        version: crate::status::STATUS_VERSION,
        host: "therver".into(),
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
            }],
        }],
    };
    let _ = &t;
    app.fincas = vec![super::super::app::FincaRow {
        slug: "therver".into(),
        name: "therver".into(),
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
        vec![("alpha:1", "local"), ("therver · Beta:2", "therver")]
    );
    assert_eq!(seen[0].window.as_deref(), Some("@3"));
    assert_eq!(seen[1].key, "therver/beta/2");
}
