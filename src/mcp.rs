//! `grove mcp`: stdio MCP server exposing Grove's projects.

use std::path::{Path, PathBuf};

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{ServiceExt, schemars, tool, tool_router};
use serde::Deserialize;

use crate::config::Config;
use crate::mcp_overview;
use crate::model::{Project, Session, State};
use crate::store::Store;
use crate::tmux::Tmux;

/// Tool logic, separate from the rmcp wrappers so tests can call it directly.
/// `Err` text is returned to Claude as a tool error.
#[derive(Clone)]
pub struct GroveMcp {
    pub store: Store,
    pub cfg: Config,
    /// Grove's own tmux server, for sessions.
    pub tmux: Tmux,
    /// The account every tool is limited to, or the error each one returns.
    pub scope: Result<String, String>,
    /// From `$GROVE_PROJECT`, set on every Grove session's window.
    pub default_project: Option<String>,
    /// From `$GROVE_SESSION`: this Claude's own session.
    pub own_session: Option<String>,
}

/// The account a server serves: `--account`, else `$GROVE_ACCOUNT`. When
/// `$CLAUDE_CONFIG_DIR` is another account's folder, it was registered in
/// the wrong place, so it refuses rather than show that account this one's
/// projects.
pub fn scope_for(
    cfg: &Config,
    flag: Option<&str>,
    grove_account: Option<&str>,
    claude_config_dir: Option<&str>,
) -> Result<String, String> {
    let name = flag
        .or(grove_account)
        .filter(|n| !n.is_empty())
        .ok_or_else(|| {
            "Grove's tools don't know which Claude account this is. In a terminal, run `grove mcp setup` and use the command it prints for this account.".to_string()
        })?;
    let account = cfg.account(Some(name)).map_err(|e| format!("{e:#}"))?;
    if let Some(dir) = claude_config_dir.filter(|d| !d.is_empty()) {
        let canon = |p: &std::path::Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        let here = canon(&crate::config::expand_tilde(dir));
        if let Some(other) = cfg
            .all_accounts()
            .into_iter()
            .find(|a| a.name != account.name && canon(&a.dir) == here)
        {
            return Err(format!(
                "these Grove tools are registered for the {} account but are running in {}'s Claude config ({dir}); run `grove mcp setup` for the right command",
                account.name, other.name
            ));
        }
    }
    Ok(account.name)
}

impl GroveMcp {
    fn account(&self) -> Result<&str, String> {
        self.scope.as_deref().map_err(Clone::clone)
    }

    fn in_scope(&self, p: &Project) -> bool {
        self.account()
            .is_ok_and(|a| self.cfg.account_for(&p.path).name == a)
    }

    /// This account's projects (unreadable project files are skipped).
    fn projects(&self) -> Result<Vec<Project>, String> {
        self.account()?;
        Ok(self
            .store
            .list_projects()
            .into_iter()
            .filter_map(Result::ok)
            .filter(|p| self.in_scope(p))
            .collect())
    }

    fn names(&self) -> String {
        let names: Vec<String> = self
            .projects()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.name)
            .collect();
        if names.is_empty() {
            "(none)".into()
        } else {
            names.join(", ")
        }
    }

    /// A project of this account by name or slug, else the session's own.
    /// One outside the account gets the same error as one that doesn't exist.
    fn project(&self, project: Option<String>) -> Result<Project, String> {
        self.account()?;
        let name = project
            .filter(|p| !p.trim().is_empty())
            .or_else(|| self.default_project.clone());
        match name {
            Some(n) => self
                .store
                .get_project(&n)
                .ok()
                .filter(|p| self.in_scope(p))
                .ok_or_else(|| format!("no project \"{n}\"; projects: {}", self.names())),
            None => Err(format!(
                "no project given and this isn't a Grove session; pass `project` (one of: {})",
                self.names()
            )),
        }
    }

    pub fn do_list_projects(&self) -> Result<String, String> {
        let lines: Vec<String> = self
            .projects()?
            .into_iter()
            .map(|p| {
                let summary = p.summary.as_deref().unwrap_or("(no summary)");
                let sessions = mcp_overview::live(self.store.list_sessions(Some(&p.slug())));
                let open = crate::todo::open_count(&self.store.read_todos(&p.slug()));
                format!(
                    "- {} — {} — {summary}\n  sessions: {}; {open} open to-do{}",
                    p.name,
                    p.path.display(),
                    mcp_overview::counts(&sessions),
                    if open == 1 { "" } else { "s" }
                )
            })
            .collect();
        Ok(if lines.is_empty() {
            "No projects registered in Grove yet.".into()
        } else {
            lines.join("\n")
        })
    }

    pub fn do_get_project(&self, project: Option<String>) -> Result<String, String> {
        let p = self.project(project)?;
        let slug = p.slug();
        let now = chrono::Utc::now();
        let sessions = mcp_overview::live(self.store.list_sessions(Some(&slug)));
        let mut out = format!(
            "Name: {}\nPath: {}\nAccount: {}\nColor: {}\nSummary: {}\n\nSessions: {}\n",
            p.name,
            p.path.display(),
            self.cfg.account_for(&p.path).name,
            p.color.as_deref().unwrap_or("(none)"),
            p.summary.as_deref().unwrap_or("(none)"),
            mcp_overview::counts(&sessions),
        );
        for s in &sessions {
            out.push_str(&format!(
                "  {slug}:{} — {} — {}\n",
                s.name,
                s.state.label(),
                mcp_overview::ago(s.updated_at, now)
            ));
        }
        let todos = crate::todo::parse(&self.store.read_todos(&slug));
        out.push_str(&format!(
            "\nTo-dos:\n{}\n\n{}\n",
            numbered(&todos),
            mcp_overview::git(&p.path)
        ));
        for doc in ["README.md", "CLAUDE.md"] {
            if let Some(e) = mcp_overview::excerpt(&p.path.join(doc), 40) {
                out.push_str(&format!("\n--- {doc} ---\n{e}\n"));
            }
        }
        Ok(out)
    }

    /// Live sessions of this account's projects (all, or one project's),
    /// most recently active first.
    fn sessions(&self, project: Option<&Project>) -> Result<Vec<(Session, Project)>, String> {
        let projects = match project {
            Some(p) => vec![p.clone()],
            None => self.projects()?,
        };
        let mut out: Vec<(Session, Project)> = projects
            .into_iter()
            .flat_map(|p| {
                mcp_overview::live(self.store.list_sessions(Some(&p.slug())))
                    .into_iter()
                    .map(move |s| (s, p.clone()))
            })
            .collect();
        out.sort_by_key(|(s, _)| std::cmp::Reverse(s.updated_at));
        Ok(out)
    }

    /// A live session of this account by id, `project:name`, or a name
    /// unique within the account.
    fn session(&self, key: &str) -> Result<(Session, Project), String> {
        let all = self.sessions(None)?;
        let handle = |s: &Session, p: &Project| format!("{}:{}", p.slug(), s.name);
        let key = key.trim();
        if let Some(hit) = all.iter().find(|(s, p)| s.id == key || handle(s, p) == key) {
            return Ok(hit.clone());
        }
        let named: Vec<_> = all.iter().filter(|(s, _)| s.name == key).collect();
        match named.as_slice() {
            [one] => Ok((*one).clone()),
            [] => {
                let names: Vec<String> = all.iter().map(|(s, p)| handle(s, p)).collect();
                Err(format!(
                    "no live session \"{key}\"; sessions: {}",
                    if names.is_empty() {
                        "(none)".into()
                    } else {
                        names.join(", ")
                    }
                ))
            }
            many => Err(format!(
                "\"{key}\" is more than one session: {}; use one of those",
                many.iter()
                    .map(|(s, p)| handle(s, p))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    pub fn do_list_sessions(&self, project: Option<String>) -> Result<String, String> {
        let p = match project.filter(|p| !p.trim().is_empty()) {
            Some(name) => Some(self.project(Some(name))?),
            None => None,
        };
        let now = chrono::Utc::now();
        let lines: Vec<String> = self
            .sessions(p.as_ref())?
            .iter()
            .map(|(s, p)| {
                let msg = s
                    .message
                    .as_deref()
                    .map(|m| format!(" — {m}"))
                    .unwrap_or_default();
                format!(
                    "- {}:{} — {} — {}{msg}",
                    p.slug(),
                    s.name,
                    s.state.label(),
                    mcp_overview::ago(s.updated_at, now)
                )
            })
            .collect();
        Ok(if lines.is_empty() {
            "No live sessions.".into()
        } else {
            lines.join("\n")
        })
    }

    pub fn do_read_session(&self, session: String, lines: Option<usize>) -> Result<String, String> {
        let (s, p) = self.session(&session)?;
        let n = lines.unwrap_or(60).clamp(1, 500);
        let handle = format!("{}:{}", p.slug(), s.name);
        let window = s
            .tmux_window
            .as_deref()
            .ok_or_else(|| format!("{handle} has no window yet (it may still be starting)"))?;
        let screen = self
            .tmux
            .capture(window, n)
            .map_err(|_| format!("{handle}'s window is gone"))?;
        let kept: Vec<&str> = screen.trim_end().lines().collect();
        Ok(kept[kept.len().saturating_sub(n)..].join("\n"))
    }

    pub fn do_start_session(
        &self,
        project: Option<String>,
        prompt: Option<String>,
        name: Option<String>,
    ) -> Result<String, String> {
        self.do_start_session_with_bin(project, prompt, name, &crate::launch::claude_bin())
    }

    /// `do_start_session` with the program to run swapped out (tests).
    pub fn do_start_session_with_bin(
        &self,
        project: Option<String>,
        prompt: Option<String>,
        name: Option<String>,
        bin: &str,
    ) -> Result<String, String> {
        let p = self.project(project)?;
        if !self.tmux.has_session(&self.cfg.tmux_session) {
            return Err("Grove isn't running: start it with `grove` in a terminal".into());
        }
        let s = crate::launch::start_with_bin(
            &self.tmux,
            &self.store,
            &self.cfg,
            &p,
            name,
            prompt,
            bin,
        )
        .map_err(|e| format!("{e:#}"))?;
        Ok(format!(
            "Started {}:{} on the {} account.",
            p.slug(),
            s.name,
            self.cfg.account_for(&p.path).name
        ))
    }

    /// Types `text` into a session that's waiting for its next message.
    pub fn do_send_prompt(&self, session: String, text: String) -> Result<String, String> {
        let (s, p) = self.session(&session)?;
        let handle = format!("{}:{}", p.slug(), s.name);
        if self.own_session.as_deref() == Some(s.id.as_str()) {
            return Err("that's this session's own window; just answer here".into());
        }
        match s.state {
            State::YourTurn => {}
            State::NeedsYou => {
                return Err(format!(
                    "{handle} needs you: answer its prompt on the dashboard first"
                ));
            }
            other => {
                return Err(format!(
                    "{handle} is {}; wait until it's your turn",
                    other.label()
                ));
            }
        }
        // Only text: an escape sequence could end the paste early and type
        // keys of its own (say, answering a permission prompt), and a bare
        // carriage return would submit.
        let text: String = text
            .chars()
            .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
            .collect();
        if text.trim().is_empty() {
            return Err("nothing to send".into());
        }
        let window = s
            .tmux_window
            .as_deref()
            .ok_or_else(|| format!("{handle} has no window"))?;
        self.tmux
            .paste_text(window, &text)
            .map_err(|_| format!("{handle}'s window is gone"))?;
        // Claude's input may need a beat after a paste before Enter (spec:
        // settled by trial).
        std::thread::sleep(std::time::Duration::from_millis(150));
        self.tmux
            .send_keys(window, &["Enter"])
            .map_err(|e| format!("{e:#}"))?;
        Ok(format!("Sent to {handle}."))
    }

    pub fn do_get_todos(&self, project: Option<String>) -> Result<String, String> {
        let p = self.project(project)?;
        Ok(numbered(&crate::todo::parse(
            &self.store.read_todos(&p.slug()),
        )))
    }

    pub fn do_edit_todos(
        &self,
        project: Option<String>,
        ops: Vec<TodoOpArg>,
    ) -> Result<String, String> {
        let p = self.project(project)?;
        let ops = ops
            .iter()
            .map(TodoOpArg::to_op)
            .collect::<Result<Vec<_>, _>>()?;
        let items = crate::todo::parse(&self.store.read_todos(&p.slug()));
        let new = crate::todo::apply_ops(&items, &ops)?;
        self.store
            .write_todos(&p.slug(), &crate::todo::serialize_items(&new))
            .map_err(|e| e.to_string())?;
        Ok(numbered(&new))
    }

    pub fn do_update_project(
        &self,
        project: Option<String>,
        summary: Option<String>,
        color: Option<String>,
    ) -> Result<String, String> {
        if summary.is_none() && color.is_none() {
            return Err("nothing to update: pass `summary` and/or `color`".into());
        }
        if let Some(c) = &color {
            let ok = c.len() == 7
                && c.starts_with('#')
                && c[1..].chars().all(|ch| ch.is_ascii_hexdigit());
            if !ok {
                return Err(format!("color {c:?} must be #RRGGBB"));
            }
        }
        let mut p = self.project(project)?;
        if summary.is_some() {
            p.summary = summary;
        }
        if color.is_some() {
            p.color = color;
        }
        self.store.save_project(&p).map_err(|e| e.to_string())?;
        self.do_get_project(Some(p.name))
    }
}

/// `1. [ ] text` lines, or `(no to-dos)`.
fn numbered(items: &[crate::todo::Item]) -> String {
    if items.is_empty() {
        return "(no to-dos)".into();
    }
    items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            format!(
                "{}. [{}] {}",
                i + 1,
                if it.done { 'x' } else { ' ' },
                it.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ProjectArg {
    /// Project name or slug. Defaults to the current Grove session's project.
    pub project: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateProjectArgs {
    /// Project name or slug. Defaults to the current Grove session's project.
    pub project: Option<String>,
    /// One or two sentences: what the project does and its main technologies.
    pub summary: Option<String>,
    /// Brand color as #RRGGBB.
    pub color: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TodoOpArg {
    /// add, check, uncheck, reword or remove.
    pub op: String,
    /// The item's number as get_todos shows it (before this call's edits). Not for add.
    pub n: Option<usize>,
    /// The text, for add and reword.
    pub text: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EditTodosArgs {
    /// Project name or slug. Defaults to the current Grove session's project.
    pub project: Option<String>,
    /// Edits, applied in order and saved together; one bad edit saves nothing.
    pub ops: Vec<TodoOpArg>,
}

impl TodoOpArg {
    fn to_op(&self) -> Result<crate::todo::Op, String> {
        use crate::todo::Op;
        let n = || self.n.ok_or_else(|| format!("{} needs `n`", self.op));
        let text = || {
            self.text
                .clone()
                .ok_or_else(|| format!("{} needs `text`", self.op))
        };
        Ok(match self.op.as_str() {
            "add" => Op::Add(text()?),
            "check" => Op::Check(n()?),
            "uncheck" => Op::Uncheck(n()?),
            "reword" => Op::Reword(n()?, text()?),
            "remove" => Op::Remove(n()?),
            other => {
                return Err(format!(
                    "unknown op \"{other}\"; use add, check, uncheck, reword or remove"
                ));
            }
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListSessionsArgs {
    /// Only this project's sessions (name or slug). All of them if left out.
    pub project: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadSessionArgs {
    /// The session: project:name as list_sessions shows it, a name, or an id.
    pub session: String,
    /// How many lines from the bottom of its screen (default 60, max 500).
    pub lines: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StartSessionArgs {
    /// Project name or slug. Defaults to the current Grove session's project.
    pub project: Option<String>,
    /// The new session's first message: more than one word, not starting with "-".
    pub prompt: Option<String>,
    /// A short name for it (default s1, s2, …).
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SendPromptArgs {
    /// The session: project:name as list_sessions shows it, a name, or an id.
    pub session: String,
    /// What to type into it. It must be waiting for you ("your turn").
    pub text: String,
}

fn result(r: Result<String, String>) -> CallToolResult {
    match r {
        Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
        Err(text) => CallToolResult::error(vec![ContentBlock::text(text)]),
    }
}

#[tool_router(server_handler)]
impl GroveMcp {
    #[tool(description = "List all Grove projects with their path and summary.")]
    fn list_projects(&self) -> CallToolResult {
        result(self.do_list_projects())
    }

    #[tool(description = "Show a Grove project's details.")]
    fn get_project(&self, Parameters(a): Parameters<ProjectArg>) -> CallToolResult {
        result(self.do_get_project(a.project))
    }

    #[tool(
        description = "Live Claude sessions in Grove projects: state (working, needs you, your turn) and when each last moved."
    )]
    fn list_sessions(&self, Parameters(a): Parameters<ListSessionsArgs>) -> CallToolResult {
        result(self.do_list_sessions(a.project))
    }

    #[tool(description = "The bottom of a Grove session's screen: what it's doing or asking.")]
    fn read_session(&self, Parameters(a): Parameters<ReadSessionArgs>) -> CallToolResult {
        result(self.do_read_session(a.session, a.lines))
    }

    #[tool(
        description = "Start a new Claude session in a Grove project, on that project's account, optionally with a first prompt. It appears on the Grove dashboard."
    )]
    fn start_session(&self, Parameters(a): Parameters<StartSessionArgs>) -> CallToolResult {
        result(self.do_start_session(a.project, a.prompt, a.name))
    }

    #[tool(
        description = "Send a prompt to a Grove session that's waiting for its next message (state: your turn). Refused while it's working or needs you."
    )]
    fn send_prompt(&self, Parameters(a): Parameters<SendPromptArgs>) -> CallToolResult {
        result(self.do_send_prompt(a.session, a.text))
    }

    #[tool(description = "A Grove project's to-do list, numbered.")]
    fn get_todos(&self, Parameters(a): Parameters<ProjectArg>) -> CallToolResult {
        result(self.do_get_todos(a.project))
    }

    #[tool(
        description = "Edit a Grove project's to-do list: add, check, uncheck, reword or remove items by number. Returns the new list."
    )]
    fn edit_todos(&self, Parameters(a): Parameters<EditTodosArgs>) -> CallToolResult {
        result(self.do_edit_todos(a.project, a.ops))
    }

    #[tool(description = "Update a Grove project's summary and/or color (#RRGGBB).")]
    fn update_project(&self, Parameters(a): Parameters<UpdateProjectArgs>) -> CallToolResult {
        result(self.do_update_project(a.project, a.summary, a.color))
    }
}

impl GroveMcp {
    /// `env` reads environment variables (tests pass their own).
    pub fn new(
        store: Store,
        tmux: Tmux,
        account_flag: Option<String>,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Self {
        let env = |k: &str| env(k).filter(|v| !v.is_empty());
        let (cfg, scope) = match crate::config::load_config(&store.paths) {
            Ok(cfg) => {
                let scope = scope_for(
                    &cfg,
                    account_flag.as_deref(),
                    env("GROVE_ACCOUNT").as_deref(),
                    env("CLAUDE_CONFIG_DIR").as_deref(),
                );
                (cfg, scope)
            }
            Err(e) => (
                Config::default(),
                Err(format!("Grove's config.toml: {e:#}")),
            ),
        };
        Self {
            store,
            cfg,
            tmux,
            scope,
            default_project: env("GROVE_PROJECT"),
            own_session: env("GROVE_SESSION"),
        }
    }
}

pub async fn serve(store: Store, tmux: Tmux, account_flag: Option<String>) -> anyhow::Result<()> {
    let service = GroveMcp::new(store, tmux, account_flag, &|k| std::env::var(k).ok())
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}

/// Where Claude keeps user-scope MCP servers for the account in `dir`:
/// `~/.claude.json` for the default `~/.claude` (used without
/// CLAUDE_CONFIG_DIR), else `<dir>/.claude.json`.
pub fn claude_json(dir: &Path, home: &Path) -> PathBuf {
    if dir == home.join(".claude") {
        home.join(".claude.json")
    } else {
        dir.join(".claude.json")
    }
}

/// `grove mcp setup`: one `claude mcp add` per account, to paste.
pub fn setup_commands(cfg: &Config, exe: &Path, home: &Path) -> String {
    let q = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
    let mut out = String::from(
        "Give Claude Grove's tools in every session, not only Grove's own.\nRun each line once (Grove doesn't change Claude's settings itself):\n",
    );
    for a in cfg.all_accounts() {
        let prefix = if a.dir == home.join(".claude") {
            String::new()
        } else {
            format!("CLAUDE_CONFIG_DIR={} ", q(&a.dir.to_string_lossy()))
        };
        out.push_str(&format!(
            "\n# {} ({})\n{prefix}claude mcp add --scope user grove -- {} mcp --account {}\n",
            a.name,
            a.dir.display(),
            q(&exe.to_string_lossy()),
            q(&a.name),
        ));
    }
    out.push_str("\nCheck with: grove doctor\n");
    out
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
