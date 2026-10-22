//! Dashboard state. No terminal or tmux I/O here, so it's all unit-testable.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use chrono::{DateTime, Utc};

use crate::model::{Session, State};
use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Projects,
    /// Pinned sessions, at the top of the right column.
    Pinned,
    Sessions,
    /// The selected project's to-do list, below its sessions.
    Todo,
}

/// The grid's reply line: which session it's for, and the text so far. The
/// target is kept by id, not tile position, as tiles move when sessions
/// come and go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub session_id: String,
    /// The finca's slug, for a finca session.
    pub finca: Option<String>,
    pub handle: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptKind {
    NewSession,
    AddProject,
    AddFinca,
    RenameSession {
        session_id: String,
    },
    /// A shell's window: a project's (`finca` None) or a finca's.
    RenameShell {
        window: String,
        finca: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Normal,
    /// The `?` key list; any key closes it.
    Help,
    /// `/`: finding a session anywhere; `sel` indexes `search::search`.
    Search {
        query: String,
        sel: usize,
    },
    /// The session grid (`v`): `sel` indexes `grid::tiles`; `reply` is the
    /// reply line's text while it's open.
    Grid {
        sel: usize,
        reply: Option<Reply>,
    },
    Prompt {
        kind: PromptKind,
        input: String,
    },
    Confirm {
        session_id: String,
        label: String,
        /// The finca's slug, for a finca session (a pin).
        finca: Option<String>,
    },
    ConfirmRemove {
        slug: String,
        name: String,
    },
    /// The add-project path doesn't exist: offer to create it.
    ConfirmCreate {
        path: String,
    },
    /// Close this shell (`x` on it in Sessions)? y / n.
    ConfirmShell {
        window: String,
        /// `project:name`.
        label: String,
    },
    /// Kill this grid tile's session? y / n, then back to the grid.
    GridKill {
        sel: usize,
        session_id: String,
        /// The finca's slug, for a finca session.
        finca: Option<String>,
        handle: String,
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
    /// Run the guided finca setup against this SSH destination.
    SetupFinca {
        dest: String,
    },
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
    /// Reopen an ended session's last conversation in a new window.
    Resume {
        session_id: String,
    },
    /// Resume an ended session on a finca, then connect into it.
    FincaResume {
        slug: String,
        via: Option<String>,
        session_id: String,
    },
    EditProject {
        slug: String,
    },
    /// A throwaway shell in the project's folder, in a popup.
    ProjectShell {
        slug: String,
    },
    RenameShell {
        window: String,
        /// The finca's slug, for a finca shell.
        finca: Option<String>,
        name: String,
    },
    /// A shell that stays open, in its own window in the project's folder.
    NewShell {
        slug: String,
    },
    /// Close a shell's window.
    CloseShell {
        window: String,
        /// `project:name`, for the footer.
        label: String,
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
    /// Type `text` into a waiting session from the grid: here, or on a
    /// finca (slug, the destination that answered).
    GridReply {
        finca: Option<(String, Option<String>)>,
        session_id: String,
        handle: String,
        text: String,
    },
    /// Write the pins file (the keys already changed `App::pins`).
    SavePins {
        pins: Vec<crate::pins::Pin>,
    },
    /// Kill a session from the grid: here, or on a finca (slug, the
    /// destination that answered).
    GridKill {
        finca: Option<(String, Option<String>)>,
        session_id: String,
        handle: String,
    },
}

#[derive(Debug, Clone)]
pub struct App {
    pub projects: Vec<ProjectRow>,
    pub fincas: Vec<FincaRow>,
    /// All sessions, oldest first.
    pub sessions: Vec<Session>,
    /// What each session's status line last recorded, by session id.
    pub stats: HashMap<String, crate::stats::SessionStats>,
    /// Each local session's conversation title (titles.rs), by session id.
    pub titles: HashMap<String, String>,
    pub pane: Pane,
    pub proj_idx: usize,
    pub sess_idx: usize,
    pub mode: Mode,
    /// One-line status/error shown in the footer.
    pub footer: Option<String>,
    /// Default for the add-project prompt.
    pub cwd: String,
    /// This machine's short host name, for the header.
    pub host: String,
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
    /// Shells that stay open, every project's (refreshed from tmux by the
    /// loop); each project's list under its sessions.
    pub shells: Vec<crate::shells::Shell>,
    /// Open shell windows per finca slug (refreshed from tmux by the loop).
    pub finca_shells: HashMap<String, Vec<crate::finca::ShellWindow>>,
    /// ~/.ssh here, for completing finca destinations.
    pub ssh_dir: std::path::PathBuf,
    /// Local sessions' screens by session id, while the grid is open.
    pub local_screens: HashMap<String, String>,
    /// Each finca's last good screens and when they came.
    pub finca_screens: HashMap<String, (DateTime<Utc>, crate::screens::Screens)>,
    /// Fincas whose Grove predates `grove screens`.
    pub too_old: HashSet<String>,
    /// The grid's column count, set when it's drawn, for hjkl.
    pub grid_cols: std::cell::Cell<usize>,
    /// Pinned sessions in pin order (pins.json, pruned on reload).
    pub pins: Vec<crate::pins::Pin>,
    /// Selected row of the Pinned pane.
    pub pin_idx: usize,
}

/// A pinned session as the Pinned pane shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct PinRow {
    pub pin: crate::pins::Pin,
    /// (slug, name, destination that last answered), for a finca's pin.
    pub finca: Option<(String, String, Option<String>)>,
    /// The Claude account, for a local pin.
    pub account: Option<crate::config::Account>,
    /// `project:name`.
    pub handle: String,
    pub state: State,
    pub updated_at: DateTime<Utc>,
    pub message: Option<String>,
    pub window: Option<String>,
    /// Its finca's last poll failed (or none has come yet).
    pub unreachable: bool,
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
            host: String::new(),
            projects: Vec::new(),
            fincas: Vec::new(),
            sessions: Vec::new(),
            stats: HashMap::new(),
            titles: HashMap::new(),
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
            shells: Vec::new(),
            finca_shells: HashMap::new(),
            ssh_dir: std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_default()
                .join(".ssh"),
            local_screens: HashMap::new(),
            finca_screens: HashMap::new(),
            too_old: HashSet::new(),
            grid_cols: std::cell::Cell::new(1),
            pins: Vec::new(),
            pin_idx: 0,
        }
    }

    /// A finca's screens from the grid's fetcher.
    pub fn apply_screens(&mut self, r: crate::screens::FetchResult) {
        match r.fetched {
            crate::screens::Fetched::Ok(s) => {
                self.too_old.remove(&r.slug);
                self.finca_screens.insert(r.slug, (r.at, s));
            }
            crate::screens::Fetched::TooOld => {
                self.too_old.insert(r.slug);
            }
            // Keep the last good screens: they go stale on their own.
            crate::screens::Fetched::Failed(_) => {}
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
        let keep_shell = self.selected_shell().map(|s| s.id.clone());
        let keep_item = self.selected_finca_item().map(|i| i.key());

        self.stats = crate::stats::load_all(&store.paths);
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
        self.titles = self
            .sessions
            .iter()
            .filter_map(|s| {
                let p = self.projects.iter().find(|p| p.slug == s.project)?;
                let t = crate::titles::for_session(&store.paths, &self.cfg, &p.path, s)?;
                Some((s.id.clone(), t))
            })
            .collect();
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
        let keep_pin = self.pins.get(self.pin_idx).cloned();
        let loaded = crate::pins::load(&store.paths);
        self.pins = loaded.clone();
        if self.prune_pins() || self.pins != loaded {
            // Best effort: a failed write just prunes again next time.
            let _ = crate::pins::save(&store.paths, &self.pins);
        }
        if let Some(i) = keep_pin.and_then(|k| self.pins.iter().position(|p| *p == k)) {
            self.pin_idx = i;
        }
        self.pin_idx = self.pin_idx.min(self.pins.len().saturating_sub(1));
        // The box goes with its last pin; focus can't stay on it.
        if self.pins.is_empty() && self.pane == Pane::Pinned {
            self.pane = Pane::Sessions;
        }
        let visible: Vec<String> = self
            .visible_sessions()
            .iter()
            .map(|s| s.id.clone())
            .collect();
        let shells: Vec<String> = self.visible_shells().iter().map(|s| s.id.clone()).collect();
        if let Some(i) = keep_session.and_then(|id| visible.iter().position(|v| *v == id)) {
            self.sess_idx = i;
        }
        if let Some(i) = keep_shell.and_then(|id| shells.iter().position(|v| *v == id)) {
            self.sess_idx = visible.len() + i;
        }
        let mut len = visible.len() + shells.len();
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

    /// The selected project's shells, listed below its sessions.
    pub fn visible_shells(&self) -> Vec<&crate::shells::Shell> {
        match self.selected_project() {
            Some(p) => self.shells.iter().filter(|s| s.project == p.slug).collect(),
            None => Vec::new(),
        }
    }

    /// The selected Sessions-pane row, when it's a shell.
    pub fn selected_shell(&self) -> Option<&crate::shells::Shell> {
        let i = self.sess_idx.checked_sub(self.visible_sessions().len())?;
        self.visible_shells().get(i).copied()
    }

    /// A shell's name as the dashboard shows it (empty once it's closed).
    pub fn shell_label(&self, window: &str, finca: Option<&str>) -> String {
        match finca {
            None => self
                .shells
                .iter()
                .find(|s| s.id == window)
                .map(|s| s.name.clone()),
            Some(slug) => self
                .finca_shells
                .get(slug)
                .and_then(|ws| ws.iter().find(|w| w.id == window))
                .map(|w| w.label()),
        }
        .unwrap_or_default()
    }

    /// Rows in the Sessions pane of a project: sessions, then shells.
    pub fn session_rows(&self) -> usize {
        self.visible_sessions().len() + self.visible_shells().len()
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

    /// Whether a pin still names a live session. A finca pin goes only
    /// when its finca is gone, or answered without the session: while it
    /// isn't answering, there's no telling.
    fn pin_alive(&self, pin: &crate::pins::Pin) -> bool {
        match &pin.finca {
            None => self
                .sessions
                .iter()
                .any(|s| s.id == pin.id && state_is_live(s.state)),
            Some(slug) => match self.fincas.iter().find(|f| &f.slug == slug) {
                None => false,
                Some(f) => match &f.reach {
                    Some(crate::finca::Reach::Reachable(st)) => st
                        .projects
                        .iter()
                        .flat_map(|p| &p.sessions)
                        .any(|s| s.id.as_deref() == Some(pin.id.as_str())),
                    _ => true,
                },
            },
        }
    }

    /// Drops pins whose sessions ended; true when any went.
    pub fn prune_pins(&mut self) -> bool {
        let before = self.pins.len();
        let pins = std::mem::take(&mut self.pins);
        self.pins = pins.into_iter().filter(|p| self.pin_alive(p)).collect();
        self.pins.len() != before
    }

    /// The Pinned pane's rows, in pin order.
    pub fn pinned_rows(&self) -> Vec<PinRow> {
        self.pins
            .iter()
            .filter(|p| self.pin_alive(p))
            .filter_map(|pin| match &pin.finca {
                None => {
                    let s = self.sessions.iter().find(|s| s.id == pin.id)?;
                    let account = self
                        .projects
                        .iter()
                        .find(|p| p.slug == s.project)
                        .map(|p| self.account_of(p));
                    Some(PinRow {
                        pin: pin.clone(),
                        finca: None,
                        account,
                        handle: format!("{}:{}", s.project, s.name),
                        state: s.state,
                        updated_at: s.updated_at,
                        message: s.message.clone(),
                        window: s.tmux_window.clone(),
                        unreachable: false,
                    })
                }
                Some(slug) => {
                    let f = self.fincas.iter().find(|f| &f.slug == slug)?;
                    let found = f
                        .last_status
                        .iter()
                        .flat_map(|st| &st.projects)
                        .find_map(|p| {
                            p.sessions
                                .iter()
                                .find(|s| s.id.as_deref() == Some(pin.id.as_str()))
                                .map(|s| (p.name.clone(), s))
                        });
                    let unreachable = !matches!(f.reach, Some(crate::finca::Reach::Reachable(_)));
                    let (handle, state, updated_at, message, window) = match found {
                        Some((project, s)) => (
                            format!("{project}:{}", s.name),
                            s.state,
                            s.updated_at,
                            s.message.clone(),
                            s.window.clone(),
                        ),
                        None => ("?".into(), State::Starting, Utc::now(), None, None),
                    };
                    Some(PinRow {
                        pin: pin.clone(),
                        finca: Some((f.slug.clone(), f.name.clone(), f.via.clone())),
                        account: None,
                        handle,
                        state,
                        updated_at,
                        message,
                        window,
                        unreachable,
                    })
                }
            })
            .collect()
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
