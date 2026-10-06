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
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::AddProject {
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
    for c in ['n', 'e', 'w', 'c', 'x'] {
        assert_eq!(handle_key(&mut app, ch(c)), Action::None, "{c}");
        assert_eq!(app.mode, Mode::Normal, "{c}");
    }
    assert_eq!(handle_key(&mut app, key(KeyCode::Enter)), Action::None);
}

#[test]
fn edit_wiki_clear_and_quit() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    assert_eq!(
        handle_key(&mut app, ch('e')),
        Action::EditProject {
            slug: "alpha".into()
        }
    );
    assert_eq!(
        handle_key(&mut app, ch('w')),
        Action::OpenWiki {
            slug: "alpha".into()
        }
    );
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
        "Projects",
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
