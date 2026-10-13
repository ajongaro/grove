//! `grove finca setup`: takes a reachable machine to a working finca, asking
//! before each change. Pure parts here are tested on strings; the wizard
//! runs against `Io`, so tests script the answers.

// Used by the wizard (next tasks).
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::Path;

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

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
