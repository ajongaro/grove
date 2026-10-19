use chrono::{Duration, TimeZone, Utc};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::*;
use super::input::handle_key;
use super::render::render;
use crate::config::Paths;
use crate::model::{Project, Session, State};
use crate::store::Store;

fn t0() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap()
}

fn session(id: &str, project: &str, name: &str, state: State, window: Option<&str>) -> Session {
    Session {
        id: id.into(),
        project: project.into(),
        name: name.into(),
        host: "local".into(),
        tmux_window: window.map(str::to_string),
        state,
        message: None,
        created_at: t0(),
        updated_at: t0(),
    }
}

fn store_with(projects: &[&str], sessions: &[Session]) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(Paths::new(dir.path().join("home")));
    for p in projects {
        store
            .add_project(&Project {
                name: p.to_string(),
                path: format!("/code/{p}").into(),
                color: None,
                summary: None,
            })
            .unwrap();
    }
    for s in sessions {
        store.save_session(s).unwrap();
    }
    (dir, store)
}

fn key(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}
fn ch(c: char) -> KeyEvent {
    key(KeyCode::Char(c))
}

fn screen(app: &App, w: u16, h: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| render(f, app, t0() + Duration::minutes(2)))
        .unwrap();
    let buf = term.backend().buffer().clone();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn loaded(projects: &[&str], sessions: &[Session]) -> (tempfile::TempDir, Store, App) {
    let (d, s) = store_with(projects, sessions);
    let mut app = App::new("/home/me/code/newproj".into());
    app.reload(&s);
    (d, s, app)
}

// ---------- state ----------

#[test]
fn reload_reads_projects_and_sessions() {
    let (_d, _s, app) = loaded(
        &["alpha", "beta"],
        &[
            session("1", "alpha", "s1", State::Working, Some("@1")),
            session("2", "beta", "s1", State::NeedsYou, Some("@2")),
        ],
    );
    assert_eq!(app.projects.len(), 2);
    assert_eq!(app.selected_project().unwrap().slug, "alpha");
    assert_eq!(app.visible_sessions().len(), 1);
    assert_eq!(app.needs_you_count(), 1);
}

#[test]
fn reload_keeps_selection_on_same_project() {
    let (_d, s, mut app) = loaded(&["beta", "gamma"], &[]);
    app.proj_idx = 1; // gamma
    s.add_project(&Project {
        name: "alpha".into(),
        path: "/a".into(),
        color: None,
        summary: None,
    })
    .unwrap();
    app.reload(&s);
    assert_eq!(app.selected_project().unwrap().slug, "gamma");
}

#[test]
fn reload_clamps_selection_when_list_shrinks() {
    let (_d, s, mut app) = loaded(
        &["alpha"],
        &[session("1", "alpha", "s1", State::Working, Some("@1"))],
    );
    app.pane = Pane::Sessions;
    app.sess_idx = 0;
    s.delete_session("1").unwrap();
    app.reload(&s);
    assert!(app.selected_session().is_none());
    assert_eq!(app.sess_idx, 0);
}

#[test]
fn malformed_project_shows_as_error_row() {
    let (_d, s) = store_with(&["alpha"], &[]);
    std::fs::write(s.paths.projects.join("broken.toml"), "name = [").unwrap();
    let mut app = App::new("/x".into());
    app.reload(&s);
    assert_eq!(app.projects.len(), 2);
    assert!(app.projects.iter().any(|p| p.error.is_some()));
}

#[test]
fn mark_missing_ends_sessions_whose_window_is_gone() {
    let sessions = vec![
        session("1", "p", "s1", State::Working, Some("@1")),
        session("2", "p", "s2", State::NeedsYou, Some("@2")),
        session("3", "p", "s3", State::Starting, None),
        session("4", "p", "s4", State::Ended, Some("@4")),
    ];
    let ended = mark_missing(&sessions, &["@1".to_string()], t0());
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].id, "2");
    assert_eq!(ended[0].state, State::Ended);
    assert_eq!(ended[0].updated_at, t0());
}

#[test]
fn ages_are_compact() {
    let now = t0();
    assert_eq!(format_age(now, now), "now");
    assert_eq!(format_age(now - Duration::seconds(45), now), "45s");
    assert_eq!(format_age(now - Duration::minutes(3), now), "3m");
    assert_eq!(format_age(now - Duration::hours(2), now), "2h");
    assert_eq!(format_age(now - Duration::days(4), now), "4d");
    assert_eq!(format_age(now + Duration::seconds(5), now), "now");
}

// ---------- keys ----------

#[test]
fn navigation_moves_within_bounds() {
    let (_d, _s, mut app) = loaded(&["a", "b", "c"], &[]);
    handle_key(&mut app, ch('j'));
    handle_key(&mut app, key(KeyCode::Down));
    handle_key(&mut app, ch('j'));
    assert_eq!(app.proj_idx, 2);
    handle_key(&mut app, ch('k'));
    assert_eq!(app.proj_idx, 1);
}

#[test]
fn tab_switches_panes() {
    let (_d, _s, mut app) = loaded(&["a"], &[]);
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(app.pane, Pane::Sessions);
    handle_key(&mut app, ch('h'));
    assert_eq!(app.pane, Pane::Projects);
}

#[test]
fn enter_on_project_starts_session() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::NewSession {
            slug: "alpha".into(),
            name: None
        }
    );
}

#[test]
fn enter_on_session_jumps_to_its_window() {
    let (_d, _s, mut app) = loaded(
        &["alpha"],
        &[session("1", "alpha", "s1", State::Working, Some("@7"))],
    );
    app.pane = Pane::Sessions;
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::Jump {
            window: "@7".into()
        }
    );
}

#[test]
fn enter_on_ended_session_does_nothing() {
    let (_d, _s, mut app) = loaded(
        &["alpha"],
        &[session("1", "alpha", "s1", State::Ended, Some("@7"))],
    );
    app.pane = Pane::Sessions;
    assert_eq!(handle_key(&mut app, key(KeyCode::Enter)), Action::None);
}

#[test]
fn n_prompts_for_name_then_starts_named_session() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    assert_eq!(handle_key(&mut app, ch('n')), Action::None);
    assert!(matches!(
        app.mode,
        Mode::Prompt {
            kind: PromptKind::NewSession,
            ..
        }
    ));
    for c in "auth fix".chars() {
        handle_key(&mut app, ch(c));
    }
    handle_key(&mut app, key(KeyCode::Backspace));
    handle_key(&mut app, ch('x'));
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::NewSession {
            slug: "alpha".into(),
            name: Some("auth fix".into())
        }
    );
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn n_with_empty_name_uses_default() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('n'));
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::NewSession {
            slug: "alpha".into(),
            name: None
        }
    );
}

#[test]
fn esc_cancels_prompt() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('n'));
    assert_eq!(handle_key(&mut app, key(KeyCode::Esc)), Action::None);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn q_inside_prompt_is_text_not_quit() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('n'));
    assert_eq!(handle_key(&mut app, ch('q')), Action::None);
    assert!(matches!(&app.mode, Mode::Prompt { input, .. } if input == "q"));
}

#[test]
fn a_prompts_with_cwd_default() {
    let (_d, _s, mut app) = loaded(&[], &[]);
    handle_key(&mut app, ch('a'));
    assert!(
        matches!(&app.mode, Mode::Prompt { kind: PromptKind::AddProject, input } if input == "/home/me/code/newproj")
    );
    // That folder doesn't exist, so Enter offers to create it.
    assert_eq!(handle_key(&mut app, key(KeyCode::Enter)), Action::None);
    assert_eq!(
        app.mode,
        Mode::ConfirmCreate {
            path: "/home/me/code/newproj".into()
        }
    );
}

#[test]
fn x_asks_then_kills_on_y() {
    let (_d, _s, mut app) = loaded(
        &["alpha"],
        &[session("1", "alpha", "s1", State::Working, Some("@7"))],
    );
    app.pane = Pane::Sessions;
    assert_eq!(handle_key(&mut app, ch('x')), Action::None);
    assert!(matches!(app.mode, Mode::Confirm { .. }));
    assert_eq!(
        handle_key(&mut app, ch('y')),
        Action::Kill {
            session_id: "1".into()
        }
    );
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn x_then_n_cancels() {
    let (_d, _s, mut app) = loaded(
        &["alpha"],
        &[session("1", "alpha", "s1", State::Working, Some("@7"))],
    );
    app.pane = Pane::Sessions;
    handle_key(&mut app, ch('x'));
    assert_eq!(handle_key(&mut app, ch('n')), Action::None);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn project_keys_need_a_project() {
    let (_d, _s, mut app) = loaded(&[], &[]);
    for c in ['n', 'e', 'c', 'x'] {
        assert_eq!(handle_key(&mut app, ch(c)), Action::None, "{c}");
        assert_eq!(app.mode, Mode::Normal, "{c}");
    }
    assert_eq!(handle_key(&mut app, key(KeyCode::Enter)), Action::None);
}

#[test]
fn edit_clear_and_quit() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    assert_eq!(
        handle_key(&mut app, ch('e')),
        Action::EditProject {
            slug: "alpha".into()
        }
    );
    // w opened the wiki, which is gone.
    assert_eq!(handle_key(&mut app, ch('w')), Action::None);
    assert_eq!(
        handle_key(&mut app, ch('c')),
        Action::ClearEnded {
            slug: "alpha".into()
        }
    );
    assert_eq!(handle_key(&mut app, ch('q')), Action::Quit);
}

#[test]
fn error_row_cannot_start_sessions() {
    let (_d, s) = store_with(&[], &[]);
    s.paths.ensure().unwrap();
    std::fs::write(s.paths.projects.join("broken.toml"), "name = [").unwrap();
    let mut app = App::new("/x".into());
    app.reload(&s);
    assert_eq!(handle_key(&mut app, key(KeyCode::Enter)), Action::None);
    assert!(app.footer.is_some());
}

// ---------- render ----------

#[test]
fn empty_state_explains_how_to_start() {
    let (_d, _s, app) = loaded(&[], &[]);
    let s = screen(&app, 90, 20);
    assert!(s.contains("No projects"), "{s}");
    assert!(s.contains("press a"), "{s}");
}

#[test]
fn renders_projects_sessions_states_and_counts() {
    let mut needy = session("2", "alpha", "auth-fix", State::NeedsYou, Some("@2"));
    needy.message = Some("Claude needs your permission to use Bash".into());
    let (_d, _s, app) = loaded(
        &["alpha", "beta"],
        &[
            session("1", "alpha", "s1", State::Working, Some("@1")),
            needy,
        ],
    );
    let s = screen(&app, 100, 20);
    for needle in [
        "PROJECTS",
        "alpha",
        "beta",
        "s1",
        "working",
        "auth-fix",
        "needs you",
        "2m",
        "permission to use Bash",
        "1 needs you",
    ] {
        assert!(s.contains(needle), "missing {needle:?}\n{s}");
    }
}

#[test]
fn prompt_is_visible() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('n'));
    handle_key(&mut app, ch('z'));
    let s = screen(&app, 90, 20);
    assert!(s.contains("Session name"), "{s}");
}

#[test]
fn footer_message_is_shown() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    app.footer = Some("project dir missing".into());
    assert!(screen(&app, 90, 20).contains("project dir missing"));
}

#[test]
fn tiny_terminal_does_not_panic() {
    let (_d, _s, app) = loaded(
        &["alpha"],
        &[session("1", "alpha", "s1", State::Working, Some("@1"))],
    );
    screen(&app, 10, 3);
}

#[test]
fn ctrl_chords_never_trigger_actions() {
    let (_d, _s, mut app) = loaded(
        &["alpha"],
        &[session("1", "alpha", "s1", State::Working, Some("@7"))],
    );
    for c in ['c', 'e', 'w', 'n', 'a', 'q', 'x'] {
        for pane in [Pane::Projects, Pane::Sessions] {
            app.pane = pane;
            let action = handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL),
            );
            assert_eq!(action, Action::None, "ctrl-{c}");
            assert_eq!(app.mode, Mode::Normal, "ctrl-{c}");
        }
    }
    assert_eq!(
        handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)),
        Action::None
    );
}

// ---------- path completion in the add prompt ----------

fn add_prompt_at(input: &str) -> App {
    let (_d, _s, mut app) = loaded(&[], &[]);
    app.mode = Mode::Prompt {
        kind: PromptKind::AddProject,
        input: input.into(),
    };
    app
}

fn prompt_text(app: &App) -> String {
    match &app.mode {
        Mode::Prompt { input, .. } => input.clone(),
        m => panic!("not in a prompt: {m:?}"),
    }
}

fn dirs(names: &[&str]) -> (tempfile::TempDir, String) {
    let d = tempfile::tempdir().unwrap();
    for n in names {
        std::fs::create_dir(d.path().join(n)).unwrap();
    }
    let base = format!("{}/", d.path().display());
    (d, base)
}

#[test]
fn tab_completes_unique_dir_in_add_prompt() {
    let (_d, base) = dirs(&["streakwithfriends", "grove"]);
    let mut app = add_prompt_at(&format!("{base}str"));
    assert_eq!(handle_key(&mut app, key(KeyCode::Tab)), Action::None);
    assert_eq!(prompt_text(&app), format!("{base}streakwithfriends/"));
    assert!(app.completions.is_empty());
}

#[test]
fn repeated_tab_cycles_through_matches() {
    let (_d, base) = dirs(&["grove", "grove-fi"]);
    let mut app = add_prompt_at(&format!("{base}gr"));
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(prompt_text(&app), format!("{base}grove"));
    assert_eq!(app.completions.len(), 2);
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(prompt_text(&app), format!("{base}grove/"));
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(prompt_text(&app), format!("{base}grove-fi/"));
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(prompt_text(&app), format!("{base}grove/"));
}

#[test]
fn typing_after_tab_clears_the_match_list() {
    let (_d, base) = dirs(&["grove", "grove-fi"]);
    let mut app = add_prompt_at(&format!("{base}gr"));
    handle_key(&mut app, key(KeyCode::Tab));
    handle_key(&mut app, ch('-'));
    assert!(app.completions.is_empty());
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(prompt_text(&app), format!("{base}grove-fi/"));
}

#[test]
fn enter_after_completion_adds_that_path() {
    let (_d, base) = dirs(&["streakwithfriends"]);
    let mut app = add_prompt_at(&format!("{base}str"));
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::AddProject {
            path: format!("{base}streakwithfriends/")
        }
    );
    assert!(app.completions.is_empty());
}

#[test]
fn tab_in_session_name_prompt_does_nothing() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('n'));
    handle_key(&mut app, ch('x'));
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(prompt_text(&app), "x");
}

#[test]
fn match_list_is_rendered_under_the_prompt() {
    let (_d, base) = dirs(&["grove", "grove-fi"]);
    let mut app = add_prompt_at(&format!("{base}gr"));
    handle_key(&mut app, key(KeyCode::Tab));
    handle_key(&mut app, key(KeyCode::Tab));
    let s = screen(&app, 120, 24);
    assert!(s.contains("grove-fi/"), "{s}");
    assert!(s.contains("Tab"), "{s}");
}

// ---------- fincas ----------

use crate::finca::{Finca, PollResult, Reach};
use crate::status::{ProjectStatus, SessionStatus, Status};

fn remote_status(host: &str, needy: usize, working: usize) -> Status {
    let mut sessions = Vec::new();
    for i in 0..needy {
        sessions.push(SessionStatus {
            id: Some(format!("{host}-n{i}")),
            name: format!("n{i}"),
            state: State::NeedsYou,
            message: Some("Permission needed: Bash".into()),
            updated_at: t0(),
            window: Some(format!("@1{i}")),
        });
    }
    for i in 0..working {
        sessions.push(SessionStatus {
            id: Some(format!("{host}-w{i}")),
            name: format!("w{i}"),
            state: State::Working,
            message: None,
            updated_at: t0(),
            window: Some(format!("@2{i}")),
        });
    }
    Status {
        version: 1,
        host: host.into(),
        generated_at: t0(),
        needs_you: needy,
        live: needy + working,
        projects: vec![ProjectStatus {
            name: "btt".into(),
            slug: "btt".into(),
            sessions,
        }],
    }
}

fn with_fincas(
    projects: &[&str],
    local: &[Session],
    fincas: &[&str],
) -> (tempfile::TempDir, Store, App) {
    let (d, s) = store_with(projects, local);
    for f in fincas {
        s.add_finca(&Finca {
            name: f.to_string(),
            ssh: format!("me@{f}"),
            grove_bin: "/g".into(),
            color: None,
            backup: None,
        })
        .unwrap();
    }
    let mut app = App::new("/x".into());
    app.reload(&s);
    (d, s, app)
}

fn poll(app: &mut App, slug: &str, reach: Reach, at: chrono::DateTime<Utc>) {
    let via = matches!(reach, Reach::Reachable(_)).then(|| format!("me@{slug}"));
    app.apply_poll(PollResult {
        slug: slug.into(),
        reach,
        via,
        at,
    });
}

#[test]
fn enter_connects_via_the_destination_that_last_answered() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["homebox"]);
    app.apply_poll(PollResult {
        slug: "homebox".into(),
        reach: Reach::Reachable(remote_status("homebox", 0, 0)),
        via: Some("me@homebox.ts.net".into()),
        at: t0(),
    });
    // A later failed poll keeps the last good destination.
    poll(
        &mut app,
        "homebox",
        Reach::Unreachable {
            error: "timed out".into(),
        },
        t0(),
    );
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::ConnectFinca {
            slug: "homebox".into(),
            via: Some("me@homebox.ts.net".into())
        }
    );
}

#[test]
fn fincas_listed_under_projects_with_counts() {
    let (_d, _s, mut app) = with_fincas(&["alpha"], &[], &["homebox", "pi", "old"]);
    poll(
        &mut app,
        "homebox",
        Reach::Reachable(remote_status("homebox", 2, 1)),
        t0(),
    );
    poll(
        &mut app,
        "pi",
        Reach::Unreachable {
            error: "timed out".into(),
        },
        t0(),
    );
    poll(&mut app, "old", Reach::NeedsUpdate, t0());
    let s = screen(&app, 110, 24);
    for needle in [
        "FINCAS",
        "homebox",
        "3 live",
        "⚑2",
        "pi",
        "unreachable",
        "needs update",
    ] {
        assert!(s.contains(needle), "missing {needle:?}\n{s}");
    }
}

#[test]
fn unpolled_finca_shows_checking() {
    let (_d, _s, app) = with_fincas(&[], &[], &["homebox"]);
    assert!(screen(&app, 100, 20).contains("checking"));
}

#[test]
fn footer_counts_remote_needs_you() {
    let (_d, _s, mut app) = with_fincas(
        &["alpha"],
        &[session("1", "alpha", "s1", State::NeedsYou, Some("@1"))],
        &["homebox"],
    );
    poll(
        &mut app,
        "homebox",
        Reach::Reachable(remote_status("homebox", 2, 0)),
        t0(),
    );
    assert_eq!(app.needs_you_count(), 3);
    let s = screen(&app, 110, 20);
    assert!(s.contains("3 needs you (2 on homebox)"), "{s}");
}

#[test]
fn selection_moves_from_projects_into_fincas() {
    let (_d, _s, mut app) = with_fincas(&["alpha"], &[], &["homebox", "pi"]);
    assert_eq!(app.selected_project().unwrap().slug, "alpha");
    handle_key(&mut app, ch('j'));
    assert!(app.selected_project().is_none());
    assert_eq!(app.selected_finca().unwrap().slug, "homebox");
    handle_key(&mut app, ch('j'));
    assert_eq!(app.selected_finca().unwrap().slug, "pi");
    handle_key(&mut app, ch('j'));
    assert_eq!(app.selected_finca().unwrap().slug, "pi", "stays at the end");
    handle_key(&mut app, ch('k'));
    handle_key(&mut app, ch('k'));
    assert_eq!(app.selected_project().unwrap().slug, "alpha");
}

#[test]
fn fincas_only_still_navigable_and_connectable() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["homebox"]);
    assert_eq!(app.selected_finca().unwrap().slug, "homebox");
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::ConnectFinca {
            slug: "homebox".into(),
            via: None
        }
    );
    let s = screen(&app, 100, 20);
    assert!(s.contains("No projects yet"), "{s}");
}

#[test]
fn finca_detail_shows_remote_sessions_grouped_by_project() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["homebox"]);
    poll(
        &mut app,
        "homebox",
        Reach::Reachable(remote_status("homebox", 1, 1)),
        t0(),
    );
    let s = screen(&app, 110, 24);
    for needle in [
        "FINCA — homebox",
        "btt",
        "n0",
        "needs you",
        "Permission needed: Bash",
        "w0",
        "working",
    ] {
        assert!(s.contains(needle), "missing {needle:?}\n{s}");
    }
}

#[test]
fn unreachable_finca_keeps_last_known_sessions_marked_stale() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["homebox"]);
    poll(
        &mut app,
        "homebox",
        Reach::Reachable(remote_status("homebox", 1, 0)),
        t0() - Duration::minutes(10),
    );
    poll(
        &mut app,
        "homebox",
        Reach::Unreachable {
            error: "Connection refused".into(),
        },
        t0(),
    );
    let s = screen(&app, 110, 24);
    assert!(s.contains("n0"), "last-known sessions kept\n{s}");
    assert!(s.contains("Connection refused"), "{s}");
    assert!(
        s.contains("12m"),
        "age since last success (render time is t0+2m)\n{s}"
    );
}

#[test]
fn session_keys_on_a_finca_point_to_connecting() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["homebox"]);
    for c in ['n', 'e', 'c', 'x'] {
        assert_eq!(handle_key(&mut app, ch(c)), Action::None, "{c}");
        assert_eq!(app.mode, Mode::Normal, "{c}");
        assert!(
            app.footer.as_deref().is_some_and(|f| f.contains("connect")),
            "{c}: {:?}",
            app.footer
        );
    }
}

#[test]
fn s_on_a_finca_opens_a_shell_there() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["homebox"]);
    assert_eq!(
        handle_key(&mut app, ch('s')),
        Action::FincaShell {
            slug: "homebox".into(),
            via: None
        }
    );
    assert!(screen(&app, 100, 20).contains("s: shell"));
}

#[test]
fn enter_in_sessions_pane_on_a_finca_connects() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["homebox"]);
    app.pane = Pane::Sessions;
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::ConnectFinca {
            slug: "homebox".into(),
            via: None
        }
    );
}

#[test]
fn reload_keeps_finca_selection_and_poll_state() {
    let (_d, s, mut app) = with_fincas(&["alpha"], &[], &["homebox"]);
    handle_key(&mut app, ch('j'));
    poll(
        &mut app,
        "homebox",
        Reach::Reachable(remote_status("homebox", 1, 0)),
        t0(),
    );
    s.add_project(&Project {
        name: "beta".into(),
        path: "/b".into(),
        color: None,
        summary: None,
    })
    .unwrap();
    app.reload(&s);
    assert_eq!(app.selected_finca().unwrap().slug, "homebox");
    assert_eq!(app.needs_you_count(), 1);
}

#[test]
fn poll_for_unknown_finca_is_ignored() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["homebox"]);
    poll(&mut app, "ghost", Reach::NeedsUpdate, t0());
    assert_eq!(app.fincas.len(), 1);
}

#[test]
fn removing_selected_finca_clamps_selection() {
    let (_d, s, mut app) = with_fincas(&["alpha"], &[], &["homebox"]);
    handle_key(&mut app, ch('j'));
    s.remove_finca("homebox").unwrap();
    app.reload(&s);
    assert_eq!(app.selected_project().unwrap().slug, "alpha");
    screen(&app, 80, 10);
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

#[test]
fn ctrl_j_and_ctrl_k_cycle_completions_both_ways() {
    let (_d, base) = dirs(&["grove", "grove-fi", "groves"]);
    let mut app = add_prompt_at(&format!("{base}gr"));
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(app.completions.len(), 3);
    handle_key(&mut app, ctrl('j'));
    assert_eq!(prompt_text(&app), format!("{base}grove/"));
    handle_key(&mut app, ctrl('j'));
    assert_eq!(prompt_text(&app), format!("{base}grove-fi/"));
    handle_key(&mut app, ctrl('k'));
    assert_eq!(prompt_text(&app), format!("{base}grove/"));
    handle_key(&mut app, ctrl('k'));
    assert_eq!(
        prompt_text(&app),
        format!("{base}groves/"),
        "wraps to the last"
    );
    assert_eq!(app.completions.len(), 3, "list stays open while cycling");
}

#[test]
fn ctrl_j_k_without_a_list_change_nothing() {
    let (_d, base) = dirs(&["grove", "grove-fi"]);
    let mut app = add_prompt_at(&format!("{base}gr"));
    handle_key(&mut app, ctrl('j'));
    handle_key(&mut app, ctrl('k'));
    assert_eq!(prompt_text(&app), format!("{base}gr"));
    assert!(app.completions.is_empty());
}

// ---------- creating a project folder ----------

#[test]
fn enter_on_missing_path_asks_to_create() {
    let (_d, base) = dirs(&["existing"]);
    let mut app = add_prompt_at(&format!("{base}brand-new"));
    assert_eq!(handle_key(&mut app, key(KeyCode::Enter)), Action::None);
    assert_eq!(
        app.mode,
        Mode::ConfirmCreate {
            path: format!("{base}brand-new")
        }
    );
    assert_eq!(
        handle_key(&mut app, ch('y')),
        Action::CreateProject {
            path: format!("{base}brand-new")
        }
    );
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn declining_create_returns_to_the_prompt_with_text() {
    let (_d, base) = dirs(&[]);
    let mut app = add_prompt_at(&format!("{base}brand-new"));
    handle_key(&mut app, key(KeyCode::Enter));
    assert_eq!(handle_key(&mut app, ch('n')), Action::None);
    assert_eq!(prompt_text(&app), format!("{base}brand-new"));
}

#[test]
fn enter_on_existing_path_adds_without_asking() {
    let (_d, base) = dirs(&["existing"]);
    let mut app = add_prompt_at(&format!("{base}existing"));
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::AddProject {
            path: format!("{base}existing")
        }
    );
}

#[test]
fn tilde_paths_are_checked_after_expansion() {
    let mut app = add_prompt_at("~");
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::AddProject { path: "~".into() }
    );
}

#[test]
fn create_confirmation_is_rendered() {
    let (_d, base) = dirs(&[]);
    let mut app = add_prompt_at(&format!("{base}brand-new"));
    handle_key(&mut app, key(KeyCode::Enter));
    let s = screen(&app, 140, 20);
    assert!(
        s.contains("Create") && s.contains("brand-new") && s.contains("git"),
        "{s}"
    );
}

// ---------- play / work ----------

/// An app whose config puts projects under /w on the work account.
fn work_and_play(projects: &[(&str, &str)]) -> (tempfile::TempDir, Store, App) {
    let (d, s) = store_with(&[], &[]);
    for (name, path) in projects {
        s.add_project(&Project {
            name: name.to_string(),
            path: path.into(),
            color: None,
            summary: None,
        })
        .unwrap();
    }
    let mut app = App::new("/x".into());
    app.cfg = toml::from_str(
        "claude_config_dir = \"~/.claude-play\"\n\
         [accounts.work]\ndir = \"~/.claude-work\"\nfolder = \"/w\"\n",
    )
    .unwrap();
    app.reload(&s);
    (d, s, app)
}

fn row_of(screen: &str, text: &str) -> usize {
    screen
        .lines()
        .position(|l| l.contains(text))
        .unwrap_or_else(|| panic!("no {text:?} in\n{screen}"))
}

#[test]
fn work_projects_come_after_play_projects() {
    let (_d, _s, mut app) = work_and_play(&[
        ("acme", "/w/acme"),
        ("beta", "/code/beta"),
        ("zed", "/code/zed"),
    ]);
    let order: Vec<_> = app.projects.iter().map(|p| p.slug.as_str()).collect();
    assert_eq!(order, ["beta", "zed", "acme"]);
    // j runs straight from the last play project into the work box.
    handle_key(&mut app, ch('j'));
    handle_key(&mut app, ch('j'));
    assert_eq!(app.selected_project().unwrap().slug, "acme");
}

#[test]
fn play_box_sits_above_the_work_box() {
    let (_d, _s, mut app) = work_and_play(&[("acme", "/w/acme"), ("beta", "/code/beta")]);
    app.proj_idx = 1; // acme
    let out = screen(&app, 100, 20);
    let (play, work) = (row_of(&out, "╭ PLAY "), row_of(&out, "╭ WORK "));
    assert!(play < work, "{out}");
    assert!((play..work).contains(&row_of(&out, "● beta")), "{out}");
    assert!(row_of(&out, "● acme") > work, "{out}");
    assert!(out.contains("SESSIONS — WORK — acme"), "{out}");
}

#[test]
fn left_column_in_equal_thirds_and_right_in_halves() {
    let (_d, s, mut app) = work_and_play(&[("acme", "/w/acme"), ("beta", "/code/beta")]);
    s.add_finca(&Finca {
        name: "homebox".into(),
        ssh: "me@homebox".into(),
        grove_bin: "/g".into(),
        color: None,
        backup: None,
    })
    .unwrap();
    app.reload(&s);
    // 30 rows between the header (with its blank row) and the footer.
    let out = screen(&app, 100, 34);
    assert_eq!(row_of(&out, "╭ PLAY "), 3, "{out}");
    assert_eq!(row_of(&out, "╭ WORK "), 13, "{out}");
    assert_eq!(row_of(&out, "╭ FINCAS "), 23, "{out}");
    assert_eq!(row_of(&out, "TO-DO"), 18, "{out}");
}

#[test]
fn an_empty_work_box_says_where_work_projects_go() {
    let (_d, _s, app) = work_and_play(&[("beta", "/code/beta")]);
    let out = screen(&app, 100, 20);
    assert!(out.contains("No projects in /w"), "{out}");
}

#[test]
fn without_account_folders_there_is_one_projects_box() {
    // A finca: everything runs on the default account.
    let (_d, _s, app) = loaded(&["alpha"], &[]);
    let out = screen(&app, 100, 12);
    assert!(out.contains(" PROJECTS "), "{out}");
    assert!(!out.contains("╭ WORK "), "{out}");
}

#[test]
fn d_asks_then_removes_the_project_on_y() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    assert_eq!(handle_key(&mut app, ch('d')), Action::None);
    assert!(screen(&app, 80, 20).contains("Remove project alpha?"));
    assert_eq!(
        handle_key(&mut app, ch('y')),
        Action::RemoveProject {
            slug: "alpha".into()
        }
    );
    assert_eq!(app.mode, Mode::Normal);
    // Anything but y cancels.
    handle_key(&mut app, ch('d'));
    assert_eq!(handle_key(&mut app, ch('n')), Action::None);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn d_in_the_sessions_pane_does_nothing() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    app.pane = Pane::Sessions;
    assert_eq!(handle_key(&mut app, ch('d')), Action::None);
    assert_eq!(app.mode, Mode::Normal);
}

// ---------- to-do ----------

/// Types `s` as individual key presses.
fn type_keys(app: &mut App, s: &str) -> Vec<Action> {
    s.chars().map(|c| handle_key(app, ch(c))).collect()
}

#[test]
fn tab_cycles_projects_sessions_and_todo() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    for want in [Pane::Sessions, Pane::Todo, Pane::Projects] {
        handle_key(&mut app, key(KeyCode::Tab));
        assert_eq!(app.pane, want);
    }
    handle_key(&mut app, key(KeyCode::BackTab));
    assert_eq!(app.pane, Pane::Todo);
}

#[test]
fn t_jumps_to_the_todo_list_and_h_goes_back() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('t'));
    assert_eq!(app.pane, Pane::Todo);
    handle_key(&mut app, ch('h'));
    assert_eq!(app.pane, Pane::Projects);
}

#[test]
fn todo_keys_save_the_selected_projects_list() {
    let (_d, s, mut app) = loaded(&["alpha", "beta"], &[]);
    handle_key(&mut app, ch('j')); // beta
    app.reload(&s); // as the event loop does after every key
    handle_key(&mut app, ch('t'));
    type_keys(&mut app, "ohi");
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::SaveTodos {
            slug: "beta".into(),
            text: "- [ ] hi\n".into()
        }
    );
}

#[test]
fn dashboard_letters_are_todo_keys_in_the_todo_pane() {
    let (_d, s, mut app) = loaded(
        &["alpha"],
        &[session("1", "alpha", "s1", State::Working, Some("@1"))],
    );
    s.write_todos("alpha", "- [ ] a\n- [ ] b\n").unwrap();
    app.reload(&s);
    handle_key(&mut app, ch('t'));
    // x toggles instead of offering to kill the session.
    assert!(matches!(
        handle_key(&mut app, ch('x')),
        Action::SaveTodos { .. }
    ));
    assert_eq!(app.mode, Mode::Normal);
    // dd deletes an item instead of offering to remove the project.
    type_keys(&mut app, "dd");
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.todos.items.len(), 1);
    // n and a don't open prompts.
    type_keys(&mut app, "n");
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn q_is_text_while_inserting_and_quits_otherwise() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('t'));
    type_keys(&mut app, "o");
    assert_eq!(handle_key(&mut app, ch('q')), Action::None);
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(app.pane, Pane::Todo, "Tab is ignored while inserting");
    handle_key(&mut app, key(KeyCode::Esc));
    assert_eq!(handle_key(&mut app, ch('q')), Action::Quit);
}

#[test]
fn big_e_opens_the_todo_file() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('t'));
    assert_eq!(
        handle_key(&mut app, ch('E')),
        Action::EditTodos {
            slug: "alpha".into()
        }
    );
}

#[test]
fn reload_shows_the_selected_projects_list_but_not_mid_edit() {
    let (_d, s, mut app) = loaded(&["alpha", "beta"], &[]);
    s.write_todos("alpha", "- [ ] alpha thing\n").unwrap();
    s.write_todos("beta", "- [ ] beta thing\n").unwrap();
    app.reload(&s);
    assert_eq!(app.todos.items[0].text, "alpha thing");
    handle_key(&mut app, ch('j'));
    app.reload(&s);
    assert_eq!(app.todos.items[0].text, "beta thing");
    // A reload while typing keeps the typing.
    handle_key(&mut app, ch('t'));
    type_keys(&mut app, "Amore");
    app.reload(&s);
    assert_eq!(app.todos.items[0].text, "beta thingmore");
}

#[test]
fn dd_in_one_project_then_p_in_another_moves_the_item() {
    let (_d, s, mut app) = loaded(&["alpha", "beta"], &[]);
    s.write_todos("alpha", "- [ ] carry me\n").unwrap();
    app.reload(&s);
    handle_key(&mut app, ch('t'));
    type_keys(&mut app, "dd");
    handle_key(&mut app, ch('h'));
    handle_key(&mut app, ch('j')); // beta
    app.reload(&s);
    handle_key(&mut app, ch('t'));
    assert_eq!(
        handle_key(&mut app, ch('p')),
        Action::SaveTodos {
            slug: "beta".into(),
            text: "- [ ] carry me\n".into()
        }
    );
}

#[test]
fn todo_list_sits_below_the_sessions() {
    let (_d, s, mut app) = loaded(&["alpha"], &[]);
    s.write_todos("alpha", "- [ ] buy milk\n- [x] ship grove\n")
        .unwrap();
    app.reload(&s);
    let out = screen(&app, 100, 20);
    let (sessions, todo) = (row_of(&out, "SESSIONS — "), row_of(&out, "╭ TO-DO "));
    assert!(sessions < todo, "{out}");
    assert!(row_of(&out, "[ ] buy milk") > todo, "{out}");
    assert!(out.contains("[x] ship grove"), "{out}");
}

#[test]
fn todo_pane_shows_insert_mode_and_an_empty_hint() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    assert!(screen(&app, 100, 20).contains("No to-dos — press o to add one"));
    handle_key(&mut app, ch('t'));
    type_keys(&mut app, "omilk");
    let out = screen(&app, 100, 20);
    assert!(out.contains("-- INSERT --"), "{out}");
    assert!(out.contains("[ ] milk█"), "{out}");
}

#[test]
fn no_todo_pane_for_a_finca() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["homebox"]);
    handle_key(&mut app, ch('t'));
    assert_ne!(app.pane, Pane::Todo);
    for _ in 0..3 {
        handle_key(&mut app, key(KeyCode::Tab));
        assert_ne!(app.pane, Pane::Todo);
    }
}

// ---------- rename ----------

#[test]
fn r_on_a_session_prompts_with_its_name_and_enter_renames() {
    let (_d, _s, mut app) = loaded(
        &["alpha"],
        &[session("1", "alpha", "s1", State::Working, Some("@1"))],
    );
    app.pane = Pane::Sessions;
    handle_key(&mut app, ch('r'));
    assert_eq!(prompt_text(&app), "s1");
    assert!(screen(&app, 100, 20).contains("Rename session"));
    handle_key(&mut app, key(KeyCode::Backspace));
    handle_key(&mut app, key(KeyCode::Backspace));
    type_keys(&mut app, "api");
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::RenameSession {
            session_id: "1".into(),
            name: "api".into()
        }
    );
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn renaming_to_the_same_or_no_name_does_nothing() {
    let (_d, _s, mut app) = loaded(
        &["alpha"],
        &[session("1", "alpha", "s1", State::Ended, None)],
    );
    app.pane = Pane::Sessions;
    handle_key(&mut app, ch('r'));
    assert_eq!(handle_key(&mut app, key(KeyCode::Enter)), Action::None);
    handle_key(&mut app, ch('r'));
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    assert_eq!(handle_key(&mut app, key(KeyCode::Enter)), Action::None);
}

#[test]
fn r_in_the_projects_pane_does_nothing() {
    let (_d, _s, mut app) = loaded(
        &["alpha"],
        &[session("1", "alpha", "s1", State::Working, Some("@1"))],
    );
    assert_eq!(handle_key(&mut app, ch('r')), Action::None);
    assert_eq!(app.mode, Mode::Normal);
}

// ---------- help and update ----------

#[test]
fn question_mark_opens_the_key_help_and_any_key_closes_it() {
    let (_d, _s, mut app) = loaded(&["alpha", "beta"], &[]);
    handle_key(&mut app, ch('?'));
    assert_eq!(app.mode, Mode::Help);
    let out = screen(&app, 120, 50);
    for k in ["C-g", "rename", "dd", "shell", "update Grove"] {
        assert!(out.contains(k), "help lacks {k:?}\n{out}");
    }
    // The closing key does nothing else.
    assert_eq!(handle_key(&mut app, ch('j')), Action::None);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.proj_idx, 0);
}

#[test]
fn question_mark_is_help_in_the_todo_list_but_text_while_typing() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('t'));
    handle_key(&mut app, ch('?'));
    assert_eq!(app.mode, Mode::Help);
    handle_key(&mut app, ch('x'));
    type_keys(&mut app, "o?");
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.todos.items[0].text, "?");
}

#[test]
fn footer_mentions_help() {
    let (_d, _s, app) = loaded(&["alpha"], &[]);
    assert!(screen(&app, 160, 12).contains("? help"));
}

#[test]
fn footer_mentions_the_project_shell() {
    let (_d, _s, app) = loaded(&["alpha"], &[]);
    assert!(screen(&app, 160, 12).contains("s shell"));
}

#[test]
fn help_lists_prompt_and_editing_keys() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('?'));
    let out = screen(&app, 140, 50);
    for k in [
        "Prompts",
        "complete a path or SSH host",
        "pick a match",
        "clear the line",
        "back to projects",
        "prefix r",
        "send the clipboard to a finca",
    ] {
        assert!(out.contains(k), "help lacks {k:?}\n{out}");
    }
}

#[test]
fn help_colors_sections_keys_and_notes() {
    use ratatui::style::Color;
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('?'));
    // Section headings take their pane's color.
    assert_eq!(fg_of(&app, 140, 50, "Fincas"), super::render::FINCAS_COLOR);
    assert_eq!(fg_of(&app, 140, 50, "Prompts"), Color::White);
    // Keys stand out from what they do; asides in parentheses are dimmed.
    assert_eq!(fg_of(&app, 140, 50, "C-g"), super::render::KEY_COLOR);
    assert_eq!(fg_of(&app, 140, 50, "back to the dashboard"), Color::Reset);
    assert_eq!(fg_of(&app, 140, 50, "(folder kept)"), Color::DarkGray);
}

#[test]
fn big_r_updates_grove() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    assert_eq!(handle_key(&mut app, ch('R')), Action::Update);
}

#[test]
fn help_fits_a_laptop_sized_window() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('?'));
    let out = screen(&app, 110, 32);
    for k in [
        "C-g",
        "open a shell there",
        "shell in its folder",
        "open in your editor",
        "rename",
    ] {
        assert!(out.contains(k), "help lacks {k:?} at 110x32\n{out}");
    }
}

#[test]
fn every_help_line_shows_in_full_at_laptop_size() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('?'));
    let out = screen(&app, 110, 32);
    for (_, keys) in super::help::SECTIONS {
        for (k, what) in *keys {
            assert!(out.contains(what), "{k}: {what:?} is cut off\n{out}");
        }
    }
}

// ---------- title colors ----------

/// Foreground color of the first cell of `text` on screen.
fn fg_of(app: &App, w: u16, h: u16, text: &str) -> ratatui::style::Color {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| render(f, app, t0() + Duration::minutes(2)))
        .unwrap();
    let buf = term.backend().buffer().clone();
    let chars: Vec<char> = text.chars().collect();
    for y in 0..h {
        let row: Vec<String> = (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect();
        for x in 0..row.len().saturating_sub(chars.len()) {
            if chars
                .iter()
                .enumerate()
                .all(|(i, c)| row[x + i] == c.to_string())
            {
                return buf[(x as u16, y)].fg;
            }
        }
    }
    panic!("{text:?} not on screen");
}

#[test]
fn each_pane_title_has_its_own_color_whatever_has_focus() {
    use ratatui::style::Color::Rgb;
    let (_d, _s, mut app) = work_and_play(&[("acme", "/w/acme"), ("beta", "/code/beta")]);
    app.cfg.account_color = Some("#7aa2f7".into());
    app.cfg.accounts.get_mut("work").unwrap().color = Some("#e8a33d".into());
    app.proj_idx = 1; // acme, on work
    for pane in [Pane::Projects, Pane::Sessions, Pane::Todo] {
        app.pane = pane;
        assert_eq!(
            fg_of(&app, 100, 24, "PLAY "),
            Rgb(0x7a, 0xa2, 0xf7),
            "{pane:?}"
        );
        assert_eq!(
            fg_of(&app, 100, 24, "WORK "),
            Rgb(0xe8, 0xa3, 0x3d),
            "{pane:?}"
        );
        assert_eq!(
            fg_of(&app, 100, 24, "SESSIONS"),
            Rgb(0xbb, 0x9a, 0xf7),
            "{pane:?}"
        );
        assert_eq!(
            fg_of(&app, 100, 24, "WORK —"),
            Rgb(0xe8, 0xa3, 0x3d),
            "{pane:?}"
        );
        assert_eq!(
            fg_of(&app, 100, 24, "TO-DO"),
            Rgb(0x7d, 0xcf, 0xff),
            "{pane:?}"
        );
        // Uncolored title text stays neutral: only the border shows focus.
        assert_eq!(
            fg_of(&app, 100, 24, "— acme"),
            ratatui::style::Color::Gray,
            "{pane:?}"
        );
    }
}

#[test]
fn fincas_title_is_pink() {
    let (_d, _s, app) = with_fincas(&["alpha"], &[], &["homebox"]);
    assert_eq!(
        fg_of(&app, 100, 20, "FINCAS"),
        ratatui::style::Color::Rgb(0xf7, 0x76, 0x8e)
    );
}

#[test]
fn j_and_k_cycle_an_open_completion_list() {
    let (_d, base) = dirs(&["grove", "grove-fi", "groves"]);
    let mut app = add_prompt_at(&format!("{base}gr"));
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(app.completions.len(), 3);
    handle_key(&mut app, ch('j'));
    assert_eq!(prompt_text(&app), format!("{base}grove/"));
    handle_key(&mut app, ch('j'));
    assert_eq!(prompt_text(&app), format!("{base}grove-fi/"));
    handle_key(&mut app, ch('k'));
    assert_eq!(prompt_text(&app), format!("{base}grove/"));
    assert_eq!(app.completions.len(), 3, "list stays open while cycling");
}

#[test]
fn j_and_k_are_letters_when_no_list_is_open() {
    let (_d, base) = dirs(&[]);
    let mut app = add_prompt_at(&base);
    type_keys(&mut app, "jk");
    assert_eq!(prompt_text(&app), format!("{base}jk"));
}

// ---------- open to-do counts ----------

#[test]
fn project_rows_show_their_open_todo_count() {
    let (_d, s, mut app) = loaded(&["alpha", "beta"], &[]);
    s.write_todos("alpha", "- [ ] a\n- [x] done\n- [ ] b\n")
        .unwrap();
    s.write_todos("beta", "- [x] all done\n").unwrap();
    app.reload(&s);
    let out = screen(&app, 100, 20);
    assert!(out.contains("● alpha  ☐2"), "{out}");
    assert!(!out.contains("☐0") && out.contains("● beta "), "{out}");
}

#[test]
fn todo_count_sits_after_live_sessions_and_needs_you() {
    let (_d, s, mut app) = loaded(
        &["alpha"],
        &[session("1", "alpha", "s1", State::NeedsYou, Some("@1"))],
    );
    s.write_todos("alpha", "- [ ] a\n").unwrap();
    app.reload(&s);
    let out = screen(&app, 100, 20);
    let row = project_row(&out, "alpha");
    assert!(row.contains("● alpha  ☐1") && row.contains("⚑1"), "{out}");
}

/// The projects box's part of the row listing project `name`, up to and
/// including its right border.
fn project_row(out: &str, name: &str) -> String {
    let row = out
        .lines()
        .find(|l| l.contains(&format!("● {name}")))
        .unwrap();
    let end = row.find("││").unwrap();
    row[..end + "│".len()].to_string()
}

/// Character column of the last `pat` in project `name`'s row.
fn col_of(out: &str, name: &str, pat: char) -> usize {
    let chars: Vec<char> = project_row(out, name).chars().collect();
    chars.iter().rposition(|c| *c == pat).unwrap()
}

#[test]
fn project_row_counts_live_sessions_by_state() {
    let (_d, _s, app) = loaded(
        &["alpha"],
        &[
            session("1", "alpha", "s1", State::Working, Some("@1")),
            session("2", "alpha", "s2", State::YourTurn, Some("@2")),
            session("3", "alpha", "s3", State::NeedsYou, Some("@3")),
            session("4", "alpha", "s4", State::Starting, None),
            session("5", "alpha", "s5", State::Ended, None),
        ],
    );
    let out = screen(&app, 100, 20);
    // Most urgent first; starting counts as working; ended isn't counted.
    assert!(out.contains("⚑1  ●1  ●2"), "{out}");
    use ratatui::style::Color;
    assert_eq!(fg_of(&app, 100, 20, "⚑1"), Color::Red);
    assert_eq!(fg_of(&app, 100, 20, "●1"), Color::Yellow);
    assert_eq!(fg_of(&app, 100, 20, "●2"), Color::Green);
}

#[test]
fn project_counts_line_up_at_the_right_edge() {
    let (_d, _s, app) = loaded(
        &["alpha", "longer-name"],
        &[
            session("1", "alpha", "s1", State::Working, Some("@1")),
            session("2", "longer-name", "s2", State::NeedsYou, Some("@2")),
            session("3", "longer-name", "s3", State::Working, Some("@3")),
        ],
    );
    let out = screen(&app, 100, 20);
    // The working slot is the same column in both rows, however long the
    // name, and sits just inside the box's right border.
    let a = col_of(&out, "alpha", '●');
    assert_eq!(a, col_of(&out, "longer-name", '●'), "{out}");
    let row: Vec<char> = out
        .lines()
        .find(|l| l.contains("alpha"))
        .unwrap()
        .chars()
        .collect();
    assert_eq!(&row[a..a + 5].iter().collect::<String>(), "●1  │", "{out}");
    // A project without live sessions shows no slots.
    assert!(!out.contains("⚑0") && !out.contains("●0"), "{out}");
}

#[test]
fn a_long_name_is_cut_short_to_keep_its_counts() {
    let long = "a-project-name-far-too-long-for-the-narrow-box";
    let (_d, _s, app) = loaded(
        &[long],
        &[session("1", long, "s1", State::Working, Some("@1"))],
    );
    let out = screen(&app, 60, 20);
    assert!(out.contains("…") && out.contains("●1"), "{out}");
}

#[test]
fn project_row_shows_only_states_it_has() {
    let (_d, _s, app) = loaded(
        &["alpha"],
        &[session("1", "alpha", "s1", State::Working, Some("@1"))],
    );
    let out = screen(&app, 100, 20);
    assert!(out.contains("●1"), "{out}");
    assert!(!out.contains("⚑"), "{out}");
}

#[test]
fn s_opens_a_shell_in_the_selected_project() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    let want = Action::ProjectShell {
        slug: "alpha".into(),
    };
    assert_eq!(handle_key(&mut app, ch('s')), want);
    app.pane = Pane::Sessions;
    assert_eq!(handle_key(&mut app, ch('s')), want);
}

#[test]
fn s_on_a_broken_project_says_why() {
    let (_d, s) = store_with(&[], &[]);
    s.paths.ensure().unwrap();
    std::fs::write(s.paths.projects.join("broken.toml"), "name = [").unwrap();
    let mut app = App::new("/x".into());
    app.reload(&s);
    assert_eq!(handle_key(&mut app, ch('s')), Action::None);
    assert!(app.footer.is_some());
}

// ---------- moving between sessions and to-do ----------

#[test]
fn j_off_the_last_session_moves_into_the_todo_list() {
    let (_d, _s, mut app) = loaded(
        &["alpha"],
        &[
            session("1", "alpha", "s1", State::Working, Some("@1")),
            session("2", "alpha", "s2", State::Working, Some("@2")),
        ],
    );
    app.pane = Pane::Sessions;
    handle_key(&mut app, ch('j'));
    assert_eq!((app.pane, app.sess_idx), (Pane::Sessions, 1));
    handle_key(&mut app, key(KeyCode::Down));
    assert_eq!(app.pane, Pane::Todo);
}

#[test]
fn j_with_no_sessions_goes_straight_to_the_todo_list() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    app.pane = Pane::Sessions;
    handle_key(&mut app, ch('j'));
    assert_eq!(app.pane, Pane::Todo);
}

#[test]
fn k_off_the_top_of_the_todo_list_goes_back_to_the_same_session() {
    let (_d, s, mut app) = loaded(
        &["alpha"],
        &[
            session("1", "alpha", "s1", State::Working, Some("@1")),
            session("2", "alpha", "s2", State::Working, Some("@2")),
        ],
    );
    s.write_todos("alpha", "- [ ] a\n- [ ] b\n").unwrap();
    app.reload(&s);
    app.pane = Pane::Sessions;
    type_keys(&mut app, "jj"); // s2, then into To-do
    assert_eq!(app.pane, Pane::Todo);
    handle_key(&mut app, ch('j'));
    assert_eq!(app.todos.idx, 1, "j moves within the list");
    handle_key(&mut app, ch('k'));
    assert_eq!(app.pane, Pane::Todo);
    handle_key(&mut app, key(KeyCode::Up));
    assert_eq!((app.pane, app.sess_idx), (Pane::Sessions, 1));
}

#[test]
fn k_in_an_empty_todo_list_goes_back_to_sessions() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('t'));
    handle_key(&mut app, ch('k'));
    assert_eq!(app.pane, Pane::Sessions);
}

#[test]
fn j_and_k_stay_letters_while_typing_a_todo() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('t'));
    type_keys(&mut app, "okj");
    assert_eq!(app.pane, Pane::Todo);
    assert_eq!(app.todos.items[0].text, "kj");
}

// ---------- ordering by live sessions ----------

fn slugs(app: &App) -> Vec<&str> {
    app.projects.iter().map(|p| p.slug.as_str()).collect()
}

#[test]
fn projects_with_more_live_sessions_come_first() {
    let (_d, _s, app) = loaded(
        &["alpha", "beta", "delta", "gamma"],
        &[
            session("1", "beta", "s1", State::Working, Some("@1")),
            session("2", "beta", "s2", State::YourTurn, Some("@2")),
            session("3", "gamma", "s1", State::NeedsYou, Some("@3")),
            // Ended sessions don't count.
            session("4", "delta", "s1", State::Ended, None),
            session("5", "delta", "s2", State::Ended, None),
        ],
    );
    // Ties (alpha, delta: none live) stay alphabetical.
    assert_eq!(slugs(&app), ["beta", "gamma", "alpha", "delta"]);
}

#[test]
fn each_account_box_is_ordered_on_its_own() {
    let (_d, s, mut app) = work_and_play(&[
        ("acme", "/w/acme"),
        ("beta", "/code/beta"),
        ("zed", "/code/zed"),
    ]);
    for (id, p) in [("1", "acme"), ("2", "acme"), ("3", "zed")] {
        s.save_session(&session(id, p, "s", State::Working, Some("@1")))
            .unwrap();
    }
    app.reload(&s);
    // zed (1 live) above beta in PLAY; acme (2 live) still in WORK below.
    assert_eq!(slugs(&app), ["zed", "beta", "acme"]);
}

#[test]
fn selection_follows_the_project_when_the_order_changes() {
    let (_d, s, mut app) = loaded(&["alpha", "beta"], &[]);
    assert_eq!(app.selected_project().unwrap().slug, "alpha");
    s.save_session(&session("1", "beta", "s1", State::Working, Some("@1")))
        .unwrap();
    app.reload(&s);
    assert_eq!(slugs(&app), ["beta", "alpha"]);
    assert_eq!(app.selected_project().unwrap().slug, "alpha");
}

#[test]
fn long_session_names_keep_a_gap_before_the_state() {
    let (_d, _s, app) = loaded(
        &["alpha"],
        &[
            session("1", "alpha", "Minecraft Server", State::Working, Some("@1")),
            session("2", "alpha", "short", State::YourTurn, Some("@2")),
        ],
    );
    let s = screen(&app, 100, 20);
    assert!(s.contains("Minecraft Server  working"), "{s}");
    let col = |needle: &str| {
        s.lines()
            .find_map(|l| l.find(needle).map(|i| l[..i].chars().count()))
            .unwrap_or_else(|| panic!("missing {needle:?}\n{s}"))
    };
    assert_eq!(col("working"), col("your turn"), "states line up\n{s}");
}

#[test]
fn long_finca_session_names_keep_a_gap_before_the_state() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["homebox"]);
    let mut st = remote_status("homebox", 0, 1);
    st.projects[0].sessions[0].name = "Minecraft Server".into();
    poll(&mut app, "homebox", Reach::Reachable(st), t0());
    let s = screen(&app, 110, 24);
    assert!(s.contains("Minecraft Server  working"), "{s}");
}

// ---------- finca sessions and shells ----------

use crate::finca::ShellWindow;

/// homebox with n0 (needs you, @10) and w0 (working, @20), and one shell.
fn finca_with_items() -> (tempfile::TempDir, Store, App) {
    let (d, s, mut app) = with_fincas(&[], &[], &["homebox"]);
    poll(
        &mut app,
        "homebox",
        Reach::Reachable(remote_status("homebox", 1, 1)),
        t0(),
    );
    app.finca_shells.insert(
        "homebox".into(),
        vec![ShellWindow {
            id: "@5".into(),
            index: 1,
            title: Some("me@homebox: ~/stacks".into()),
        }],
    );
    app.pane = Pane::Sessions;
    (d, s, app)
}

#[test]
fn enter_on_a_finca_session_jumps_straight_into_it() {
    let (_d, _s, mut app) = finca_with_items();
    handle_key(&mut app, ch('j'));
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::FincaJump {
            slug: "homebox".into(),
            via: Some("me@homebox".into()),
            window: "@20".into(),
        }
    );
}

#[test]
fn enter_on_a_finca_shell_goes_back_to_that_window() {
    let (_d, _s, mut app) = finca_with_items();
    for _ in 0..5 {
        handle_key(&mut app, ch('j'));
    }
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::FincaShellWindow {
            slug: "homebox".into(),
            window: "@5".into(),
        },
        "j stops on the last row, the shell"
    );
}

#[test]
fn enter_on_the_finca_row_still_opens_its_dashboard() {
    let (_d, _s, mut app) = finca_with_items();
    app.pane = Pane::Projects;
    assert!(matches!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::ConnectFinca { .. }
    ));
}

#[test]
fn a_session_from_an_older_finca_grove_opens_its_dashboard() {
    let (_d, _s, mut app) = finca_with_items();
    let mut st = remote_status("homebox", 1, 0);
    st.projects[0].sessions[0].window = None;
    poll(&mut app, "homebox", Reach::Reachable(st), t0());
    assert!(matches!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::ConnectFinca { .. }
    ));
}

#[test]
fn finca_selection_survives_a_reload() {
    let (_d, s, mut app) = finca_with_items();
    handle_key(&mut app, ch('j'));
    handle_key(&mut app, ch('j'));
    app.reload(&s);
    assert_eq!(app.sess_idx, 2, "still on the shell");
}

#[test]
fn finca_pane_lists_sessions_then_shells() {
    let (_d, _s, app) = finca_with_items();
    let out = screen(&app, 110, 24);
    let (n0, w0, shells, shell) = (
        row_of(&out, "n0"),
        row_of(&out, "w0"),
        row_of(&out, "Shells"),
        row_of(&out, "shell 1"),
    );
    assert!(n0 < w0 && w0 < shells && shells < shell, "{out}");
    assert!(out.contains("me@homebox: ~/stacks"), "{out}");
}

use super::complete::complete_ssh_host;

#[test]
fn big_f_prompts_for_a_destination_and_enter_sets_it_up() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('F'));
    assert!(matches!(
        app.mode,
        Mode::Prompt {
            kind: PromptKind::AddFinca,
            ..
        }
    ));
    for c in "me@newbox".chars() {
        handle_key(&mut app, ch(c));
    }
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::SetupFinca {
            dest: "me@newbox".into()
        }
    );
}

#[test]
fn a_on_a_finca_adds_a_finca() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["homebox"]);
    handle_key(&mut app, ch('a'));
    assert!(matches!(
        app.mode,
        Mode::Prompt {
            kind: PromptKind::AddFinca,
            ..
        }
    ));
}

#[test]
fn ssh_completion_keeps_the_user_part() {
    let hosts = vec![
        "homebox".to_string(),
        "homebox.local".to_string(),
        "pi".to_string(),
    ];
    let c = complete_ssh_host("me@p", &hosts);
    assert_eq!(c.input, "me@pi");
    let c = complete_ssh_host("me@home", &hosts);
    assert_eq!(c.input, "me@homebox");
    assert_eq!(c.candidates, ["me@homebox", "me@homebox.local"]);
    assert_eq!(complete_ssh_host("zz", &hosts).input, "zz");
}

#[test]
fn tab_in_the_finca_prompt_completes_from_ssh_config() {
    let (d, _s, mut app) = loaded(&["alpha"], &[]);
    let ssh = d.path().join("ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::write(ssh.join("config"), "Host homebox\n").unwrap();
    app.ssh_dir = ssh;
    handle_key(&mut app, ch('F'));
    handle_key(&mut app, ch('h'));
    handle_key(&mut app, key(KeyCode::Tab));
    assert!(matches!(&app.mode, Mode::Prompt { input, .. } if input == "homebox"));
}

// ---------- header ----------

#[test]
fn header_shows_the_logo_and_session_totals() {
    let (_d, _s, mut app) = loaded(
        &["alpha", "beta"],
        &[
            session("1", "alpha", "s1", State::NeedsYou, Some("@1")),
            session("2", "beta", "s2", State::YourTurn, Some("@2")),
            session("3", "beta", "s3", State::YourTurn, Some("@3")),
            session("4", "beta", "s4", State::Starting, None),
            session("5", "beta", "s5", State::Ended, None),
        ],
    );
    app.host = "testbox".into();
    let out = screen(&app, 110, 30);
    let top: Vec<&str> = out.lines().take(3).collect();
    assert!(top[0].contains("█▀▀ █▀█ █▀█ █ █ █▀▀"), "{out}");
    assert!(top[1].contains("█▄█ █▀▄ █▄█ ▀▄▀ ██▄"), "{out}");
    // The olive branch's stem runs off the right of the wordmark, and the
    // two-row header has a blank row under it.
    assert!(
        top[1].find("▀▀▀▀▀").unwrap() > top[1].find("██▄").unwrap(),
        "{out}"
    );
    assert!(top[2].trim().is_empty(), "{out}");
    // Two olives on the branch: one among the leaves, one at the tip.
    let logo: String = top[0].chars().take(40).collect();
    assert_eq!(logo.matches('●').count(), 2, "{out}");
    assert!(out.lines().nth(3).unwrap().contains("╭ "), "{out}");
    assert!(
        top[0].contains("⚑1 needs you  ●2 your turn  ●1 working"),
        "{out}"
    );
    assert!(top[1].contains("testbox"), "{out}");
    use ratatui::style::Color;
    assert_eq!(fg_of(&app, 110, 30, "⚑1 needs"), Color::Red);
    assert_eq!(fg_of(&app, 110, 30, "●2 your"), Color::Yellow);
    assert_eq!(fg_of(&app, 110, 30, "●1 working"), Color::Green);
}

#[test]
fn header_counts_fincas_that_are_up() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["homebox", "pi"]);
    app.host = "testbox".into();
    poll(
        &mut app,
        "homebox",
        Reach::Reachable(remote_status("homebox", 0, 0)),
        t0(),
    );
    poll(
        &mut app,
        "pi",
        Reach::Unreachable {
            error: "no route".into(),
        },
        t0(),
    );
    let out = screen(&app, 110, 30);
    let second = out.lines().nth(1).unwrap();
    assert!(second.contains("testbox · fincas 1/2 up"), "{out}");
}

#[test]
fn header_is_dropped_in_a_short_window() {
    let (_d, _s, app) = loaded(&["alpha"], &[]);
    let out = screen(&app, 110, 18);
    assert!(!out.contains("█▀▀▀"), "{out}");
    assert!(out.lines().next().unwrap().contains("alpha"), "{out}");
}

fn screens_doc(sessions: &[(&str, &str, &str, State)]) -> crate::screens::Screens {
    crate::screens::Screens {
        version: 1,
        host: "pi".into(),
        sessions: sessions
            .iter()
            .map(|(id, project, name, state)| crate::screens::ScreenSession {
                id: id.to_string(),
                project: project.to_string(),
                name: name.to_string(),
                state: *state,
                updated_at: t0(),
                window: Some("@1".into()),
                screen: format!("screen of {name}"),
            })
            .collect(),
    }
}

fn screens_at(app: &mut App, slug: &str, at: chrono::DateTime<Utc>, doc: crate::screens::Screens) {
    app.apply_screens(crate::screens::FetchResult {
        slug: slug.into(),
        at,
        fetched: crate::screens::Fetched::Ok(doc),
    });
}

#[test]
fn tiles_list_local_then_finca_sessions() {
    let (_d, _s, mut app) = with_fincas(
        &["alpha", "beta"],
        &[
            session("b1", "beta", "s1", State::Working, Some("@2")),
            session("a1", "alpha", "s1", State::YourTurn, Some("@1")),
            session("a2", "alpha", "old", State::Ended, None),
        ],
        &["pi"],
    );
    poll(
        &mut app,
        "pi",
        Reach::Reachable(remote_status("pi", 0, 0)),
        t0(),
    );
    app.local_screens.insert("a1".into(), "alpha screen".into());
    screens_at(
        &mut app,
        "pi",
        t0(),
        screens_doc(&[("x", "blog", "s2", State::NeedsYou)]),
    );
    let tiles = super::grid::tiles(&app, t0());
    let names: Vec<_> = tiles
        .iter()
        .map(|t| (t.finca.as_ref().map(|f| f.1.clone()), t.handle()))
        .collect();
    assert_eq!(
        names,
        [
            (None, "alpha:s1".to_string()),
            (None, "beta:s1".to_string()),
            (Some("pi".to_string()), "blog:s2".to_string())
        ]
    );
    assert_eq!(tiles[0].screen.as_deref(), Some("alpha screen"));
    assert_eq!(tiles[2].screen.as_deref(), Some("screen of s2"));
    assert_eq!(tiles[2].session_id.as_deref(), Some("x"));
    assert_eq!(tiles[2].via.as_deref(), Some("me@pi"));
    assert!(!tiles[2].stale);
}

#[test]
fn finca_tiles_go_stale_and_fall_back_to_status() {
    let (_d, _s, mut app) = with_fincas(&["alpha"], &[], &["pi"]);
    screens_at(
        &mut app,
        "pi",
        t0(),
        screens_doc(&[("x", "blog", "s2", State::YourTurn)]),
    );
    let later = t0() + Duration::seconds(super::grid::STALE_SECS + 1);
    assert!(super::grid::tiles(&app, later)[0].stale);
    // A Grove too old for screens: tiles from the status poll, no screen.
    let (_d2, _s2, mut app2) = with_fincas(&["alpha"], &[], &["pi"]);
    poll(
        &mut app2,
        "pi",
        Reach::Reachable(remote_status("pi", 0, 1)),
        t0(),
    );
    app2.apply_screens(crate::screens::FetchResult {
        slug: "pi".into(),
        at: t0(),
        fetched: crate::screens::Fetched::TooOld,
    });
    let t = super::grid::tiles(&app2, t0());
    assert_eq!(t.len(), 1, "{t:?}");
    assert_eq!(t[0].screen, None);
    assert_eq!(t[0].session_id, None);
    assert!(app2.too_old.contains("pi"));
}

fn grid_app() -> (tempfile::TempDir, Store, App) {
    // Oldest first: w1, y1, n1.
    let aged = |mut s: Session, mins: i64| {
        s.created_at = t0() + Duration::minutes(mins);
        s
    };
    let (d, s, mut app) = loaded(
        &["alpha"],
        &[
            aged(session("w1", "alpha", "w1", State::Working, Some("@1")), 0),
            aged(session("y1", "alpha", "y1", State::YourTurn, Some("@2")), 1),
            aged(session("n1", "alpha", "n1", State::NeedsYou, Some("@3")), 2),
        ],
    );
    app.grid_cols.set(2);
    handle_key(&mut app, ch('v'));
    (d, s, app)
}

fn grid_sel(app: &App) -> usize {
    match &app.mode {
        Mode::Grid { sel, .. } => *sel,
        m => panic!("not in the grid: {m:?}"),
    }
}

#[test]
fn v_opens_the_grid_and_esc_closes_it() {
    let (_d, _s, mut app) = grid_app();
    assert_eq!(grid_sel(&app), 0);
    handle_key(&mut app, key(KeyCode::Esc));
    assert_eq!(app.mode, Mode::Normal);
    handle_key(&mut app, ch('v'));
    handle_key(&mut app, ch('v'));
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn hjkl_and_tab_move_in_the_grid() {
    let (_d, _s, mut app) = grid_app();
    // [w1 y1] [n1]
    handle_key(&mut app, ch('l'));
    assert_eq!(grid_sel(&app), 1);
    handle_key(&mut app, ch('j'));
    assert_eq!(grid_sel(&app), 2);
    handle_key(&mut app, ch('k'));
    assert_eq!(grid_sel(&app), 0);
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(grid_sel(&app), 1);
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(grid_sel(&app), 2);
    handle_key(&mut app, key(KeyCode::BackTab));
    assert_eq!(grid_sel(&app), 1);
}

#[test]
fn enter_jumps_to_the_tiles_window() {
    let (_d, _s, mut app) = grid_app();
    handle_key(&mut app, ch('l'));
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::Jump {
            window: "@2".into()
        }
    );
}

#[test]
fn i_replies_only_on_your_turn() {
    let (_d, _s, mut app) = grid_app();
    assert_eq!(handle_key(&mut app, ch('i')), Action::None);
    assert!(
        app.footer.as_deref().unwrap().contains("is working"),
        "{:?}",
        app.footer
    );
    handle_key(&mut app, key(KeyCode::Tab)); // y1
    handle_key(&mut app, ch('i'));
    for c in "ok go".chars() {
        handle_key(&mut app, ch(c));
    }
    let a = handle_key(&mut app, key(KeyCode::Enter));
    assert_eq!(
        a,
        Action::GridReply {
            finca: None,
            session_id: "y1".into(),
            handle: "alpha:y1".into(),
            text: "ok go".into()
        }
    );
    // Kept until the send succeeds.
    assert!(
        matches!(&app.mode, Mode::Grid { reply: Some(r), .. } if r.text == "ok go" && r.session_id == "y1")
    );
    handle_key(&mut app, key(KeyCode::Esc));
    assert!(matches!(&app.mode, Mode::Grid { reply: None, .. }));
    handle_key(&mut app, key(KeyCode::Tab)); // n1
    handle_key(&mut app, ch('i'));
    assert!(
        app.footer.as_deref().unwrap().contains("needs you"),
        "{:?}",
        app.footer
    );
}

#[test]
fn a_stale_finca_tile_refuses_replies() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["pi"]);
    screens_at(
        &mut app,
        "pi",
        Utc::now() - Duration::seconds(60),
        screens_doc(&[("x", "blog", "s2", State::YourTurn)]),
    );
    app.mode = Mode::Grid {
        sel: 0,
        reply: None,
    };
    assert_eq!(handle_key(&mut app, ch('i')), Action::None);
    assert!(
        app.footer.as_deref().unwrap().contains("isn't answering"),
        "{:?}",
        app.footer
    );
    assert!(matches!(&app.mode, Mode::Grid { reply: None, .. }));
}

#[test]
fn the_selection_survives_tiles_going_away() {
    let (_d, _s, mut app) = grid_app();
    app.mode = Mode::Grid {
        sel: 2,
        reply: None,
    };
    app.sessions.retain(|s| s.id == "w1");
    handle_key(&mut app, ch('l'));
    assert_eq!(grid_sel(&app), 0);
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::Jump {
            window: "@1".into()
        }
    );
}

#[test]
fn grid_draws_tiles_with_titles_and_screens() {
    let (_d, _s, mut app) = with_fincas(
        &["alpha"],
        &[
            session("y1", "alpha", "fix", State::YourTurn, Some("@2")),
            session("w1", "alpha", "s1", State::Working, Some("@1")),
        ],
        &["pi"],
    );
    app.local_screens
        .insert("y1".into(), "Done. Tests pass.\nWant me to commit?".into());
    screens_at(
        &mut app,
        "pi",
        t0() + Duration::minutes(2),
        screens_doc(&[("x", "blog", "s2", State::NeedsYou)]),
    );
    handle_key(&mut app, ch('v'));
    let out = screen(&app, 110, 32);
    for want in [
        "SESSIONS · 3 live",
        "alpha:fix ● your turn",
        "alpha:s1 ● working",
        "pi · blog:s2 ⚑ needs you",
        "Want me to commit?",
        "screen of s2",
        "Enter jump · i reply · x kill · Tab next waiting · hjkl move",
    ] {
        assert!(out.contains(want), "missing {want:?}:\n{out}");
    }
    assert_eq!(app.grid_cols.get(), 2);
}

#[test]
fn grid_reply_line_and_scroll_note() {
    let sessions: Vec<_> = (0..10)
        .map(|i| {
            let mut s = session(
                &format!("s{i}"),
                "alpha",
                &format!("s{i}"),
                State::YourTurn,
                Some("@1"),
            );
            s.created_at = t0() + Duration::minutes(i);
            s
        })
        .collect();
    let (_d, _s, mut app) = loaded(&["alpha"], &sessions);
    app.mode = Mode::Grid {
        sel: 9,
        reply: Some(Reply {
            session_id: "s9".into(),
            finca: None,
            handle: "alpha:s9".into(),
            text: "yes please".into(),
        }),
    };
    let out = screen(&app, 80, 24);
    assert!(out.contains("showing"), "{out}");
    assert!(out.contains("reply to alpha:s9 ▸ yes please"), "{out}");
    assert!(
        out.contains("alpha:s9"),
        "selected tile scrolled into view:\n{out}"
    );
}

#[test]
fn grid_tiles_survive_wide_characters_and_tiny_terminals() {
    let (_d, _s, mut app) = loaded(
        &["alpha"],
        &[session("y1", "alpha", "fix", State::YourTurn, Some("@2"))],
    );
    app.local_screens
        .insert("y1".into(), "表示テスト🙂\ttabbed ".repeat(20));
    handle_key(&mut app, ch('v'));
    let out = screen(&app, 60, 20);
    // Every body row of the tile still ends with its right border.
    // (The selected tile's border is the thick one.)
    let side = |c: char| c == '│' || c == '┃';
    let body: Vec<&str> = out.lines().filter(|l| l.starts_with(side)).collect();
    assert!(!body.is_empty(), "{out}");
    assert!(body.iter().all(|l| l.trim_end().ends_with(side)), "{out}");
    let _ = screen(&app, 30, 6); // no panic
}

#[test]
fn an_empty_grid_says_how_to_start() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('v'));
    assert!(screen(&app, 110, 32).contains("No live sessions"));
}

#[test]
fn help_lists_the_grid_keys() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('?'));
    let out = screen(&app, 110, 32);
    assert!(out.contains("Grid"), "{out}");
    assert!(out.contains("reply to a waiting session"), "{out}");
    assert!(out.contains("session grid"), "{out}");
}

/// Opens the reply line on y1 in the grid app and types `text`.
fn reply_on_y1(app: &mut App, text: &str) {
    handle_key(app, key(KeyCode::Tab)); // y1
    handle_key(app, ch('i'));
    for c in text.chars() {
        handle_key(app, ch(c));
    }
}

#[test]
fn a_reply_follows_its_session_when_tiles_shift() {
    let (_d, _s, mut app) = grid_app();
    reply_on_y1(&mut app, "go");
    // w1 ends while typing: y1 moves from index 1 to 0, n1 into 1.
    app.sessions.retain(|s| s.id != "w1");
    let a = handle_key(&mut app, key(KeyCode::Enter));
    assert_eq!(
        a,
        Action::GridReply {
            finca: None,
            session_id: "y1".into(),
            handle: "alpha:y1".into(),
            text: "go".into()
        }
    );
}

#[test]
fn a_reply_to_a_session_that_ended_is_refused() {
    let (_d, _s, mut app) = grid_app();
    reply_on_y1(&mut app, "go");
    app.sessions.retain(|s| s.id != "y1");
    assert_eq!(handle_key(&mut app, key(KeyCode::Enter)), Action::None);
    assert!(
        app.footer.as_deref().unwrap().contains("alpha:y1"),
        "{:?}",
        app.footer
    );
    // The text is kept.
    assert!(matches!(&app.mode, Mode::Grid { reply: Some(r), .. } if r.text == "go"));
}

#[test]
fn a_failed_send_shows_its_error_in_the_reply_line() {
    let (_d, _s, mut app) = grid_app();
    reply_on_y1(&mut app, "go");
    app.footer = Some("couldn't type into alpha:y1: boom".into());
    let out = screen(&app, 110, 32);
    assert!(out.contains("reply to alpha:y1 ▸ go"), "{out}");
    assert!(out.contains("boom"), "{out}");
}

#[test]
fn a_finca_tile_gone_stale_while_typing_refuses_the_send() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["pi"]);
    screens_at(
        &mut app,
        "pi",
        Utc::now(),
        screens_doc(&[("x", "blog", "s2", State::YourTurn)]),
    );
    app.mode = Mode::Grid {
        sel: 0,
        reply: None,
    };
    handle_key(&mut app, ch('i'));
    handle_key(&mut app, ch('y'));
    // No fetch since: the screens are a minute old by Enter.
    let doc = screens_doc(&[("x", "blog", "s2", State::YourTurn)]);
    app.finca_screens
        .insert("pi".into(), (Utc::now() - Duration::seconds(60), doc));
    assert_eq!(handle_key(&mut app, key(KeyCode::Enter)), Action::None);
    assert!(
        app.footer.as_deref().unwrap().contains("isn't answering"),
        "{:?}",
        app.footer
    );
}

#[test]
fn an_unreachable_finca_has_no_tiles() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["pi"]);
    poll(
        &mut app,
        "pi",
        Reach::Reachable(remote_status("pi", 0, 1)),
        t0(),
    );
    assert_eq!(super::grid::tiles(&app, t0()).len(), 1);
    poll(
        &mut app,
        "pi",
        Reach::Unreachable {
            error: "timed out".into(),
        },
        t0(),
    );
    assert!(super::grid::tiles(&app, t0()).is_empty());
}

#[test]
fn x_in_the_grid_kills_after_y_and_stays_in_the_grid() {
    let (_d, _s, mut app) = grid_app();
    handle_key(&mut app, ch('l')); // y1
    assert_eq!(handle_key(&mut app, ch('x')), Action::None);
    assert!(
        matches!(&app.mode, Mode::GridKill { session_id, handle, .. } if session_id == "y1" && handle == "alpha:y1"),
        "{:?}",
        app.mode
    );
    assert_eq!(
        handle_key(&mut app, ch('y')),
        Action::GridKill {
            finca: None,
            session_id: "y1".into(),
            handle: "alpha:y1".into(),
        }
    );
    assert_eq!(grid_sel(&app), 1);
}

#[test]
fn anything_but_y_cancels_a_grid_kill() {
    let (_d, _s, mut app) = grid_app();
    handle_key(&mut app, ch('x'));
    assert_eq!(handle_key(&mut app, ch('n')), Action::None);
    assert_eq!(grid_sel(&app), 0);
    handle_key(&mut app, ch('x'));
    assert_eq!(handle_key(&mut app, key(KeyCode::Esc)), Action::None);
    assert_eq!(grid_sel(&app), 0);
}

#[test]
fn a_grid_kill_targets_its_session_even_if_tiles_move() {
    let (_d, _s, mut app) = grid_app();
    handle_key(&mut app, ch('l')); // y1
    handle_key(&mut app, ch('x'));
    // w1 ends while the popup is up: y1 is now the first tile.
    app.sessions.retain(|s| s.id != "w1");
    assert_eq!(
        handle_key(&mut app, ch('y')),
        Action::GridKill {
            finca: None,
            session_id: "y1".into(),
            handle: "alpha:y1".into(),
        }
    );
    // Gone by then: nothing to kill.
    handle_key(&mut app, ch('x'));
    app.sessions.clear();
    assert_eq!(handle_key(&mut app, ch('y')), Action::None);
    assert!(app.footer.as_deref().unwrap().contains("has ended"));
}

#[test]
fn x_on_a_finca_tile_kills_it_there() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["pi"]);
    screens_at(
        &mut app,
        "pi",
        Utc::now(),
        screens_doc(&[("x9", "blog", "s2", State::Working)]),
    );
    app.mode = Mode::Grid {
        sel: 0,
        reply: None,
    };
    handle_key(&mut app, ch('x'));
    match handle_key(&mut app, ch('y')) {
        Action::GridKill {
            finca: Some((slug, _)),
            session_id,
            handle,
        } => {
            assert_eq!((slug.as_str(), session_id.as_str()), ("pi", "x9"));
            assert_eq!(handle, "blog:s2");
        }
        a => panic!("{a:?}"),
    }
}

#[test]
fn x_refuses_stale_and_old_finca_tiles() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["pi"]);
    screens_at(
        &mut app,
        "pi",
        Utc::now() - Duration::seconds(60),
        screens_doc(&[("x9", "blog", "s2", State::Working)]),
    );
    app.mode = Mode::Grid {
        sel: 0,
        reply: None,
    };
    assert_eq!(handle_key(&mut app, ch('x')), Action::None);
    assert!(app.footer.as_deref().unwrap().contains("isn't answering"));
    assert!(matches!(&app.mode, Mode::Grid { .. }));
}

#[test]
fn the_grid_kill_popup_draws_over_the_grid() {
    let (_d, _s, mut app) = grid_app();
    handle_key(&mut app, ch('x'));
    let out = screen(&app, 110, 32);
    assert!(out.contains("Kill session alpha:w1?"), "{out}");
    assert!(out.contains("SESSIONS"), "{out}");
}

// ---------- pins ----------

fn pin(finca: Option<&str>, id: &str) -> crate::pins::Pin {
    crate::pins::Pin {
        finca: finca.map(str::to_string),
        id: id.into(),
    }
}

#[test]
fn pins_load_on_reload_and_ended_local_ones_are_pruned_and_saved() {
    let (_d, s, mut app) = loaded(
        &["alpha"],
        &[
            session("a1", "alpha", "s1", State::Working, Some("@1")),
            session("a2", "alpha", "s2", State::Ended, None),
        ],
    );
    crate::pins::save(
        &s.paths,
        &[pin(None, "a1"), pin(None, "a2"), pin(None, "gone")],
    )
    .unwrap();
    app.reload(&s);
    assert_eq!(app.pins, vec![pin(None, "a1")]);
    assert_eq!(crate::pins::load(&s.paths), vec![pin(None, "a1")]);
    let rows = app.pinned_rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].handle, "alpha:s1");
    assert_eq!(rows[0].window.as_deref(), Some("@1"));
    assert_eq!(
        rows[0].account.as_ref().map(|a| a.name.as_str()),
        Some("default")
    );
    assert!(rows[0].finca.is_none() && !rows[0].unreachable);
}

#[test]
fn finca_pins_go_only_when_a_good_poll_lacks_them() {
    let (_d, s, mut app) = with_fincas(&[], &[], &["pi"]);
    crate::pins::save(
        &s.paths,
        &[pin(Some("pi"), "pi-n0"), pin(Some("pi"), "pi-w0")],
    )
    .unwrap();
    poll(
        &mut app,
        "pi",
        Reach::Reachable(remote_status("pi", 1, 1)),
        t0(),
    );
    app.reload(&s);
    let rows = app.pinned_rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].handle, "btt:n0");
    assert_eq!(rows[0].state, State::NeedsYou);
    assert_eq!(
        rows[0].finca,
        Some(("pi".into(), "pi".into(), Some("me@pi".into())))
    );
    assert_eq!(rows[0].window.as_deref(), Some("@10"));
    // Unreachable: kept, marked.
    poll(
        &mut app,
        "pi",
        Reach::Unreachable { error: "x".into() },
        t0(),
    );
    app.reload(&s);
    assert_eq!(app.pins.len(), 2);
    assert!(app.pinned_rows().iter().all(|r| r.unreachable));
    // A good poll without w0: it goes, even before the next reload.
    poll(
        &mut app,
        "pi",
        Reach::Reachable(remote_status("pi", 1, 0)),
        t0(),
    );
    assert_eq!(app.pinned_rows().len(), 1);
    app.reload(&s);
    assert_eq!(crate::pins::load(&s.paths), vec![pin(Some("pi"), "pi-n0")]);
}

#[test]
fn pins_of_a_removed_finca_go() {
    let (_d, s, mut app) = with_fincas(&[], &[], &["pi"]);
    crate::pins::save(&s.paths, &[pin(Some("pi"), "pi-n0"), pin(Some("old"), "x")]).unwrap();
    app.reload(&s);
    assert_eq!(app.pins, vec![pin(Some("pi"), "pi-n0")]);
    // Not polled yet: shown as unreachable, with a placeholder handle.
    let rows = app.pinned_rows();
    assert!(rows[0].unreachable);
}

fn pinned_app() -> (tempfile::TempDir, Store, App) {
    // alpha: a1 (working, @1), a2 (your turn, @2); pi: pi-n0, pi-w0.
    let (d, s, mut app) = with_fincas(
        &["alpha"],
        &[
            session("a1", "alpha", "s1", State::Working, Some("@1")),
            session("a2", "alpha", "s2", State::YourTurn, Some("@2")),
        ],
        &["pi"],
    );
    poll(
        &mut app,
        "pi",
        Reach::Reachable(remote_status("pi", 1, 1)),
        t0(),
    );
    app.reload(&s);
    (d, s, app)
}

/// Applies a SavePins action the way perform + reload would.
fn save_pins(s: &Store, app: &mut App, a: Action) {
    match a {
        Action::SavePins { pins } => crate::pins::save(&s.paths, &pins).unwrap(),
        other => panic!("{other:?}"),
    }
    app.reload(s);
}

#[test]
fn p_pins_and_unpins_a_local_session() {
    let (_d, s, mut app) = pinned_app();
    app.pane = Pane::Sessions;
    app.sess_idx = 1; // a2
    let a = handle_key(&mut app, ch('p'));
    assert_eq!(app.footer.as_deref(), Some("Pinned alpha:s2 (1)"));
    save_pins(&s, &mut app, a);
    assert_eq!(app.pins, vec![pin(None, "a2")]);
    let a = handle_key(&mut app, ch('p'));
    assert_eq!(app.footer.as_deref(), Some("Unpinned alpha:s2"));
    save_pins(&s, &mut app, a);
    assert!(app.pins.is_empty());
}

#[test]
fn p_pins_a_finca_session_and_explains_old_ones() {
    let (_d, s, mut app) = pinned_app();
    app.proj_idx = app.projects.len(); // pi
    app.pane = Pane::Sessions;
    app.sess_idx = 0; // pi-n0
    let a = handle_key(&mut app, ch('p'));
    save_pins(&s, &mut app, a);
    assert_eq!(app.pins, vec![pin(Some("pi"), "pi-n0")]);
    // A finca Grove that doesn't send ids.
    let mut st = remote_status("pi", 1, 0);
    st.projects[0].sessions[0].id = None;
    poll(&mut app, "pi", Reach::Reachable(st), t0());
    app.sess_idx = 0;
    assert_eq!(handle_key(&mut app, ch('p')), Action::None);
    assert_eq!(
        app.footer.as_deref(),
        Some("update Grove on pi to pin its sessions")
    );
}

#[test]
fn number_keys_jump_to_pins() {
    let (_d, s, mut app) = pinned_app();
    crate::pins::save(&s.paths, &[pin(None, "a2"), pin(Some("pi"), "pi-w0")]).unwrap();
    app.reload(&s);
    assert_eq!(
        handle_key(&mut app, ch('1')),
        Action::Jump {
            window: "@2".into()
        }
    );
    assert_eq!(
        handle_key(&mut app, ch('2')),
        Action::FincaJump {
            slug: "pi".into(),
            via: Some("me@pi".into()),
            window: "@20".into()
        }
    );
    assert_eq!(handle_key(&mut app, ch('3')), Action::None);
    assert_eq!(app.footer.as_deref(), Some("no pin 3"));
    poll(
        &mut app,
        "pi",
        Reach::Unreachable { error: "x".into() },
        t0(),
    );
    assert_eq!(handle_key(&mut app, ch('2')), Action::None);
    assert_eq!(app.footer.as_deref(), Some("pi isn't answering"));
}

#[test]
fn number_keys_do_nothing_in_the_todo_list() {
    let (_d, s, mut app) = pinned_app();
    crate::pins::save(&s.paths, &[pin(None, "a2")]).unwrap();
    app.reload(&s);
    app.pane = Pane::Todo;
    assert!(!matches!(
        handle_key(&mut app, ch('1')),
        Action::Jump { .. }
    ));
}

#[test]
fn the_pinned_pane_moves_jumps_unpins_and_kills() {
    let (_d, s, mut app) = pinned_app();
    crate::pins::save(&s.paths, &[pin(None, "a1"), pin(Some("pi"), "pi-n0")]).unwrap();
    app.reload(&s);
    // Tab: Projects → Pinned → Sessions.
    app.pane = Pane::Projects;
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(app.pane, Pane::Pinned);
    handle_key(&mut app, ch('j'));
    assert_eq!(app.pin_idx, 1);
    assert!(matches!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::FincaJump { .. }
    ));
    // x on the finca pin: confirm, then a remote kill.
    handle_key(&mut app, ch('x'));
    assert!(matches!(&app.mode, Mode::Confirm { finca: Some(f), .. } if f == "pi"));
    assert_eq!(
        handle_key(&mut app, ch('y')),
        Action::GridKill {
            finca: Some(("pi".into(), Some("me@pi".into()))),
            session_id: "pi-n0".into(),
            handle: "btt:n0".into(),
        }
    );
    // j at the bottom goes down to Sessions; k at its top comes back.
    app.pane = Pane::Pinned;
    handle_key(&mut app, ch('j'));
    assert_eq!(app.pane, Pane::Sessions);
    app.sess_idx = 0;
    handle_key(&mut app, ch('k'));
    assert_eq!(app.pane, Pane::Pinned);
    // p unpins the selected row.
    app.pin_idx = 0;
    let a = handle_key(&mut app, ch('p'));
    save_pins(&s, &mut app, a);
    assert_eq!(app.pins, vec![pin(Some("pi"), "pi-n0")]);
    // x on a local pin: the usual kill.
    app.pane = Pane::Pinned;
    app.pin_idx = 0;
    crate::pins::save(&s.paths, &[pin(None, "a1")]).unwrap();
    app.reload(&s);
    handle_key(&mut app, ch('x'));
    assert_eq!(
        handle_key(&mut app, ch('y')),
        Action::Kill {
            session_id: "a1".into()
        }
    );
}

#[test]
fn tab_skips_pinned_when_there_are_none() {
    let (_d, _s, mut app) = pinned_app();
    app.pane = Pane::Projects;
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(app.pane, Pane::Sessions);
}
