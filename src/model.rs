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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
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
