//! Data types persisted under `GROVE_HOME`.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// Free text: the label on the project's row. Change it any time.
    pub name: String,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// The coding agent its sessions run; unset, config.toml's `agent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<Agent>,
    /// The project's file name (`projects/<id>.toml`), set when loaded. It's
    /// the project's identity: sessions, TODOs and tmux windows use it, so
    /// renaming (`name`) moves nothing. Unset, it's the slug of `name`.
    #[serde(skip)]
    pub id: Option<String>,
}

impl Project {
    pub fn slug(&self) -> String {
        self.id.clone().unwrap_or_else(|| slugify(&self.name))
    }
}

/// The coding agent a session runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    #[default]
    Claude,
    OpenCode,
}

impl Agent {
    pub fn is_claude(&self) -> bool {
        *self == Agent::Claude
    }

    /// Its program, as found on PATH.
    pub fn program(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::OpenCode => "opencode",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Starting,
    Working,
    NeedsYou,
    YourTurn,
    Ended,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            State::Starting => "starting",
            State::Working => "working",
            State::NeedsYou => "needs you",
            State::YourTurn => "your turn",
            State::Ended => "ended",
        }
    }

    /// The state as a stage of growth; needs you is a thirsty plant.
    pub fn glyph(self) -> &'static str {
        match self {
            State::Starting => "🌱",
            State::Working => "🌿",
            State::YourTurn => "🌳",
            State::NeedsYou => "💧",
            State::Ended => "🍂",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    /// Project slug.
    pub project: String,
    pub name: String,
    /// Always "local" in v1; v2 adds remote hosts.
    pub host: String,
    pub tmux_window: Option<String>,
    pub state: State,
    pub message: Option<String>,
    /// The Claude conversation the session last ran, as its hooks report
    /// it: /clear and /resume start a new one in the same window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<String>,
    /// This run of its window, also in the window's `GROVE_LAUNCH`: hooks
    /// from an earlier run (a restarted session's old Claude, exiting) are
    /// ignored rather than marking the new run ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<String>,
    /// The agent it runs. OpenCode's conversation is `ses_` plus the
    /// session's id without dashes (`opencode_session`).
    #[serde(default, skip_serializing_if = "Agent::is_claude")]
    pub agent: Agent,
    /// A turn ended and it hasn't been on screen since: "done" rather than
    /// "your turn". Set by its hooks, cleared by the dashboard once its
    /// window is in a focused terminal.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unseen: bool,
    /// What it last did: the opening of Claude's last reply (reply.rs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// How a session that finished a turn you haven't seen yet shows.
pub const DONE_GLYPH: &str = "🍎";
pub const DONE_LABEL: &str = "done";

impl Session {
    /// Its glyph and label on the dashboard: `state`'s, or done's.
    pub fn shown(&self) -> (&'static str, &'static str) {
        shown(self.state, self.unseen)
    }

    /// The conversation to resume: the first one has the session's own id.
    pub fn conversation_id(&self) -> &str {
        self.conversation.as_deref().unwrap_or(&self.id)
    }
}

/// The glyph and label of `state`, or done's when a finished turn is
/// `unseen`.
pub fn shown(state: State, unseen: bool) -> (&'static str, &'static str) {
    if unseen && state == State::YourTurn {
        (DONE_GLYPH, DONE_LABEL)
    } else {
        (state.glyph(), state.label())
    }
}

/// The OpenCode session id for Grove session `id`: OpenCode wants ids that
/// start with `ses`, and `-s` creates one that doesn't exist yet.
pub fn opencode_session(id: &str) -> String {
    format!("ses_{}", id.replace('-', ""))
}

/// Lowercase; every run of non-alphanumerics becomes one `-`; trimmed.
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_are_growth_stages() {
        let all = [
            State::Starting,
            State::Working,
            State::YourTurn,
            State::NeedsYou,
            State::Ended,
        ];
        let glyphs: Vec<_> = all.iter().map(|s| s.glyph()).collect();
        assert_eq!(glyphs, ["🌱", "🌿", "🌳", "💧", "🍂"]);
    }

    #[test]
    fn conversation_defaults_to_the_session_id() {
        // Sessions saved before Grove recorded conversations.
        let s: Session = serde_json::from_str(
            r#"{"id":"abc","project":"p","name":"s1","host":"local","tmux_window":null,
               "state":"ended","message":null,
               "created_at":"2026-10-06T10:00:00Z","updated_at":"2026-10-06T10:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(s.conversation, None);
        assert_eq!(s.conversation_id(), "abc");
        let s = Session {
            conversation: Some("def".into()),
            launch: None,
            agent: Default::default(),
            unseen: false,
            ..s
        };
        assert_eq!(s.conversation_id(), "def");
        assert!(
            serde_json::to_string(&s)
                .unwrap()
                .contains(r#""conversation":"def""#)
        );
    }

    #[test]
    fn slugify_collapses_and_trims() {
        assert_eq!(slugify("My App"), "my-app");
        assert_eq!(slugify("  BTT -- site!! "), "btt-site");
        assert_eq!(slugify("grove-fi"), "grove-fi");
    }

    #[test]
    fn state_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&State::NeedsYou).unwrap(),
            "\"needs_you\""
        );
    }
}
