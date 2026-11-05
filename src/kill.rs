//! `grove kill`: stops Grove's tmux server, and with it the dashboard and
//! every session and shell in it. (`q` on the dashboard only detaches.)
//! Conversations stay in Claude's history, so sessions can be resumed.

use anyhow::Result;

use crate::model::{Session, State};
use crate::store::Store;
use crate::tmux::Tmux;

/// The sessions a kill would end: live ones with a window on this server.
pub fn live_sessions(store: &Store) -> Vec<Session> {
    store
        .list_sessions(None)
        .into_iter()
        .filter(|s| s.state != State::Ended && s.tmux_window.is_some())
        .collect()
}

/// Asked before killing, when there's something to lose.
pub fn question(live: &[Session]) -> Option<String> {
    if live.is_empty() {
        return None;
    }
    let busy = live
        .iter()
        .filter(|s| matches!(s.state, State::Working | State::Starting))
        .count();
    let busy = if busy > 0 {
        format!(" ({busy} working)")
    } else {
        String::new()
    };
    Some(format!(
        "This ends {} Claude session(s){busy}. Kill Grove?",
        live.len()
    ))
}

/// Marks `live` ended, then stops the server. Returns false when Grove
/// wasn't running. Marked first: run from a shell inside Grove, this
/// process goes down with the server.
pub fn kill(tmux: &Tmux, store: &Store, live: &[Session]) -> Result<bool> {
    if !tmux.server_running() {
        return Ok(false);
    }
    let now = chrono::Utc::now();
    for s in live {
        store.save_session(&Session {
            state: State::Ended,
            message: None,
            updated_at: now,
            ..s.clone()
        })?;
    }
    tmux.kill_server();
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;

    fn sess(id: &str, state: State, window: Option<&str>) -> Session {
        Session {
            id: id.into(),
            project: "alpha".into(),
            name: id.into(),
            host: "local".into(),
            tmux_window: window.map(str::to_string),
            state,
            message: None,
            conversation: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn asks_only_when_sessions_would_end() {
        assert_eq!(question(&[]), None);
        let live = [
            sess("a", State::Working, Some("@1")),
            sess("b", State::YourTurn, Some("@2")),
        ];
        assert_eq!(
            question(&live).unwrap(),
            "This ends 2 Claude session(s) (1 working). Kill Grove?"
        );
        assert_eq!(
            question(&live[1..]).unwrap(),
            "This ends 1 Claude session(s). Kill Grove?"
        );
    }

    #[test]
    fn stops_the_server_and_ends_its_sessions() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path().join("home"));
        paths.ensure().unwrap();
        let store = Store::new(paths);
        let tmux = Tmux::new(format!("grove-test-{}-kill", std::process::id()));
        // Not running: nothing to do.
        assert!(!kill(&tmux, &store, &[]).unwrap());

        tmux.new_session_detached(
            "grove",
            "dashboard",
            d.path(),
            &["sleep".into(), "60".into()],
        )
        .unwrap();
        for s in [
            sess("live", State::Working, Some("@1")),
            sess("done", State::Ended, Some("@2")),
            sess("away", State::Working, None),
        ] {
            store.save_session(&s).unwrap();
        }
        let live = live_sessions(&store);
        assert_eq!(
            live.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["live"]
        );
        assert!(kill(&tmux, &store, &live).unwrap());
        assert!(!tmux.server_running());
        let state = |id: &str| store.get_session(id).unwrap().state;
        assert_eq!(state("live"), State::Ended);
        assert_eq!(state("away"), State::Working);
    }
}
