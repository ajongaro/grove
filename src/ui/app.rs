//! Dashboard state. No terminal or tmux I/O here, so it's all unit-testable.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};

use crate::model::{Session, State};
use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Projects,
    Sessions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    NewSession,
    AddProject,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Normal,
    Prompt { kind: PromptKind, input: String },
    Confirm { session_id: String, label: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectRow {
    pub slug: String,
    pub name: String,
    pub path: PathBuf,
    pub color: Option<String>,
    /// Set when the project file couldn't be parsed.
    pub error: Option<String>,
}

/// What the event loop should do after a key press.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    None,
    Quit,
    NewSession { slug: String, name: Option<String> },
    AddProject { path: String },
    Jump { window: String },
    Kill { session_id: String },
    EditProject { slug: String },
    OpenWiki { slug: String },
    ClearEnded { slug: String },
}

#[derive(Debug, Clone)]
pub struct App {
    pub projects: Vec<ProjectRow>,
    /// All sessions, oldest first.
    pub sessions: Vec<Session>,
    pub pane: Pane,
    pub proj_idx: usize,
    pub sess_idx: usize,
    pub mode: Mode,
    /// One-line status/error shown in the footer.
    pub footer: Option<String>,
    /// Default for the add-project prompt.
    pub cwd: String,
    /// Tab-completion matches in the add-project prompt (full paths).
    pub completions: Vec<String>,
    /// Which match the prompt currently shows, once Tab cycles.
    pub completion_idx: Option<usize>,
}

impl App {
    pub fn new(cwd: String) -> Self {
        Self {
            projects: Vec::new(),
            sessions: Vec::new(),
            pane: Pane::Projects,
            proj_idx: 0,
            sess_idx: 0,
            mode: Mode::Normal,
            footer: None,
            cwd,
            completions: Vec::new(),
            completion_idx: None,
        }
    }

    /// Re-reads projects and sessions, keeping the selection on the same
    /// project/session when they still exist.
    pub fn reload(&mut self, store: &Store) {
        let keep_project = self.selected_project().map(|p| p.slug.clone());
        let keep_session = self.selected_session().map(|s| s.id.clone());

        self.projects = store
            .list_projects()
            .into_iter()
            .map(|r| match r {
                Ok(p) => ProjectRow {
                    slug: p.slug(),
                    name: p.name,
                    path: p.path,
                    color: p.color,
                    error: None,
                },
                Err((path, err)) => {
                    let stem = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    ProjectRow {
                        slug: stem.clone(),
                        name: stem,
                        path,
                        color: None,
                        error: Some(err),
                    }
                }
            })
            .collect();
        self.sessions = store.list_sessions(None);

        if let Some(i) =
            keep_project.and_then(|slug| self.projects.iter().position(|p| p.slug == slug))
        {
            self.proj_idx = i;
        }
        self.proj_idx = self.proj_idx.min(self.projects.len().saturating_sub(1));
        let visible: Vec<String> = self
            .visible_sessions()
            .iter()
            .map(|s| s.id.clone())
            .collect();
        if let Some(i) = keep_session.and_then(|id| visible.iter().position(|v| *v == id)) {
            self.sess_idx = i;
        }
        self.sess_idx = self.sess_idx.min(visible.len().saturating_sub(1));
    }

    pub fn selected_project(&self) -> Option<&ProjectRow> {
        self.projects.get(self.proj_idx)
    }

    /// Sessions of the selected project, oldest first.
    pub fn visible_sessions(&self) -> Vec<&Session> {
        match self.selected_project() {
            Some(p) => self
                .sessions
                .iter()
                .filter(|s| s.project == p.slug)
                .collect(),
            None => Vec::new(),
        }
    }

    pub fn selected_session(&self) -> Option<&Session> {
        self.visible_sessions().get(self.sess_idx).copied()
    }

    pub fn needs_you_count(&self) -> usize {
        self.sessions
            .iter()
            .filter(|s| s.state == State::NeedsYou)
            .count()
    }

    /// needs-you count per project slug (for the project list badges).
    pub fn needs_you_by_project(&self) -> HashMap<String, usize> {
        let mut m = HashMap::new();
        for s in self.sessions.iter().filter(|s| s.state == State::NeedsYou) {
            *m.entry(s.project.clone()).or_insert(0) += 1;
        }
        m
    }
}

/// Sessions whose tmux window no longer exists, returned as `Ended` copies
/// ready to save. Sessions without a window yet (still starting) are left alone.
pub fn mark_missing(
    sessions: &[Session],
    live_windows: &[String],
    now: DateTime<Utc>,
) -> Vec<Session> {
    sessions
        .iter()
        .filter(|s| state_is_live(s.state))
        .filter(|s| {
            s.tmux_window
                .as_ref()
                .is_some_and(|w| !live_windows.contains(w))
        })
        .map(|s| Session {
            state: State::Ended,
            message: None,
            updated_at: now,
            ..s.clone()
        })
        .collect()
}

/// "now", "45s", "3m", "2h", "4d".
pub fn format_age(since: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - since).num_seconds();
    match secs {
        s if s < 5 => "now".into(),
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

pub fn state_is_live(s: State) -> bool {
    s != State::Ended
}
