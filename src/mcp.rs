//! `grove mcp`: stdio MCP server exposing Grove's projects.

use std::path::{Path, PathBuf};

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{ServiceExt, schemars, tool, tool_router};
use serde::Deserialize;

use crate::config::Config;
use crate::model::Project;
use crate::store::Store;
use crate::tmux::Tmux;

/// Tool logic, separate from the rmcp wrappers so tests can call it directly.
/// `Err` text is returned to Claude as a tool error.
#[derive(Clone)]
pub struct GroveMcp {
    pub store: Store,
    pub cfg: Config,
    /// Grove's own tmux server, for sessions.
    #[allow(dead_code)] // read by the session tools (plan tasks 5 and 6)
    pub tmux: Tmux,
    /// The account every tool is limited to, or the error each one returns.
    pub scope: Result<String, String>,
    /// From `$GROVE_PROJECT`, set on every Grove session's window.
    pub default_project: Option<String>,
    /// From `$GROVE_SESSION`: this Claude's own session.
    #[allow(dead_code)] // read by send_prompt (plan task 6)
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
                format!("- {} — {} — {summary}", p.name, p.path.display())
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
        Ok(format!(
            "Name: {}\nPath: {}\nColor: {}\nSummary: {}",
            p.name,
            p.path.display(),
            p.color.as_deref().unwrap_or("(none)"),
            p.summary.as_deref().unwrap_or("(none)"),
        ))
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
#[allow(dead_code)] // read by grove doctor (plan task 7)
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
