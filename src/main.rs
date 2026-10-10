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
        /// Claude account for its sessions (a name from config.toml).
        #[arg(long)]
        account: Option<String>,
    },
    /// Show or set the Claude account a project's sessions use.
    Account {
        project: String,
        /// An account name from config.toml; omit to show the current one.
        name: Option<String>,
    },
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
        /// SSH destination to fall back to when the first doesn't answer,
        /// e.g. a Tailscale name for when you're away from home.
        #[arg(long)]
        backup: Option<String>,
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
        Some(Cmd::Add { path, new, account }) => exit_on_err(add(&paths, path, new, account)),
        Some(Cmd::Account { project, name }) => exit_on_err(account_cmd(&paths, &project, name)),
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

fn add(paths: &Paths, path: Option<PathBuf>, new: bool, account: Option<String>) -> Result<()> {
    use std::io::{BufRead, IsTerminal, Write};
    paths.ensure()?;
    // Check the account before creating anything.
    let cfg = load_config(paths)?;
    if let Some(a) = &account {
        cfg.account(Some(a))?;
    }
    let path = match path {
        Some(p) => p,
        None => std::env::current_dir()?,
    };
    let store = Store::new(paths.clone());
    let text = path.to_string_lossy().into_owned();
    if config::expand_tilde(&text).exists() {
        let name = ui::run::add_project_at(&store, &text)?;
        println!("Added project {name}");
        return set_account(&store, &cfg, &name, account);
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
    set_account(&store, &cfg, &name, account)
}

/// Stores `account` on the project (the default account is stored as none).
fn set_account(store: &Store, cfg: &Config, project: &str, account: Option<String>) -> Result<()> {
    let Some(name) = account else {
        return Ok(());
    };
    let a = cfg.account(Some(&name))?;
    let mut p = store.get_project(project)?;
    p.account = (a.name != cfg.default_account().name).then_some(a.name.clone());
    store.save_project(&p)?;
    println!(
        "{} sessions use the {} account ({})",
        p.name,
        a.name,
        a.dir.display()
    );
    Ok(())
}

fn account_cmd(paths: &Paths, project: &str, name: Option<String>) -> Result<()> {
    let cfg = load_config(paths)?;
    let store = Store::new(paths.clone());
    if name.is_some() {
        return set_account(&store, &cfg, project, name);
    }
    let p = store.get_project(project)?;
    let a = cfg.account(p.account.as_deref())?;
    println!(
        "{} sessions use the {} account ({})",
        p.name,
        a.name,
        a.dir.display()
    );
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
    runfiles::write_all(paths, &exe, &cfg.tmux_session)?;

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
    }
    Ok(())
}
