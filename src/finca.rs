//! Fincas: other machines running their own Grove, reached over SSH.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::model::slugify;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finca {
    pub name: String,
    /// Anything `ssh` accepts as a destination, including config aliases.
    pub ssh: String,
    /// Absolute path of grove on that machine (non-interactive ssh has no
    /// login-profile PATH).
    pub grove_bin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Tried when `ssh` doesn't answer, e.g. a Tailscale name for when
    /// you're away from the home network.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
}

impl Finca {
    pub fn slug(&self) -> String {
        slugify(&self.name)
    }

    /// SSH destinations in the order to try them: `ssh`, then `backup`.
    pub fn destinations(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.ssh.as_str()).chain(self.backup.as_deref())
    }
}

/// `$GROVE_SSH`, else `ssh`. Tests point this at a fake.
pub fn ssh_program() -> String {
    std::env::var("GROVE_SSH")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "ssh".into())
}

/// Options for every non-interactive ssh call.
pub const SSH_BATCH_OPTS: [&str; 4] = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=5"];

/// Single-quotes `s` for a POSIX shell.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Where grove usually lives; `~` is expanded by the remote shell.
const CANDIDATES: [&str; 4] = [
    "~/.local/bin/grove",
    "~/.cargo/bin/grove",
    "/opt/homebrew/bin/grove",
    "/usr/local/bin/grove",
];

/// Exit code the probe script uses for "no grove found".
const NO_GROVE: i32 = 3;

/// Checks `dest` is reachable without a password and finds grove there.
/// Returns grove's absolute path on the remote machine.
pub fn probe(dest: &str, bin: Option<&str>) -> Result<String> {
    // POSIX sh that also runs under zsh/bash as a login shell's -c string.
    let explicit = bin.map(sh_quote).unwrap_or_default();
    let script = format!(
        "for b in {explicit} {}; do if [ -x \"$b\" ] && \"$b\" --version >/dev/null 2>&1; then echo \"$b\"; exit 0; fi; done; exit {NO_GROVE}",
        CANDIDATES.join(" ")
    );
    let out = std::process::Command::new(ssh_program())
        .args(SSH_BATCH_OPTS)
        .arg(dest)
        .arg(script)
        .stdin(std::process::Stdio::null())
        .output()
        .with_context(|| format!("running {}", ssh_program()))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match out.status.code() {
        Some(0) if stdout.starts_with('/') => {
            Ok(stdout.lines().last().unwrap_or(&stdout).to_string())
        }
        Some(NO_GROVE) => bail!(
            "no grove found on {dest} (looked in {}{}); install it there first",
            if bin.is_some() { "--bin, " } else { "" },
            CANDIDATES.join(", ")
        ),
        _ => bail!(
            "can't reach {dest} over ssh without a password: {}\nSet up key-based SSH (`ssh-copy-id {dest}`) and connect once interactively to accept the host key.",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }
}

/// Outcome of polling one finca.
#[derive(Debug, Clone, PartialEq)]
pub enum Reach {
    Reachable(crate::status::Status),
    Unreachable {
        error: String,
    },
    /// Grove is there but too old (or too new) to report status.
    NeedsUpdate,
}

/// One poll result, sent from the poller thread.
#[derive(Debug, Clone)]
pub struct PollResult {
    pub slug: String,
    pub reach: Reach,
    /// The destination that answered; None when none did.
    pub via: Option<String>,
    pub at: chrono::DateTime<chrono::Utc>,
}

/// Polls each destination in turn until one answers.
#[cfg(test)]
pub fn poll_one(f: &Finca, timeout: std::time::Duration) -> Reach {
    poll_via(f, timeout).0
}

/// Like `poll_one`, also returning the destination that answered. Each
/// destination gets the full `timeout` (an unresolvable `.local` name can
/// hang past ssh's ConnectTimeout).
pub fn poll_via(f: &Finca, timeout: std::time::Duration) -> (Reach, Option<String>) {
    let mut errors = Vec::new();
    for dest in f.destinations() {
        match poll_dest(f, dest, timeout) {
            Reach::Unreachable { error } => errors.push(if f.backup.is_some() {
                format!("{dest}: {error}")
            } else {
                error
            }),
            reach => return (reach, Some(dest.to_string())),
        }
    }
    let error = errors.join("; ");
    (Reach::Unreachable { error }, None)
}

/// Runs `grove status --json` at `dest`, killing it after `timeout`.
fn poll_dest(f: &Finca, dest: &str, timeout: std::time::Duration) -> Reach {
    use std::io::Read;
    use std::process::{Command, Stdio};

    let remote_cmd = format!("{} status --json", sh_quote(&f.grove_bin));
    let mut child = match Command::new(ssh_program())
        .args(SSH_BATCH_OPTS)
        .arg(dest)
        .arg(remote_cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return Reach::Unreachable {
                error: format!("can't run {}: {e}", ssh_program()),
            };
        }
    };
    // Drain both pipes on threads so a chatty child can't block on a full pipe.
    let drain = |mut r: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = r.read_to_string(&mut s);
            s
        })
    };
    let out = drain(Box::new(child.stdout.take().expect("piped")));
    let err = drain(Box::new(child.stderr.take().expect("piped")));

    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(e) => {
                return Reach::Unreachable {
                    error: e.to_string(),
                };
            }
        }
    };
    let Some(status) = status else {
        return Reach::Unreachable {
            error: format!("timed out after {}s", timeout.as_secs_f32()),
        };
    };
    // The killed ssh's descendants may still hold the pipes; only join after exit.
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    match status.code() {
        Some(0) => match crate::status::parse(&stdout) {
            Ok(st) => Reach::Reachable(st),
            Err(crate::status::ParseError::WrongVersion(_)) => Reach::NeedsUpdate,
            Err(crate::status::ParseError::Invalid(e)) => Reach::Unreachable {
                error: format!("bad status output: {e}"),
            },
        },
        // ssh itself failed: network, auth, host key.
        Some(255) => Reach::Unreachable {
            error: stderr.trim().to_string(),
        },
        // grove ran but didn't know `status` (clap usage error) or failed.
        _ => Reach::NeedsUpdate,
    }
}

/// Background thread polling every finca each `interval` (and on `kick`).
pub struct Poller {
    pub results: std::sync::mpsc::Receiver<PollResult>,
    kick: std::sync::mpsc::Sender<()>,
}

impl Poller {
    pub fn start(
        store: crate::store::Store,
        interval: std::time::Duration,
        timeout: std::time::Duration,
    ) -> Self {
        let (tx, results) = std::sync::mpsc::channel();
        let (kick, kicked) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            loop {
                let fincas: Vec<Finca> = store
                    .list_fincas()
                    .into_iter()
                    .filter_map(Result::ok)
                    .collect();
                // Poll in parallel so one slow finca doesn't delay the rest.
                let handles: Vec<_> = fincas
                    .into_iter()
                    .map(|f| {
                        let tx = tx.clone();
                        std::thread::spawn(move || {
                            let (reach, via) = poll_via(&f, timeout);
                            tx.send(PollResult {
                                slug: f.slug(),
                                reach,
                                via,
                                at: chrono::Utc::now(),
                            })
                            .is_ok()
                        })
                    })
                    .collect();
                let alive = handles.into_iter().all(|h| h.join().unwrap_or(false));
                if !alive {
                    return; // dashboard dropped the receiver
                }
                match kicked.recv_timeout(interval) {
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        });
        Self { results, kick }
    }

    /// Poll again now (e.g. after a finca was added).
    pub fn kick(&self) {
        let _ = self.kick.send(());
    }
}

/// The tmux session name for a finca connection.
pub fn session_name(slug: &str) -> String {
    format!("finca-{slug}")
}

/// The command string ssh hands the remote shell: `bin` run via a login shell.
pub fn remote_login_cmd(bin: &str) -> String {
    format!("exec \"$SHELL\" -lc {}", sh_quote(&sh_quote(bin)))
}

/// `sh -c` command that runs interactive ssh to `dest` (one of the finca's
/// destinations) into its grove and, if ssh fails, pauses so the error can
/// be read.
pub fn connect_argv(f: &Finca, dest: &str) -> Vec<String> {
    // Run grove under the remote user's login shell so its profile PATH
    // applies (e.g. Homebrew's tmux and claude on macOS). Quoting layers:
    // this local sh → ssh's remote shell → the login shell's -c.
    let remote = remote_login_cmd(&f.grove_bin);
    let ssh = format!(
        "{} -t -o ServerAliveInterval=15 -o ServerAliveCountMax=3 {} {}",
        sh_quote(&ssh_program()),
        sh_quote(dest),
        sh_quote(&remote)
    );
    let script = format!(
        "{ssh} || {{ code=$?; echo; echo \"ssh to {} exited ($code) — press Enter to close\"; read _; }}",
        f.name.replace(['"', '$', '`', '\\'], "")
    );
    vec!["sh".into(), "-c".into(), script]
}

pub fn ensure_finca_session(tmux: &crate::tmux::Tmux, f: &Finca, dest: &str) -> Result<bool> {
    let name = session_name(&f.slug());
    if tmux.has_session(&name) {
        return Ok(false);
    }
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/".into());
    tmux.new_session_detached(&name, &f.name, &home, &connect_argv(f, dest))?;
    tmux.set_option(&name, "key-table", "grove-finca")?;
    tmux.set_option(&name, "prefix", "None")?;
    // When ssh ends (remote q, dropped link) move the client to another
    // session (normally the dashboard) instead of detaching it.
    tmux.set_option(&name, "detach-on-destroy", "off")?;
    if let Some(c) = &f.color {
        let _ = tmux.set_option(&name, "status-style", &format!("bg={c},fg=#ffffff"));
    }
    Ok(true)
}

/// Connects the current client to the finca (creating the session if
/// needed) at `via`, the destination that last answered a poll, else `ssh`.
pub fn connect(tmux: &crate::tmux::Tmux, f: &Finca, via: Option<&str>) -> Result<()> {
    ensure_finca_session(tmux, f, via.unwrap_or(&f.ssh))?;
    tmux.switch_client(&format!("={}", session_name(&f.slug())))
}

/// Closes the finca's connection session, if open.
pub fn disconnect(tmux: &crate::tmux::Tmux, slug: &str) {
    let _ = tmux.kill_session(&session_name(slug));
}

#[cfg(test)]
pub mod testing {
    //! A stand-in for `ssh`: skips options and the destination, then runs the
    //! remote command locally with `$FAKE_HOME` as HOME.
    use std::path::{Path, PathBuf};

    pub fn fake_ssh(dir: &Path, exit_code: Option<i32>) -> PathBuf {
        let p = dir.join("fake ssh");
        let body = match exit_code {
            Some(code) => format!("#!/bin/sh\necho 'ssh: connect to host x: Connection refused' >&2\nexit {code}\n"),
            None => "#!/bin/sh\nwhile [ $# -gt 0 ]; do case \"$1\" in -o) shift 2;; -t|-T) shift;; *) break;; esac; done\nshift\nHOME=\"$FAKE_HOME\" exec sh -c \"$*\"\n".into(),
        };
        std::fs::write(&p, body).unwrap();
        std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        p
    }

    /// A fake grove that answers --version and `status --json` with `json`.
    pub fn fake_grove(dir: &Path, json: &str) -> PathBuf {
        let p = dir.join("grove");
        let body = format!(
            "#!/bin/sh\ncase \"$1\" in --version) echo 'grove 0.1.0';; status) cat <<'JSON'\n{json}\nJSON\n;; *) exit 2;; esac\n"
        );
        std::fs::write(&p, body).unwrap();
        std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        p
    }
}

#[cfg(test)]
#[path = "finca_tests.rs"]
mod tests;
