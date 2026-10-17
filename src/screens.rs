//! Live session screens: `grove screens --json` on a machine, fetched from
//! fincas by the session grid, and `grove type-prompt` for replying there.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::State;
use crate::store::Store;
use crate::tmux::Tmux;

pub const SCREENS_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Screens {
    pub version: u32,
    pub host: String,
    pub sessions: Vec<ScreenSession>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ScreenSession {
    pub id: String,
    pub project: String,
    pub name: String,
    pub state: State,
    pub updated_at: DateTime<Utc>,
    pub window: Option<String>,
    pub screen: String,
}

/// The last `lines` lines of `screen`, trailing blank lines dropped.
pub fn tail(screen: &str, lines: usize) -> String {
    let all: Vec<&str> = screen.trim_end().lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// This machine's live sessions and the bottom `lines` of each screen.
pub fn local(store: &Store, tmux: &Tmux, lines: usize, host: &str) -> Screens {
    let mut live: Vec<_> = store
        .list_sessions(None)
        .into_iter()
        .filter(|s| s.state != State::Ended)
        .collect();
    live.sort_by(|a, b| (&a.project, a.created_at).cmp(&(&b.project, b.created_at)));
    let sessions = live
        .into_iter()
        .map(|s| {
            // A window marked as another session's is a stale record's.
            let screen = s
                .tmux_window
                .as_deref()
                .filter(|w| {
                    tmux.window_option(w, "@grove_session")
                        .is_none_or(|id| id == s.id)
                })
                .and_then(|w| tmux.capture(w, lines).ok())
                .map(|text| tail(&text, lines))
                .unwrap_or_default();
            ScreenSession {
                id: s.id,
                project: s.project,
                name: s.name,
                state: s.state,
                updated_at: s.updated_at,
                window: s.tmux_window,
                screen,
            }
        })
        .collect();
    Screens {
        version: SCREENS_VERSION,
        host: host.to_string(),
        sessions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use crate::model::Session;

    fn session(id: &str, project: &str, state: State, window: Option<&str>, mins: i64) -> Session {
        let t = chrono::Utc::now() - chrono::Duration::minutes(mins);
        Session {
            id: id.into(),
            project: project.into(),
            name: format!("s-{id}"),
            host: "local".into(),
            tmux_window: window.map(str::to_string),
            state,
            message: None,
            created_at: t,
            updated_at: t,
        }
    }

    #[test]
    fn tail_drops_trailing_blanks_and_keeps_the_last_lines() {
        assert_eq!(tail("a\nb\nc\n\n\n", 2), "b\nc");
        assert_eq!(tail("", 5), "");
        assert_eq!(tail("one", 0), "");
    }

    #[test]
    fn local_lists_live_sessions_with_screens() {
        let d = tempfile::tempdir().unwrap();
        let store = Store::new(Paths::new(d.path().join("home")));
        store.paths.ensure().unwrap();
        let tmux = Tmux::new(format!("grove-test-{}-screens", std::process::id()));
        tmux.new_session_detached(
            "grove",
            "w",
            d.path(),
            &[
                "sh".into(),
                "-c".into(),
                "printf 'hello\\nfrom beta\\n'; sleep 30".into(),
            ],
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let w = tmux.list_windows_as("grove", "#{window_id}").unwrap()[0].clone();
        store
            .save_session(&session("b1", "beta", State::YourTurn, Some(&w), 5))
            .unwrap();
        tmux.set_window_option(&w, "@grove_session", "b1").unwrap();
        store
            .save_session(&session("a1", "alpha", State::Working, None, 9))
            .unwrap();
        store
            .save_session(&session("a2", "alpha", State::Ended, None, 1))
            .unwrap();
        // A stale record pointing at someone else's window shows nothing.
        store
            .save_session(&session("b2", "beta", State::Working, Some(&w), 1))
            .unwrap();
        let out = local(&store, &tmux, 40, "homebox");
        tmux.kill_server();
        let ids: Vec<_> = out.sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["a1", "b1", "b2"]);
        assert_eq!(out.version, SCREENS_VERSION);
        assert_eq!(out.host, "homebox");
        assert!(
            out.sessions[1].screen.ends_with("hello\nfrom beta"),
            "{:?}",
            out.sessions[1].screen
        );
        assert_eq!(out.sessions[0].screen, "");
        assert_eq!(out.sessions[2].screen, "");
    }
}
