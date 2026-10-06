//! The dashboard event loop and the side effects behind each `Action`.

use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use ratatui::crossterm::event::{self, Event};

use super::app::{Action, App, mark_missing};
use super::input::handle_key;
use super::render::render;
use crate::config::{Config, editor, expand_tilde};
use crate::launch;
use crate::model::{Project, State};
use crate::store::Store;
use crate::tmux::Tmux;

pub struct Ctx {
    pub store: Store,
    pub cfg: Config,
    pub tmux: Tmux,
}

/// What happened, for the footer.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Nothing,
    Quit,
    Message(String),
}

const NEEDS_YOU_STYLE: &str = "bg=red,fg=white,bold";
const RECONCILE_EVERY: Duration = Duration::from_secs(2);

pub fn perform(ctx: &Ctx, action: Action) -> Result<Outcome> {
    let store = &ctx.store;
    Ok(match action {
        Action::None => Outcome::Nothing,
        Action::Quit => Outcome::Quit,
        Action::NewSession { slug, name } => {
            let project = store.get_project(&slug)?;
            let s = launch::start(&ctx.tmux, store, &ctx.cfg, &project, name)?;
            if let Some(w) = &s.tmux_window {
                ctx.tmux.select_window(w)?;
            }
            Outcome::Message(format!("Started {} in {}", s.name, project.name))
        }
        Action::AddProject { path } => {
            Outcome::Message(format!("Added {}", add_project_at(store, &path)?))
        }
        Action::Jump { window } => {
            ctx.tmux.select_window(&window)?;
            Outcome::Nothing
        }
        Action::Kill { session_id } => {
            let Some(mut s) = store.get_session(&session_id) else {
                return Ok(Outcome::Nothing);
            };
            if let Some(w) = &s.tmux_window {
                // Already gone is fine: the goal is "not running".
                let _ = ctx.tmux.kill_window(w);
            }
            s.state = State::Ended;
            s.message = None;
            s.updated_at = chrono::Utc::now();
            store.save_session(&s)?;
            Outcome::Message(format!("Killed {}", s.name))
        }
        Action::EditProject { slug } => {
            let file = store.paths.projects.join(format!("{slug}.toml"));
            open_in_editor(ctx, &file.to_string_lossy())?;
            Outcome::Nothing
        }
        Action::OpenWiki { slug } => {
            let dir = store.paths.wiki.join(&slug);
            std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
            open_in_editor(ctx, &dir.to_string_lossy())?;
            Outcome::Nothing
        }
        Action::ClearEnded { slug } => {
            let ended: Vec<_> = store
                .list_sessions(Some(&slug))
                .into_iter()
                .filter(|s| s.state == State::Ended)
                .collect();
            for s in &ended {
                store.delete_session(&s.id)?;
            }
            Outcome::Message(format!("Cleared {} ended session(s)", ended.len()))
        }
    })
}

fn open_in_editor(ctx: &Ctx, target: &str) -> Result<()> {
    let mut argv: Vec<String> = editor(&ctx.cfg)
        .split_whitespace()
        .map(str::to_string)
        .collect();
    argv.push(target.to_string());
    ctx.tmux.display_popup(&ctx.store.paths.home, &argv)
}

/// Registers the directory at `path` as a project named after the folder.
pub fn add_project_at(store: &Store, path: &str) -> Result<String> {
    let dir = expand_tilde(path.trim());
    let dir = dir
        .canonicalize()
        .with_context(|| format!("{} does not exist", dir.display()))?;
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "root".into());
    store.add_project(&Project {
        name: name.clone(),
        path: dir,
        color: None,
        summary: None,
    })?;
    Ok(name)
}

/// Ends sessions whose window vanished and flags windows of sessions that
/// need you. `flagged` remembers which windows are currently flagged.
/// Errors (leaving sessions untouched) if the tmux session can't be listed,
/// so a tmux hiccup never marks everything ended.
pub fn reconcile(ctx: &Ctx, flagged: &mut HashMap<String, bool>) -> Result<()> {
    let windows = ctx.tmux.list_windows(&ctx.cfg.tmux_session)?;
    let sessions = ctx.store.list_sessions(None);
    for s in mark_missing(&sessions, &windows, chrono::Utc::now()) {
        ctx.store.save_session(&s)?;
        if let Some(w) = &s.tmux_window {
            flagged.remove(w);
        }
    }
    for s in ctx.store.list_sessions(None) {
        let Some(w) = s.tmux_window.as_ref().filter(|w| windows.contains(w)) else {
            continue;
        };
        let want = s.state == State::NeedsYou;
        if flagged.get(w) == Some(&want) {
            continue;
        }
        let style = if want {
            NEEDS_YOU_STYLE.to_string()
        } else {
            match ctx.store.get_project(&s.project).ok().and_then(|p| p.color) {
                Some(c) => format!("fg={c}"),
                None => "default".to_string(),
            }
        };
        ctx.tmux
            .set_window_option(w, "window-status-style", &style)?;
        flagged.insert(w.clone(), want);
    }
    Ok(())
}

/// Runs the dashboard until the process is killed. `q` detaches the tmux
/// client rather than exiting, so window 0 always has the dashboard.
pub fn run_dashboard(ctx: Ctx) -> Result<()> {
    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |_res| {
        let _ = tx.send(());
    })?;
    use notify::Watcher;
    ctx.store.paths.ensure()?;
    watcher.watch(
        &ctx.store.paths.sessions,
        notify::RecursiveMode::NonRecursive,
    )?;
    watcher.watch(
        &ctx.store.paths.projects,
        notify::RecursiveMode::NonRecursive,
    )?;

    let cwd = std::env::current_dir()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut app = App::new(cwd);
    app.reload(&ctx.store);

    let mut terminal = ratatui::init();
    let result = event_loop(&ctx, &mut app, &mut terminal, &rx);
    ratatui::restore();
    result
}

fn event_loop(
    ctx: &Ctx,
    app: &mut App,
    terminal: &mut ratatui::DefaultTerminal,
    rx: &mpsc::Receiver<()>,
) -> Result<()> {
    let mut flagged = HashMap::new();
    let mut last_reconcile = Instant::now() - RECONCILE_EVERY;
    loop {
        if last_reconcile.elapsed() >= RECONCILE_EVERY {
            if let Err(e) = reconcile(ctx, &mut flagged) {
                app.footer = Some(format!("tmux: {e:#}"));
            }
            app.reload(&ctx.store);
            last_reconcile = Instant::now();
        }
        terminal.draw(|f| render(f, app, chrono::Utc::now()))?;

        if event::poll(Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
        {
            let action = handle_key(app, key);
            match perform(ctx, action) {
                Ok(Outcome::Quit) => {
                    let _ = ctx.tmux.cmd().arg("detach-client").status();
                }
                Ok(Outcome::Message(m)) => app.footer = Some(m),
                Ok(Outcome::Nothing) => {}
                Err(e) => app.footer = Some(format!("{e:#}")),
            }
            app.reload(&ctx.store);
        }
        // Hook writes land here; coalesce bursts into one reload.
        if rx.try_recv().is_ok() {
            std::thread::sleep(Duration::from_millis(150));
            while rx.try_recv().is_ok() {}
            app.reload(&ctx.store);
        }
    }
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
