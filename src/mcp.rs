//! `grove mcp`: stdio MCP server exposing Grove's projects.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{ServiceExt, schemars, tool, tool_router};
use serde::Deserialize;

use crate::model::Project;
use crate::store::Store;

/// Tool logic, separate from the rmcp wrappers so tests can call it directly.
/// `Err` text is returned to Claude as a tool error.
#[derive(Clone)]
pub struct GroveMcp {
    pub store: Store,
    /// From `$GROVE_PROJECT`, set on every Grove session's window.
    pub default_project: Option<String>,
}

impl GroveMcp {
    fn project(&self, project: Option<String>) -> Result<Project, String> {
        let name = project
            .filter(|p| !p.trim().is_empty())
            .or_else(|| self.default_project.clone());
        match name {
            Some(n) => self.store.get_project(&n).map_err(|e| e.to_string()),
            None => {
                let names: Vec<String> = self
                    .store
                    .list_projects()
                    .into_iter()
                    .filter_map(Result::ok)
                    .map(|p| p.name)
                    .collect();
                Err(format!(
                    "no project given and this isn't a Grove session; pass `project` (one of: {})",
                    if names.is_empty() {
                        "(none)".into()
                    } else {
                        names.join(", ")
                    }
                ))
            }
        }
    }

    pub fn do_list_projects(&self) -> Result<String, String> {
        let lines: Vec<String> = self
            .store
            .list_projects()
            .into_iter()
            .filter_map(Result::ok)
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

pub async fn serve(store: Store) -> anyhow::Result<()> {
    let default_project = std::env::var("GROVE_PROJECT")
        .ok()
        .filter(|p| !p.is_empty());
    let service = GroveMcp {
        store,
        default_project,
    }
    .serve(rmcp::transport::stdio())
    .await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
