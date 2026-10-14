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
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if err.is_empty() {
            bail!("ssh exited {}", out.status.code().unwrap_or(-1));
        }
        bail!("{err}");
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .last()
        .unwrap_or_default()
        .trim()
        .to_string())
}

#[derive(Debug, Clone, PartialEq)]
pub enum Clip {
    /// A file copied in Finder.
    File(std::path::PathBuf),
    /// An image, written to this temp PNG (the caller deletes it).
    Image(std::path::PathBuf),
    /// A folder copied in Finder: not sent.
    Folder(std::path::PathBuf),
    Nothing,
}

/// `$GROVE_OSASCRIPT` (tests), else osascript on macOS.
fn osascript() -> Option<String> {
    match std::env::var("GROVE_OSASCRIPT") {
        Ok(p) if !p.is_empty() => Some(p),
        _ => cfg!(target_os = "macos").then(|| "osascript".to_string()),
    }
}

/// What's on this machine's clipboard that can be sent: a Finder file
/// first (Finder may also put its icon there as an image), then an image.
pub fn read_clipboard(run_dir: &Path) -> Clip {
    let Some(osa) = osascript() else {
        return Clip::Nothing;
    };
    let file = std::process::Command::new(&osa)
        .args(["-e", "POSIX path of (the clipboard as «class furl»)"])
        .output();
    if let Ok(o) = file
        && o.status.success()
    {
        // Only the newline: file names may end in spaces.
        let out = String::from_utf8_lossy(&o.stdout);
        let p = std::path::PathBuf::from(out.trim_end_matches(['\n', '\r']));
        if p.is_file() {
            return Clip::File(p);
        }
        // A copied folder comes with its icon as an image: never send that.
        if p.is_dir() {
            return Clip::Folder(p);
        }
    }
    let tmp = run_dir.join(format!("clipboard-{}.png", std::process::id()));
    let ok = std::process::Command::new(&osa)
        .args([
            "-e",
            "on run argv",
            "-e",
            "set f to open for access POSIX file (item 1 of argv) with write permission",
            "-e",
            "set eof f to 0",
            "-e",
            "write (the clipboard as «class PNGf») to f",
            "-e",
            "close access f",
            "-e",
            "end run",
        ])
        .arg(&tmp)
        .output()
        .is_ok_and(|o| o.status.success());
    if ok && std::fs::metadata(&tmp).is_ok_and(|m| m.len() > 0) {
        Clip::Image(tmp)
    } else {
        let _ = std::fs::remove_file(&tmp);
        Clip::Nothing
    }
}

/// The status line after a failed send.
pub fn failure(finca: &str, e: &anyhow::Error) -> String {
    format!("couldn't send to {finca}: {e:#}")
}

/// `C-v` in a finca window: sends the clipboard's file or image and pastes
/// its path in `pane`, or passes `C-v` through when there's nothing to
/// send. Reports through tmux, never stdout (run-shell would show it).
pub fn paste(paths: &crate::config::Paths, tmux: &crate::tmux::Tmux, session: &str, pane: &str) {
    let clip = read_clipboard(&paths.run);
    let (file, original, temp) = match clip {
        Clip::Nothing => {
            let _ = tmux.send_keys(pane, &["C-v"]);
            return;
        }
        Clip::Folder(p) => {
            tmux.display_message(pane, &format!("can't send a folder ({})", p.display()));
            return;
        }
        Clip::File(p) => {
            let n = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            (p, n, false)
        }
        Clip::Image(p) => (p, "clipboard.png".to_string(), true),
    };
    let name = remote_name(&original, chrono::Local::now());
    let slug = finca::finca_for_session(session).unwrap_or(session);
    let mut finca_name = slug.to_string();
    let result = (|| {
        let f = crate::store::Store::new(paths.clone()).get_finca(slug)?;
        finca_name = f.name.clone();
        let dest = pick_dest(&f, tmux.session_option(session, "@grove_dest").as_deref())?;
        send(&dest, &file, &name)
    })();
    if temp {
        let _ = std::fs::remove_file(&file);
    }
    match result {
        Ok(remote) => {
            let _ = tmux.paste_text(pane, &format!("{remote} "));
            tmux.display_message(pane, &format!("sent {original} to {finca_name}"));
        }
        Err(e) => tmux.display_message(pane, &failure(&finca_name, &e)),
    }
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
