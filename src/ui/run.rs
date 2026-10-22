//! The dashboard event loop and the side effects behind each `Action`.

use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use ratatui::crossterm::event::{self, Event};

use super::app::{Action, App, Mode, mark_missing};
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
const NEEDS_YOU_MARK: &str = "💧 ";
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
            let s = launch::start(&ctx.tmux, store, &cfg, &project, name, None)?;
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
        Action::FincaResume {
            slug,
            via,
            session_id,
        } => {
            let f = store.get_finca(&slug)?;
            let via = via.filter(|v| f.destinations().any(|d| d == v));
            let dest = via.as_deref().unwrap_or(&f.ssh);
            let window = crate::screens::resume_session_remote(&f, dest, &session_id)
                .map_err(|e| anyhow::anyhow!("{}: {e}", f.name))?;
            return perform(ctx, Action::FincaJump { slug, via, window });
        }
        Action::Resume { session_id } => {
            let Some(old) = store.get_session(&session_id) else {
                return Ok(Outcome::Nothing);
            };
            let project = store.get_project(&old.project)?;
            let cfg = load_config(&store.paths)?;
            let s = launch::resume(&ctx.tmux, store, &cfg, &project, &session_id)?;
            if let Some(w) = &s.tmux_window {
                ctx.tmux.select_window(w)?;
            }
            Outcome::Message(format!("Resumed {} in {}", s.name, project.name))
        }
        Action::Kill { session_id } => {
            let name = store.get_session(&session_id).map(|s| s.name);
            match crate::launch::kill(&ctx.tmux, store, &session_id)? {
                Some(_) => Outcome::Message(format!("Killed {}", name.unwrap_or_default())),
                None => Outcome::Nothing,
            }
        }
        Action::SavePins { pins } => {
            crate::pins::save(&store.paths, &pins)?;
            Outcome::Nothing
        }
        Action::GridKill {
            finca,
            session_id,
            handle: _,
        } => {
            let killed = match finca {
                None => crate::launch::kill(&ctx.tmux, store, &session_id)?
                    .ok_or_else(|| "that session has ended".to_string()),
                Some((slug, via)) => {
                    let f = store.get_finca(&slug)?;
                    let dest = via.unwrap_or_else(|| f.ssh.clone());
                    crate::screens::kill_session_remote(&f, &dest, &session_id)
                }
            };
            match killed {
                Ok(h) => Outcome::Message(format!("Killed {h}")),
                Err(e) => anyhow::bail!(e),
            }
        }
        Action::EditProject { slug } => {
            let file = store.paths.projects.join(format!("{slug}.toml"));
            open_in_editor(ctx, &file.to_string_lossy())?;
            Outcome::Nothing
        }
        Action::ProjectShell { slug } => {
            let project = store.get_project(&slug)?;
            if !project.path.is_dir() {
                bail!("{} doesn't exist", project.path.display());
            }
            let argv = shell_argv(std::env::var("SHELL").ok().as_deref());
            ctx.tmux
                .display_popup(&project.path, Some(&project.name), &argv)?;
            Outcome::Nothing
        }
        Action::NewShell { slug } => {
            let project = store.get_project(&slug)?;
            if !project.path.is_dir() {
                bail!("{} doesn't exist", project.path.display());
            }
            let shell = std::env::var("SHELL")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "/bin/sh".into());
            let sh = crate::shells::open(&ctx.tmux, &ctx.cfg.tmux_session, &project, &[shell])?;
            ctx.tmux.select_window(&sh.id)?;
            Outcome::Nothing
        }
        Action::CloseShell { window, label } => {
            // Already gone (it exited meanwhile) is fine.
            let _ = ctx.tmux.kill_window(&window);
            Outcome::Message(format!("Closed {label}"))
        }
        Action::ConnectFinca { slug, via } => {
            let f = store.get_finca(&slug)?;
            // A stale via (backup since removed from the file) falls back to ssh.
            let via = via.filter(|v| f.destinations().any(|d| d == v));
            crate::finca::connect(&ctx.tmux, &f, via.as_deref())?;
            Outcome::Nothing
        }
        Action::FincaJump { slug, via, window } => {
            let f = store.get_finca(&slug)?;
            let via = via.filter(|v| f.destinations().any(|d| d == v));
            // Still connect if the window can't be selected (it just ended):
            // the finca's dashboard is the next best place.
            let missed = crate::finca::focus(&f, via.as_deref().unwrap_or(&f.ssh), &window).err();
            crate::finca::connect(&ctx.tmux, &f, via.as_deref())?;
            match missed {
                Some(e) => Outcome::Message(format!("{e:#}")),
                None => Outcome::Nothing,
            }
        }
        Action::SetupFinca { dest } => {
            let exe = crate::runfiles::grove_exe()?;
            let argv = vec![
                exe.to_string_lossy().into_owned(),
                "finca".into(),
                "setup".into(),
                dest,
                "--pause".into(),
            ];
            ctx.tmux.display_popup(&ctx.store.paths.home, None, &argv)?;
            Outcome::Nothing
        }
        Action::FincaShellWindow { slug, window } => {
            ctx.tmux.select_window(&window)?;
            ctx.tmux
                .switch_client(&format!("={}", crate::finca::shell_session_name(&slug)))?;
            Outcome::Nothing
        }
        Action::FincaShell { slug, via } => {
            let f = store.get_finca(&slug)?;
            let via = via.filter(|v| f.destinations().any(|d| d == v));
            crate::finca::open_shell(&ctx.tmux, &f, via.as_deref())?;
            Outcome::Nothing
        }
        Action::RenameShell {
            window,
            finca,
            name,
        } => {
            let name = name.trim();
            let old = match finca {
                None => crate::shells::rename(&ctx.tmux, &ctx.cfg.tmux_session, &window, name)?,
                Some(slug) => {
                    let wins = crate::finca::shell_windows(&ctx.tmux, &slug);
                    let Some(w) = wins.iter().find(|w| w.id == window) else {
                        bail!("that shell has closed");
                    };
                    crate::finca::rename_shell(&ctx.tmux, &window, name)?;
                    w.label()
                }
            };
            Outcome::Message(format!("Renamed {old} to {name}"))
        }
        Action::RenameSession { session_id, name } => {
            Outcome::Message(rename_session(ctx, &session_id, &name)?)
        }
        Action::Update => {
            let exe = crate::runfiles::grove_exe()?;
            let cwd = ctx
                .tmux
                .show_environment(&ctx.cfg.tmux_session, "GROVE_CWD")
                .unwrap_or_else(|| store.paths.home.to_string_lossy().into_owned());
            // If the binary changed, this restarts the dashboard (ending
            // this process); otherwise it returns and we say so.
            let out = update_command(&exe, &cwd).output()?;
            if !out.status.success() {
                bail!("grove: {}", String::from_utf8_lossy(&out.stderr).trim());
            }
            Outcome::Message("Grove is up to date (run files and tmux config refreshed)".into())
        }
        Action::SaveTodos { slug, text } => {
            store.write_todos(&slug, &text)?;
            Outcome::Nothing
        }
        Action::GridReply {
            finca,
            session_id,
            handle: _,
            text,
        } => {
            let sent = match finca {
                None => {
                    let s = store
                        .get_session(&session_id)
                        .filter(|s| s.state != State::Ended)
                        .ok_or_else(|| anyhow::anyhow!("that session has ended"))?;
                    let cfg = load_config(&store.paths)?;
                    let account = store
                        .get_project(&s.project)
                        .map(|p| cfg.account_for(&p.path).name)
                        .unwrap_or_default();
                    crate::prompt::type_into(&ctx.tmux, &s, &account, &text)
                }
                Some((slug, via)) => {
                    let f = store.get_finca(&slug)?;
                    let dest = via.unwrap_or_else(|| f.ssh.clone());
                    crate::screens::type_prompt_remote(&f, &dest, &session_id, &text)
                }
            };
            match sent {
                Ok(h) => Outcome::Message(format!("sent to {h}")),
                Err(e) => anyhow::bail!(e),
            }
        }
        Action::EditTodos { slug } => {
            // The editor can't save into a folder that isn't there yet.
            std::fs::create_dir_all(&store.paths.todos)?;
            open_in_editor(ctx, &store.todos_file(&slug).to_string_lossy())?;
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

/// `grove` as if typed in `cwd`, in its own process group so it survives
/// the dashboard restart it may cause.
pub fn update_command(exe: &std::path::Path, cwd: &str) -> std::process::Command {
    use std::os::unix::process::CommandExt;
    let mut c = std::process::Command::new(exe);
    c.current_dir(cwd).process_group(0);
    c
}

/// This machine's name without its domain (`hostname -s`), or empty.
fn short_host() -> String {
    std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Your login shell ($SHELL), or sh, for the project-shell popup. It runs
/// without $TMUX: a popup can't hold panes, so tools that split one when
/// they see it (fzf-tmux, often aliased as fzf) would open behind the
/// popup, out of reach of the keyboard. GROVE_POPUP tells `grove` there
/// that it's already inside Grove.
pub(crate) fn shell_argv(shell: Option<&str>) -> Vec<String> {
    let shell = shell.filter(|s| !s.is_empty()).unwrap_or("/bin/sh");
    [
        "env",
        "-u",
        "TMUX",
        "-u",
        "TMUX_PANE",
        "GROVE_POPUP=1",
        shell,
    ]
    .map(str::to_string)
    .to_vec()
}

fn open_in_editor(ctx: &Ctx, target: &str) -> Result<()> {
    let mut argv: Vec<String> = editor(&ctx.cfg)
        .split_whitespace()
        .map(str::to_string)
        .collect();
    argv.push(target.to_string());
    ctx.tmux.display_popup(&ctx.store.paths.home, None, &argv)
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
    })?;
    Ok(name)
}

/// Forgets a project and its ended sessions. Refuses while one of its
/// sessions is still running in `session`'s windows; the folder is never
/// touched.
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
    let shells = crate::shells::list(tmux, session)
        .into_iter()
        .filter(|s| s.project == slug)
        .count();
    if shells > 0 {
        bail!(
            "{} has {shells} open shell(s); close them first (x)",
            p.name
        );
    }
    for s in &sessions {
        store.delete_session(&s.id)?;
    }
    store.remove_project(&slug)?;
    Ok(format!("Removed {} (its folder is kept)", p.name))
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
        ctx.tmux.rename_window(w, &window_title(&s))?;
        flagged.insert(w.clone(), want);
    }
    Ok(())
}

/// A session's tmux window name: "<project>:<session>" as launch names it,
/// with the needs-you marker in front (in the name, so it shows under any
/// status-bar theme).
pub fn window_title(s: &crate::model::Session) -> String {
    let base = format!("{}:{}", s.project, s.name);
    if s.state == State::NeedsYou {
        format!("{NEEDS_YOU_MARK}{base}")
    } else {
        base
    }
}

/// Renames a session and its window. Refuses a name another live session
/// of the project has (auto-naming relies on live names being unique).
pub fn rename_session(ctx: &Ctx, session_id: &str, name: &str) -> Result<String> {
    let name = name.trim();
    let Some(mut s) = ctx.store.get_session(session_id) else {
        bail!("no session {session_id}");
    };
    let taken = ctx
        .store
        .list_sessions(Some(&s.project))
        .into_iter()
        .any(|o| o.id != s.id && o.state != State::Ended && o.name == name);
    if taken {
        bail!(
            "another live session in {} is already called {name}",
            s.project
        );
    }
    let old = std::mem::replace(&mut s.name, name.to_string());
    s.updated_at = chrono::Utc::now();
    ctx.store.save_session(&s)?;
    if let Some(w) = &s.tmux_window {
        // Already gone is fine: there's no window to rename.
        let _ = ctx.tmux.rename_window(w, &window_title(&s));
    }
    Ok(format!("Renamed {old} to {name}"))
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
/// Open shells: the projects' (`S`) and each finca's, for their panes.
pub fn refresh_shells(ctx: &Ctx, app: &mut App) {
    app.shells = crate::shells::list(&ctx.tmux, &ctx.cfg.tmux_session);
    app.finca_shells = app
        .fincas
        .iter()
        .map(|f| {
            (
                f.slug.clone(),
                crate::finca::shell_windows(&ctx.tmux, &f.slug),
            )
        })
        .filter(|(_, w)| !w.is_empty())
        .collect();
}

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
    app.host = short_host();
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
    // The grid's finca screens, fetched only while it's open.
    let mut fetcher: Option<crate::screens::Fetcher> = None;
    let mut last_capture = Instant::now() - Duration::from_secs(1);
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
            refresh_shells(ctx, app);
            app.reload(&ctx.store);
            last_reconcile = Instant::now();
        }
        refresh_grid(ctx, app, &mut fetcher, &mut last_capture);
        terminal.draw(|f| render(f, app, chrono::Utc::now()))?;

        if event::poll(Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
        {
            let action = handle_key(app, key);
            let was_reply = matches!(action, Action::GridReply { .. });
            let was_kill = matches!(action, Action::GridKill { .. });
            if matches!(action, Action::FincaResume { .. }) {
                app.footer = Some("resuming…".into());
                terminal.draw(|f| render(f, app, chrono::Utc::now()))?;
            }
            let was_shell = matches!(action, Action::NewShell { .. } | Action::CloseShell { .. });
            if was_reply || was_kill {
                app.footer = Some(
                    if was_reply {
                        "sending…"
                    } else {
                        "killing…"
                    }
                    .into(),
                );
                terminal.draw(|f| render(f, app, chrono::Utc::now()))?;
            }
            match perform(ctx, action) {
                Ok(Outcome::Quit) => {
                    let _ = ctx.tmux.cmd().arg("detach-client").status();
                }
                Ok(Outcome::Message(m)) => {
                    // Sent: close the reply line (a failed send keeps it).
                    if was_reply && let Mode::Grid { sel, .. } = app.mode {
                        app.mode = Mode::Grid { sel, reply: None };
                    }
                    app.footer = Some(m)
                }
                Ok(Outcome::Nothing) => {}
                Err(e) => app.footer = Some(format!("{e:#}")),
            }
            // Keys typed while a send was out (a slow finca) would land on
            // whatever is selected now: a second Enter could resend or jump.
            if was_reply || was_kill {
                while event::poll(Duration::ZERO)? {
                    let _ = event::read()?;
                }
            }
            if was_shell {
                refresh_shells(ctx, app);
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

/// While the grid is open: local screens every 500 ms, and a fetcher for
/// the fincas' (started on opening, dropped on closing).
fn refresh_grid(
    ctx: &Ctx,
    app: &mut App,
    fetcher: &mut Option<crate::screens::Fetcher>,
    last_capture: &mut Instant,
) {
    const LINES: usize = 60;
    let in_grid = matches!(app.mode, Mode::Grid { .. });
    if !in_grid {
        if fetcher.take().is_some() {
            app.local_screens.clear();
        }
        return;
    }
    // Reachable fincas, at the destination that last answered; refreshed
    // each pass, so one that comes back (or answers elsewhere) is picked up.
    let targets: Vec<_> = app
        .fincas
        .iter()
        .filter(|r| matches!(r.reach, Some(crate::finca::Reach::Reachable(_))))
        .filter_map(|r| Some((ctx.store.get_finca(&r.slug).ok()?, r.via.clone()?)))
        .collect();
    match fetcher {
        Some(f) => f.set_targets(targets),
        None => {
            *fetcher = Some(crate::screens::Fetcher::start(
                targets,
                LINES,
                Duration::from_secs(2),
                Duration::from_secs(5),
            ))
        }
    }
    if let Some(f) = fetcher {
        while let Ok(r) = f.results.try_recv() {
            let newly_old = matches!(r.fetched, crate::screens::Fetched::TooOld)
                && !app.too_old.contains(&r.slug);
            let slug = r.slug.clone();
            app.apply_screens(r);
            if newly_old {
                let name = app
                    .fincas
                    .iter()
                    .find(|f| f.slug == slug)
                    .map_or(slug.clone(), |f| f.name.clone());
                app.footer = Some(format!("update Grove on {name} for live screens"));
            }
        }
    }
    if last_capture.elapsed() >= Duration::from_millis(500) {
        for s in app.sessions.iter().filter(|s| s.state != State::Ended) {
            let screen = s
                .tmux_window
                .as_deref()
                .and_then(|w| ctx.tmux.capture(w, LINES).ok())
                .map(|t| crate::screens::tail(&t, LINES))
                .unwrap_or_else(|| "(window gone)".into());
            app.local_screens.insert(s.id.clone(), screen);
        }
        *last_capture = Instant::now();
    }
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
