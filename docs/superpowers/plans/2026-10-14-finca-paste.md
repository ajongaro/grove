# Finca Paste Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In a finca window, `C-v` sends this Mac's clipboard (a file copied in Finder, or an image) to the finca and pastes its path there. `grove send <finca> [file]` does the same from the command line.

**Architecture:**
- A new `src/transfer.rs` holds:
  - the name rule;
  - the clipboard reader (osascript, swappable for a fake via `GROVE_OSASCRIPT`);
  - `send`, which streams a file over the finca's shared SSH connection into `~/.grove/inbox`;
  - `paste`, the `C-v` flow.
- The tmux binding runs a hidden `grove paste` in the background and passes the session and pane.
- Finca sessions record the address they connected through as `@grove_dest`.

**Tech Stack:** Rust 2024, clap, chrono, tmux CLI, OpenSSH, macOS osascript.

**Spec:** `docs/superpowers/specs/2026-10-14-finca-paste-design.md`

## Global Constraints

- The clipboard order is: a file reference first, then an image, else pass `C-v` through.
- Inbox: `~/.grove/inbox` on the finca. Names are `YYYYMMDD-HHMMSS-<clean name>`, and a clipboard image is named `clipboard.png`. A send writes `<name>.part`, then renames it, and removes inbox files older than 7 days.
- `grove paste` never prints. tmux would show the output of a `run-shell` command. It reports through `display-message` and always exits 0.
- Local sessions (the root table) don't get a `C-v` binding.
- No personal hosts in code, tests or docs (use `homebox`, `me@homebox.local`).
- Every commit ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Run `cargo test` and `cargo clippy --all-targets -- -D warnings` before each commit.

## Review Focus

1. **Finder copy of an image file:** the file is sent, not its icon. Covered by the file-first order. Test: Task 2, `file_reference_wins_over_an_image`.
2. **Failed image read leaves an empty temp file:** it must be removed, and treated as no image. Test: Task 2, `text_clipboard_is_nothing_and_leaves_no_temp_file`.
3. **Finca unreachable on `C-v`:** a status message, nothing pasted, and no output that tmux would show. Test: Task 3, `paste_reports_a_failed_send_and_pastes_nothing`.
4. **File names with spaces, quotes or non-ASCII characters:** cleaned, never breaking the remote command. Test: Task 1, `names_are_cleaned_and_stamped`.
5. **A recorded `@grove_dest` that's no longer one of the finca's destinations** (the file was edited): ignore it and probe. Test: Task 1, `recorded_destination_must_belong_to_the_finca`.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/transfer.rs` (new) | `remote_name`, `pick_dest`, `send`, `Clip`/`read_clipboard`, `paste` |
| `src/transfer_tests.rs` (new) | unit tests |
| `src/finca.rs` | `finca_for_session`; `@grove_dest` on both session kinds |
| `src/tmux.rs` | `session_option`, `send_keys`, `paste_text`, `display_message` |
| `src/runfiles.rs` | the `C-v` bindings |
| `src/main.rs` | `send`, hidden `paste` |
| `tests/cli.rs` | `paste` on a test tmux server |
| `src/ui/help.rs`, `README.md` | docs |

---

### Task 1: Names, destinations, and `grove send <finca> <file>`

**Files:** Create `src/transfer.rs` and `src/transfer_tests.rs`. Modify `src/main.rs` and `src/finca.rs`.

**Interfaces (produces):**
- `pub fn remote_name(original: &str, now: chrono::DateTime<chrono::Local>) -> String`
- `pub fn pick_dest(f: &Finca, recorded: Option<&str>) -> Result<String>`
- `pub fn send(dest: &str, file: &Path, name: &str) -> Result<String>` (the remote path)
- `pub fn finca_for_session(name: &str) -> Option<&str>` in finca.rs
- CLI: `grove send <finca> [file]` (the clipboard comes in Task 2)

- [ ] **Step 1: Failing tests** in `src/transfer_tests.rs`:

```rust
use super::*;
use crate::finca::testing::fake_ssh;
use crate::finca::tests::with_env;

fn at(h: u32, m: u32, s: u32) -> chrono::DateTime<chrono::Local> {
    use chrono::TimeZone;
    chrono::Local.with_ymd_and_hms(2026, 10, 14, h, m, s).unwrap()
}

#[test]
fn names_are_cleaned_and_stamped() {
    assert_eq!(remote_name("shot.png", at(9, 30, 12)), "20261014-093012-shot.png");
    assert_eq!(
        remote_name("Screen Shot 'a' é.png", at(9, 30, 12)),
        "20261014-093012-Screen-Shot--a---.png"
    );
    assert_eq!(remote_name("", at(1, 2, 3)), "20261014-010203-file");
}

#[test]
fn session_names_map_to_finca_slugs() {
    assert_eq!(crate::finca::finca_for_session("finca-homebox"), Some("homebox"));
    assert_eq!(crate::finca::finca_for_session("finca-homebox-sh"), Some("homebox"));
    assert_eq!(crate::finca::finca_for_session("grove"), None);
}

fn finca(ssh: &str, backup: Option<&str>) -> Finca {
    Finca {
        name: "homebox".into(),
        ssh: ssh.into(),
        grove_bin: "/g".into(),
        color: None,
        backup: backup.map(str::to_string),
    }
}

#[test]
fn recorded_destination_must_belong_to_the_finca() {
    let d = tempfile::tempdir().unwrap();
    let ssh = fake_ssh(d.path(), None);
    let f = finca("me@homebox.local", Some("me@homebox.ts.net"));
    with_env(&ssh, d.path(), || {
        assert_eq!(pick_dest(&f, Some("me@homebox.ts.net")).unwrap(), "me@homebox.ts.net");
        // Not one of its destinations (the file changed): probe instead.
        assert_eq!(pick_dest(&f, Some("me@elsewhere")).unwrap(), "me@homebox.local");
        assert_eq!(pick_dest(&f, None).unwrap(), "me@homebox.local");
    });
    let down = fake_ssh(&d.path().join("down"), Some(255));
    let err = with_env(&down, d.path(), || pick_dest(&f, None)).unwrap_err();
    assert!(format!("{err:#}").contains("homebox"), "{err:#}");
}

#[test]
fn send_lands_in_the_inbox_and_clears_old_files() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("remote home");
    let inbox = home.join(".grove/inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    let old = inbox.join("20200101-000000-old.png");
    std::fs::write(&old, "x").unwrap();
    std::process::Command::new("touch")
        .args(["-t", "202001010000"])
        .arg(&old)
        .status()
        .unwrap();
    let src = d.path().join("shot.png");
    std::fs::write(&src, b"\x89PNG data").unwrap();
    let ssh = fake_ssh(d.path(), None);
    let path = with_env(&ssh, &home, || send("me@box", &src, "20261014-093012-shot.png")).unwrap();
    assert_eq!(path, inbox.join("20261014-093012-shot.png").to_string_lossy());
    assert_eq!(std::fs::read(&path).unwrap(), b"\x89PNG data");
    assert!(!old.exists(), "older than 7 days");
    assert!(!inbox.join("20261014-093012-shot.png.part").exists());
}
```

The fake ssh creates `<dir>/fake-ssh`. If `fake_ssh` writes to a fixed name in `dir`, create `d.path().join("down")` first with `std::fs::create_dir_all`.

- [ ] **Step 2: Run** `cargo test --bin grove transfer::` and expect a compile failure.

- [ ] **Step 3: Implement.** `src/transfer.rs`:

```rust
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
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '-' })
        .collect();
    let clean = if clean.is_empty() { "file".to_string() } else { clean };
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
    Ok(String::from_utf8_lossy(&out.stdout).lines().last().unwrap_or_default().trim().to_string())
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
```

In `src/finca.rs`, next to `shell_session_name`:

```rust
/// The finca slug of a finca connection (`finca-<slug>`) or shell
/// (`finca-<slug>-sh`) session.
pub fn finca_for_session(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("finca-")?;
    Some(rest.strip_suffix("-sh").unwrap_or(rest)).filter(|s| !s.is_empty())
}
```

Check that `session_name(slug)` is `finca-{slug}`. If it differs, match whatever it produces and adjust the test.

In `src/main.rs`, add `mod transfer;`. Add `Cmd::Send { finca: String, file: Option<PathBuf> }`, documented "Send a file (or this Mac's clipboard) to a finca's ~/.grove/inbox; prints the path there", and handle it with `exit_on_err(send_cmd(&paths, finca, file))`:

```rust
fn send_cmd(paths: &Paths, name: String, file: Option<PathBuf>) -> Result<()> {
    let store = Store::new(paths.clone());
    let f = store.get_finca(&name)?;
    let Some(file) = file else {
        bail!("give a file to send");
    };
    let original = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let dest = transfer::pick_dest(&f, None)?;
    let path = transfer::send(&dest, &file, &transfer::remote_name(&original, chrono::Local::now()))?;
    println!("{path}");
    Ok(())
}
```

- [ ] **Step 4: Run** `cargo test --bin grove transfer::` and expect 4 passes. Run `cargo test` and clippy. (Dead-code allowances for the parts Task 3 uses should be marked "used by paste (Task 3)".)

- [ ] **Step 5: Commit** `feat(fincas): grove send puts a file in a finca's inbox`.

---

### Task 2: Reading the clipboard

**Files:** Modify `src/transfer.rs`, `src/transfer_tests.rs` and `src/main.rs`.

**Interfaces (produces):**
- `pub enum Clip { File(PathBuf), Image(PathBuf), Nothing }`
- `pub fn read_clipboard(run_dir: &Path) -> Clip`. The `Image` path is a temp file the caller deletes.
- `grove send <finca>` with no file sends the clipboard.

- [ ] **Step 1: Failing tests:**

```rust
/// A fake osascript: `FAKE_CLIP_FILE` answers the file query, and
/// `FAKE_CLIP_PNG` (a file) the image query. Like the real one, a failed
/// image read leaves an empty file behind.
fn fake_osascript(dir: &Path) -> std::path::PathBuf {
    crate::finca::tests::script(
        dir,
        "osascript",
        "for a; do last=$a; done\n\
         case \"$*\" in\n\
         *furl*) [ -n \"$FAKE_CLIP_FILE\" ] || exit 1; echo \"$FAKE_CLIP_FILE\";;\n\
         *PNGf*) : > \"$last\"; [ -n \"$FAKE_CLIP_PNG\" ] || exit 1; cat \"$FAKE_CLIP_PNG\" > \"$last\";;\n\
         *) exit 1;;\n\
         esac",
    )
}

fn with_clip<T>(dir: &Path, file: Option<&Path>, png: Option<&Path>, f: impl FnOnce() -> T) -> T {
    let _g = crate::finca::tests::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let set = |k: &str, v: Option<&Path>| match v {
        Some(v) => unsafe { std::env::set_var(k, v) },
        None => unsafe { std::env::remove_var(k) },
    };
    set("GROVE_OSASCRIPT", Some(&fake_osascript(dir)));
    set("FAKE_CLIP_FILE", file);
    set("FAKE_CLIP_PNG", png);
    let r = f();
    for k in ["GROVE_OSASCRIPT", "FAKE_CLIP_FILE", "FAKE_CLIP_PNG"] {
        unsafe { std::env::remove_var(k) };
    }
    r
}

#[test]
fn file_reference_wins_over_an_image() {
    let d = tempfile::tempdir().unwrap();
    let doc = d.path().join("report.pdf");
    std::fs::write(&doc, "pdf").unwrap();
    let png = d.path().join("icon.png");
    std::fs::write(&png, "icon").unwrap();
    let clip = with_clip(d.path(), Some(&doc), Some(&png), || read_clipboard(d.path()));
    assert_eq!(clip, Clip::File(doc));
}

#[test]
fn image_is_written_to_a_temp_png() {
    let d = tempfile::tempdir().unwrap();
    let png = d.path().join("shot.png");
    std::fs::write(&png, "pixels").unwrap();
    let clip = with_clip(d.path(), None, Some(&png), || read_clipboard(d.path()));
    let Clip::Image(p) = clip else { panic!("{clip:?}") };
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "pixels");
}

#[test]
fn text_clipboard_is_nothing_and_leaves_no_temp_file() {
    let d = tempfile::tempdir().unwrap();
    let run = d.path().join("run");
    std::fs::create_dir_all(&run).unwrap();
    let clip = with_clip(d.path(), None, None, || read_clipboard(&run));
    assert_eq!(clip, Clip::Nothing);
    assert_eq!(std::fs::read_dir(&run).unwrap().count(), 0);
}
```

- [ ] **Step 2: Run** `cargo test --bin grove transfer::` and expect a compile failure.

- [ ] **Step 3: Implement.** Before writing it, run the real osascript `on run argv` form once by hand (Step 3a below) to confirm that the argv form writes the PNG.

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum Clip {
    /// A file copied in Finder.
    File(std::path::PathBuf),
    /// An image, written to this temp PNG (the caller deletes it).
    Image(std::path::PathBuf),
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
        let p = std::path::PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
        if p.is_file() {
            return Clip::File(p);
        }
    }
    let tmp = run_dir.join(format!("clipboard-{}.png", std::process::id()));
    let ok = std::process::Command::new(&osa)
        .args([
            "-e", "on run argv",
            "-e", "set f to open for access POSIX file (item 1 of argv) with write permission",
            "-e", "set eof f to 0",
            "-e", "write (the clipboard as «class PNGf») to f",
            "-e", "close access f",
            "-e", "end run",
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
```

Step 3a, by hand on this Mac, with an image on the clipboard: run the same `-e` lines with a temp path argument, and check `file <path>` says PNG. If `open for access` stays open after a failed `write`, add `try` / `on error` / `close access f` / `error number -1` around the write, and keep the empty-file cleanup.

In `send_cmd`, replace the `bail!("give a file to send")` branch with:

```rust
let (file, original, temp) = match file {
    Some(p) => {
        let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        (p, n, false)
    }
    None => match transfer::read_clipboard(&paths.run) {
        transfer::Clip::File(p) => {
            let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            (p, n, false)
        }
        transfer::Clip::Image(p) => (p, "clipboard.png".to_string(), true),
        transfer::Clip::Nothing => bail!("no file or image on the clipboard (or it can't be read here); give a file"),
    },
};
```

Then delete `file` after sending when `temp` is set, whether the send succeeded or not. `paths.ensure()` must run before reading, so that `run` exists.

- [ ] **Step 4: Run** the transfer tests (7 should pass), then `cargo test` and clippy.

- [ ] **Step 5: Commit** `feat(fincas): grove send sends the clipboard (Finder file or image)`.

---

### Task 3: `C-v` in finca windows

**Files:** Modify `src/transfer.rs`, `src/finca.rs`, `src/tmux.rs`, `src/runfiles.rs`, `src/main.rs`, `tests/cli.rs`, `src/ui/help.rs` and `README.md`.

**Interfaces (produces):**
- `Tmux::session_option(session, opt) -> Option<String>`, `send_keys(target, &[&str]) -> Result<()>`, `paste_text(target, text) -> Result<()>`, `display_message(target, msg)`;
- `pub fn paste(paths: &Paths, tmux: &Tmux, session: &str, pane: &str)`, which never fails outward;
- hidden `Cmd::Paste { #[arg(long)] session: String, #[arg(long)] pane: String }`;
- the `@grove_dest` session option.

- [ ] **Step 1: Failing tests.**

In `src/runfiles.rs` `tmux_conf_has_grove_keys_and_loads_local_tweaks_last`, add:

```rust
for table in ["grove-finca", "grove-shell"] {
    let line = c
        .lines()
        .find(|l| l.starts_with(&format!("bind -T {table} C-v run-shell -b ")))
        .unwrap_or_else(|| panic!("C-v in {table}: {c}"));
    assert!(line.contains("paste --session") && line.contains("#{pane_id}"), "{line}");
}
assert!(!c.lines().any(|l| l.starts_with("bind -n C-v")), "local sessions keep C-v");
```

In `tests/cli.rs`:

```rust
/// A test tmux server with session `finca-homebox` (a pane running
/// `cat -v` in raw-ish mode), a finca file, a fake ssh and a fake osascript.
struct PasteRig {
    _d: tempfile::TempDir,
    home: std::path::PathBuf,
    remote: std::path::PathBuf,
    socket: String,
    env: Vec<(String, std::path::PathBuf)>,
}

impl PasteRig {
    fn new(tag: &str, ssh_exit: Option<i32>) -> Self {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        std::fs::create_dir_all(home.join("fincas")).unwrap();
        std::fs::create_dir_all(home.join("run")).unwrap();
        std::fs::write(
            home.join("fincas/homebox.toml"),
            "name = \"homebox\"\nssh = \"me@homebox.local\"\ngrove_bin = \"/g\"\n",
        )
        .unwrap();
        let remote = d.path().join("remote home");
        std::fs::create_dir_all(&remote).unwrap();
        let ssh = d.path().join("fake ssh");
        let body = match ssh_exit {
            Some(c) => format!("#!/bin/sh\necho 'connection refused' >&2\nexit {c}\n"),
            None => "#!/bin/sh\nwhile [ $# -gt 0 ]; do case \"$1\" in -o) shift 2;; -t|-T) shift;; *) break;; esac; done\nshift\nHOME=\"$FAKE_HOME\" exec sh -c \"$*\"\n".into(),
        };
        std::fs::write(&ssh, body).unwrap();
        let osa = d.path().join("osascript");
        std::fs::write(&osa, "#!/bin/sh\nfor a; do last=$a; done\ncase \"$*\" in\n*furl*) [ -n \"$FAKE_CLIP_FILE\" ] || exit 1; echo \"$FAKE_CLIP_FILE\";;\n*PNGf*) : > \"$last\"; [ -n \"$FAKE_CLIP_PNG\" ] || exit 1; cat \"$FAKE_CLIP_PNG\" > \"$last\";;\n*) exit 1;;\nesac\n").unwrap();
        for p in [&ssh, &osa] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let socket = format!("grove-test-{}-{tag}", std::process::id());
        let rig = Self {
            env: vec![
                ("GROVE_SSH".into(), ssh),
                ("FAKE_HOME".into(), remote.clone()),
                ("GROVE_OSASCRIPT".into(), osa),
            ],
            home,
            remote,
            socket,
            _d: d,
        };
        rig.tmux(&["new-session", "-d", "-s", "finca-homebox", "-x", "120", "-y", "10",
            "sh", "-c", "stty -icanon -iexten -echo; cat -v"]);
        rig.tmux(&["set-option", "-t", "finca-homebox", "@grove_dest", "me@homebox.local"]);
        rig
    }

    fn tmux(&self, args: &[&str]) -> String {
        let out = Command::new("tmux")
            .args(["-L", &self.socket, "-f", "/dev/null"])
            .args(args)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn paste(&self, clip_png: Option<&std::path::Path>) -> std::process::Output {
        let pane = self.tmux(&["display-message", "-p", "-t", "finca-homebox", "#{pane_id}"]);
        let mut c = grove(&self.home);
        c.args(["paste", "--session", "finca-homebox", "--pane", &pane])
            .env("GROVE_TMUX_SOCKET", &self.socket)
            .env_remove("FAKE_CLIP_FILE");
        for (k, v) in &self.env {
            c.env(k, v);
        }
        match clip_png {
            Some(p) => c.env("FAKE_CLIP_PNG", p),
            None => c.env_remove("FAKE_CLIP_PNG"),
        };
        c.output().unwrap()
    }

    /// The pane's text, after giving cat a moment to echo.
    fn screen(&self) -> String {
        for _ in 0..50 {
            let s = self.tmux(&["capture-pane", "-p", "-t", "finca-homebox"]);
            if !s.trim().is_empty() {
                return s;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        String::new()
    }
}

impl Drop for PasteRig {
    fn drop(&mut self) {
        let sock = self.tmux(&["display-message", "-p", "#{socket_path}"]);
        self.tmux(&["kill-server"]);
        let _ = std::fs::remove_file(sock);
    }
}

#[test]
fn paste_sends_a_clipboard_image_and_types_its_path() {
    let rig = PasteRig::new("paste-img", None);
    let png = rig.home.join("shot.png");
    std::fs::write(&png, "pixels").unwrap();
    let out = rig.paste(Some(&png));
    assert!(out.status.success() && out.stdout.is_empty() && out.stderr.is_empty(), "{out:?}");
    let inbox = rig.remote.join(".grove/inbox");
    let sent: Vec<_> = std::fs::read_dir(&inbox).unwrap().map(|e| e.unwrap().path()).collect();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].to_string_lossy().ends_with("-clipboard.png"));
    assert_eq!(std::fs::read_to_string(&sent[0]).unwrap(), "pixels");
    assert!(rig.screen().contains(&*sent[0].to_string_lossy()), "{}", rig.screen());
    assert_eq!(std::fs::read_dir(rig.home.join("run")).unwrap().count(), 0, "temp png removed");
}

#[test]
fn paste_without_an_image_passes_c_v_through() {
    let rig = PasteRig::new("paste-text", None);
    let out = rig.paste(None);
    assert!(out.status.success(), "{out:?}");
    assert!(rig.screen().contains("^V"), "{}", rig.screen());
}

#[test]
fn paste_reports_a_failed_send_and_pastes_nothing() {
    let rig = PasteRig::new("paste-down", Some(255));
    let png = rig.home.join("shot.png");
    std::fs::write(&png, "pixels").unwrap();
    let out = rig.paste(Some(&png));
    assert!(out.status.success() && out.stdout.is_empty() && out.stderr.is_empty(), "{out:?}");
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(rig.tmux(&["capture-pane", "-p", "-t", "finca-homebox"]).trim().is_empty());
}
```

Check: if the pane's `stty` line races the first paste, sleep about 100ms after `new-session`.

- [ ] **Step 2: Run** `cargo test --test cli paste` and `cargo test --bin grove runfiles::` and expect failures (`paste` is an unknown subcommand, and there's no binding).

- [ ] **Step 3: Implement.**

`src/tmux.rs`:

```rust
/// A session's option (e.g. a user `@` option), if set.
pub fn session_option(&self, session: &str, option: &str) -> Option<String> {
    let out = self.cmd().args(["show-options", "-qv", "-t", session, option]).output().ok()?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !v.is_empty()).then_some(v)
}

pub fn send_keys(&self, target: &str, keys: &[&str]) -> Result<()> {
    let mut args = vec!["send-keys", "-t", target];
    args.extend_from_slice(keys);
    self.run(&args).map(|_| ())
}

/// Pastes `text` into `target` as a paste (bracketed if the program asked).
pub fn paste_text(&self, target: &str, text: &str) -> Result<()> {
    self.run(&["set-buffer", "-b", "grove-paste", "--", text])?;
    self.run(&["paste-buffer", "-p", "-d", "-b", "grove-paste", "-t", target]).map(|_| ())
}

pub fn display_message(&self, target: &str, msg: &str) {
    let _ = self.run(&["display-message", "-d", "4000", "-t", target, msg]);
}
```

`src/finca.rs`: in `ensure_finca_session` and `ensure_shell_session`, after `new_session_detached`, add `tmux.set_option(&name, "@grove_dest", dest)?;` with the comment `// Where this connection goes, for C-v (transfer::paste).`

`src/transfer.rs`:

```rust
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
        Clip::File(p) => {
            let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            (p, n, false)
        }
        Clip::Image(p) => (p, "clipboard.png".to_string(), true),
    };
    let name = remote_name(&original, chrono::Local::now());
    let result = (|| {
        let slug = finca::finca_for_session(session)
            .ok_or_else(|| anyhow::anyhow!("{session} isn't a finca window"))?;
        let f = crate::store::Store::new(paths.clone()).get_finca(slug)?;
        let dest = pick_dest(&f, tmux.session_option(session, "@grove_dest").as_deref())?;
        Ok::<_, anyhow::Error>((f.name.clone(), send(&dest, &file, &name)?))
    })();
    if temp {
        let _ = std::fs::remove_file(&file);
    }
    match result {
        Ok((finca_name, remote)) => {
            let _ = tmux.paste_text(pane, &format!("{remote} "));
            tmux.display_message(pane, &format!("sent {original} to {finca_name}"));
        }
        Err(e) => tmux.display_message(pane, &format!("couldn't send: {e:#}")),
    }
}
```

`src/main.rs`: add `#[command(hide = true)] Paste { #[arg(long)] session: String, #[arg(long)] pane: String }`, handled as:

```rust
Some(Cmd::Paste { session, pane }) => {
    let _ = paths.ensure();
    transfer::paste(&paths, &grove_tmux(&paths), &session, &pane);
}
```

`grove_tmux` passes `-f run/tmux.conf`. That's harmless for commands sent to a running server.

`src/runfiles.rs`, in the `format!`: compute

```rust
let exe = std::env::current_exe()
    .map(|p| p.to_string_lossy().into_owned())
    .unwrap_or_else(|_| "grove".into());
let paste = format!(
    "run-shell -b \"'{}' paste --session '#{{session_name}}' --pane '#{{pane_id}}' >/dev/null 2>&1\"",
    exe.replace('\'', "")
);
```

and add the line `bind -T grove-finca C-v {paste}` plus the same for `grove-shell`, with a comment: `# C-v in a finca window sends this Mac's clipboard file or image there (transfer.rs).`

`src/ui/help.rs` "Anywhere": `("C-v", "in a finca: send the clipboard's file or image, paste its path")`.

README:
- In the Fincas section, add a bullet. **`C-v`** in a finca window sends a file you copied in Finder (⌘C) or an image on the clipboard (a screenshot taken with ⌘⇧⌃4) to the finca's `~/.grove/inbox`, and pastes its path. Claude there reads it as an image. Text passes through, so use ⌘V for text. Inbox files go after 7 days. macOS only, for now.
- Add `grove send homebox ~/Desktop/shot.png  # prints the path there (no file: the clipboard)` to the command block.
- Add `| C-v | finca window | send the clipboard's file or image there, paste its path |` to the key table.

- [ ] **Step 4: Run** `cargo test` (including the 3 cli paste tests and the runfiles test) and clippy. All should pass.

- [ ] **Step 5: Commit** `feat(fincas): C-v in a finca window sends the clipboard and pastes its path`.

---

### Task 4: Live verification

- [ ] `cargo install --path .`, then `grove finca update`, so both fincas have the new build. The receiving side only needs sh, but the new `@grove_dest` is set by the local Grove, so the user must restart the dashboard with `R`.
- [ ] `grove send <pi finca> <a local png>` prints a path under `~/.grove/inbox` on the finca, and `ssh` there shows the file with the same size.
- [ ] Ask the user to:
  - take a screenshot with ⌘⇧⌃4 and press `C-v` in a Claude session on each finca;
  - check it attaches as an image (`[Image #1]`) or that Claude reads it;
  - also try a Finder-copied file, and text passing through.
- [ ] Commit any fixes.
