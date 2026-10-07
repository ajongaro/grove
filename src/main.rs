mod config;
mod finca;
mod hook;
mod launch;
mod log;
mod mcp;
mod model;
mod runfiles;
mod state;
mod status;
mod store;
mod tmux;
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
    Add { path: Option<PathBuf> },
    /// Manage fincas: other machines running their own Grove.
    #[command(subcommand)]
    Finca(FincaCmd),
    /// Print this machine's sessions (used by other Groves to poll fincas).
    Status {
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Internal: the dashboard that runs in tmux window 0.
    #[command(hide = true)]
    Dashboard,
    /// Internal: called by Claude Code hooks with the event JSON on stdin.
    #[command(hide = true)]
    Hook,
    /// Internal: stdio MCP server for projects and the wiki.
    #[command(hide = true)]
    Mcp,
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
    },
    /// Forget a finca.
    Rm { name: String },
    /// List fincas.
    Ls,
}

fn main() {
    let cli = Cli::parse();
    let paths = Paths::from_env();
    match cli.cmd {
        Some(Cmd::Hook) => run_hook(paths),
        Some(Cmd::Mcp) => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            if let Err(e) = rt.block_on(mcp::serve(Store::new(paths.clone()))) {
                log::error(&paths, "mcp", &format!("{e:#}"));
                std::process::exit(1);
            }
        }
        Some(Cmd::Add { path }) => exit_on_err(add(&paths, path)),
        Some(Cmd::Status { json }) => print_status(&paths, json),
        Some(Cmd::Finca(cmd)) => exit_on_err(finca_cmd(&paths, cmd)),
        Some(Cmd::Dashboard) => exit_on_err(dashboard(paths)),
        None => exit_on_err(start(&paths)),
    }
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

fn add(paths: &Paths, path: Option<PathBuf>) -> Result<()> {
    paths.ensure()?;
    let path = match path {
        Some(p) => p,
        None => std::env::current_dir()?,
    };
    let name = ui::run::add_project_at(&Store::new(paths.clone()), &path.to_string_lossy())?;
    println!("Added project {name}");
    Ok(())
}

fn grove_tmux(paths: &Paths) -> Tmux {
    Tmux {
        socket: Some(TMUX_SOCKET.into()),
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
    claude_dir(&cfg)?;
    let v = tmux::version()?;
    if v < (3, 2) {
        bail!("tmux {}.{} is too old; Grove needs 3.2 or newer", v.0, v.1);
    }
    if !launch::claude_bin().starts_with('/') {
        bail!("`claude` was not found on PATH");
    }
    Ok(cfg)
}

/// `grove`: create or attach the Grove tmux session (dashboard in window 0).
fn start(paths: &Paths) -> Result<()> {
    let cfg = preflight(paths)?;
    paths.ensure()?;
    let exe = std::env::current_exe().context("locating the grove binary")?;
    let exe = exe.canonicalize().unwrap_or(exe);
    runfiles::write_all(paths, &exe)?;

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
        tmux.set_window_option(&format!("{session}:0"), "@grove_build", &stamp)?;
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
    if inside.contains(&format!("/{TMUX_SOCKET},")) {
        // Already inside Grove: just go to the dashboard.
        tmux.select_window(&format!("{target}:0"))?;
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

fn print_status(paths: &Paths, json: bool) {
    let st = status::build(&Store::new(paths.clone()), &status::hostname());
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
        FincaCmd::Add { name, ssh, bin } => {
            let grove_bin = finca::probe(&ssh, bin.as_deref())?;
            store.add_finca(&finca::Finca { name: name.clone(), ssh, grove_bin: grove_bin.clone(), color: None })?;
            println!("Added finca {name} (grove at {grove_bin})");
        }
        FincaCmd::Rm { name } => {
            let f = store.remove_finca(&name)?;
            println!("Removed finca {}", f.name);
        }
        FincaCmd::Ls => {
            for r in store.list_fincas() {
                match r {
                    Ok(f) => println!("{}\t{}\t{}", f.name, f.ssh, f.grove_bin),
                    Err((path, e)) => println!("{}\terror: {e}", path.display()),
                }
            }
        }
    }
    Ok(())
}
