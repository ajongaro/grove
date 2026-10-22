//! `grove hook`: called by Claude Code hooks. Must never fail or block.

use crate::log;
use crate::state::{self, HookEvent};
use crate::store::Store;

/// Applies one hook event. Returns text to print on stdout (SessionStart
/// context), if any. Never returns an error: problems are logged.
pub fn run_hook(store: &Store, input: &str) -> Option<String> {
    let ev: HookEvent = match serde_json::from_str(input) {
        Ok(ev) => ev,
        Err(e) => {
            log::error(
                &store.paths,
                "hook",
                &format!("unparseable input ({e}): {:.200}", input),
            );
            return None;
        }
    };
    // Prefer the id Grove put on the window: /clear and /resume start a new
    // conversation id in the same window. Sessions Grove didn't launch aren't
    // ours to track.
    let mut session = std::env::var("GROVE_SESSION")
        .ok()
        .and_then(|id| store.get_session(&id))
        .or_else(|| store.get_session(&ev.session_id))?;

    // Which conversation to resume once the window has gone.
    let follow = session.conversation_id() != ev.session_id;
    if follow {
        session.conversation = Some(ev.session_id.clone());
    }

    if ev.hook_event_name == "SessionStart" {
        if follow {
            save(store, &mut session);
        }
        return Some(session_context(store, &session.project));
    }

    match state::next(session.state, &ev) {
        Some((state, message)) => {
            session.state = state;
            session.message = message;
        }
        None if !follow => return None,
        None => {}
    }
    save(store, &mut session);
    None
}

fn save(store: &Store, session: &mut crate::model::Session) {
    session.updated_at = chrono::Utc::now();
    if let Err(e) = store.save_session(session) {
        log::error(
            &store.paths,
            "hook",
            &format!("saving session {}: {e:#}", session.id),
        );
    }
}

/// A few lines Claude Code adds to the new session's context: the project
/// and its summary.
fn session_context(store: &Store, slug: &str) -> String {
    let (name, path, summary) = match store.get_project(slug) {
        Ok(p) => (p.name, p.path.display().to_string(), p.summary),
        Err(_) => (slug.to_string(), "unknown path".to_string(), None),
    };
    format!(
        "Grove project: {name} ({path})\nSummary: {}\n",
        summary
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "none yet (set it with the grove update_project tool)".into())
    )
}
