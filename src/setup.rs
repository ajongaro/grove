//! `grove finca setup`: takes a reachable machine to a working finca, asking
//! before each change. Pure parts here are tested on strings; the wizard
//! runs against `Io`, so tests script the answers.

use std::collections::BTreeSet;
use std::path::Path;

use crate::finca::{self, Finca, interactive_ssh, run_remote};
use anyhow::{Result, bail};

/// What one SSH call learns about a machine (see `finca::facts_script`).
#[derive(Debug, Clone, PartialEq)]
pub struct Facts {
    /// `std::env::consts::OS` form: macos, linux.
    pub os: String,
    /// `std::env::consts::ARCH` form: aarch64, x86_64.
    pub arch: String,
    pub host: String,
    pub home: String,
    /// Tools found: tmux, claude, cargo, brew, apt-get, dnf.
    pub tools: BTreeSet<String>,
    /// A working grove there: (path, `--version` line).
    pub grove: Option<(String, String)>,
}

impl Facts {
    pub fn has(&self, tool: &str) -> bool {
        self.tools.contains(tool)
    }

    pub fn same_platform_as_here(&self) -> bool {
        self.os == std::env::consts::OS && self.arch == std::env::consts::ARCH
    }
}

/// Parses `key=value` lines; other lines (login chatter) are ignored.
pub fn parse_facts(out: &str) -> Option<Facts> {
    let (mut os, mut arch, mut host, mut home) = (None, None, None, None);
    let (mut grove, mut version) = (None, None);
    let mut tools = BTreeSet::new();
    for line in out.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim().to_string();
        match k {
            "os" => {
                os = Some(match v.as_str() {
                    "Darwin" => "macos".to_string(),
                    "Linux" => "linux".to_string(),
                    other => other.to_lowercase(),
                })
            }
            "arch" => {
                arch = Some(if v == "arm64" {
                    "aarch64".to_string()
                } else {
                    v
                })
            }
            "host" => host = Some(v),
            "home" => home = Some(v),
            "has" => {
                tools.insert(v);
            }
            "grove" => grove = Some(v),
            "grove_version" => version = Some(v),
            _ => {}
        }
    }
    Some(Facts {
        os: os?,
        arch: arch?,
        host: host?,
        home: home?,
        tools,
        grove: grove.zip(version),
    })
}

/// The command that installs `tool` there, if Grove knows one.
pub fn install_command(tool: &str, f: &Facts) -> Option<&'static str> {
    match tool {
        "claude" => Some("curl -fsSL https://claude.ai/install.sh | bash"),
        "cargo" => Some("curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"),
        "tmux" if f.os == "macos" => f.has("brew").then_some("brew install tmux"),
        "tmux" if f.has("apt-get") => Some("sudo apt-get install -y tmux"),
        "tmux" if f.has("dnf") => Some("sudo dnf install -y tmux"),
        _ => None,
    }
}

/// The Tailscale name of the peer whose host name is `host`, from
/// `tailscale status --json`.
pub fn tailscale_backup(json: &str, host: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    v.get("Peer")?.as_object()?.values().find_map(|p| {
        let name = p.get("HostName")?.as_str()?;
        let dns = p.get("DNSName")?.as_str()?.trim_end_matches('.');
        (name.eq_ignore_ascii_case(host) && !dns.is_empty()).then(|| dns.to_string())
    })
}

/// Hosts to offer for a destination: `Host` aliases in `config` (no
/// patterns) and plain names in `known_hosts`; sorted, no duplicates.
#[allow(dead_code)] // Used by the dashboard prompt (next task).
pub fn ssh_hosts(ssh_dir: &Path) -> Vec<String> {
    let mut hosts = BTreeSet::new();
    if let Ok(cfg) = std::fs::read_to_string(ssh_dir.join("config")) {
        for line in cfg.lines() {
            let mut words = line.split_whitespace();
            if words.next().is_some_and(|w| w.eq_ignore_ascii_case("host")) {
                hosts.extend(
                    words
                        .filter(|w| !w.contains(['*', '?', '!']))
                        .map(str::to_string),
                );
            }
        }
    }
    if let Ok(known) = std::fs::read_to_string(ssh_dir.join("known_hosts")) {
        for first in known.lines().filter_map(|l| l.split_whitespace().next()) {
            hosts.extend(
                first
                    .split(',')
                    .filter(|h| !h.starts_with('|') && !h.starts_with('['))
                    .map(str::to_string),
            );
        }
    }
    hosts.into_iter().collect()
}

pub trait Io {
    fn say(&mut self, line: &str);
    /// A yes/no question; Enter means yes.
    fn confirm(&mut self, question: &str) -> bool;
    /// A question with a default for Enter.
    fn ask(&mut self, question: &str, default: &str) -> String;
    /// Runs `argv` on the terminal (it may prompt); true if it exited 0.
    fn interactive(&mut self, argv: &[String]) -> bool;
}

/// Prompts on stdin/stdout.
pub struct TerminalIo;

impl TerminalIo {
    fn line(prompt: &str) -> String {
        use std::io::Write;
        print!("{prompt}");
        let _ = std::io::stdout().flush();
        let mut s = String::new();
        let _ = std::io::stdin().read_line(&mut s);
        s.trim().to_string()
    }
}

impl Io for TerminalIo {
    fn say(&mut self, line: &str) {
        println!("{line}");
    }
    fn confirm(&mut self, question: &str) -> bool {
        !Self::line(&format!("{question} [Y/n] "))
            .to_lowercase()
            .starts_with('n')
    }
    fn ask(&mut self, question: &str, default: &str) -> String {
        match Self::line(&format!("{question} [{default}] ")) {
            s if s.is_empty() => default.to_string(),
            s => s,
        }
    }
    fn interactive(&mut self, argv: &[String]) -> bool {
        std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .status()
            .is_ok_and(|s| s.success())
    }
}

pub struct Setup<'a> {
    pub store: &'a crate::store::Store,
    /// ~/.ssh on this machine.
    pub ssh_dir: std::path::PathBuf,
    /// This grove, copied to fincas of the same OS and CPU.
    pub exe: std::path::PathBuf,
    /// Grove's source, built on fincas of another OS or CPU.
    pub source: Option<std::path::PathBuf>,
    /// `tailscale status --json` here, if tailscale is installed.
    pub tailscale_json: Option<String>,
    /// Wraps remote commands: the login shell in real use.
    pub wrap: fn(&str) -> String,
}

fn ok(dest: &str, cmd: &str) -> bool {
    run_remote(dest, cmd).is_ok_and(|o| o.status.success())
}

impl Setup<'_> {
    pub fn run(&self, io: &mut dyn Io, dest: &str) -> Result<Finca> {
        self.ssh_ready(io, dest)?;
        let mut facts = self.facts(dest)?;
        let host = facts.host.clone();
        io.say(&format!(
            "✓ {dest} answers: {host}, {} {}",
            facts.os, facts.arch
        ));

        let existing = self
            .store
            .list_fincas()
            .into_iter()
            .filter_map(Result::ok)
            .find(|f| f.ssh == dest || f.backup.as_deref() == Some(dest));
        let name = match &existing {
            Some(f) => {
                io.say(&format!("{} is already a finca: checking it", f.name));
                f.name.clone()
            }
            None => self.pick_name(io, &host),
        };

        for tool in ["tmux", "claude"] {
            if facts.has(tool) {
                io.say(&format!("✓ {tool}"));
                continue;
            }
            let Some(cmd) = install_command(tool, &facts) else {
                bail!("{host} needs {tool}: install it there, then run this again");
            };
            if !io.confirm(&format!("Install {tool} on {host}? ({cmd})")) {
                bail!("{host} needs {tool}: install it there, then run this again");
            }
            io.interactive(&interactive_ssh(dest, Some(cmd), &[]));
            facts = self.facts(dest)?;
            if !facts.has(tool) {
                bail!(
                    "{tool} still isn't on {host}'s login PATH; if it was installed to ~/.local/bin, add that to PATH in the shell profile there"
                );
            }
            io.say(&format!("✓ {tool} installed"));
        }

        let grove_bin = self.grove(io, dest, &mut facts)?;
        self.claude_login(io, dest, &host)?;
        let backup = match &existing {
            Some(f) => f.backup.clone(),
            None => self.backup(io, dest, &host),
        };

        let finca = Finca {
            name: name.clone(),
            ssh: existing
                .as_ref()
                .map_or(dest.to_string(), |f| f.ssh.clone()),
            grove_bin,
            color: existing.as_ref().and_then(|f| f.color.clone()),
            backup,
        };
        self.store.save_finca(&finca)?;
        io.say(&format!("✓ Saved finca {name}"));
        Ok(finca)
    }

    fn ssh_ready(&self, io: &mut dyn Io, dest: &str) -> Result<()> {
        let check = || run_remote(dest, "true");
        let out = check()?;
        if out.status.success() {
            io.say(&format!("✓ SSH to {dest}"));
            return Ok(());
        }
        let mut err = String::from_utf8_lossy(&out.stderr).to_string();
        if err.contains("Host key verification failed")
            && io.confirm(&format!("Accept {dest}'s host key?"))
        {
            io.interactive(&interactive_ssh(
                dest,
                Some("true"),
                &["-o", "StrictHostKeyChecking=accept-new"],
            ));
            let out = check()?;
            if out.status.success() {
                io.say(&format!("✓ SSH to {dest}"));
                return Ok(());
            }
            err = String::from_utf8_lossy(&out.stderr).to_string();
        }
        if !err.contains("Permission denied") {
            bail!("can't reach {dest}: {}", err.trim());
        }
        let has_key = std::fs::read_dir(&self.ssh_dir).is_ok_and(|d| {
            d.filter_map(|e| e.ok()).any(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.starts_with("id_") && n.ends_with(".pub")
            })
        });
        if !has_key {
            if !io.confirm("You have no SSH key. Create one (ssh-keygen)?") {
                bail!(
                    "can't reach {dest} without a password: create a key and run `ssh-copy-id {dest}`"
                );
            }
            io.interactive(&["ssh-keygen".into(), "-t".into(), "ed25519".into()]);
        }
        if !io.confirm(&format!(
            "Copy your SSH key to {dest}? (you'll type its password once)"
        )) {
            bail!("can't reach {dest} without a password: run `ssh-copy-id {dest}`");
        }
        io.interactive(&["ssh-copy-id".into(), dest.into()]);
        let out = check()?;
        if !out.status.success() {
            bail!(
                "still can't reach {dest} without a password: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        io.say(&format!("✓ SSH to {dest}"));
        Ok(())
    }

    fn facts(&self, dest: &str) -> Result<Facts> {
        let out = run_remote(dest, &(self.wrap)(&finca::facts_script()))?;
        parse_facts(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| {
            anyhow::anyhow!(
                "couldn't read {dest}'s details: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )
        })
    }

    fn pick_name(&self, io: &mut dyn Io, host: &str) -> String {
        loop {
            let name = io.ask("Name for this finca?", host);
            let slug = crate::model::slugify(&name);
            if slug.is_empty() {
                io.say("A name needs a letter or digit.");
            } else if self.store.get_finca(&slug).is_ok() {
                io.say(&format!("There's already a finca called {name}."));
            } else {
                return name;
            }
        }
    }

    fn grove(&self, io: &mut dyn Io, dest: &str, facts: &mut Facts) -> Result<String> {
        let want = format!("grove {}", env!("CARGO_PKG_VERSION"));
        if let Some((path, version)) = &facts.grove
            && *version == want
        {
            io.say(&format!("✓ {version} at {path}"));
            return Ok(path.clone());
        }
        let bin = facts
            .grove
            .as_ref()
            .map_or(format!("{}/.local/bin/grove", facts.home), |(p, _)| {
                p.clone()
            });
        if !facts.same_platform_as_here() && !facts.has("cargo") {
            let cmd = install_command("cargo", facts).unwrap_or_default();
            if !io.confirm(&format!(
                "Building Grove there needs Rust. Install it? ({cmd})"
            )) {
                bail!("{} needs Rust (cargo) to build Grove", facts.host);
            }
            io.interactive(&interactive_ssh(dest, Some(cmd), &[]));
            *facts = self.facts(dest)?;
        }
        if !io.confirm(&format!("Install Grove at {bin}?")) {
            bail!(
                "{} needs Grove: install it there, then run this again",
                facts.host
            );
        }
        if !facts.same_platform_as_here() {
            io.say("Building Grove there (this takes a few minutes)…");
        }
        let v = finca::install_grove(
            dest,
            facts,
            &bin,
            &self.exe,
            self.source.as_deref(),
            self.wrap,
        )?;
        io.say(&format!("✓ {v} at {bin}"));
        Ok(bin)
    }

    fn claude_login(&self, io: &mut dyn Io, dest: &str, host: &str) -> Result<()> {
        let status = (self.wrap)("claude auth status >/dev/null 2>&1");
        if ok(dest, &status) {
            io.say("✓ Claude is logged in");
            return Ok(());
        }
        if io.confirm(&format!("Log in to Claude on {host}?")) {
            io.interactive(&interactive_ssh(dest, Some("claude auth login"), &[]));
            if ok(dest, &status) {
                io.say("✓ Claude is logged in");
                return Ok(());
            }
        }
        bail!("Claude isn't logged in on {host}: run `claude auth login` there")
    }

    fn backup(&self, io: &mut dyn Io, dest: &str, host: &str) -> Option<String> {
        let ts = tailscale_backup(self.tailscale_json.as_deref()?, host)?;
        if ts == dest || !io.confirm(&format!("Use {ts} (Tailscale) as the backup route?")) {
            return None;
        }
        if ok(&ts, "true") {
            io.say(&format!("✓ backup {ts}"));
            Some(ts)
        } else {
            io.say(&format!("{ts} didn't answer over SSH; no backup set"));
            None
        }
    }
}

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
