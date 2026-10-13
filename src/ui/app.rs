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
    /// The selected project's to-do list, below its sessions.
    Todo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptKind {
    NewSession,
    AddProject,
    RenameSession { session_id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Normal,
    /// The `?` key list; any key closes it.
    Help,
    Prompt {
        kind: PromptKind,
        input: String,
    },
    Confirm {
        session_id: String,
        label: String,
    },
    ConfirmRemove {
        slug: String,
        name: String,
    },
    /// The add-project path doesn't exist: offer to create it.
    ConfirmCreate {
        path: String,
    },
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

/// A finca in the left pane, with whatever the poller last reported.
#[derive(Debug, Clone)]
pub struct FincaRow {
    pub slug: String,
    pub name: String,
    pub color: Option<String>,
    /// Set when the finca file couldn't be parsed.
    pub error: Option<String>,
    /// Latest poll outcome; None until the first poll returns.
    pub reach: Option<crate::finca::Reach>,
    /// When `reach` was recorded.
    pub checked_at: Option<DateTime<Utc>>,
    /// Last successful status and when it arrived, kept while unreachable.
    pub last_status: Option<crate::status::Status>,
    pub last_ok: Option<DateTime<Utc>>,
    /// The destination that last answered a poll; Enter connects there.
    pub via: Option<String>,
}

/// What the event loop should do after a key press.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    None,
    Quit,
    NewSession {
        slug: String,
        name: Option<String>,
    },
    AddProject {
        path: String,
    },
    /// Create the folder (with git init) and register it.
    CreateProject {
        path: String,
    },
    Jump {
        window: String,
    },
    Kill {
        session_id: String,
    },
    EditProject {
        slug: String,
    },
    /// A login shell on the finca, at `via` like `ConnectFinca`.
    FincaShell {
        slug: String,
        via: Option<String>,
    },
    ClearEnded {
        slug: String,
    },
    /// Forget the project (its folder is kept).
    RemoveProject {
        slug: String,
    },
    ConnectFinca {
        slug: String,
        /// The destination that last answered; None means the primary.
        via: Option<String>,
    },
    /// Connect to the finca straight into one of its sessions' windows.
    FincaJump {
        slug: String,
        via: Option<String>,
        window: String,
    },
    /// Back to one of the windows of the finca's shell session.
    FincaShellWindow {
        slug: String,
        window: String,
    },
    RenameSession {
        session_id: String,
        name: String,
    },
    /// Run the installed grove: refreshes run files and tmux config, and
    /// restarts this dashboard if the binary changed.
    Update,
    /// Write the project's to-do list.
    SaveTodos {
        slug: String,
        text: String,
    },
    /// Open the project's to-do file in the editor.
    EditTodos {
        slug: String,
    },
}

#[derive(Debug, Clone)]
pub struct App {
    pub projects: Vec<ProjectRow>,
    pub fincas: Vec<FincaRow>,
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
    /// For naming each project's Claude account.
    pub cfg: crate::config::Config,
    /// The to-do list of `todo_slug`.
    pub todos: crate::todo::Todos,
    /// The project whose list `todos` holds: the selected one, if usable.
    pub todo_slug: Option<String>,
    /// Unchecked to-dos per project slug, for the project rows.
    pub open_todos: HashMap<String, usize>,
    /// Open shell windows per finca slug (refreshed from tmux by the loop).
    pub finca_shells: HashMap<String, Vec<crate::finca::ShellWindow>>,
}

/// A row of the selected finca's pane: a remote session or a local shell.
#[derive(Debug, Clone, PartialEq)]
pub enum FincaItem {
    Session {
        project: String,
        session: crate::status::SessionStatus,
    },
    Shell(crate::finca::ShellWindow),
}

impl FincaItem {
    /// Identifies the row across reloads.
    fn key(&self) -> String {
        match self {
            FincaItem::Session { project, session } => format!("s:{project}:{}", session.name),
            FincaItem::Shell(w) => format!("w:{}", w.id),
        }
    }
}

impl App {
    pub fn new(cwd: String) -> Self {
        Self {
            projects: Vec::new(),
            fincas: Vec::new(),
            sessions: Vec::new(),
            pane: Pane::Projects,
            proj_idx: 0,
            sess_idx: 0,
            mode: Mode::Normal,
            footer: None,
            cwd,
            completions: Vec::new(),
            completion_idx: None,
            cfg: crate::config::Config::default(),
            todos: crate::todo::Todos::default(),
            todo_slug: None,
            open_todos: HashMap::new(),
            finca_shells: HashMap::new(),
        }
    }

    /// The left pane's project boxes, top to bottom: the default account,
    /// then each account that owns a folder. One entry means one box.
    pub fn groups(&self) -> Vec<crate::config::Account> {
        let mut out = vec![self.cfg.default_account()];
        for (name, a) in &self.cfg.accounts {
            if a.folder.is_some() {
                out.push(self.cfg.account(Some(name)).expect("listed account"));
            }
        }
        out
    }

    /// The account the project's folder puts it on.
    pub fn account_of(&self, p: &ProjectRow) -> crate::config::Account {
        self.cfg.account_for(&p.path)
    }

    /// Re-reads projects and sessions, keeping the selection on the same
    /// project/session when they still exist.
    pub fn reload(&mut self, store: &Store) {
        let keep_project = self.selected_project().map(|p| p.slug.clone());
        let keep_finca = self.selected_finca().map(|f| f.slug.clone());
        let keep_session = self.selected_session().map(|s| s.id.clone());
        let keep_item = self.selected_finca_item().map(|i| i.key());

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
        // Box order (each box's projects one contiguous run), then most live
        // sessions first; the sort is stable, so ties stay alphabetical.
        let groups = self.groups();
        let mut live: HashMap<&str, usize> = HashMap::new();
        for s in self.sessions.iter().filter(|s| state_is_live(s.state)) {
            *live.entry(s.project.as_str()).or_default() += 1;
        }
        let key = |p: &ProjectRow| {
            let name = self.cfg.account_for(&p.path).name;
            let group = groups.iter().position(|g| g.name == name).unwrap_or(0);
            let n = live.get(p.slug.as_str()).copied().unwrap_or(0);
            (group, std::cmp::Reverse(n))
        };
        let mut projects = std::mem::take(&mut self.projects);
        projects.sort_by_cached_key(key);
        self.projects = projects;

        if let Some(i) =
            keep_project.and_then(|slug| self.projects.iter().position(|p| p.slug == slug))
        {
            self.proj_idx = i;
        }
        self.fincas = load_fincas(store, std::mem::take(&mut self.fincas));
        if let Some(i) = keep_finca.and_then(|slug| self.fincas.iter().position(|f| f.slug == slug))
        {
            self.proj_idx = self.projects.len() + i;
        }
        self.proj_idx = self.proj_idx.min(self.left_len().saturating_sub(1));
        let visible: Vec<String> = self
            .visible_sessions()
            .iter()
            .map(|s| s.id.clone())
            .collect();
        if let Some(i) = keep_session.and_then(|id| visible.iter().position(|v| *v == id)) {
            self.sess_idx = i;
        }
        let mut len = visible.len();
        if self.selected_finca().is_some() {
            let items: Vec<String> = self.finca_items().iter().map(FincaItem::key).collect();
            if let Some(i) = keep_item.and_then(|k| items.iter().position(|v| *v == k)) {
                self.sess_idx = i;
            }
            len = items.len();
        }
        self.sess_idx = self.sess_idx.min(len.saturating_sub(1));

        self.open_todos = self
            .projects
            .iter()
            .filter(|p| p.error.is_none())
            .map(|p| {
                (
                    p.slug.clone(),
                    crate::todo::open_count(&store.read_todos(&p.slug)),
                )
            })
            .collect();
        let slug = self
            .selected_project()
            .filter(|p| p.error.is_none())
            .map(|p| p.slug.clone());
        if slug != self.todo_slug {
            self.todos.reset();
            self.todo_slug = slug;
        }
        match &self.todo_slug {
            Some(slug) => self.todos.load(&store.read_todos(slug)),
            None if self.pane == Pane::Todo => self.pane = Pane::Projects,
            None => {}
        }
    }

    /// The finca selected in the left pane (below the projects), if any.
    pub fn selected_finca(&self) -> Option<&FincaRow> {
        self.proj_idx
            .checked_sub(self.projects.len())
            .and_then(|i| self.fincas.get(i))
    }

    /// Rows in the left pane: projects, then fincas.
    pub fn left_len(&self) -> usize {
        self.projects.len() + self.fincas.len()
    }

    /// Records a poll result for the matching finca.
    pub fn apply_poll(&mut self, r: crate::finca::PollResult) {
        let Some(row) = self.fincas.iter_mut().find(|f| f.slug == r.slug) else {
            return;
        };
        if let crate::finca::Reach::Reachable(st) = &r.reach {
            row.last_status = Some(st.clone());
            row.last_ok = Some(r.at);
            row.via = r.via;
        }
        row.reach = Some(r.reach);
        row.checked_at = Some(r.at);
    }

    /// (finca name, sessions needing you) for each currently reachable finca
    /// with at least one.
    pub fn remote_needs_you(&self) -> Vec<(String, usize)> {
        self.fincas
            .iter()
            .filter_map(|f| match &f.reach {
                Some(crate::finca::Reach::Reachable(st)) if st.needs_you > 0 => {
                    Some((f.name.clone(), st.needs_you))
                }
                _ => None,
            })
            .collect()
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

    /// The selected finca's rows: its sessions (from the last good status),
    /// project by project, then its open shell windows.
    pub fn finca_items(&self) -> Vec<FincaItem> {
        let Some(f) = self.selected_finca() else {
            return Vec::new();
        };
        let sessions = f
            .last_status
            .iter()
            .flat_map(|st| &st.projects)
            .flat_map(|p| {
                p.sessions.iter().map(|s| FincaItem::Session {
                    project: p.name.clone(),
                    session: s.clone(),
                })
            });
        let shells = self
            .finca_shells
            .get(&f.slug)
            .into_iter()
            .flatten()
            .cloned()
            .map(FincaItem::Shell);
        sessions.chain(shells).collect()
    }

    pub fn selected_finca_item(&self) -> Option<FincaItem> {
        self.finca_items().into_iter().nth(self.sess_idx)
    }

    /// Local sessions needing you plus those on reachable fincas.
    pub fn needs_you_count(&self) -> usize {
        let local = self
            .sessions
            .iter()
            .filter(|s| s.state == State::NeedsYou)
            .count();
        local
            + self
                .remote_needs_you()
                .iter()
                .map(|(_, n)| n)
                .sum::<usize>()
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

/// Finca rows from disk, carrying over poll state from `old` by slug.
fn load_fincas(store: &Store, old: Vec<FincaRow>) -> Vec<FincaRow> {
    let mut old: HashMap<String, FincaRow> = old.into_iter().map(|f| (f.slug.clone(), f)).collect();
    store
        .list_fincas()
        .into_iter()
        .map(|r| match r {
            Ok(f) => {
                let slug = f.slug();
                let prev = old.remove(&slug);
                FincaRow {
                    name: f.name,
                    color: f.color,
                    error: None,
                    reach: prev.as_ref().and_then(|p| p.reach.clone()),
                    checked_at: prev.as_ref().and_then(|p| p.checked_at),
                    last_status: prev.as_ref().and_then(|p| p.last_status.clone()),
                    last_ok: prev.as_ref().and_then(|p| p.last_ok),
                    via: prev.and_then(|p| p.via),
                    slug,
                }
            }
            Err((path, err)) => {
                let stem = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                FincaRow {
                    slug: stem.clone(),
                    name: stem,
                    color: None,
                    error: Some(err),
                    reach: None,
                    checked_at: None,
                    last_status: None,
                    last_ok: None,
                    via: None,
                }
            }
        })
        .collect()
}
