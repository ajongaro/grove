# Session Grid Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `v` on the dashboard opens a grid of live tiles, one per session
here and on fincas, with `hjkl` to move, `Tab` to jump to the next waiting
one, `Enter` to jump in, and `i` to reply in place.

**Architecture:**
- The checks behind MCP `send_prompt` move into `src/prompt.rs`, which the
  grid shares.
- `src/screens.rs` builds a machine's screens document (the `grove
  screens` command), fetches it from fincas over the shared SSH
  connection on a background `Fetcher`, and types replies on fincas
  (`grove type-prompt`).
- `src/ui/grid.rs` holds the pure layout and movement math, the tile
  list, and the drawing. `Mode::Grid` in `app.rs` makes it a dashboard
  mode.
- `run.rs` refreshes local screens every 500 ms while the grid is open.

**Tech Stack:** Rust 2024, ratatui 0.30 (crossterm), tmux CLI, ssh,
serde_json, clap.

**Spec:** `docs/superpowers/specs/2026-10-16-session-grid-design.md`

## Global Constraints

- **Keys:** `h j k l`/arrows move, `Tab`/`Shift-Tab` (crossterm
  `BackTab`) go to the next/previous waiting tile, `Enter` jumps, `i`
  replies (your turn only), `v`/`Esc` go back, `?` opens help, `q`
  detaches.
- **Tiles:**
  - minimum 40 columns × 8 rows; columns = width/40, at least 1 and at
    most the tile count;
  - border title `<project>:<name> <icon> <state>  <age>`, coloured with
    `render::state_color`; the icon is ⚑ for needs you, ● otherwise;
  - finca tiles are prefixed `<finca> · ` in `FINCAS_COLOR`;
  - the selected tile has a thick ACCENT border.
- **Local screens:** `capture-pane -p -J` (never `-e`), every 500 ms,
  only while the grid is open.
- **Finca screens:**
  - `grove screens --json --lines N` (default 40, max 200), fetched
    every 2 s per finca with a 5 s timeout;
  - stale after 6 s with no good result: dimmed, `stale` in the title,
    reply refused (`<finca> isn't answering`);
  - an older Grove (unknown command): status-only tiles, and a
    once-per-grid footer note `update Grove on <finca> for live screens`.
- **Replies:**
  - local replies go through `prompt::type_into`; finca replies through
    `grove type-prompt <session-id>`, with the text on stdin;
  - a failed reply keeps the typed text; success shows
    `sent to <handle>` and closes the reply line;
  - `i` on a needs-you session says `<handle> needs you: Enter to answer
    it there`; on working or starting, `<handle> is working; wait for
    your turn`.
- **Workflow:**
  - Tests: `cargo test --bin grove <filter>`; full suite `cargo test`.
  - Before every commit: `cargo fmt` and
    `cargo clippy --all-targets -- -D warnings`.
  - Commit only after those pass (`&&`, never `;`). Commits end with
    `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
  - `cargo fmt` reflows code, so re-check edits by grepping, not from
    memory.
  - Use generic names in tests and docs (`alpha`, `beta`, `pi`,
    `homebox`).

## Review Focus

- **The tile list changes under the selection** (a session ends or
  starts while the grid is open). `sel` is an index: it must be clamped
  to the new length on every key and draw, never panic, and `Enter`/`i`
  must act on the tile that's highlighted. Tests in Task 6 (clamp) and
  Task 7.
- **A stale or too-old finca tile:** `i` must refuse with the stale
  message, and must not fire a remote command. `Enter` still connects
  (connecting may wake it). Test in Task 7.
- **Replying to a finca session whose finca no longer answers, or whose
  session ended in the meantime:** `type-prompt` exits 1 with a message;
  the dashboard shows it and keeps the text. Tests in Task 3 and Task 9.
- **A very narrow or short terminal** (below 40×8, e.g. 30×10): one
  tile, clipped, no panic, no zero-width division. Test in Task 5.
- **Screen text with wide or odd characters** (CJK, emoji, tabs) in a
  tile: it's trimmed by display width, so tile borders stay intact. Test
  in Task 8.

---

### Task 1: `src/prompt.rs`: shared reply checks

**Files:**
- Create: `src/prompt.rs`
- Modify: `src/main.rs` (`mod prompt;`), `src/mcp.rs` (`do_send_prompt` delegates)
- Test: unit tests in `src/prompt.rs`; the existing `src/mcp_tests.rs` send_prompt tests must keep passing unchanged

**Interfaces:**
- Produces:
  - `pub fn prompt::clean(text: &str) -> Result<String, String>`:
    removes control characters other than `\n` and `\t`; errors on empty
    text, or on any line starting with `!` or `/` (lines split on `\n`,
    U+2028, U+2029).
  - `pub fn prompt::type_into(tmux: &Tmux, s: &Session, account: &str, text: &str) -> Result<String, String>`:
    the state check, `clean`, the window-ownership check, the paste, a
    150 ms pause, then Enter. Returns the handle `<project>:<name>`
    (`s.project` is the slug).

- [ ] **Step 1: Write the failing tests** (bottom of the new `src/prompt.rs`; module body empty for now)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_keeps_text_and_drops_controls() {
        assert_eq!(clean("hi\u{1b}[201~ there\r").unwrap(), "hi[201~ there");
        assert_eq!(clean("a\n\tb").unwrap(), "a\n\tb");
    }

    #[test]
    fn clean_refuses_empty_shell_and_slash_lines() {
        assert!(clean("  \u{7} ").is_err());
        for bad in ["!ls", "  /help", "ok\n!rm -rf ~", "ok\u{2028}/x", "ok\u{2029}!x"] {
            let e = clean(bad).unwrap_err();
            assert!(e.contains('!') || e.contains('/'), "{bad}: {e}");
        }
        assert!(clean("why does a/b fail? use x!").is_ok());
    }
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --bin grove prompt::tests 2>&1 | grep -E "^error" | head -3`
Expected: `cannot find function clean` (after `mod prompt;` is added to main.rs).

- [ ] **Step 3: Implement**, moving the logic out of `mcp.rs::do_send_prompt` and `mcp.rs::window`:

```rust
//! Typing a prompt into a session that's waiting for its next message.
//! Shared by the MCP `send_prompt` tool, the session grid and
//! `grove type-prompt`, so all three follow the same rules.

use crate::model::{Session, State};
use crate::tmux::Tmux;

/// Only text: control characters other than newline and tab go (an escape
/// sequence could end the paste and type keys of its own; a bare carriage
/// return would submit). Refuses text that's empty, or with a line starting
/// with `!` (a shell command in Claude's input) or `/` (a slash command):
/// neither goes through that session's permission prompts, and without
/// bracketed paste each line submits on its own.
pub fn clean(text: &str) -> Result<String, String> {
    let text: String = text
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect();
    if text.trim().is_empty() {
        return Err("nothing to send".into());
    }
    let starts = |c: char| {
        text.split(['\n', '\u{2028}', '\u{2029}'])
            .any(|l| l.trim_start().starts_with(c))
    };
    if starts('!') {
        return Err("a line starting with \"!\" would run a shell command there; rephrase it".into());
    }
    if starts('/') {
        return Err("a line starting with \"/\" would run a slash command there; rephrase it".into());
    }
    Ok(text)
}

/// The session's tmux window, if it's still that session's. Window ids
/// restart on a new tmux server, so a stale record could name someone
/// else's window: Grove marks each window with its session's id.
fn window(tmux: &Tmux, s: &Session, account: &str, handle: &str) -> Result<String, String> {
    let w = s
        .tmux_window
        .clone()
        .ok_or_else(|| format!("{handle} has no window yet (it may still be starting)"))?;
    let ours = match tmux.window_option(&w, "@grove_session") {
        Some(id) => id == s.id,
        // Windows from before Grove marked them: at least the account.
        None => tmux.window_option(&w, "@grove_account").as_deref() == Some(account),
    };
    if ours { Ok(w) } else { Err(format!("{handle}'s window is gone")) }
}

/// Types `text` into `s` (its project on `account`) and presses Enter.
/// Returns the session's handle, `project:name`.
pub fn type_into(tmux: &Tmux, s: &Session, account: &str, text: &str) -> Result<String, String> {
    let handle = format!("{}:{}", s.project, s.name);
    match s.state {
        State::YourTurn => {}
        State::NeedsYou => {
            return Err(format!("{handle} needs you: answer its prompt on the dashboard first"));
        }
        other => return Err(format!("{handle} is {}; wait until it's your turn", other.label())),
    }
    let text = clean(text)?;
    let w = window(tmux, s, account, &handle)?;
    tmux.paste_text(&w, &text)
        .map_err(|e| format!("couldn't type into {handle}: {e:#}"))?;
    // Claude's input may need a beat after a paste before Enter.
    std::thread::sleep(std::time::Duration::from_millis(150));
    tmux.send_keys(&w, &["Enter"]).map_err(|e| format!("{e:#}"))?;
    Ok(handle)
}
```

In `src/mcp.rs`:
- `do_send_prompt` keeps its `session()` lookup and its own-session
  check;
- then it returns
  `crate::prompt::type_into(&self.tmux, &s, &self.cfg().account_for(&p.path).name, &text).map(|h| format!("Sent to {h}."))`;
- delete the moved code: the state match, the control filter, the `!`/`/`
  checks, the paste and the sleep;
- `read_session` still needs the window check, so keep `GroveMcp::window`
  for it.

The handle in `type_into` uses `s.project`, the slug, which is the same
as `p.slug()` in mcp. The mcp tests check error text with `contains`
("working", "needs you", "gone", '!', '/'), so the wording above
satisfies them.

- [ ] **Step 4: Run the tests and watch them pass**

Run: `cargo test --bin grove -- prompt mcp 2>&1 | grep "test result"`
Expected: `ok` (all prompt tests, plus every existing mcp test unchanged).

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && \
git add -A src && git commit -m "refactor: share send_prompt's checks as prompt::type_into

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `grove screens`: a machine's live screens as JSON

**Files:**
- Create: `src/screens.rs`
- Modify: `src/main.rs` (`mod screens;`, hidden `Screens` subcommand)
- Test: unit tests in `src/screens.rs`, plus `tests/cli.rs` (CLI JSON shape)

**Interfaces:**
- Produces:
  - `#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)] pub struct Screens { pub version: u32, pub host: String, pub sessions: Vec<ScreenSession> }`
  - `pub struct ScreenSession { pub id: String, pub project: String, pub name: String, pub state: State, pub updated_at: DateTime<Utc>, pub window: Option<String>, pub screen: String }`
    (with the same derives)
  - `pub const SCREENS_VERSION: u32 = 1;`
  - `pub fn local(store: &Store, tmux: &Tmux, lines: usize, host: &str) -> Screens`:
    live sessions ordered by project slug, then `created_at`. A session's
    `screen` is the captured tail with trailing blank lines trimmed.
    It's empty when there's no window, the capture fails, or the window's
    `@grove_session` names another session.
  - `pub fn tail(screen: &str, lines: usize) -> String` (pure): trailing
    blank lines dropped, last `lines` kept.

- [ ] **Step 1: Write the failing tests** (in `src/screens.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use crate::model::Session;

    fn session(id: &str, project: &str, state: State, window: Option<&str>, mins: i64) -> Session {
        let t = chrono::Utc::now() - chrono::Duration::minutes(mins);
        Session {
            id: id.into(),
            project: project.into(),
            name: format!("s-{id}"),
            host: "local".into(),
            tmux_window: window.map(str::to_string),
            state,
            message: None,
            created_at: t,
            updated_at: t,
        }
    }

    #[test]
    fn tail_drops_trailing_blanks_and_keeps_the_last_lines() {
        assert_eq!(tail("a\nb\nc\n\n\n", 2), "b\nc");
        assert_eq!(tail("", 5), "");
        assert_eq!(tail("one", 0), "");
    }

    #[test]
    fn local_lists_live_sessions_with_screens() {
        let d = tempfile::tempdir().unwrap();
        let store = Store::new(Paths::new(d.path().join("home")));
        store.paths.ensure().unwrap();
        let tmux = Tmux::new(format!("grove-test-{}-screens", std::process::id()));
        tmux.new_session_detached(
            "grove",
            "w",
            d.path(),
            &["sh".into(), "-c".into(), "printf 'hello\\nfrom beta\\n'; sleep 30".into()],
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let w = tmux.list_windows_as("grove", "#{window_id}").unwrap()[0].clone();
        store.save_session(&session("b1", "beta", State::YourTurn, Some(&w), 5)).unwrap();
        tmux.set_window_option(&w, "@grove_session", "b1").unwrap();
        store.save_session(&session("a1", "alpha", State::Working, None, 9)).unwrap();
        store.save_session(&session("a2", "alpha", State::Ended, None, 1)).unwrap();
        // A stale record pointing at someone else's window shows nothing.
        store.save_session(&session("b2", "beta", State::Working, Some(&w), 1)).unwrap();
        let out = local(&store, &tmux, 40, "homebox");
        tmux.kill_server();
        let ids: Vec<_> = out.sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["a1", "b1", "b2"]);
        assert_eq!(out.version, SCREENS_VERSION);
        assert_eq!(out.host, "homebox");
        assert!(out.sessions[1].screen.ends_with("hello\nfrom beta"), "{:?}", out.sessions[1].screen);
        assert_eq!(out.sessions[0].screen, "");
        assert_eq!(out.sessions[2].screen, "");
    }
}
```

In `tests/cli.rs`:

```rust
#[test]
fn screens_prints_json_with_no_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let out = grove(&dir.path().join("home")).args(["screens", "--json"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["version"], 1);
    assert_eq!(v["sessions"], serde_json::json!([]));
}
```

(Check that `tests/cli.rs` already depends on serde_json; it's a normal
dependency, so integration tests can use it.)

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --bin grove screens::tests 2>&1 | grep -E "^error" | head -3`
Expected: `cannot find function tail` and similar errors.

- [ ] **Step 3: Implement** `src/screens.rs` (above the tests):

```rust
//! Live session screens: `grove screens --json` on a machine, fetched from
//! fincas by the session grid, and `grove type-prompt` for replying there.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::State;
use crate::store::Store;
use crate::tmux::Tmux;

pub const SCREENS_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Screens {
    pub version: u32,
    pub host: String,
    pub sessions: Vec<ScreenSession>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ScreenSession {
    pub id: String,
    pub project: String,
    pub name: String,
    pub state: State,
    pub updated_at: DateTime<Utc>,
    pub window: Option<String>,
    pub screen: String,
}

/// The last `lines` lines of `screen`, trailing blank lines dropped.
pub fn tail(screen: &str, lines: usize) -> String {
    let all: Vec<&str> = screen.trim_end().lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// This machine's live sessions and the bottom `lines` of each screen.
pub fn local(store: &Store, tmux: &Tmux, lines: usize, host: &str) -> Screens {
    let mut live: Vec<_> = store
        .list_sessions(None)
        .into_iter()
        .filter(|s| s.state != State::Ended)
        .collect();
    live.sort_by(|a, b| (&a.project, a.created_at).cmp(&(&b.project, b.created_at)));
    let sessions = live
        .into_iter()
        .map(|s| {
            let screen = s
                .tmux_window
                .as_deref()
                .filter(|w| {
                    tmux.window_option(w, "@grove_session")
                        .is_none_or(|id| id == s.id)
                })
                .and_then(|w| tmux.capture(w, lines).ok())
                .map(|text| tail(&text, lines))
                .unwrap_or_default();
            ScreenSession {
                id: s.id,
                project: s.project,
                name: s.name,
                state: s.state,
                updated_at: s.updated_at,
                window: s.tmux_window,
                screen,
            }
        })
        .collect();
    Screens { version: SCREENS_VERSION, host: host.to_string(), sessions }
}
```

In `src/main.rs`, add a hidden subcommand next to `Status`:

```rust
    /// Internal: live sessions with their screens, for the session grid.
    #[command(hide = true)]
    Screens {
        #[arg(long)]
        json: bool,
        /// Lines of each screen (most recent), at most 200.
        #[arg(long, default_value_t = 40)]
        lines: usize,
    },
```

The handler: `Some(Cmd::Screens { lines, .. }) => print_screens(&paths, lines.min(200))`.
`print_screens` builds the store and `grove_tmux(&paths)`, gets the host
the same way `print_status` does (reuse its helper; find it with
`grep -n "fn print_status" -A20 src/main.rs`), and prints
`serde_json::to_string(&screens::local(...))` on one line. JSON is the
only output format; `--json` is accepted for symmetry with `status`.

- [ ] **Step 4: Run the tests and watch them pass**

Run: `cargo test --bin grove screens::tests 2>&1 | grep "test result"; cargo test --test cli screens 2>&1 | grep "test result"`
Expected: both `ok`.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && \
git add -A src tests && git commit -m "feat: grove screens prints live sessions with their screens

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Remote side: `grove type-prompt`, fetching screens, replying on a finca

**Files:**
- Modify: `src/finca.rs` (add `run_timed`), `src/screens.rs`, `src/main.rs` (hidden `TypePrompt`)
- Test: `src/screens.rs` tests (using `finca::testing::fake_ssh`), `tests/cli.rs`

**Interfaces:**
- Consumes: `prompt::type_into` (Task 1), `Screens` (Task 2), `finca::{login_shell_cmd, sh_quote, ssh_program, SSH_BATCH_OPTS, share_opts}`.
- Produces:
  - `pub struct finca::RemoteOut { pub code: Option<i32>, pub stdout: String, pub stderr: String }`:
    `code` is None when the command timed out.
  - `pub fn finca::run_timed(dest: &str, remote_cmd: &str, input: Option<Vec<u8>>, timeout: Duration) -> RemoteOut`
  - `pub enum screens::Fetched { Ok(Screens), TooOld, Failed(String) }`
  - `pub fn screens::fetch(f: &Finca, dest: &str, lines: usize, timeout: Duration) -> Fetched`
  - `pub fn screens::type_prompt_remote(f: &Finca, dest: &str, session_id: &str, text: &str) -> Result<String, String>`:
    Ok(handle), or Err(message).
  - CLI: `grove type-prompt <session-id>` (hidden). It reads the text
    from stdin, prints the handle and exits 0, or prints the error and
    exits 1.

- [ ] **Step 1: Write the failing tests**

In `src/screens.rs` tests:

```rust
    fn finca_at(bin: &std::path::Path) -> crate::finca::Finca {
        crate::finca::Finca {
            name: "pi".into(),
            ssh: "me@pi".into(),
            grove_bin: bin.to_string_lossy().into(),
            color: None,
            backup: None,
        }
    }

    /// Runs `f` with the fake ssh (runs the remote command locally).
    fn via_fake_ssh<T>(d: &std::path::Path, f: impl FnOnce() -> T) -> T {
        let ssh = crate::finca::testing::fake_ssh(d, None);
        crate::finca::tests::with_env(&ssh, d, f)
    }

    #[test]
    fn fetch_parses_the_finca_document() {
        let d = tempfile::tempdir().unwrap();
        let doc = r#"{"version":1,"host":"pi","sessions":[{"id":"x","project":"blog","name":"s2","state":"your_turn","updated_at":"2026-10-16T12:00:00Z","window":"@3","screen":"hi"}]}"#;
        let bin = crate::finca::tests::script(d.path(), "grove", &format!("echo 'login noise'; echo '{doc}'"));
        let got = via_fake_ssh(d.path(), || fetch(&finca_at(&bin), "me@pi", 40, std::time::Duration::from_secs(5)));
        let Fetched::Ok(s) = got else { panic!("{got:?}") };
        assert_eq!(s.sessions[0].name, "s2");
        assert_eq!(s.sessions[0].screen, "hi");
    }

    #[test]
    fn fetch_from_an_older_grove_is_too_old() {
        let d = tempfile::tempdir().unwrap();
        let bin = crate::finca::tests::script(
            d.path(),
            "grove",
            "echo \"error: unrecognized subcommand 'screens'\" >&2; exit 2",
        );
        let got = via_fake_ssh(d.path(), || fetch(&finca_at(&bin), "me@pi", 40, std::time::Duration::from_secs(5)));
        assert!(matches!(got, Fetched::TooOld), "{got:?}");
    }

    #[test]
    fn fetch_that_times_out_fails() {
        let d = tempfile::tempdir().unwrap();
        let bin = crate::finca::tests::script(d.path(), "grove", "sleep 5");
        let got = via_fake_ssh(d.path(), || fetch(&finca_at(&bin), "me@pi", 40, std::time::Duration::from_millis(300)));
        assert!(matches!(got, Fetched::Failed(_)), "{got:?}");
    }

    #[test]
    fn type_prompt_remote_passes_text_on_stdin_and_reports_errors() {
        let d = tempfile::tempdir().unwrap();
        let got = d.path().join("got");
        let bin = crate::finca::tests::script(
            d.path(),
            "grove",
            &format!(
                "[ \"$1\" = type-prompt ] || exit 9; cat > '{}'; [ \"$2\" = sid ] && echo blog:s2 || {{ echo 'blog:s9 is working; wait until it'\"'\"'s your turn' >&2; exit 1; }}",
                got.display()
            ),
        );
        let f = finca_at(&bin);
        let ok = via_fake_ssh(d.path(), || type_prompt_remote(&f, "me@pi", "sid", "hello there"));
        assert_eq!(ok, Ok("blog:s2".into()));
        assert_eq!(std::fs::read_to_string(&got).unwrap(), "hello there");
        let err = via_fake_ssh(d.path(), || type_prompt_remote(&f, "me@pi", "other", "x")).unwrap_err();
        assert!(err.contains("working"), "{err}");
    }

    #[test]
    fn type_prompt_remote_on_an_older_grove_says_update() {
        let d = tempfile::tempdir().unwrap();
        let bin = crate::finca::tests::script(
            d.path(),
            "grove",
            "echo \"error: unrecognized subcommand 'type-prompt'\" >&2; exit 2",
        );
        let err = via_fake_ssh(d.path(), || type_prompt_remote(&finca_at(&bin), "me@pi", "sid", "x")).unwrap_err();
        assert!(err.contains("too old") && err.contains("grove finca update"), "{err}");
    }
```

Check the real names and signatures of `finca::tests::with_env` and
`finca::tests::script`, and the `Finca` fields
(`grep -n "pub struct Finca" -A12 src/finca.rs`), and adjust the
`finca_at` literal to match. `with_env`'s second argument is the fake
remote HOME.

In `tests/cli.rs`:

```rust
#[test]
fn type_prompt_unknown_session_exits_1_with_a_message() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = grove(&dir.path().join("home"))
        .args(["type-prompt", "nope"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(b"hello").unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no live session"), "{}", String::from_utf8_lossy(&out.stderr));
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --bin grove screens::tests 2>&1 | grep -E "^error" | head -3`
Expected: `cannot find function fetch` and similar errors.

- [ ] **Step 3: Implement**

In `src/finca.rs`, next to `poll_dest`:

```rust
/// What a remote command printed, and its exit code (None: timed out).
#[derive(Debug)]
pub struct RemoteOut {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `remote_cmd` at `dest` over the shared connection, with `input` on
/// its stdin, killing it after `timeout`.
pub fn run_timed(
    dest: &str,
    remote_cmd: &str,
    input: Option<Vec<u8>>,
    timeout: std::time::Duration,
) -> RemoteOut {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    let failed = |e: String| RemoteOut { code: Some(255), stdout: String::new(), stderr: e };
    let mut child = match Command::new(ssh_program())
        .args(SSH_BATCH_OPTS)
        .args(share_opts())
        .arg(dest)
        .arg(remote_cmd)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return failed(format!("can't run {}: {e}", ssh_program())),
    };
    if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = stdin.write_all(&bytes);
        });
    }
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
    let code = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s.code(),
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return RemoteOut {
                    code: None,
                    stdout: String::new(),
                    stderr: format!("timed out after {}s", timeout.as_secs_f32()),
                };
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(e) => return failed(e.to_string()),
        }
    };
    RemoteOut { code, stdout: out.join().unwrap_or_default(), stderr: err.join().unwrap_or_default() }
}
```

(The timeout path doesn't join the drain threads: a killed ssh's
descendants may still hold the pipes, as `poll_dest` notes.)

In `src/screens.rs`:

```rust
/// A finca's screens, or why there are none.
#[derive(Debug)]
pub enum Fetched {
    Ok(Screens),
    /// Its Grove predates `grove screens`.
    TooOld,
    Failed(String),
}

/// clap's answer to a subcommand an older Grove doesn't have.
fn unknown_command(out: &crate::finca::RemoteOut) -> bool {
    out.code == Some(2) && out.stderr.contains("unrecognized subcommand")
}

/// `grove screens --json` on the finca at `dest`.
pub fn fetch(f: &crate::finca::Finca, dest: &str, lines: usize, timeout: std::time::Duration) -> Fetched {
    use crate::finca::{login_shell_cmd, sh_quote};
    let cmd = login_shell_cmd(&format!("{} screens --json --lines {lines}", sh_quote(&f.grove_bin)));
    let out = crate::finca::run_timed(dest, &cmd, None, timeout);
    if unknown_command(&out) {
        return Fetched::TooOld;
    }
    match out.code {
        // The JSON is one line; a login profile may print before it.
        Some(0) => match serde_json::from_str(out.stdout.trim().lines().last().unwrap_or("")) {
            Ok(s) => Fetched::Ok(s),
            Err(e) => Fetched::Failed(format!("bad screens output: {e}")),
        },
        _ => Fetched::Failed(out.stderr.trim().to_string()),
    }
}

/// `grove type-prompt <session_id>` on the finca, with `text` on stdin.
pub fn type_prompt_remote(
    f: &crate::finca::Finca,
    dest: &str,
    session_id: &str,
    text: &str,
) -> Result<String, String> {
    use crate::finca::{login_shell_cmd, sh_quote};
    let cmd = login_shell_cmd(&format!(
        "{} type-prompt {}",
        sh_quote(&f.grove_bin),
        sh_quote(session_id)
    ));
    let out = crate::finca::run_timed(dest, &cmd, Some(text.as_bytes().to_vec()), std::time::Duration::from_secs(15));
    if unknown_command(&out) {
        return Err(format!(
            "{}'s Grove is too old to reply from here: update it (grove finca update)",
            f.name
        ));
    }
    match out.code {
        Some(0) => Ok(out.stdout.trim().lines().last().unwrap_or("").to_string()),
        _ => Err(out.stderr.trim().lines().last().unwrap_or("couldn't reach it").to_string()),
    }
}
```

In `src/main.rs`, add a hidden subcommand:

```rust
    /// Internal: type stdin into a waiting session (the session grid, from
    /// another machine).
    #[command(hide = true)]
    TypePrompt { session: String },
```

Its handler:

```rust
        Some(Cmd::TypePrompt { session }) => {
            let mut text = String::new();
            let _ = std::io::stdin().read_to_string(&mut text);
            match type_prompt(&paths, &session, &text) {
                Ok(handle) => println!("{handle}"),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
```

and the function:

```rust
/// `grove type-prompt`: the reply checks of `prompt::type_into`, by id.
fn type_prompt(paths: &Paths, id: &str, text: &str) -> std::result::Result<String, String> {
    let store = Store::new(paths.clone());
    let s = store
        .get_session(id)
        .filter(|s| s.state != crate::model::State::Ended)
        .ok_or_else(|| format!("no live session {id}"))?;
    let cfg = load_config(paths).map_err(|e| format!("{e:#}"))?;
    let account = store
        .get_project(&s.project)
        .map(|p| cfg.account_for(&p.path).name)
        .unwrap_or_default();
    crate::prompt::type_into(&grove_tmux(paths), &s, &account, text)
}
```

(`std::io::Read` is already imported in main.rs.)

- [ ] **Step 4: Run the tests and watch them pass**

Run: `cargo test --bin grove screens::tests 2>&1 | grep "test result"; cargo test --test cli type_prompt 2>&1 | grep "test result"`
Expected: both `ok`.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && \
git add -A src tests && git commit -m "feat: fetch finca screens and reply there (grove type-prompt)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: `screens::Fetcher`: background finca screen fetching

**Files:**
- Modify: `src/screens.rs`
- Test: `src/screens.rs` tests

**Interfaces:**
- Consumes: `screens::fetch` (Task 3).
- Produces:
  - `pub struct screens::FetchResult { pub slug: String, pub at: DateTime<Utc>, pub fetched: Fetched }`
  - `pub struct screens::Fetcher { pub results: Receiver<FetchResult>, stop: Arc<AtomicBool> }`
  - `impl Fetcher { pub fn start(targets: Vec<(Finca, String)>, lines: usize, interval: Duration, timeout: Duration) -> Self }`:
    fetches every target in parallel each `interval`. Stops when dropped
    (`Drop` sets `stop`) or when the receiver goes away.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn fetcher_reports_each_finca_and_stops_when_dropped() {
        let d = tempfile::tempdir().unwrap();
        let doc = r#"{"version":1,"host":"pi","sessions":[]}"#;
        let bin = crate::finca::tests::script(d.path(), "grove", &format!("echo '{doc}'"));
        let ssh = crate::finca::testing::fake_ssh(d.path(), None);
        crate::finca::tests::with_env(&ssh, d.path(), || {
            let fetcher = Fetcher::start(
                vec![(finca_at(&bin), "me@pi".into())],
                40,
                std::time::Duration::from_millis(100),
                std::time::Duration::from_secs(5),
            );
            let r = fetcher.results.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
            assert_eq!(r.slug, "pi");
            assert!(matches!(r.fetched, Fetched::Ok(_)), "{:?}", r.fetched);
            // A second round arrives on the interval.
            assert!(fetcher.results.recv_timeout(std::time::Duration::from_secs(5)).is_ok());
            let stop = fetcher.stop.clone();
            drop(fetcher);
            assert!(stop.load(std::sync::atomic::Ordering::Relaxed));
        });
    }
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test --bin grove fetcher_reports 2>&1 | grep -E "^error" | head -3`
Expected: `cannot find type Fetcher`.

- [ ] **Step 3: Implement**

```rust
/// One finca's fetch, as the grid receives it.
#[derive(Debug)]
pub struct FetchResult {
    pub slug: String,
    pub at: DateTime<Utc>,
    pub fetched: Fetched,
}

/// Fetches fincas' screens in the background while the grid is open.
pub struct Fetcher {
    pub results: std::sync::mpsc::Receiver<FetchResult>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Fetcher {
    /// Every `interval`, fetches each `(finca, destination)` in parallel.
    pub fn start(
        targets: Vec<(crate::finca::Finca, String)>,
        lines: usize,
        interval: std::time::Duration,
        timeout: std::time::Duration,
    ) -> Self {
        use std::sync::atomic::Ordering;
        let (tx, results) = std::sync::mpsc::channel();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = stop.clone();
        std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let handles: Vec<_> = targets
                    .iter()
                    .cloned()
                    .map(|(f, dest)| {
                        let tx = tx.clone();
                        std::thread::spawn(move || {
                            let fetched = fetch(&f, &dest, lines, timeout);
                            tx.send(FetchResult { slug: f.slug(), at: Utc::now(), fetched }).is_ok()
                        })
                    })
                    .collect();
                if !handles.into_iter().all(|h| h.join().unwrap_or(false)) {
                    return; // the grid dropped the receiver
                }
                std::thread::sleep(interval);
            }
        });
        Self { results, stop }
    }
}

impl Drop for Fetcher {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
```

(`Finca` must be `Clone`. Check, and add the derive if needed.)

- [ ] **Step 4: Run it and watch it pass**

Run: `cargo test --bin grove fetcher_reports 2>&1 | grep "test result"`
Expected: `ok`.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && \
git add -A src && git commit -m "feat: fetch finca screens in the background

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Grid math: layout, scrolling, movement, next waiting

**Files:**
- Create: `src/ui/grid.rs`
- Modify: `src/ui/mod.rs` (`mod grid;`)
- Test: unit tests in `src/ui/grid.rs`

**Interfaces:**
- Produces (all pure):
  - `pub const MIN_W: u16 = 40; pub const MIN_H: u16 = 8;`
  - `#[derive(Debug, Clone, Copy, PartialEq)] pub struct Layout { pub cols: usize, pub rows: usize, pub tile_w: u16, pub tile_h: u16 }`:
    `rows` is the number of visible rows.
  - `pub fn layout(n: usize, width: u16, height: u16) -> Layout`
  - `pub fn first_row(sel: usize, cols: usize, rows: usize) -> usize`:
    the top visible row that keeps `sel` in view (minimal scroll down).
  - `#[derive(Debug, Clone, Copy, PartialEq)] pub enum Dir { Left, Right, Up, Down }`
  - `pub fn step(sel: usize, n: usize, cols: usize, dir: Dir) -> usize`
  - `pub fn next_waiting(states: &[State], from: usize, forward: bool) -> usize`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;

    #[test]
    fn layout_fills_wide_and_tall_screens() {
        assert_eq!(layout(4, 110, 28), Layout { cols: 2, rows: 2, tile_w: 55, tile_h: 14 });
        assert_eq!(layout(1, 200, 60), Layout { cols: 1, rows: 1, tile_w: 200, tile_h: 60 });
        // 200 wide fits 5 columns; 7 tiles is 2 rows.
        assert_eq!(layout(7, 200, 60), Layout { cols: 5, rows: 2, tile_w: 40, tile_h: 30 });
    }

    #[test]
    fn layout_scrolls_instead_of_shrinking() {
        // 80x24: 2 columns, 3 rows of 8; 10 tiles need 5 rows.
        let l = layout(10, 80, 24);
        assert_eq!((l.cols, l.rows, l.tile_h), (2, 3, 8));
    }

    #[test]
    fn tiny_terminals_get_one_clipped_tile() {
        assert_eq!(layout(3, 30, 6), Layout { cols: 1, rows: 1, tile_w: 30, tile_h: 6 });
        assert_eq!(layout(0, 0, 0), Layout { cols: 1, rows: 1, tile_w: 0, tile_h: 0 });
    }

    #[test]
    fn first_row_keeps_the_selection_visible() {
        assert_eq!(first_row(0, 2, 3), 0);
        assert_eq!(first_row(5, 2, 3), 0); // row 2
        assert_eq!(first_row(6, 2, 3), 1); // row 3
        assert_eq!(first_row(9, 2, 3), 2); // row 4
        assert_eq!(first_row(4, 2, 0), 2); // no visible rows: no panic
    }

    #[test]
    fn step_moves_within_the_grid() {
        // 5 tiles, 3 columns: [0 1 2] [3 4]
        assert_eq!(step(0, 5, 3, Dir::Left), 0);
        assert_eq!(step(1, 5, 3, Dir::Right), 2);
        assert_eq!(step(2, 5, 3, Dir::Right), 2);
        assert_eq!(step(4, 5, 3, Dir::Right), 4);
        assert_eq!(step(1, 5, 3, Dir::Down), 4);
        assert_eq!(step(2, 5, 3, Dir::Down), 4); // onto a short last row
        assert_eq!(step(4, 5, 3, Dir::Down), 4);
        assert_eq!(step(4, 5, 3, Dir::Up), 1);
        assert_eq!(step(0, 0, 3, Dir::Down), 0);
    }

    #[test]
    fn next_waiting_wraps_and_skips_working() {
        use State::*;
        let s = [Working, NeedsYou, Working, YourTurn, Starting];
        assert_eq!(next_waiting(&s, 0, true), 1);
        assert_eq!(next_waiting(&s, 1, true), 3);
        assert_eq!(next_waiting(&s, 3, true), 1);
        assert_eq!(next_waiting(&s, 1, false), 3);
        assert_eq!(next_waiting(&[Working, Working], 1, true), 1);
        assert_eq!(next_waiting(&[], 0, true), 0);
    }
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --bin grove ui::grid 2>&1 | grep -E "^error" | head -3`
Expected: `cannot find function layout`.

- [ ] **Step 3: Implement**

```rust
//! The session grid (`v`): every live session as a tile showing the bottom
//! of its screen. Layout and movement are pure; drawing is at the bottom.

use crate::model::State;

/// The smallest tile worth drawing (border included).
pub const MIN_W: u16 = 40;
pub const MIN_H: u16 = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub cols: usize,
    /// Rows on screen (more may be scrolled to).
    pub rows: usize,
    pub tile_w: u16,
    pub tile_h: u16,
}

/// Tiles of at least MIN_W × MIN_H, filling `width` × `height`; extra rows
/// scroll rather than shrink.
pub fn layout(n: usize, width: u16, height: u16) -> Layout {
    let cols = usize::from(width / MIN_W).clamp(1, n.max(1));
    let rows = n.div_ceil(cols).clamp(1, usize::from(height / MIN_H).max(1));
    Layout {
        cols,
        rows,
        tile_w: width / cols as u16,
        tile_h: height / rows as u16,
    }
}

/// The top visible row that keeps `sel` on screen.
pub fn first_row(sel: usize, cols: usize, rows: usize) -> usize {
    (sel / cols.max(1)).saturating_sub(rows.saturating_sub(1))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// The neighbour of `sel` in `dir`, staying put at an edge.
pub fn step(sel: usize, n: usize, cols: usize, dir: Dir) -> usize {
    if n == 0 {
        return 0;
    }
    let cols = cols.max(1);
    let sel = sel.min(n - 1);
    let col = sel % cols;
    match dir {
        Dir::Left if col > 0 => sel - 1,
        Dir::Right if col + 1 < cols && sel + 1 < n => sel + 1,
        Dir::Up if sel >= cols => sel - cols,
        // Onto a short last row: its last tile.
        Dir::Down if (sel / cols + 1) * cols < n => (sel + cols).min(n - 1),
        _ => sel,
    }
}

/// The next tile (wrapping) that needs you or is your turn; `from` if none.
pub fn next_waiting(states: &[State], from: usize, forward: bool) -> usize {
    let n = states.len();
    (1..n)
        .map(|k| if forward { (from + k) % n } else { (from + n - k % n) % n })
        .find(|&i| matches!(states[i], State::NeedsYou | State::YourTurn))
        .unwrap_or(from)
}
```

Check `layout(0, 0, 0)`: `cols = 0.clamp(1, 1) = 1`; `rows =
0.clamp(1, 1) = 1`; the tile size is 0, with no division by zero. Check
`layout(4, 110, 28)`: `cols = 2`; `rows = 2.clamp(1, 3) = 2`; the tiles
are 55×14.

- [ ] **Step 4: Run them and watch them pass**

Run: `cargo test --bin grove ui::grid 2>&1 | grep "test result"`
Expected: `ok`.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && \
git add -A src && git commit -m "feat(grid): layout, scrolling and movement

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Tiles and app state

**Files:**
- Modify: `src/ui/grid.rs` (Tile, `tiles`), `src/ui/app.rs` (Mode::Grid, screen state, `apply_screens`)
- Test: `src/ui/tests.rs`

**Interfaces:**
- Consumes: `screens::{Screens, Fetched, FetchResult}`, `App.{projects, sessions, fincas}`, `FincaRow.{last_status, via, reach}`.
- Produces:
  - `Mode::Grid { sel: usize, reply: Option<String> }`
  - App fields:
    - `pub local_screens: HashMap<String, String>` (session id → screen);
    - `pub finca_screens: HashMap<String, (DateTime<Utc>, Screens)>`
      (slug → last good result);
    - `pub too_old: HashSet<String>` (finca slugs);
    - `pub grid_cols: std::cell::Cell<usize>` (set by drawing, read by
      keys; starts at 1).
  - `impl App { pub fn apply_screens(&mut self, r: FetchResult) }`
  - `#[derive(Debug, Clone, PartialEq)] pub struct Tile { pub finca: Option<(String, String)> /* (slug, name) */, pub via: Option<String>, pub project: String, pub name: String, pub session_id: Option<String>, pub state: State, pub updated_at: DateTime<Utc>, pub window: Option<String>, pub screen: Option<String>, pub stale: bool }`
  - `impl Tile { pub fn handle(&self) -> String /* project:name */ }`
  - `pub const STALE_SECS: i64 = 6;`
  - `pub fn tiles(app: &App, now: DateTime<Utc>) -> Vec<Tile>`

- [ ] **Step 1: Write the failing tests** (`src/ui/tests.rs`)

```rust
fn screens_doc(sessions: &[(&str, &str, &str, State)]) -> crate::screens::Screens {
    crate::screens::Screens {
        version: 1,
        host: "pi".into(),
        sessions: sessions
            .iter()
            .map(|(id, project, name, state)| crate::screens::ScreenSession {
                id: id.to_string(),
                project: project.to_string(),
                name: name.to_string(),
                state: *state,
                updated_at: t0(),
                window: Some("@1".into()),
                screen: format!("screen of {name}"),
            })
            .collect(),
    }
}

#[test]
fn tiles_list_local_then_finca_sessions() {
    let (_d, store, mut app) = loaded(
        &["alpha", "beta"],
        &[
            session("b1", "beta", "s1", State::Working, Some("@2")),
            session("a1", "alpha", "s1", State::YourTurn, Some("@1")),
            session("a2", "alpha", "old", State::Ended, None),
        ],
    );
    add_finca(&store, &mut app, "pi");
    app.local_screens.insert("a1".into(), "alpha screen".into());
    app.apply_screens(crate::screens::FetchResult {
        slug: "pi".into(),
        at: t0(),
        fetched: crate::screens::Fetched::Ok(screens_doc(&[("x", "blog", "s2", State::NeedsYou)])),
    });
    let tiles = super::grid::tiles(&app, t0());
    let names: Vec<_> = tiles.iter().map(|t| (t.finca.as_ref().map(|f| f.1.as_str()), t.handle())).collect();
    assert_eq!(names, [(None, "alpha:s1".into()), (None, "beta:s1".into()), (Some("pi"), "blog:s2".into())]);
    assert_eq!(tiles[0].screen.as_deref(), Some("alpha screen"));
    assert_eq!(tiles[2].screen.as_deref(), Some("screen of s2"));
    assert_eq!(tiles[2].session_id.as_deref(), Some("x"));
    assert!(!tiles[2].stale);
}

#[test]
fn finca_tiles_go_stale_and_fall_back_to_status() {
    let (_d, store, mut app) = loaded(&["alpha"], &[]);
    add_finca(&store, &mut app, "pi");
    app.apply_screens(crate::screens::FetchResult {
        slug: "pi".into(),
        at: t0(),
        fetched: crate::screens::Fetched::Ok(screens_doc(&[("x", "blog", "s2", State::YourTurn)])),
    });
    let later = t0() + Duration::seconds(super::grid::STALE_SECS + 1);
    assert!(super::grid::tiles(&app, later)[0].stale);
    // A Grove too old for screens: tiles from the status poll, no screen.
    let (_d2, store2, mut app2) = loaded(&["alpha"], &[]);
    add_finca(&store2, &mut app2, "pi");
    set_finca_status(&mut app2, "pi", &[("blog", "s2", State::YourTurn)]);
    app2.apply_screens(crate::screens::FetchResult {
        slug: "pi".into(),
        at: t0(),
        fetched: crate::screens::Fetched::TooOld,
    });
    let t = super::grid::tiles(&app2, t0());
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].screen, None);
    assert_eq!(t[0].session_id, None);
    assert!(app2.too_old.contains("pi"));
}
```

`add_finca` and `set_finca_status` are helpers for the existing finca UI
tests. Find them with
`grep -n "fn .*finca" src/ui/tests.rs | head`. If none fit, write them:
- `add_finca` saves a `Finca` to the store and reloads;
- `set_finca_status` calls `app.apply_poll(PollResult { slug, reach: Reach::Reachable(status), via: Some("me@pi".into()), at: t0() })`
  with a `Status` holding those sessions.

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --bin grove tiles_ finca_tiles 2>&1 | grep -E "^error" | head -3`
Expected: `no field local_screens` and similar errors.

- [ ] **Step 3: Implement**

`src/ui/app.rs`:
- add `Grid { sel: usize, reply: Option<String> }` to `Mode`;
- add the four fields to `App`, initialized in `App::new`:
  `HashMap::new()`, `HashMap::new()`, `HashSet::new()`,
  `std::cell::Cell::new(1)`;
- add the method:

```rust
    /// A finca's screens from the grid's fetcher.
    pub fn apply_screens(&mut self, r: crate::screens::FetchResult) {
        match r.fetched {
            crate::screens::Fetched::Ok(s) => {
                self.too_old.remove(&r.slug);
                self.finca_screens.insert(r.slug, (r.at, s));
            }
            crate::screens::Fetched::TooOld => {
                self.too_old.insert(r.slug);
            }
            // Leave the last good screens: they go stale on their own.
            crate::screens::Fetched::Failed(_) => {}
        }
    }
```

`src/ui/grid.rs`:

```rust
use chrono::{DateTime, Utc};

use super::app::App;

/// Seconds without a good fetch before a finca's tiles go stale.
pub const STALE_SECS: i64 = 6;

#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    /// The finca's (slug, name), or None for this machine.
    pub finca: Option<(String, String)>,
    /// The finca destination that last answered.
    pub via: Option<String>,
    pub project: String,
    pub name: String,
    /// None for a finca tile from the status poll (its Grove is too old to
    /// reply to).
    pub session_id: Option<String>,
    pub state: State,
    pub updated_at: DateTime<Utc>,
    pub window: Option<String>,
    /// None: no live screen for this tile.
    pub screen: Option<String>,
    pub stale: bool,
}

impl Tile {
    pub fn handle(&self) -> String {
        format!("{}:{}", self.project, self.name)
    }
}

/// Every live session: this machine's (projects in pane order, oldest
/// first), then each finca's in the Fincas pane order.
pub fn tiles(app: &App, now: DateTime<Utc>) -> Vec<Tile> {
    let mut out = Vec::new();
    for p in &app.projects {
        let slug = p.project.slug();
        for s in app.sessions.iter().filter(|s| s.project == slug && s.state != State::Ended) {
            out.push(Tile {
                finca: None,
                via: None,
                project: s.project.clone(),
                name: s.name.clone(),
                session_id: Some(s.id.clone()),
                state: s.state,
                updated_at: s.updated_at,
                window: s.tmux_window.clone(),
                screen: Some(app.local_screens.get(&s.id).cloned().unwrap_or_default()),
                stale: false,
            });
        }
    }
    for f in &app.fincas {
        let finca = Some((f.slug.clone(), f.name.clone()));
        if let (Some((at, screens)), false) = (app.finca_screens.get(&f.slug), app.too_old.contains(&f.slug)) {
            let stale = (now - *at).num_seconds() > STALE_SECS;
            for s in &screens.sessions {
                out.push(Tile {
                    finca: finca.clone(),
                    via: f.via.clone(),
                    project: s.project.clone(),
                    name: s.name.clone(),
                    session_id: Some(s.id.clone()),
                    state: s.state,
                    updated_at: s.updated_at,
                    window: s.window.clone(),
                    screen: Some(s.screen.clone()),
                    stale,
                });
            }
        } else if let Some(st) = &f.last_status {
            for p in &st.projects {
                for s in p.sessions.iter().filter(|s| s.state != State::Ended) {
                    out.push(Tile {
                        finca: finca.clone(),
                        via: f.via.clone(),
                        project: p.name.clone(),
                        name: s.name.clone(),
                        session_id: None,
                        state: s.state,
                        updated_at: s.updated_at,
                        window: s.window.clone(),
                        screen: None,
                        stale: false,
                    });
                }
            }
        }
    }
    out
}
```

Check the real field names. `ProjectRow` is probably `p.project` with a
`slug()`. `ProjectStatus` has `name` and `sessions`. Find them with
`grep -n "pub struct ProjectRow" -A8 src/ui/app.rs` and
`grep -n "pub struct ProjectStatus" -A6 src/status.rs`, and adapt. If the
status uses a project's display name where the screens use its slug, use
`slugify(&p.name)` so both kinds of finca tile show the same handle.

- [ ] **Step 4: Run them and watch them pass**

Run: `cargo test --bin grove -- tiles_ finca_tiles 2>&1 | grep "test result"`
Expected: `ok`.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && \
git add -A src && git commit -m "feat(grid): tiles from local and finca sessions

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Grid keys and the reply line

**Files:**
- Modify: `src/ui/input.rs`, `src/ui/app.rs` (Action::GridReply)
- Test: `src/ui/tests.rs`

**Interfaces:**
- Consumes: `grid::{tiles, step, next_waiting, Dir, Tile}`, `App.grid_cols`, `Mode::Grid`.
- Produces:
  - `Action::GridReply { finca: Option<(String, Option<String>)> /* (slug, via) */, session_id: String, handle: String, text: String }`
  - `v` in Normal mode (any pane but To-do, which keeps its own keys)
    opens `Mode::Grid { sel: 0, reply: None }`.
  - In the grid:
    - `Enter` on a local tile gives `Action::Jump { window }`; on a finca
      tile, `Action::FincaJump { slug, via, window }`; with no window,
      a footer message;
    - `Esc`/`v` go back to `Mode::Normal`;
    - `?` opens `Mode::Help` (closing help returns to Normal, the
      dashboard);
    - `q` gives `Action::Quit`.
  - The reply line:
    - characters append, `Backspace` deletes, `C-u` clears, `Esc`
      cancels (reply None);
    - `Enter` gives `GridReply` and leaves the text in place; run.rs
      clears it on success (Task 9).

- [ ] **Step 1: Write the failing tests**

```rust
fn grid_app() -> (tempfile::TempDir, Store, App) {
    let (d, s, mut app) = loaded(
        &["alpha"],
        &[
            session("w1", "alpha", "w1", State::Working, Some("@1")),
            session("y1", "alpha", "y1", State::YourTurn, Some("@2")),
            session("n1", "alpha", "n1", State::NeedsYou, Some("@3")),
        ],
    );
    app.grid_cols.set(2);
    handle_key(&mut app, ch('v'));
    (d, s, app)
}

fn grid_sel(app: &App) -> usize {
    match &app.mode {
        Mode::Grid { sel, .. } => *sel,
        m => panic!("not in the grid: {m:?}"),
    }
}

#[test]
fn v_opens_the_grid_and_esc_closes_it() {
    let (_d, _s, mut app) = grid_app();
    assert_eq!(grid_sel(&app), 0);
    handle_key(&mut app, key(KeyCode::Esc));
    assert_eq!(app.mode, Mode::Normal);
    handle_key(&mut app, ch('v'));
    handle_key(&mut app, ch('v'));
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn hjkl_and_tab_move_in_the_grid() {
    let (_d, _s, mut app) = grid_app();
    // [w1 y1] [n1]
    handle_key(&mut app, ch('l'));
    assert_eq!(grid_sel(&app), 1);
    handle_key(&mut app, ch('j'));
    assert_eq!(grid_sel(&app), 2);
    handle_key(&mut app, ch('k'));
    assert_eq!(grid_sel(&app), 0);
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(grid_sel(&app), 1);
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(grid_sel(&app), 2);
    handle_key(&mut app, key(KeyCode::BackTab));
    assert_eq!(grid_sel(&app), 1);
}

#[test]
fn enter_jumps_to_the_tiles_window() {
    let (_d, _s, mut app) = grid_app();
    handle_key(&mut app, ch('l'));
    assert_eq!(handle_key(&mut app, key(KeyCode::Enter)), Action::Jump { window: "@2".into() });
}

#[test]
fn i_replies_only_on_your_turn() {
    let (_d, _s, mut app) = grid_app();
    assert_eq!(handle_key(&mut app, ch('i')), Action::None);
    assert!(app.footer.as_deref().unwrap().contains("is working"), "{:?}", app.footer);
    handle_key(&mut app, key(KeyCode::Tab)); // y1
    handle_key(&mut app, ch('i'));
    for c in "ok go".chars() {
        handle_key(&mut app, ch(c));
    }
    let a = handle_key(&mut app, key(KeyCode::Enter));
    assert_eq!(
        a,
        Action::GridReply { finca: None, session_id: "y1".into(), handle: "alpha:y1".into(), text: "ok go".into() }
    );
    // Kept until the send succeeds.
    assert!(matches!(&app.mode, Mode::Grid { reply: Some(t), .. } if t == "ok go"));
    handle_key(&mut app, key(KeyCode::Esc));
    assert!(matches!(&app.mode, Mode::Grid { reply: None, .. }));
    handle_key(&mut app, key(KeyCode::Tab)); // n1
    handle_key(&mut app, ch('i'));
    assert!(app.footer.as_deref().unwrap().contains("needs you"), "{:?}", app.footer);
}

#[test]
fn a_stale_finca_tile_refuses_replies() {
    let (_d, store, mut app) = loaded(&["alpha"], &[]);
    add_finca(&store, &mut app, "pi");
    app.apply_screens(crate::screens::FetchResult {
        slug: "pi".into(),
        at: Utc::now() - Duration::seconds(60),
        fetched: crate::screens::Fetched::Ok(screens_doc(&[("x", "blog", "s2", State::YourTurn)])),
    });
    handle_key(&mut app, ch('v'));
    assert_eq!(handle_key(&mut app, ch('i')), Action::None);
    assert!(app.footer.as_deref().unwrap().contains("isn't answering"), "{:?}", app.footer);
    assert!(matches!(&app.mode, Mode::Grid { reply: None, .. }));
}

#[test]
fn the_selection_survives_tiles_going_away() {
    let (_d, _s, mut app) = grid_app();
    app.mode = Mode::Grid { sel: 2, reply: None };
    app.sessions.retain(|s| s.id == "w1");
    handle_key(&mut app, ch('l'));
    assert_eq!(grid_sel(&app), 0);
    assert_eq!(handle_key(&mut app, key(KeyCode::Enter)), Action::Jump { window: "@1".into() });
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --bin grove -- v_opens hjkl_and enter_jumps i_replies a_stale the_selection 2>&1 | grep -E "^error|test result" | head -3`
Expected: compile errors (`Mode::Grid` match arms, `GridReply`), or failures.

- [ ] **Step 3: Implement**

In `src/ui/app.rs`, add to `Action`:

```rust
    /// Type `text` into a waiting session from the grid: here, or on a
    /// finca (slug, the destination that answered).
    GridReply {
        finca: Option<(String, Option<String>)>,
        session_id: String,
        handle: String,
        text: String,
    },
```

In `src/ui/input.rs`:
- in `handle_key`, add the arm `Mode::Grid { sel, reply } => grid_key(app, sel, reply, key),`;
- in `normal_key`, after the modifier check and before `finca_key`:

```rust
    if key.code == KeyCode::Char('v') {
        app.mode = Mode::Grid { sel: 0, reply: None };
        return Action::None;
    }
```

(The To-do pane returns earlier, so `v` there stays a to-do key.)

```rust
fn grid_key(app: &mut App, sel: usize, reply: Option<String>, key: KeyEvent) -> Action {
    use super::grid::{Dir, next_waiting, step, tiles};
    let now = chrono::Utc::now();
    let tiles = tiles(app, now);
    let sel = sel.min(tiles.len().saturating_sub(1));
    if let Some(mut text) = reply {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => text = String::new(),
            KeyCode::Enter => {
                let action = match tiles.get(sel) {
                    Some(t) => match &t.session_id {
                        Some(id) => Action::GridReply {
                            finca: t.finca.as_ref().map(|(slug, _)| (slug.clone(), t.via.clone())),
                            session_id: id.clone(),
                            handle: t.handle(),
                            text: text.clone(),
                        },
                        None => Action::None,
                    },
                    None => Action::None,
                };
                app.mode = Mode::Grid { sel, reply: Some(text) };
                return action;
            }
            KeyCode::Char('u') if ctrl => text.clear(),
            KeyCode::Backspace => {
                text.pop();
            }
            KeyCode::Char(c) if !ctrl => text.push(c),
            _ => {}
        }
        let reply = (key.code != KeyCode::Esc).then_some(text);
        app.mode = Mode::Grid { sel, reply };
        return Action::None;
    }
    app.footer = None;
    let n = tiles.len();
    let cols = app.grid_cols.get();
    let mut sel = sel;
    let mut action = Action::None;
    match key.code {
        KeyCode::Esc | KeyCode::Char('v') => {
            app.mode = Mode::Normal;
            return Action::None;
        }
        KeyCode::Char('?') => {
            app.mode = Mode::Help;
            return Action::None;
        }
        KeyCode::Char('q') => return Action::Quit,
        KeyCode::Char('h') | KeyCode::Left => sel = step(sel, n, cols, Dir::Left),
        KeyCode::Char('l') | KeyCode::Right => sel = step(sel, n, cols, Dir::Right),
        KeyCode::Char('k') | KeyCode::Up => sel = step(sel, n, cols, Dir::Up),
        KeyCode::Char('j') | KeyCode::Down => sel = step(sel, n, cols, Dir::Down),
        KeyCode::Tab | KeyCode::BackTab => {
            let states: Vec<_> = tiles.iter().map(|t| t.state).collect();
            sel = next_waiting(&states, sel, key.code == KeyCode::Tab);
        }
        KeyCode::Enter => match tiles.get(sel) {
            Some(t) => match (&t.finca, &t.window) {
                (None, Some(w)) => action = Action::Jump { window: w.clone() },
                (Some((slug, _)), Some(w)) => {
                    action = Action::FincaJump { slug: slug.clone(), via: t.via.clone(), window: w.clone() }
                }
                (_, None) => app.footer = Some(format!("{} has no window yet", t.handle())),
            },
            None => {}
        },
        KeyCode::Char('i') => {
            if let Some(t) = tiles.get(sel) {
                let h = t.handle();
                match t.state {
                    _ if t.stale => {
                        let name = t.finca.as_ref().map_or("it", |f| f.1.as_str());
                        app.footer = Some(format!("{name} isn't answering"));
                    }
                    _ if t.session_id.is_none() => {
                        let name = t.finca.as_ref().map_or("it", |f| f.1.as_str());
                        app.footer = Some(format!("update Grove on {name} to reply from here"));
                    }
                    State::YourTurn => {
                        app.mode = Mode::Grid { sel, reply: Some(String::new()) };
                        return Action::None;
                    }
                    State::NeedsYou => app.footer = Some(format!("{h} needs you: Enter to answer it there")),
                    _ => app.footer = Some(format!("{h} is working; wait for your turn")),
                }
            }
        }
        _ => {}
    }
    app.mode = Mode::Grid { sel, reply: None };
    action
}
```

Import `crate::model::State` in input.rs if it isn't already. `Action`
must derive `PartialEq` for these tests; it already does if other tests
compare actions. Check for this:
`grep -n "derive" src/ui/app.rs | sed -n 1,6p`.

- [ ] **Step 4: Run them and watch them pass**

Run: `cargo test --bin grove -- v_opens hjkl_and enter_jumps i_replies a_stale the_selection 2>&1 | grep "test result"`
Expected: `ok`, 6 passed.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && \
git add -A src && git commit -m "feat(grid): keys and the reply line

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Drawing the grid

**Files:**
- Modify: `src/ui/grid.rs` (`render_grid`), `src/ui/render.rs` (call it in grid mode; `state_color`, `ACCENT`, `FINCAS_COLOR`, `render_header`, `HEADER_*` visible to the grid)
- Test: `src/ui/tests.rs`

**Interfaces:**
- Consumes: `tiles`, `layout`, `first_row`, `App.grid_cols`, `app::format_age`.
- Produces: `pub fn grid::render_grid(frame: &mut Frame, app: &App, sel: usize, reply: Option<&str>, area: Rect, now: DateTime<Utc>)`:
  - draws the top line, the tiles, and the reply line or hints into
    `area` (everything below the header);
  - sets `app.grid_cols`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn grid_draws_tiles_with_titles_and_screens() {
    let (_d, store, mut app) = loaded(
        &["alpha"],
        &[
            session("y1", "alpha", "fix", State::YourTurn, Some("@2")),
            session("w1", "alpha", "s1", State::Working, Some("@1")),
        ],
    );
    add_finca(&store, &mut app, "pi");
    app.local_screens.insert("y1".into(), "Done. Tests pass.\nWant me to commit?".into());
    app.apply_screens(crate::screens::FetchResult {
        slug: "pi".into(),
        at: t0() + Duration::minutes(2),
        fetched: crate::screens::Fetched::Ok(screens_doc(&[("x", "blog", "s2", State::NeedsYou)])),
    });
    handle_key(&mut app, ch('v'));
    let out = screen(&app, 110, 32);
    for want in [
        "SESSIONS · 3 live",
        "alpha:fix ● your turn",
        "alpha:s1 ● working",
        "pi · blog:s2 ⚑ needs you",
        "Want me to commit?",
        "screen of s2",
        "Enter jump · i reply · Tab next waiting · hjkl move",
    ] {
        assert!(out.contains(want), "missing {want:?}:\n{out}");
    }
    assert_eq!(app.grid_cols.get(), 2);
}

#[test]
fn grid_reply_line_and_scroll_note() {
    let sessions: Vec<_> = (0..10)
        .map(|i| session(&format!("s{i}"), "alpha", &format!("s{i}"), State::YourTurn, Some("@1")))
        .collect();
    let (_d, _s, mut app) = loaded(&["alpha"], &sessions);
    app.mode = Mode::Grid { sel: 9, reply: Some("yes please".into()) };
    let out = screen(&app, 80, 24);
    assert!(out.contains("showing"), "{out}");
    assert!(out.contains("reply ▸ yes please"), "{out}");
    assert!(out.contains("alpha:s9"), "selected tile scrolled into view:\n{out}");
}

#[test]
fn grid_tiles_survive_wide_characters_and_tiny_terminals() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[session("y1", "alpha", "fix", State::YourTurn, Some("@2"))]);
    app.local_screens.insert("y1".into(), "表示テスト🙂\ttabbed ".repeat(20));
    handle_key(&mut app, ch('v'));
    let out = screen(&app, 60, 20);
    // The tile's right border is intact on every body row.
    let rows: Vec<&str> = out.lines().filter(|l| l.contains('│')).collect();
    assert!(!rows.is_empty(), "{out}");
    let _ = screen(&app, 30, 6); // no panic
}

#[test]
fn an_empty_grid_says_how_to_start() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('v'));
    assert!(screen(&app, 110, 32).contains("No live sessions"));
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --bin grove -- grid_draws grid_reply grid_tiles an_empty_grid 2>&1 | grep -E "test result|panicked" | head`
Expected: FAILED (the dashboard draws instead of the grid).

- [ ] **Step 3: Implement**

In `src/ui/render.rs`, at the start of `render` after the header split
(the header draws as it does today):

```rust
    if let Mode::Grid { sel, reply } = &app.mode {
        let area = Rect { height: main.height + footer.height, ..main };
        super::grid::render_grid(frame, app, *sel, reply.as_deref(), area, now);
        return;
    }
```

Make `state_color` and `ACCENT` `pub(super)` (`FINCAS_COLOR` already
is). Import `Mode` and `Rect` if needed.

In `src/ui/grid.rs`:

```rust
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout as Split, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::render::{ACCENT, FINCAS_COLOR, state_color};

pub fn render_grid(
    frame: &mut Frame,
    app: &App,
    sel: usize,
    reply: Option<&str>,
    area: Rect,
    now: DateTime<Utc>,
) {
    let tiles = tiles(app, now);
    let [top, body, bottom] = Split::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(if reply.is_some() { 2 } else { 1 }),
    ])
    .areas(area);
    let l = layout(tiles.len(), body.width, body.height);
    app.grid_cols.set(l.cols);
    let sel = sel.min(tiles.len().saturating_sub(1));
    let first = first_row(sel, l.cols, l.rows);
    let shown = first * l.cols..((first + l.rows) * l.cols).min(tiles.len());

    // Top line: totals, what's shown, and the way back.
    let count = |f: &dyn Fn(State) -> bool| tiles.iter().filter(|t| f(t.state)).count();
    let mut spans = vec![
        Span::raw(" SESSIONS").fg(super::render::SESSIONS_COLOR),
        Span::raw(format!(" · {} live", tiles.len())),
    ];
    if shown.len() < tiles.len() {
        spans.push(Span::raw(format!(" · showing {}–{}", shown.start + 1, shown.end)).dark_gray());
    }
    for (icon, n, c) in [
        ("⚑", count(&|s| s == State::NeedsYou), Color::Red),
        ("●", count(&|s| s == State::YourTurn), Color::Yellow),
        ("●", count(&|s| matches!(s, State::Working | State::Starting)), Color::Green),
    ] {
        spans.push(Span::raw(format!("  {icon}{n}")).fg(if n == 0 { Color::DarkGray } else { c }));
    }
    for f in &app.fincas {
        if !app.too_old.contains(&f.slug) && matches!(f.reach, Some(crate::finca::Reach::Unreachable { .. })) {
            spans.push(Span::raw(format!(" · {} unreachable", f.name)).dark_gray());
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), top);
    frame.render_widget(
        Paragraph::new("v back · ? help ").alignment(ratatui::layout::Alignment::Right).dark_gray(),
        top,
    );

    if tiles.is_empty() {
        frame.render_widget(
            Paragraph::new(" No live sessions. Esc back, then Enter on a project to start one.").dark_gray(),
            body,
        );
    }
    for (k, i) in shown.clone().enumerate() {
        let (r, c) = ((k / l.cols) as u16, (k % l.cols) as u16);
        let rect = Rect {
            x: body.x + c * l.tile_w,
            y: body.y + r * l.tile_h,
            width: l.tile_w,
            height: l.tile_h,
        }
        .intersection(body);
        draw_tile(frame, &tiles[i], i == sel, rect, now);
    }

    // Bottom: the reply line, or a message, or the hints.
    match reply {
        Some(text) => {
            let lines = vec![
                Line::from(vec![Span::raw(" reply ▸ ").fg(ACCENT), Span::raw(text.to_string()), Span::raw("_").dark_gray()]),
                Line::from(" Enter send · Esc cancel").dark_gray(),
            ];
            frame.render_widget(Paragraph::new(lines), bottom);
        }
        None => {
            let text = app
                .footer
                .clone()
                .unwrap_or_else(|| "Enter jump · i reply · Tab next waiting · hjkl move".into());
            frame.render_widget(Paragraph::new(format!(" {text}")).dark_gray(), bottom);
        }
    }
}

fn draw_tile(frame: &mut Frame, t: &Tile, selected: bool, rect: Rect, now: DateTime<Utc>) {
    if rect.width < 3 || rect.height < 2 {
        return;
    }
    let colour = if t.stale { Color::DarkGray } else { state_color(t.state) };
    let icon = if t.state == State::NeedsYou { "⚑" } else { "●" };
    let mut title = vec![Span::raw(" ")];
    if let Some((_, name)) = &t.finca {
        title.push(Span::raw(name.clone()).fg(if t.stale { Color::DarkGray } else { FINCAS_COLOR }));
        title.push(Span::raw(" · ").dark_gray());
    }
    title.push(Span::raw(t.handle()).fg(if t.stale { Color::DarkGray } else { Color::White }));
    title.push(Span::raw(format!(" {icon} {}", t.state.label())).fg(colour));
    title.push(Span::raw(format!("  {}", super::app::format_age(t.updated_at, now))).dark_gray());
    if t.stale {
        title.push(Span::raw("  stale").dark_gray());
    }
    title.push(Span::raw(" "));
    let border = if selected {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else if t.stale {
        Style::default().fg(Color::DarkGray)
    } else if t.finca.is_some() {
        Style::default().fg(FINCAS_COLOR)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if selected { BorderType::Thick } else { BorderType::Rounded })
        .border_style(border)
        .title(Line::from(title));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let body: Vec<Line> = match &t.screen {
        None => vec![Line::from(t.state.label().to_string()).dark_gray()],
        Some(s) => {
            let width = usize::from(inner.width);
            let lines: Vec<&str> = s.lines().collect();
            let keep = &lines[lines.len().saturating_sub(usize::from(inner.height))..];
            keep.iter().map(|l| Line::from(fit(l, width))).collect()
        }
    };
    let style = if t.stale { Style::default().fg(Color::DarkGray) } else { Style::default() };
    frame.render_widget(Paragraph::new(body).style(style), inner);
}

/// `s` with tabs as spaces, cut to `width` display columns.
fn fit(s: &str, width: usize) -> String {
    let mut out = String::new();
    for ch in s.replace('\t', "    ").chars() {
        let w = UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]) as &str);
        if out.width() + w > width {
            break;
        }
        out.push(ch);
    }
    out
}
```

Check that `unicode-width` is a dependency. ratatui depends on it, but
Grove may not list it. If it's missing, run `cargo add unicode-width`
(no specific version needed; reuse the one ratatui already pulls in), or
use `ratatui::text::Line::width()` instead.

`format_age` lives in `app.rs`, so check that it's `pub`. If the names
`ACCENT`, `SESSIONS_COLOR` or `FINCAS_COLOR` differ, adapt.

- [ ] **Step 4: Run them and watch them pass, then look**

Run: `cargo test --bin grove -- grid_draws grid_reply grid_tiles an_empty_grid 2>&1 | grep "test result"`
Expected: `ok`, 4 passed.

Then add a temporary `zz_preview` test that prints `screen(&app, 110, 32)`
for the first test's app, and run it with `--nocapture`. Check the look
(borders, colours by eye in the printed text), then delete the test.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && \
git add -A src Cargo.toml Cargo.lock && git commit -m "feat(grid): draw the session grid

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: Wiring: live local screens, the fetcher, replies

**Files:**
- Modify: `src/ui/run.rs`
- Test: `src/ui/run_tests.rs`

**Interfaces:**
- Consumes: `screens::{tail, Fetcher, type_prompt_remote}`, `prompt::type_into`, `Action::GridReply`, `App.{local_screens, apply_screens, too_old}`.
- Produces:
  - while `Mode::Grid`, every 500 ms: every live local session's window
    is captured (`tmux.capture(w, 60)`, run through
    `screens::tail(…, 60)`) into `app.local_screens`. Windows that fail
    to capture get `(window gone)`;
  - a `screens::Fetcher` starts on entering the grid, with targets
    `(Finca, via)` for every finca that has a `via`. It's dropped on
    leaving. Its results go to `app.apply_screens`;
  - a once-per-grid-open footer note for each finca newly in `too_old`;
  - `perform(GridReply)`:
    - local: look the session up in the store; account =
      `cfg.account_for(project.path).name`; run `prompt::type_into`;
    - finca: run `screens::type_prompt_remote(finca, via or ssh, id, text)`;
    - Ok gives `Outcome::Message("sent to <handle>")`, and the loop then
      sets the reply to None;
    - Err gives `Outcome::Message(<error>)`, keeping the reply text.

- [ ] **Step 1: Write the failing test** (`src/ui/run_tests.rs`; it uses the file's `setup`, `sess` and `T`)

```rust
#[test]
fn grid_reply_types_into_a_local_waiting_session() {
    let t = setup("gridreply");
    let typed = t.root.join("typed");
    t.ctx
        .tmux
        .new_session_detached(
            "grove",
            "w",
            &t.root,
            &["sh".into(), "-c".into(), format!("stty raw -echo; cat > '{}'", typed.display())],
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let w = t.ctx.tmux.list_windows_as("grove", "#{window_id}").unwrap()[0].clone();
    for (id, state) in [("s1", State::YourTurn), ("s2", State::Working)] {
        t.ctx.store.save_session(&sess(id, "alpha", state, Some(&w))).unwrap();
    }
    // The window is s1's.
    t.ctx.tmux.set_window_option(&w, "@grove_session", "s1").unwrap();
    let reply = |id: &str| {
        perform(
            &t.ctx,
            Action::GridReply {
                finca: None,
                session_id: id.into(),
                handle: format!("alpha:{id}"),
                text: "hello there".into(),
            },
        )
    };
    let sent = reply("s1");
    let busy = reply("s2");
    std::thread::sleep(std::time::Duration::from_millis(400));
    match sent {
        Ok(Outcome::Message(m)) => assert!(m.contains("sent to alpha:s1"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert!(format!("{:#}", busy.unwrap_err()).contains("working"));
    assert!(std::fs::read_to_string(&typed).unwrap_or_default().contains("hello there"));
}
```

`Outcome` must derive `Debug` for `{other:?}`. Check this
(`grep -n "enum Outcome" -B2 src/ui/run.rs`) and add the derive if it's
missing.

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test --bin grove grid_reply_types 2>&1 | grep -E "^error|test result" | head -3`
Expected: the `GridReply` match arm is missing in `perform` (non-exhaustive match).

- [ ] **Step 3: Implement**

In `perform`:

```rust
        Action::GridReply { finca, session_id, handle: _, text } => {
            let sent = match finca {
                None => {
                    let s = store
                        .get_session(&session_id)
                        .ok_or_else(|| anyhow::anyhow!("that session has ended"))?;
                    let account = store
                        .get_project(&s.project)
                        .map(|p| ctx.cfg.account_for(&p.path).name)
                        .unwrap_or_default();
                    crate::prompt::type_into(&ctx.tmux, &s, &account, &text)
                }
                Some((slug, via)) => {
                    let f = store.get_finca(&slug)?;
                    let dest = via.unwrap_or_else(|| f.ssh.clone());
                    crate::screens::type_prompt_remote(&f, &dest, &session_id, &text)
                }
            };
            match sent {
                Ok(h) => Outcome::Message(format!("sent to {h}")),
                Err(e) => anyhow::bail!(e),
            }
        }
```

(Match `store`/`ctx` to how the other arms reach them.)

In the run loop:

```rust
    let mut fetcher: Option<crate::screens::Fetcher> = None;
    let mut last_capture = Instant::now() - Duration::from_secs(1);
    // ... inside the loop, before terminal.draw:
        let in_grid = matches!(app.mode, Mode::Grid { .. });
        if in_grid && fetcher.is_none() {
            let targets = app
                .fincas
                .iter()
                .filter_map(|r| {
                    let f = ctx.store.get_finca(&r.slug).ok()?;
                    Some((f, r.via.clone()?))
                })
                .collect();
            fetcher = Some(crate::screens::Fetcher::start(
                targets,
                60,
                Duration::from_secs(2),
                Duration::from_secs(5),
            ));
        } else if !in_grid && fetcher.is_some() {
            fetcher = None;
            app.local_screens.clear();
        }
        if let Some(f) = &fetcher {
            while let Ok(r) = f.results.try_recv() {
                let newly_old = matches!(r.fetched, crate::screens::Fetched::TooOld)
                    && !app.too_old.contains(&r.slug);
                let name = r.slug.clone();
                app.apply_screens(r);
                if newly_old {
                    app.footer = Some(format!("update Grove on {name} for live screens"));
                }
            }
        }
        if in_grid && last_capture.elapsed() >= Duration::from_millis(500) {
            for s in app.sessions.iter().filter(|s| s.state != crate::model::State::Ended) {
                let screen = s
                    .tmux_window
                    .as_deref()
                    .and_then(|w| ctx.tmux.capture(w, 60).ok())
                    .map(|t| crate::screens::tail(&t, 60))
                    .unwrap_or_else(|| "(window gone)".into());
                app.local_screens.insert(s.id.clone(), screen);
            }
            last_capture = Instant::now();
        }
```

After `perform` returns in the key branch, clear the reply on a
successful send:

```rust
            let was_reply = matches!(action, Action::GridReply { .. });
            match perform(ctx, action) {
                Ok(Outcome::Message(m)) => {
                    if was_reply && let Mode::Grid { sel, .. } = app.mode {
                        app.mode = Mode::Grid { sel, reply: None };
                    }
                    app.footer = Some(m)
                }
                // ... other arms unchanged; Err keeps the reply text ...
            }
```

(`perform` takes `action` by value, so compute `was_reply` first.)

Also, when the grid is open, poll for input every 250 ms as today; the
500 ms capture runs inside that loop.

- [ ] **Step 4: Run it and watch it pass, then the full suite**

Run: `cargo test --bin grove grid_reply_types 2>&1 | grep "test result"; cargo test 2>&1 | grep -E "test result|panicked"`
Expected: all `ok`.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && \
git add -A src && git commit -m "feat(grid): live screens, finca fetching and replies

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: Help, hints, README, and the live check

**Files:**
- Modify: `src/ui/help.rs` (Grid section; `v` in Dashboard), `src/ui/render.rs` (dashboard footer hint `v grid`), `README.md`
- Test: `src/ui/tests.rs` (the existing `every_help_line_shows_in_full_at_laptop_size` must pass; add one assertion)

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn help_lists_the_grid_keys() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('?'));
    let out = screen(&app, 110, 32);
    assert!(out.contains("Grid"), "{out}");
    assert!(out.contains("i") && out.contains("reply to a waiting session"), "{out}");
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test --bin grove help_lists_the_grid 2>&1 | grep -E "test result|panicked"`
Expected: FAILED.

- [ ] **Step 3: Implement**

In `src/ui/help.rs`:
- in `SECTIONS`, add `("v", "session grid")` to the Dashboard section;
- add a section titled `Grid` (white heading, like the non-pane
  sections):

```rust
        ("h j k l", "move between tiles"),
        ("Tab", "next waiting (S-Tab back)"),
        ("Enter", "jump to its window"),
        ("i", "reply to a waiting session"),
        ("v Esc", "back to the dashboard"),
```

Keep every line short enough for
`every_help_line_shows_in_full_at_laptop_size`. If the help popup runs
out of room at 110×32, shorten the new lines rather than older ones.

In the dashboard footer hints (`render_footer`), add `v grid` next to
the existing hints, keeping within the width the existing hint test
allows.

In `README.md`:
- add `| v | dashboard | session grid: every live session's screen |`
  to the key table;
- add a short section after "Using it":

````markdown
### Session grid

`v` on the dashboard shows every live session, here and on your fincas, as
a tile with the bottom of its screen, refreshed as it changes. `h j k l`
move, `Tab` jumps to the next one that needs you or is waiting, `Enter`
goes to its window (`C-g` comes back), and `i` replies to a session
that's waiting for its next message without leaving the grid. Permission
prompts are answered in the session itself. A finca's tiles dim and say
`stale` when it stops answering; one running an older Grove shows states
without screens until you `grove finca update`.
````

- [ ] **Step 4: Run all the checks**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test 2>&1 | grep -E "test result|panicked"`
Expected: clippy is clean; every suite is `ok`.

- [ ] **Step 5: Commit, install, deploy, and do the live check**

```bash
git add -A && git commit -m "feat(grid): help, hints and README

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
cargo install --path .
scripts/deploy-fincas.sh
```

Live check (with the user):
1. Press `R` on the dashboard, and on each finca's dashboard.
2. Start a couple of sessions here and on the Pi, then press `v`.
3. Every live session appears, with screens updating.
4. `Tab` lands on a waiting one.
5. `i`, type, `Enter` replies to a local session and to a finca session.
6. `Enter` into a finca session, then `C-g` back.
7. Disconnect Tailscale briefly and watch the finca's tiles go `stale`.
