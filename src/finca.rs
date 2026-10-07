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
}

impl Finca {
    pub fn slug(&self) -> String {
        slugify(&self.name)
    }
}

/// `$GROVE_SSH`, else `ssh`. Tests point this at a fake.
pub fn ssh_program() -> String {
    std::env::var("GROVE_SSH").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "ssh".into())
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
        Some(0) if stdout.starts_with('/') => Ok(stdout.lines().last().unwrap_or(&stdout).to_string()),
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
