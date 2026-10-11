//! The dashboard event loop and the side effects behind each `Action`.

use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use ratatui::crossterm::event::{self, Event};

use super::app::{Action, App, mark_missing};
use super::input::handle_key;
use super::render::render;
use crate::config::{Config, editor, expand_tilde, load_config};
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

/// Prefix on the tmux window name of a session that needs you.
const NEEDS_YOU_MARK: &str = "⚑ ";
const RECONCILE_EVERY: Duration = Duration::from_secs(2);
const FINCA_POLL_EVERY: Duration = Duration::from_secs(30);
const FINCA_POLL_TIMEOUT: Duration = Duration::from_secs(10);

pub fn perform(ctx: &Ctx, action: Action) -> Result<Outcome> {
    let store = &ctx.store;
    Ok(match action {
        Action::None => Outcome::Nothing,
        Action::Quit => Outcome::Quit,
        Action::NewSession { slug, name } => {
            let project = store.get_project(&slug)?;
            // Re-read so config.toml edits apply without restarting the dashboard.
            let cfg = load_config(&store.paths)?;
            let s = launch::start(&ctx.tmux, store, &cfg, &project, name)?;
            if let Some(w) = &s.tmux_window {
                ctx.tmux.select_window(w)?;
            }
            Outcome::Message(format!("Started {} in {}", s.name, project.name))
        }
        Action::CreateProject { path } => Outcome::Message(format!(
            "Created {} (git init)",
            create_project_at(store, &path)?
        )),
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
        Action::ConnectFinca { slug, via } => {
            let f = store.get_finca(&slug)?;
            // A stale via (backup since removed from the file) falls back to ssh.
            let via = via.filter(|v| f.destinations().any(|d| d == v));
            crate::finca::connect(&ctx.tmux, &f, via.as_deref())?;
            Outcome::Nothing
        }
        Action::RemoveProject { slug } => Outcome::Message(remove_project(
            store,
            &ctx.tmux,
            &ctx.cfg.tmux_session,
            &slug,
        )?),
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

/// Creates the folder at `path` (and any missing parents), runs `git init`
/// in it, and registers it as a project. Refuses an existing path.
pub fn create_project_at(store: &Store, path: &str) -> Result<String> {
    let dir = expand_tilde(path.trim());
    if dir.exists() {
        bail!(
            "{} already exists; use add (a) for an existing folder",
            dir.display()
        );
    }
    // Remember the topmost folder we create, to undo cleanly if git fails.
    let mut first_new = dir.clone();
    while let Some(parent) = first_new.parent() {
        if parent.exists() {
            break;
        }
        first_new = parent.to_path_buf();
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let git = std::process::Command::new("git")
        .args(["init", "-q"])
        .arg(&dir)
        .output();
    match git {
        Ok(o) if o.status.success() => {}
        other => {
            let _ = std::fs::remove_dir_all(&first_new);
            let why = match other {
                Ok(o) => String::from_utf8_lossy(&o.stderr).trim().to_string(),
                Err(e) => e.to_string(),
            };
            bail!("git init failed in {}: {why}", dir.display());
        }
    }
    add_project_at(store, &dir.to_string_lossy())
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
        account: None,
    })?;
    Ok(name)
}

/// Forgets a project and its ended sessions. Refuses while one of its
/// sessions is still running in `session`'s windows; the folder and wiki are
/// never touched, so adding it back brings its wiki back.
pub fn remove_project(store: &Store, tmux: &Tmux, session: &str, project: &str) -> Result<String> {
    let p = store.get_project(project)?;
    let slug = p.slug();
    // No tmux server (e.g. after a reboot) means nothing is running.
    let windows = tmux.list_windows(session).unwrap_or_default();
    let sessions = store.list_sessions(Some(&slug));
    let running = sessions
        .iter()
        .filter(|s| s.state != State::Ended)
        .filter(|s| s.tmux_window.as_ref().is_some_and(|w| windows.contains(w)))
        .count();
    if running > 0 {
        bail!(
            "{} has {running} running session(s); kill them first (x)",
            p.name
        );
    }
    for s in &sessions {
        store.delete_session(&s.id)?;
    }
    store.remove_project(&slug)?;
    Ok(format!("Removed {} (its folder and wiki are kept)", p.name))
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
        // The marker goes in the window name (launch names windows
        // "<project>:<session>") so it shows under any status-bar theme.
        let base = format!("{}:{}", s.project, s.name);
        let name = if want {
            format!("{NEEDS_YOU_MARK}{base}")
        } else {
            base
        };
        ctx.tmux.rename_window(w, &name)?;
        flagged.insert(w.clone(), want);
    }
    Ok(())
}

/// Every session the dashboard knows of, local and on fincas, for alerts.
pub fn observe(app: &App) -> Vec<crate::alert::Observed> {
    let project_name = |slug: &str| {
        app.projects
            .iter()
            .find(|p| p.slug == slug)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| slug.to_string())
    };
    let local = app.sessions.iter().map(|s| crate::alert::Observed {
        key: s.id.clone(),
        source: "local".into(),
        state: s.state,
        window: s.tmux_window.clone(),
        label: format!("{}:{}", project_name(&s.project), s.name),
        message: s.message.clone(),
    });
    let remote = app.fincas.iter().flat_map(|f| {
        let st = f.last_status.iter();
        st.flat_map(move |st| {
            st.projects.iter().flat_map(move |p| {
                p.sessions.iter().map(move |s| crate::alert::Observed {
                    key: format!("{}/{}/{}", f.slug, p.slug, s.name),
                    source: f.slug.clone(),
                    state: s.state,
                    window: None,
                    label: format!("{} · {}:{}", f.name, p.name, s.name),
                    message: s.message.clone(),
                })
            })
        })
    });
    local.chain(remote).collect()
}

/// Updates the add-project default from the `GROVE_CWD` that `grove` sets on
/// the tmux session each time it attaches.
pub fn refresh_cwd(ctx: &Ctx, app: &mut App) {
    if let Some(cwd) = ctx
        .tmux
        .show_environment(&ctx.cfg.tmux_session, "GROVE_CWD")
    {
        app.cwd = cwd;
    }
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
    // Separate watcher for fincas: only their changes should trigger an
    // immediate re-poll (session files change constantly).
    let (ftx, frx) = mpsc::channel::<()>();
    let mut finca_watcher = notify::recommended_watcher(move |_res| {
        let _ = ftx.send(());
    })?;
    finca_watcher.watch(&ctx.store.paths.fincas, notify::RecursiveMode::NonRecursive)?;
    let poller =
        crate::finca::Poller::start(ctx.store.clone(), FINCA_POLL_EVERY, FINCA_POLL_TIMEOUT);

    let cwd = std::env::current_dir()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut app = App::new(cwd);
    app.cfg = ctx.cfg.clone();
    app.reload(&ctx.store);

    let mut terminal = ratatui::init();
    let result = event_loop(&ctx, &mut app, &mut terminal, &rx, &frx, &poller);
    ratatui::restore();
    result
}

fn event_loop(
    ctx: &Ctx,
    app: &mut App,
    terminal: &mut ratatui::DefaultTerminal,
    rx: &mpsc::Receiver<()>,
    finca_rx: &mpsc::Receiver<()>,
    poller: &crate::finca::Poller,
) -> Result<()> {
    let mut flagged = HashMap::new();
    let mut alerts = crate::alert::Alerts::default();
    let mut last_reconcile = Instant::now() - RECONCILE_EVERY;
    loop {
        if ctx.cfg.alerts {
            let watched = || ctx.tmux.focused_windows();
            for a in alerts.check(observe(app), watched) {
                crate::alert::send(&a);
            }
        }
        if last_reconcile.elapsed() >= RECONCILE_EVERY {
            if let Err(e) = reconcile(ctx, &mut flagged) {
                app.footer = Some(format!("tmux: {e:#}"));
            }
            refresh_cwd(ctx, app);
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
        while let Ok(r) = poller.results.try_recv() {
            app.apply_poll(r);
        }
        if finca_rx.try_recv().is_ok() {
            while finca_rx.try_recv().is_ok() {}
            app.reload(&ctx.store);
            poller.kick();
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
