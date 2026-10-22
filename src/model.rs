//! Data types persisted under `GROVE_HOME`.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

impl Project {
    pub fn slug(&self) -> String {
        slugify(&self.name)
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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Session {
    /// The conversation to resume: the first one has the session's own id.
    pub fn conversation_id(&self) -> &str {
        self.conversation.as_deref().unwrap_or(&self.id)
    }
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
