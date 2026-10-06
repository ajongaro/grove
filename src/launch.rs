//! Builds and starts a Claude Code session in its own tmux window.

use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::config::{Config, Paths, claude_dir};
use crate::model::{Project, Session, State};
use crate::store::Store;
use crate::tmux::Tmux;

#[derive(Debug, Clone, PartialEq)]
pub struct LaunchSpec {
    pub window_name: String,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub argv: Vec<String>,
}

/// `claude` resolved to an absolute path via PATH, else plain "claude".
/// Resolving here means the window doesn't depend on the tmux server's PATH.
pub fn claude_bin() -> String {
    std::env::var_os("PATH")
        .and_then(|p| {
            std::env::split_paths(&p)
                .map(|d| d.join("claude"))
                .find(|c| c.is_file())
        })
        .map(|c| c.to_string_lossy().into_owned())
        .unwrap_or_else(|| "claude".into())
}

fn build_with_bin(
    cfg: &Config,
    paths: &Paths,
    project: &Project,
    session_id: &str,
    name: &str,
    bin: &str,
) -> Result<LaunchSpec> {
    let account = claude_dir(cfg)?;
    if !project.path.is_dir() {
        bail!(
            "project directory {} does not exist",
            project.path.display()
        );
    }
    let slug = project.slug();
    let window_name = format!("{slug}:{name}");
    let lossy = |p: PathBuf| p.to_string_lossy().into_owned();
    let mut argv = vec![
        bin.to_string(),
        "--session-id".into(),
        session_id.into(),
        "--settings".into(),
        lossy(paths.run.join("hooks.json")),
        "--mcp-config".into(),
        lossy(paths.run.join("mcp.json")),
        "-n".into(),
        window_name.clone(),
    ];
    argv.extend(cfg.claude_args.iter().cloned());
    Ok(LaunchSpec {
        window_name,
        cwd: project.path.clone(),
        env: vec![
            ("CLAUDE_CONFIG_DIR".into(), lossy(account)),
            ("GROVE_HOME".into(), lossy(paths.home.clone())),
            ("GROVE_PROJECT".into(), slug),
            ("GROVE_SESSION".into(), session_id.into()),
        ],
        argv,
    })
}

/// `s1`, `s2`, … — the lowest number not used by a live session.
pub fn next_session_name(store: &Store, slug: &str) -> String {
    let taken: Vec<String> = store
        .list_sessions(Some(slug))
        .into_iter()
        .filter(|s| s.state != State::Ended)
        .map(|s| s.name)
        .collect();
    (1..)
        .map(|n| format!("s{n}"))
        .find(|n| !taken.contains(n))
        .expect("unbounded range")
}

/// Records a `starting` session, opens its window, and stores the window id.
pub fn start(
    tmux: &Tmux,
    store: &Store,
    cfg: &Config,
    project: &Project,
    name: Option<String>,
) -> Result<Session> {
    start_with_bin(tmux, store, cfg, project, name, &claude_bin())
}

/// `start` with the program to run swapped out (tests use a stand-in).
pub fn start_with_bin(
    tmux: &Tmux,
    store: &Store,
    cfg: &Config,
    project: &Project,
    name: Option<String>,
    bin: &str,
) -> Result<Session> {
    let slug = project.slug();
    let name = name
        .map(|n| n.trim().replace([':', '.'], "-"))
        .filter(|n| !n.is_empty());
    let name = name.unwrap_or_else(|| next_session_name(store, &slug));
    let id = uuid::Uuid::new_v4().to_string();
    // Validate everything before touching disk or tmux.
    let spec = build_with_bin(cfg, &store.paths, project, &id, &name, bin)?;

    let now = chrono::Utc::now();
    let mut session = Session {
        id,
        project: slug,
        name,
        host: "local".into(),
        tmux_window: None,
        state: State::Starting,
        message: None,
        created_at: now,
        updated_at: now,
    };
    // Written before the window exists so the first hook finds it.
    store.save_session(&session)?;
    let window = match tmux.new_window(
        &cfg.tmux_session,
        &spec.window_name,
        &spec.cwd,
        &spec.env,
        &spec.argv,
    ) {
        Ok(w) => w,
        Err(e) => {
            let _ = store.delete_session(&session.id);
            return Err(e);
        }
    };
    if let Some(color) = &project.color {
        let _ = tmux.set_window_option(&window, "window-status-style", &format!("fg={color}"));
    }
    // Re-read: a hook may already have moved the state on.
    if let Some(latest) = store.get_session(&session.id) {
        session = latest;
    }
    session.tmux_window = Some(window);
    store.save_session(&session)?;
    Ok(session)
}

#[cfg(test)]
#[path = "launch_tests.rs"]
mod tests;
