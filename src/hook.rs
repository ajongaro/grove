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
            log::error(&store.paths, "hook", &format!("unparseable input ({e}): {:.200}", input));
            return None;
        }
    };
    // Sessions Grove didn't launch (or forks of them) aren't ours to track.
    let mut session = store.get_session(&ev.session_id)?;

    if ev.hook_event_name == "SessionStart" {
        return Some(session_context(store, &session.project));
    }

    let (state, message) = state::next(session.state, &ev)?;
    session.state = state;
    session.message = message;
    session.updated_at = chrono::Utc::now();
    if let Err(e) = store.save_session(&session) {
        log::error(&store.paths, "hook", &format!("saving session {}: {e:#}", session.id));
    }
    None
}

/// A few lines Claude Code adds to the new session's context: the project
/// and the titles (never bodies) of its wiki sections.
fn session_context(store: &Store, slug: &str) -> String {
    let (name, path, summary) = match store.get_project(slug) {
        Ok(p) => (p.name, p.path.display().to_string(), p.summary),
        Err(_) => (slug.to_string(), "unknown path".to_string(), None),
    };
    let titles: Vec<String> = store.wiki_sections(slug).into_iter().map(|(t, _)| t).collect();
    let wiki = if titles.is_empty() {
        "none yet (add with the grove write_wiki_section tool)".to_string()
    } else {
        format!("{} (read with the grove read_wiki tool)", titles.join(", "))
    };
    format!(
        "Grove project: {name} ({path})\nSummary: {}\nWiki sections: {wiki}\n",
        summary.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "none yet".into())
    )
}
