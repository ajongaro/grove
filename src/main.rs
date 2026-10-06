mod config;
mod hook;
mod launch;
mod log;
mod model;
mod runfiles;
mod state;
mod store;
mod tmux;

use std::io::Read;

use clap::{Parser, Subcommand};

use crate::config::Paths;
use crate::store::Store;

#[derive(Parser)]
#[command(name = "grove", about = "A tmux dashboard for Claude Code sessions", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Internal: called by Claude Code hooks with the event JSON on stdin.
    Hook,
}

fn main() {
    let cli = Cli::parse();
    let paths = Paths::from_env();
    match cli.cmd {
        Some(Cmd::Hook) => {
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
        None => {}
    }
}
