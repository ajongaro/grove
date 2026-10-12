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
    assert_eq!(app.needs_you_by_project().get("beta"), Some(&1));
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
            name: format!("n{i}"),
            state: State::NeedsYou,
            message: Some("Permission needed: Bash".into()),
            updated_at: t0(),
        });
    }
    for i in 0..working {
        sessions.push(SessionStatus {
            name: format!("w{i}"),
            state: State::Working,
            message: None,
            updated_at: t0(),
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
    let (_d, _s, mut app) = with_fincas(&[], &[], &["therver"]);
    app.apply_poll(PollResult {
        slug: "therver".into(),
        reach: Reach::Reachable(remote_status("therver", 0, 0)),
        via: Some("me@therver.ts.net".into()),
        at: t0(),
    });
    // A later failed poll keeps the last good destination.
    poll(
        &mut app,
        "therver",
        Reach::Unreachable {
            error: "timed out".into(),
        },
        t0(),
    );
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::ConnectFinca {
            slug: "therver".into(),
            via: Some("me@therver.ts.net".into())
        }
    );
}

#[test]
fn fincas_listed_under_projects_with_counts() {
    let (_d, _s, mut app) = with_fincas(&["alpha"], &[], &["therver", "pi-five-ssd", "old"]);
    poll(
        &mut app,
        "therver",
        Reach::Reachable(remote_status("therver", 2, 1)),
        t0(),
    );
    poll(
        &mut app,
        "pi-five-ssd",
        Reach::Unreachable {
            error: "timed out".into(),
        },
        t0(),
    );
    poll(&mut app, "old", Reach::NeedsUpdate, t0());
    let s = screen(&app, 110, 24);
    for needle in [
        "FINCAS",
        "therver",
        "3 live",
        "⚑2",
        "pi-five-ssd",
        "unreachable",
        "needs update",
    ] {
        assert!(s.contains(needle), "missing {needle:?}\n{s}");
    }
}

#[test]
fn unpolled_finca_shows_checking() {
    let (_d, _s, app) = with_fincas(&[], &[], &["therver"]);
    assert!(screen(&app, 100, 20).contains("checking"));
}

#[test]
fn footer_counts_remote_needs_you() {
    let (_d, _s, mut app) = with_fincas(
        &["alpha"],
        &[session("1", "alpha", "s1", State::NeedsYou, Some("@1"))],
        &["therver"],
    );
    poll(
        &mut app,
        "therver",
        Reach::Reachable(remote_status("therver", 2, 0)),
        t0(),
    );
    assert_eq!(app.needs_you_count(), 3);
    let s = screen(&app, 110, 20);
    assert!(s.contains("3 needs you (2 on therver)"), "{s}");
}

#[test]
fn selection_moves_from_projects_into_fincas() {
    let (_d, _s, mut app) = with_fincas(&["alpha"], &[], &["therver", "pi"]);
    assert_eq!(app.selected_project().unwrap().slug, "alpha");
    handle_key(&mut app, ch('j'));
    assert!(app.selected_project().is_none());
    assert_eq!(app.selected_finca().unwrap().slug, "pi");
    handle_key(&mut app, ch('j'));
    assert_eq!(app.selected_finca().unwrap().slug, "therver");
    handle_key(&mut app, ch('j'));
    assert_eq!(
        app.selected_finca().unwrap().slug,
        "therver",
        "stays at the end"
    );
    handle_key(&mut app, ch('k'));
    handle_key(&mut app, ch('k'));
    assert_eq!(app.selected_project().unwrap().slug, "alpha");
}

#[test]
fn fincas_only_still_navigable_and_connectable() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["therver"]);
    assert_eq!(app.selected_finca().unwrap().slug, "therver");
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::ConnectFinca {
            slug: "therver".into(),
            via: None
        }
    );
    let s = screen(&app, 100, 20);
    assert!(s.contains("No projects yet"), "{s}");
}

#[test]
fn finca_detail_shows_remote_sessions_grouped_by_project() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["therver"]);
    poll(
        &mut app,
        "therver",
        Reach::Reachable(remote_status("therver", 1, 1)),
        t0(),
    );
    let s = screen(&app, 110, 24);
    for needle in [
        "FINCA — therver",
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
    let (_d, _s, mut app) = with_fincas(&[], &[], &["therver"]);
    poll(
        &mut app,
        "therver",
        Reach::Reachable(remote_status("therver", 1, 0)),
        t0() - Duration::minutes(10),
    );
    poll(
        &mut app,
        "therver",
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
    let (_d, _s, mut app) = with_fincas(&[], &[], &["therver"]);
    for c in ['n', 'e', 'c', 'x'] {
        assert_eq!(handle_key(&mut app, ch(c)), Action::None, "{c}");
        assert_eq!(app.mode, Mode::Normal, "{c}");
        assert!(
            app.footer.as_deref().is_some_and(|f| f.contains("connect")),
            "{c}: {:?}",
            app.footer
        );
    }
    handle_key(&mut app, ch('a'));
    assert!(matches!(
        app.mode,
        Mode::Prompt {
            kind: PromptKind::AddProject,
            ..
        }
    ));
}

#[test]
fn s_on_a_finca_opens_a_shell_there() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["therver"]);
    assert_eq!(
        handle_key(&mut app, ch('s')),
        Action::FincaShell {
            slug: "therver".into(),
            via: None
        }
    );
    assert!(screen(&app, 100, 20).contains("s: shell"));
}

#[test]
fn s_on_a_project_does_nothing() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    assert_eq!(handle_key(&mut app, ch('s')), Action::None);
}

#[test]
fn enter_in_sessions_pane_on_a_finca_connects() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["therver"]);
    app.pane = Pane::Sessions;
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::ConnectFinca {
            slug: "therver".into(),
            via: None
        }
    );
}

#[test]
fn reload_keeps_finca_selection_and_poll_state() {
    let (_d, s, mut app) = with_fincas(&["alpha"], &[], &["therver"]);
    handle_key(&mut app, ch('j'));
    poll(
        &mut app,
        "therver",
        Reach::Reachable(remote_status("therver", 1, 0)),
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
    assert_eq!(app.selected_finca().unwrap().slug, "therver");
    assert_eq!(app.needs_you_count(), 1);
}

#[test]
fn poll_for_unknown_finca_is_ignored() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["therver"]);
    poll(&mut app, "ghost", Reach::NeedsUpdate, t0());
    assert_eq!(app.fincas.len(), 1);
}

#[test]
fn removing_selected_finca_clamps_selection() {
    let (_d, s, mut app) = with_fincas(&["alpha"], &[], &["therver"]);
    handle_key(&mut app, ch('j'));
    s.remove_finca("therver").unwrap();
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
    let (_d, _s, mut app) = with_fincas(&[], &[], &["therver"]);
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
fn big_r_updates_grove() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    assert_eq!(handle_key(&mut app, ch('R')), Action::Update);
}

#[test]
fn help_fits_a_laptop_sized_window() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('?'));
    let out = screen(&app, 110, 32);
    for k in ["C-g", "open a shell there", "open in your editor", "rename"] {
        assert!(out.contains(k), "help lacks {k:?} at 110x32\n{out}");
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
    let (_d, _s, app) = with_fincas(&["alpha"], &[], &["therver"]);
    assert_eq!(
        fg_of(&app, 100, 20, "FINCAS"),
        ratatui::style::Color::Rgb(0xf7, 0x76, 0x8e)
    );
}
