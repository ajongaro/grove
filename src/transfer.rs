//! Sending files to fincas: `grove send`, and `C-v` in a finca window
//! (`grove paste`), which sends this machine's clipboard and pastes the
//! path there.

use std::path::Path;

use anyhow::{Result, bail};

use crate::finca::{self, Finca, sh_quote};

/// `YYYYMMDD-HHMMSS-<name>`, keeping only letters, digits, `.`, `-`, `_`.
pub fn remote_name(original: &str, now: chrono::DateTime<chrono::Local>) -> String {
    let clean: String = original
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let clean = if clean.is_empty() {
        "file".to_string()
    } else {
        clean
    };
    format!("{}-{clean}", now.format("%Y%m%d-%H%M%S"))
}

/// The address to send through: the one the session connected with, if
/// it's still one of the finca's, else the first that answers.
pub fn pick_dest(f: &Finca, recorded: Option<&str>) -> Result<String> {
    if let Some(r) = recorded.filter(|r| f.destinations().any(|d| d == *r)) {
        return Ok(r.to_string());
    }
    match f
        .destinations()
        .find(|d| finca::run_remote(d, "true").is_ok_and(|o| o.status.success()))
    {
        Some(d) => Ok(d.to_string()),
        None => bail!("{} doesn't answer over SSH", f.name),
    }
}

/// Streams `file` to `~/.grove/inbox/<name>` at `dest` and returns that path.
pub fn send(dest: &str, file: &Path, name: &str) -> Result<String> {
    let input = std::fs::File::open(file)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", file.display()))?;
    let n = sh_quote(name);
    let part = sh_quote(&format!("{name}.part"));
    let cmd = format!(
        "set -e; d=\"$HOME/.grove/inbox\"; mkdir -p \"$d\"; \
         find \"$d\" -type f -mtime +7 -exec rm -f {{}} + 2>/dev/null || true; \
         cat > \"$d\"/{part}; mv -f \"$d\"/{part} \"$d\"/{n}; printf '%s\\n' \"$d\"/{n}"
    );
    let out = finca::remote_stdin(dest, &cmd, input.into())?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .last()
        .unwrap_or_default()
        .trim()
        .to_string())
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
