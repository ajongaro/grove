mod alert;
mod config;
mod doctor;
mod finca;
mod hook;
mod launch;
mod log;
mod mcp;
mod mcp_overview;
mod model;
mod pins;
mod prompt;
mod runfiles;
mod screens;
mod setup;
mod shells;
mod state;
mod stats;
mod status;
mod statusline;
mod store;
mod ticker;
mod titles;
mod tmux;
mod todo;
mod transfer;
mod ui;

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use crate::config::{Config, Paths, claude_dir, load_config};
use crate::store::Store;
use crate::tmux::Tmux;

/// Grove's own tmux server, so its config never touches your default one.
const TMUX_SOCKET: &str = "grove";

/// `$GROVE_TMUX_SOCKET` (tests) or Grove's own server.
fn tmux_socket() -> String {
    std::env::var("GROVE_TMUX_SOCKET")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| TMUX_SOCKET.into())
}

#[derive(Parser)]
#[command(
    name = "grove",
    about = "A tmux dashboard for Claude Code sessions",
    version
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Register a project directory (default: the current directory).
    Add {
        path: Option<PathBuf>,
        /// Create the folder (and git init it) if it doesn't exist, without asking.
        #[arg(long)]
        new: bool,
    },
    /// Forget a project. Its folder is kept.
    Rm { project: String },
    /// Check tmux, claude, accounts, projects and fincas; lists every problem.
    Doctor,
    /// Manage fincas: other machines running their own Grove.
    #[command(subcommand)]
    Finca(FincaCmd),
    /// Print this machine's sessions (used by other Groves to poll fincas).
    Status {
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Internal: make a window current in the Grove session, so the next
    /// attach (another Grove connecting to this finca) lands in it.
    #[command(hide = true)]
    Focus { window: String },
    /// Internal: C-v in a finca window (sends the clipboard, pastes the path).
    #[command(hide = true)]
    Paste {
        #[arg(long)]
        session: String,
        #[arg(long)]
        pane: String,
    },
    /// Send a file (or this Mac's clipboard) to a finca's ~/.grove/inbox;
    /// prints the path there.
    Send {
        finca: String,
        file: Option<PathBuf>,
    },
    /// Internal: the dashboard that runs in tmux window 0.
    #[command(hide = true)]
    Dashboard,
    /// Internal: live sessions with their screens, for the session grid.
    #[command(hide = true)]
    Screens {
        #[arg(long)]
        json: bool,
        /// Lines of each screen (most recent), at most 200.
        #[arg(long, default_value_t = 40)]
        lines: usize,
    },
    /// Internal: type stdin into a waiting session (the session grid, from
    /// another machine).
    #[command(hide = true)]
    TypePrompt { session: String },
    /// Internal: kill a live session (the session grid, from another
    /// machine).
    #[command(hide = true)]
    KillSession { session: String },
    /// Internal: resume an ended session and print its window (finding a
    /// session, from another machine).
    #[command(hide = true)]
    ResumeSession { session: String },
    /// Internal: called by Claude Code hooks with the event JSON on stdin.
    #[command(hide = true)]
    Hook,
    /// Internal: Claude Code's status line for Grove's sessions.
    #[command(hide = true)]
    Statusline,
    /// MCP server giving Claude Grove's projects, sessions and to-dos.
    /// `grove mcp setup` prints how to add it to each Claude account.
    Mcp {
        /// The Claude account these tools are for (a name from config.toml).
        #[arg(long)]
        account: Option<String>,
        #[command(subcommand)]
        cmd: Option<McpCmd>,
    },
}

#[derive(Subcommand)]
enum McpCmd {
    /// Print the command that adds Grove's tools to each Claude account.
    Setup,
}

#[derive(Subcommand)]
enum FincaCmd {
    /// Register a machine reachable over key-based SSH that has grove installed.
    Add {
        name: String,
        /// SSH destination, e.g. user@host or an ~/.ssh/config alias.
        ssh: String,
        /// grove's path on that machine, if not in a usual place.
        #[arg(long)]
        bin: Option<String>,
        /// SSH destination to fall back to when the first doesn't answer,
        /// e.g. a Tailscale name for when you're away from home.
        #[arg(long)]
        backup: Option<String>,
    },
    /// Forget a finca.
    Rm { name: String },
    /// List fincas.
    Ls,
    /// Install this build of grove on fincas (all, or the named ones).
    Update { names: Vec<String> },
    /// Set up a machine as a finca, step by step (asks before each change).
    Setup {
        /// SSH destination, e.g. me@homebox.local; asked for if left out.
        dest: Option<String>,
        /// Wait for Enter before exiting (the dashboard's popup).
        #[arg(long, hide = true)]
        pause: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    let paths = Paths::from_env();
    match cli.cmd {
        Some(Cmd::Hook) => run_hook(paths),
        Some(Cmd::Statusline) => run_statusline(paths),
        Some(Cmd::Mcp {
            cmd: Some(McpCmd::Setup),
            ..
        }) => exit_on_err(mcp_setup(&paths)),
        Some(Cmd::Mcp { account, cmd: None }) => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            let tmux = grove_tmux(&paths);
            if let Err(e) = rt.block_on(mcp::serve(Store::new(paths.clone()), tmux, account)) {
                log::error(&paths, "mcp", &format!("{e:#}"));
                std::process::exit(1);
            }
        }
        Some(Cmd::Add { path, new }) => exit_on_err(add(&paths, path, new)),
        Some(Cmd::Rm { project }) => exit_on_err(rm(&paths, &project)),
        Some(Cmd::Doctor) => exit_on_err(doctor(&paths)),
        Some(Cmd::Status { json }) => print_status(&paths, json),
        Some(Cmd::TypePrompt { session }) => {
            let mut text = String::new();
            let _ = std::io::stdin().read_to_string(&mut text);
            match type_prompt(&paths, &session, &text) {
                Ok(handle) => println!("{handle}"),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Some(Cmd::KillSession { session }) => {
            let store = Store::new(paths.clone());
            match launch::kill(&grove_tmux(&paths), &store, &session) {
                Ok(Some(handle)) => println!("{handle}"),
                Ok(None) => {
                    eprintln!("no live session {session}");
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("{e:#}");
                    std::process::exit(1);
                }
            }
        }
        Some(Cmd::ResumeSession { session }) => match resume_session(&paths, &session) {
            Ok(window) => println!("{window}"),
            Err(e) => {
                eprintln!("{e:#}");
                std::process::exit(1);
            }
        },
        Some(Cmd::Screens { lines, .. }) => {
            let st = screens::local(
                &Store::new(paths.clone()),
                &grove_tmux(&paths),
                lines.min(200),
                &status::hostname(),
            );
            println!("{}", serde_json::to_string(&st).expect("screens serialize"));
        }
        Some(Cmd::Paste { session, pane }) => {
            let _ = paths.ensure();
            transfer::paste(&paths, &grove_tmux(&paths), &session, &pane);
        }
        Some(Cmd::Send { finca, file }) => exit_on_err(send_cmd(&paths, finca, file)),
        Some(Cmd::Focus { window }) => exit_on_err(grove_tmux(&paths).select_window(&window)),
        Some(Cmd::Finca(cmd)) => exit_on_err(finca_cmd(&paths, cmd)),
        Some(Cmd::Dashboard) => exit_on_err(dashboard(paths)),
        None => exit_on_err(start(&paths)),
    }
}

/// `grove mcp setup`.
fn mcp_setup(paths: &Paths) -> Result<()> {
    let cfg = load_config(paths)?;
    let home = PathBuf::from(std::env::var("HOME").context("HOME isn't set")?);
    print!(
        "{}",
        mcp::setup_commands(&cfg, &runfiles::grove_exe()?, &home)
    );
    Ok(())
}

fn exit_on_err(r: Result<()>) {
    if let Err(e) = r {
        eprintln!("grove: {e:#}");
        std::process::exit(1);
    }
}

fn run_hook(paths: Paths) {
    // A Grove bug must never block Claude: swallow panics, exit 0.
    std::panic::set_hook(Box::new(|_| {}));
    let store = Store::new(paths);
    let result = std::panic::catch_unwind(|| {
        let mut input = String::new();
        let _ = std::io::stdin().read_to_string(&mut input);
        hook::run_hook(&store, &input)
    });
    match result {
        Ok(Some(out)) => print!("{out}"),
        Ok(None) => {}
        Err(_) => log::error(&store.paths, "hook", "panic while handling hook"),
    }
}

fn run_statusline(paths: Paths) {
    // Like hooks: a Grove bug must never break Claude's screen.
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(|| {
        let mut input = String::new();
        let _ = std::io::stdin().read_to_string(&mut input);
        statusline::run(&paths, &input)
    });
    match result {
        Ok(line) => println!("{line}"),
        Err(_) => println!("grove"),
    }
}

fn add(paths: &Paths, path: Option<PathBuf>, new: bool) -> Result<()> {
    use std::io::{BufRead, IsTerminal, Write};
    paths.ensure()?;
    let cfg = load_config(paths)?;
    let path = match path {
        Some(p) => p,
        None => std::env::current_dir()?,
    };
    let store = Store::new(paths.clone());
    let text = path.to_string_lossy().into_owned();
    if config::expand_tilde(&text).exists() {
        let name = ui::run::add_project_at(&store, &text)?;
        println!("Added project {name}");
        return print_account(&store, &cfg, &name);
    }
    if !new {
        if !std::io::stdin().is_terminal() {
            bail!("{text} doesn't exist; use `grove add --new {text}` to create it");
        }
        print!("{text} doesn't exist. Create it and git init it? [y/N] ");
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().lock().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            bail!("not created");
        }
    }
    let name = ui::run::create_project_at(&store, &text)?;
    println!("Created project {name} (git init)");
    print_account(&store, &cfg, &name)
}

/// Says which account the new project's folder puts it on.
fn print_account(store: &Store, cfg: &Config, project: &str) -> Result<()> {
    let p = store.get_project(project)?;
    let a = cfg.account_for(&p.path);
    println!(
        "Its sessions use the {} account ({})",
        a.name,
        a.dir.display()
    );
    Ok(())
}

fn rm(paths: &Paths, project: &str) -> Result<()> {
    let cfg = load_config(paths)?;
    let store = Store::new(paths.clone());
    let msg = ui::run::remove_project(&store, &grove_tmux(paths), &cfg.tmux_session, project)?;
    println!("{msg}");
    Ok(())
}

fn doctor(paths: &Paths) -> Result<()> {
    use doctor::Level;
    let probes = doctor::Probes {
        tmux_version: &tmux::version,
        claude: &|| Some(launch::claude_bin()).filter(|c| c.starts_with('/')),
        has: &runfiles::on_path,
        poll: &|f| finca::poll_via(f, std::time::Duration::from_secs(10)),
    };
    let checks = doctor::run(paths, &probes);
    let width = checks.iter().map(|c| c.what.len()).max().unwrap_or(0);
    for c in &checks {
        let mark = match c.level {
            Level::Ok => "✓",
            Level::Warn => "!",
            Level::Fail => "✗",
        };
        println!("{mark} {:width$}  {}", c.what, c.detail);
    }
    let failed = checks.iter().filter(|c| c.level == Level::Fail).count();
    if failed > 0 {
        bail!("{failed} problem(s) above");
    }
    Ok(())
}

fn grove_tmux(paths: &Paths) -> Tmux {
    Tmux {
        socket: Some(tmux_socket()),
        config: Some(paths.run.join("tmux.conf")),
    }
}

fn dashboard(paths: Paths) -> Result<()> {
    let cfg = load_config(&paths)?;
    let tmux = grove_tmux(&paths);
    ui::run::run_dashboard(ui::run::Ctx {
        store: Store::new(paths),
        cfg,
        tmux,
    })
}

/// Checks everything a session launch depends on, before touching tmux.
fn preflight(paths: &Paths) -> Result<Config> {
    let cfg = load_config(paths)?;
    let v = tmux::version()?;
    if v < (3, 2) {
        bail!("tmux {}.{} is too old; Grove needs 3.2 or newer", v.0, v.1);
    }
    // Where Claude can't run (a Pi Zero), Grove is for shells and to-dos.
    if setup::claude_runs_on(std::env::consts::ARCH) {
        claude_dir(&cfg)?;
        if !launch::claude_bin().starts_with('/') {
            bail!("`claude` was not found on PATH");
        }
    }
    Ok(cfg)
}

/// `grove`: create or attach the Grove tmux session (dashboard in window 0).
fn start(paths: &Paths) -> Result<()> {
    // The project shell popup runs without $TMUX, so attaching from there
    // would open Grove inside itself.
    if std::env::var_os("GROVE_POPUP").is_some() {
        println!("You're already in Grove: exit this shell (C-d) for the dashboard.");
        return Ok(());
    }
    let cfg = preflight(paths)?;
    paths.ensure()?;
    let exe = runfiles::grove_exe()?;
    runfiles::write_all(paths, &exe, &cfg.tmux_session, cfg.statusline)?;

    let tmux = grove_tmux(paths);
    let session = &cfg.tmux_session;
    if tmux.has_session(session) {
        // Pick up tmux.conf changes from a newer grove.
        let conf = paths.run.join("tmux.conf");
        let _ = tmux.cmd().args(["source-file"]).arg(&conf).status();
    }
    // Also brings window 0 back if the dashboard ever died.
    let argv = vec![exe.to_string_lossy().into_owned(), "dashboard".to_string()];
    let stamp = launch::build_stamp(&exe);
    if launch::ensure_dashboard(&tmux, session, &paths.home, &argv)? {
        tmux.set_window_option(&launch::dashboard_target(session), "@grove_build", &stamp)?;
    } else {
        // After a reinstall, the running dashboard is still the old code.
        launch::ensure_fresh_dashboard(&tmux, session, &stamp)?;
    }
    // The dashboard offers this folder as the default for `a` (add project).
    if let Ok(cwd) = std::env::current_dir() {
        let _ = tmux.set_environment(session, "GROVE_CWD", &cwd.to_string_lossy());
    }

    let inside = std::env::var("TMUX").unwrap_or_default();
    let target = format!("={session}");
    if inside.contains(&format!("/{},", tmux_socket())) {
        // Already inside Grove: just go to the dashboard.
        tmux.select_window(&launch::dashboard_target(session))?;
        return Ok(());
    }
    // Outside tmux, or inside another tmux server (nests; TMUX must be unset).
    let err = tmux
        .cmd()
        .args(["attach-session", "-t", &target])
        .env_remove("TMUX")
        .exec();
    Err(err).context("attaching to the grove tmux session")
}

/// `grove type-prompt`: the reply checks of `prompt::type_into`, by id.
fn type_prompt(paths: &Paths, id: &str, text: &str) -> std::result::Result<String, String> {
    let store = Store::new(paths.clone());
    let s = store
        .get_session(id)
        .filter(|s| s.state != model::State::Ended)
        .ok_or_else(|| format!("no live session {id}"))?;
    let cfg = load_config(paths).map_err(|e| format!("{e:#}"))?;
    let account = store
        .get_project(&s.project)
        .map(|p| cfg.account_for(&p.path).name)
        .unwrap_or_default();
    prompt::type_into(&grove_tmux(paths), &s, &account, text)
}

/// Resumes ended session `id` here; the new window's id.
fn resume_session(paths: &Paths, id: &str) -> Result<String> {
    let store = Store::new(paths.clone());
    let Some(s) = store.get_session(id) else {
        anyhow::bail!("no session {id}");
    };
    let cfg = load_config(paths)?;
    let project = store.get_project(&s.project)?;
    let s = launch::resume(&grove_tmux(paths), &store, &cfg, &project, id)?;
    Ok(s.tmux_window.unwrap_or_default())
}

fn print_status(paths: &Paths, json: bool) {
    // Windows of the running Grove tmux session; none if the server is down
    // (e.g. after a reboot), so stale session files don't report as live.
    let session = load_config(paths)
        .map(|c| c.tmux_session)
        .unwrap_or_else(|_| "grove".into());
    let windows = grove_tmux(paths).list_windows(&session).unwrap_or_default();
    let st = status::build(&Store::new(paths.clone()), &status::hostname(), &windows);
    if json {
        println!("{}", serde_json::to_string(&st).expect("status serializes"));
        return;
    }
    println!("{}: {} live, {} need you", st.host, st.live, st.needs_you);
    for p in &st.projects {
        for s in &p.sessions {
            println!("  {}:{}  {}", p.name, s.name, s.state.label());
        }
    }
}

fn finca_cmd(paths: &Paths, cmd: FincaCmd) -> Result<()> {
    paths.ensure()?;
    let store = Store::new(paths.clone());
    match cmd {
        FincaCmd::Add {
            name,
            ssh,
            bin,
            backup,
        } => {
            // Either destination will do, so adding works away from home too.
            let grove_bin = match (finca::probe(&ssh, bin.as_deref()), &backup) {
                (Ok(b), _) => b,
                (Err(e), None) => return Err(e),
                (Err(e), Some(dest)) => finca::probe(dest, bin.as_deref())
                    .map_err(|e2| anyhow::anyhow!("{e:#}\n\nbackup: {e2:#}"))?,
            };
            store.add_finca(&finca::Finca {
                name: name.clone(),
                ssh,
                grove_bin: grove_bin.clone(),
                color: None,
                backup,
            })?;
            println!("Added finca {name} (grove at {grove_bin})");
        }
        FincaCmd::Rm { name } => {
            let f = store.remove_finca(&name)?;
            finca::disconnect(&grove_tmux(paths), &f.slug());
            println!("Removed finca {}", f.name);
        }
        FincaCmd::Ls => {
            for r in store.list_fincas() {
                match r {
                    Ok(f) => println!(
                        "{}\t{}\t{}\t{}",
                        f.name,
                        f.ssh,
                        f.backup.as_deref().unwrap_or("-"),
                        f.grove_bin
                    ),
                    Err((path, e)) => println!("{}\terror: {e}", path.display()),
                }
            }
        }
        FincaCmd::Setup { dest, pause } => {
            // Without a terminal, end of input would read as Enter: "yes".
            if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                bail!("finca setup asks before each change, so it needs a terminal");
            }
            let mut io = setup::TerminalIo;
            let dest = match dest {
                Some(d) => d,
                None => setup::Io::ask(&mut io, "SSH destination (user@host)?", ""),
            };
            let r = setup_finca(&store, &mut io, dest.trim());
            if let Err(e) = &r {
                eprintln!("grove: {e:#}");
            }
            if pause {
                setup::Io::ask(&mut io, "Press Enter to close", "");
            }
            if r.is_err() {
                // Already printed, before the pause.
                std::process::exit(1);
            }
        }
        FincaCmd::Update { names } => {
            let fincas: Vec<finca::Finca> = if names.is_empty() {
                store
                    .list_fincas()
                    .into_iter()
                    .filter_map(Result::ok)
                    .collect()
            } else {
                names
                    .iter()
                    .map(|n| store.get_finca(n))
                    .collect::<Result<_>>()?
            };
            let exe = std::env::current_exe()?;
            let mut failed = 0;
            for f in &fincas {
                match update_one(f, &exe) {
                    Ok(line) => println!("== {}: {line}", f.name),
                    Err(e) => {
                        failed += 1;
                        eprintln!("== {}: {e:#}", f.name);
                    }
                }
            }
            if failed > 0 {
                bail!("{failed} finca(s) not updated");
            }
        }
    }
    Ok(())
}

fn send_cmd(paths: &Paths, name: String, file: Option<PathBuf>) -> Result<()> {
    let store = Store::new(paths.clone());
    let f = store.get_finca(&name)?;
    paths.ensure()?;
    let name_of = |p: &std::path::Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let (file, original, temp) = match file {
        Some(p) => {
            let n = name_of(&p);
            (p, n, false)
        }
        None => match transfer::read_clipboard(&paths.run) {
            transfer::Clip::File(p) => {
                let n = name_of(&p);
                (p, n, false)
            }
            transfer::Clip::Image(p) => (p, "clipboard.png".to_string(), true),
            transfer::Clip::Folder(p) => bail!("can't send a folder ({})", p.display()),
            transfer::Clip::Nothing => {
                bail!("no file or image on the clipboard (or it can't be read here); give a file")
            }
        },
    };
    let sent = transfer::pick_dest(&f, None).and_then(|dest| {
        transfer::send(
            &dest,
            &file,
            &transfer::remote_name(&original, chrono::Local::now()),
        )
    });
    if temp {
        let _ = std::fs::remove_file(&file);
    }
    println!("{}", sent?);
    Ok(())
}

/// Runs the setup wizard on the terminal against `dest`.
fn setup_finca(store: &Store, io: &mut setup::TerminalIo, dest: &str) -> Result<finca::Finca> {
    if dest.is_empty() {
        bail!("no destination given");
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let tailscale_json = std::process::Command::new("tailscale")
        .args(["status", "--json"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let s = setup::Setup {
        store,
        ssh_dir: home.join(".ssh"),
        exe: std::env::current_exe()?,
        source: finca::source_dir(),
        release_build: finca::RELEASE_BUILD,
        download_url: None,
        tailscale_json,
        wrap: finca::login_shell_cmd,
    };
    s.run(io, dest)
}

/// Installs this build on `f` through whichever destination answers.
fn update_one(f: &finca::Finca, exe: &std::path::Path) -> Result<String> {
    let dest = f
        .destinations()
        .find(|d| finca::run_remote(d, "true").is_ok_and(|o| o.status.success()))
        .ok_or_else(|| anyhow::anyhow!("unreachable"))?;
    let out = finca::run_remote(dest, &finca::login_shell_cmd(&finca::facts_script()))?;
    let facts = setup::parse_facts(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| anyhow::anyhow!("couldn't read the machine's details"))?;
    let source = finca::source_dir();
    let downloaded;
    let (how, label) = match finca::update_plan(&facts, finca::RELEASE_BUILD, source.as_deref()) {
        finca::Plan::Copy => (finca::How::Copy(exe), "copied"),
        finca::Plan::Download => {
            let version = env!("CARGO_PKG_VERSION");
            let target = finca::release_target(&facts.os, &facts.arch)?;
            downloaded = finca::fetch_release(
                &Paths::from_env().run,
                &finca::release_url(version),
                version,
                &target,
            )?;
            (finca::How::Copy(&downloaded), "downloaded")
        }
        finca::Plan::Build => (
            finca::How::Build(source.as_deref().expect("Build needs a source")),
            "built there",
        ),
    };
    let v = finca::install_grove(dest, &facts, &f.grove_bin, how, finca::login_shell_cmd)?;
    let how = label;
    Ok(format!("{v} ({how}, via {dest})"))
}
