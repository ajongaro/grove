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
    // An event from an earlier run of this session's window: the old Claude
    // of a restart, still exiting while the new one starts.
    // A window from before runs had tokens has none, and is old too.
    if let Some(current) = &session.launch
        && std::env::var("GROVE_LAUNCH").ok().as_ref() != Some(current)
    {
        return None;
    }

    // Which conversation to resume once the window has gone.
    let follow = session.conversation_id() != ev.session_id;
    if follow {
        session.conversation = Some(ev.session_id.clone());
    }

    if ev.hook_event_name == "SessionStart" {
        // Claude is up and waiting for its first prompt.
        let ready = session.state == crate::model::State::Starting;
        if ready {
            session.state = crate::model::State::YourTurn;
        }
        if follow || ready {
            save(store, &mut session);
        }
        return Some(session_context(store, &session.project));
    }

    match state::next(session.state, &ev) {
        Some((state, message)) => {
            session.unseen = state::ends_turn(session.state, state);
            session.state = state;
            session.message = message;
        }
        None if !follow => return None,
        None => {}
    }
    save(store, &mut session);
    None
}

/// Applies one OpenCode event from Grove's plugin. Only the window's own
/// session counts: subagents run in sessions of their own, in the same
/// OpenCode, and their turns aren't this session's.
pub fn run_opencode_hook(store: &Store, input: &str) {
    let ev: state::OpenCodeEvent = match serde_json::from_str(input) {
        Ok(ev) => ev,
        Err(e) => {
            log::error(
                &store.paths,
                "hook",
                &format!("unparseable OpenCode event ({e}): {:.200}", input),
            );
            return;
        }
    };
    let Some(mut session) = std::env::var("GROVE_SESSION")
        .ok()
        .and_then(|id| store.get_session(&id))
    else {
        return;
    };
    if let Some(current) = &session.launch
        && std::env::var("GROVE_LAUNCH").ok().as_ref() != Some(current)
    {
        return;
    }
    if ev.data.session_id.as_deref() != Some(session.conversation_id()) {
        return;
    }
    if ev.kind == "session.renamed" {
        if let Some(title) = &ev.data.title {
            crate::titles::remember(&store.paths, session.conversation_id(), title);
        }
        return;
    }
    if let Some((state, message)) = state::next_opencode(session.state, &ev) {
        session.unseen = state::ends_turn(session.state, state);
        session.state = state;
        session.message = message;
        save(store, &mut session);
    }
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
pub fn session_context(store: &Store, slug: &str) -> String {
    let (name, path, summary) = match store.get_project(slug) {
        Ok(p) => (p.name, p.path.display().to_string(), p.summary),
        Err(_) => (slug.to_string(), "unknown path".to_string(), None),
    };
    let mut out = format!(
        "Grove project: {name} ({path})\nSummary: {}\n",
        summary
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "none yet (set it with the grove update_project tool)".into())
    );
    let cfg = crate::config::load_config(&store.paths).unwrap_or_default();
    if cfg.claude_todos {
        out.push_str(&todo_context(&store.read_todos(slug)));
    }
    out
}

/// How many open TODO items to list at the start of a session; the rest
/// are a get_todos away.
const TODOS_SHOWN: usize = 10;

/// The project's TODO list and how Claude should keep it.
fn todo_context(text: &str) -> String {
    let open: Vec<String> = crate::todo::parse(text)
        .into_iter()
        .filter(|i| !i.done)
        .map(|i| crate::todo::describe(&i))
        .collect();
    let mut out = String::from(
        "\nThis project's TODO list is kept in Grove, where the user sees it on their dashboard. \
         Use it as the project's TODO list instead of a TODO file or code comments: read it with \
         the grove get_todos tool, and with edit_todos add follow-ups and things you notice but \
         won't do now, and check items off when you finish them. Your own task list is still \
         for the steps of the current job. Items are listed high priority first; edit_todos \
         takes a priority (high, normal, low) and a due date (YYYY-MM-DD). Only set a priority \
         or due date when the user gave one; never invent them.\n",
    );
    if open.is_empty() {
        out.push_str("Open TODOs: none.\n");
    } else {
        out.push_str(&format!("Open TODOs ({}):\n", open.len()));
        for t in open.iter().take(TODOS_SHOWN) {
            out.push_str(&format!("- {t}\n"));
        }
        if open.len() > TODOS_SHOWN {
            out.push_str(&format!(
                "- …and {} more (get_todos)\n",
                open.len() - TODOS_SHOWN
            ));
        }
    }
    out
}
