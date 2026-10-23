//! Thin wrapper over the tmux CLI. `socket` selects the tmux server (`-L`),
//! so Grove runs on its own server and tests use a throwaway one.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result, anyhow, bail};

#[derive(Debug, Clone, Default)]
pub struct Tmux {
    pub socket: Option<String>,
    /// Config file passed with `-f` when this call starts the server.
    pub config: Option<PathBuf>,
}

impl Tmux {
    #[cfg(test)]
    pub fn new(socket: impl Into<String>) -> Self {
        Self {
            socket: Some(socket.into()),
            config: None,
        }
    }

    pub fn cmd(&self) -> Command {
        let mut c = Command::new("tmux");
        if let Some(s) = &self.socket {
            c.args(["-L", s]);
        }
        if let Some(f) = &self.config {
            c.arg("-f").arg(f);
        }
        c
    }

    /// Runs tmux with `args`; errors carry tmux's stderr.
    fn run(&self, args: &[&str]) -> Result<String> {
        let out = self
            .cmd()
            .args(args)
            .stdin(Stdio::null())
            .output()
            .context("running tmux")?;
        check(&out, args)
    }

    pub fn has_session(&self, name: &str) -> bool {
        self.run(&["has-session", "-t", &format!("={name}")])
            .is_ok()
    }

    pub fn new_session_detached(
        &self,
        name: &str,
        window_name: &str,
        cwd: &Path,
        argv: &[String],
    ) -> Result<()> {
        let mut c = self.cmd();
        c.args(["new-session", "-d", "-s", name, "-n", window_name, "-c"])
            .arg(cwd)
            .args(argv);
        let out = c.stdin(Stdio::null()).output().context("running tmux")?;
        check(&out, &["new-session"]).map(|_| ())
    }

    /// Creates a window (without switching to it) and returns its id (`@N`).
    pub fn new_window(
        &self,
        session: &str,
        name: &str,
        cwd: &Path,
        env: &[(String, String)],
        argv: &[String],
    ) -> Result<String> {
        let mut c = self.cmd();
        // Trailing ':' targets the session; tmux picks the next free index.
        c.args([
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{window_id}",
            "-t",
            &format!("={session}:"),
            "-n",
            name,
            "-c",
        ])
        .arg(cwd);
        for (k, v) in env {
            c.arg("-e").arg(format!("{k}={v}"));
        }
        c.args(argv);
        let out = c.stdin(Stdio::null()).output().context("running tmux")?;
        Ok(check(&out, &["new-window"])?.trim().to_string())
    }

    /// Window ids (`@N`) in `session`.
    pub fn list_windows(&self, session: &str) -> Result<Vec<String>> {
        self.list_windows_as(session, "#{window_id}")
    }

    /// One line per window in `session`, in tmux `format`.
    pub fn list_windows_as(&self, session: &str, format: &str) -> Result<Vec<String>> {
        let out = self.run(&["list-windows", "-t", &format!("={session}"), "-F", format])?;
        Ok(out.lines().map(str::to_string).collect())
    }

    /// Windows on screen in a client whose terminal has focus (tmux tracks
    /// focus through `focus-events`, which Grove's tmux.conf turns on).
    pub fn focused_windows(&self) -> Vec<String> {
        let clients = self
            .run(&[
                "list-clients",
                "-F",
                "#{window_id} #{client_flags} #{session_name}",
            ])
            .unwrap_or_default();
        let windows = self
            .run(&["list-windows", "-a", "-F", "#{session_name} #{window_id}"])
            .unwrap_or_default();
        focused(&clients, &windows)
    }

    /// Creates a detached session with one window running `argv` and
    /// returns that window's id.
    pub fn new_session_window(&self, name: &str, argv: &[&str]) -> Result<String> {
        let mut args = vec!["new-session", "-d", "-P", "-F", "#{window_id}", "-s", name];
        args.extend(argv);
        Ok(self.run(&args)?.trim().to_string())
    }

    /// Shows window `src` in `session` too, at its next free index.
    pub fn link_window(&self, src: &str, session: &str) -> Result<()> {
        self.run(&[
            "link-window",
            "-d",
            "-s",
            src,
            "-t",
            &format!("={session}:"),
        ])
        .map(|_| ())
    }

    /// Splits `target` side by side, running `argv` in the new pane.
    pub fn split_window(&self, target: &str, cwd: &Path, argv: &[String]) -> Result<()> {
        let mut c = self.cmd();
        c.args(["split-window", "-h", "-d", "-t", target, "-c"])
            .arg(cwd)
            .args(argv);
        let out = c.stdin(Stdio::null()).output().context("running tmux")?;
        check(&out, &["split-window"]).map(|_| ())
    }

    pub fn select_layout(&self, target: &str, layout: &str) -> Result<()> {
        self.run(&["select-layout", "-t", target, layout])
            .map(|_| ())
    }

    pub fn pane_count(&self, target: &str) -> usize {
        self.run(&["list-panes", "-t", target, "-F", "#{pane_id}"])
            .map_or(0, |o| o.lines().count())
    }

    /// Session names on the server.
    pub fn list_sessions(&self) -> Vec<String> {
        self.run(&["list-sessions", "-F", "#{session_name}"])
            .map(|o| o.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// The width of the client this runs under, if any.
    pub fn client_width(&self) -> Option<u16> {
        self.run(&["display-message", "-p", "#{client_width}"])
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    pub fn select_window(&self, target: &str) -> Result<()> {
        self.run(&["select-window", "-t", target]).map(|_| ())
    }

    pub fn kill_window(&self, id: &str) -> Result<()> {
        self.run(&["kill-window", "-t", id]).map(|_| ())
    }

    pub fn set_window_option(&self, id: &str, option: &str, value: &str) -> Result<()> {
        self.run(&["set-window-option", "-t", id, option, value])
            .map(|_| ())
    }

    pub fn display_popup(&self, cwd: &Path, title: Option<&str>, argv: &[String]) -> Result<()> {
        let mut c = self.cmd();
        c.args(["display-popup", "-E", "-w", "80%", "-h", "80%", "-d"])
            .arg(cwd);
        if let Some(t) = title {
            c.arg("-T").arg(format!(" {} ", literal(t)));
        }
        c.args(argv);
        let out = c.stdin(Stdio::null()).output().context("running tmux")?;
        check(&out, &["display-popup"]).map(|_| ())
    }

    /// A window option's value (e.g. a user option like `@grove_build`).
    pub fn window_option(&self, target: &str, option: &str) -> Option<String> {
        let v = self
            .run(&["show-window-options", "-v", "-t", target, option])
            .ok()?;
        Some(v.trim().to_string()).filter(|v| !v.is_empty())
    }

    /// Kills whatever runs in the window and restarts its original command.
    pub fn respawn_window(&self, target: &str) -> Result<()> {
        self.run(&["respawn-window", "-k", "-t", target])
            .map(|_| ())
    }

    pub fn rename_window(&self, id: &str, name: &str) -> Result<()> {
        self.run(&["rename-window", "-t", id, name]).map(|_| ())
    }

    /// Sets a session option.
    pub fn set_option(&self, session: &str, option: &str, value: &str) -> Result<()> {
        // set-option takes a pane target; an exact session match needs the ":".
        self.run(&["set-option", "-t", &format!("={session}:"), option, value])
            .map(|_| ())
    }

    /// Moves the current client to `target`.
    pub fn switch_client(&self, target: &str) -> Result<()> {
        self.run(&["switch-client", "-t", target]).map(|_| ())
    }

    pub fn kill_session(&self, name: &str) -> Result<()> {
        self.run(&["kill-session", "-t", &format!("={name}")])
            .map(|_| ())
    }

    /// Sets a session-level environment variable.
    pub fn set_environment(&self, session: &str, key: &str, value: &str) -> Result<()> {
        self.run(&["set-environment", "-t", &format!("={session}"), key, value])
            .map(|_| ())
    }

    /// Reads a session-level environment variable, if set.
    pub fn show_environment(&self, session: &str, key: &str) -> Option<String> {
        let out = self
            .run(&["show-environment", "-t", &format!("={session}"), key])
            .ok()?;
        // "KEY=value", or "-KEY" when it was removed.
        out.trim()
            .strip_prefix(&format!("{key}="))
            .map(str::to_string)
    }

    /// A session's option (e.g. a user `@` option), if set.
    pub fn session_option(&self, session: &str, option: &str) -> Option<String> {
        let out = self
            .cmd()
            .args(["show-options", "-qv", "-t", session, option])
            .output()
            .ok()?;
        let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && !v.is_empty()).then_some(v)
    }

    /// The last `lines` lines of a pane's screen and scrollback, with
    /// wrapped lines joined.
    pub fn capture(&self, target: &str, lines: usize) -> Result<String> {
        let start = format!("-{lines}");
        self.run(&["capture-pane", "-p", "-J", "-t", target, "-S", &start])
    }

    pub fn send_keys(&self, target: &str, keys: &[&str]) -> Result<()> {
        let mut args = vec!["send-keys", "-t", target];
        args.extend_from_slice(keys);
        self.run(&args).map(|_| ())
    }

    /// Pastes `text` into `target` as a paste (bracketed if the program
    /// asked). The text goes in on stdin, so tmux's command size doesn't
    /// limit it, through a buffer of its own, so pastes running at the same
    /// time can't swap texts.
    pub fn paste_text(&self, target: &str, text: &str) -> Result<()> {
        use std::io::Write;
        let buf = format!("grove-paste-{}", uuid::Uuid::new_v4());
        let mut child = self
            .cmd()
            .args(["load-buffer", "-b", &buf, "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("running tmux")?;
        child
            .stdin
            .take()
            .context("tmux stdin")?
            .write_all(text.as_bytes())?;
        check(&child.wait_with_output()?, &["load-buffer"])?;
        self.run(&["paste-buffer", "-p", "-d", "-b", &buf, "-t", target])
            .map(|_| ())
    }

    pub fn display_message(&self, target: &str, msg: &str) {
        // The message carries file names and errors: shown, never expanded.
        let _ = self.run(&["display-message", "-d", "4000", "-t", target, &literal(msg)]);
    }

    #[cfg(test)]
    /// Kills the server and removes its socket file (tmux leaves it behind).
    pub fn kill_server(&self) {
        let socket = self.run(&["display-message", "-p", "#{socket_path}"]).ok();
        let _ = self.cmd().arg("kill-server").stderr(Stdio::null()).status();
        if let Some(path) = socket
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The windows focused clients show, from `list-clients` lines (window,
/// flags, session) and `list-windows -a` lines (session, window). A desk
/// column's client (desk.rs) is inside the desk, so the desk's focus is
/// what counts: every window its columns show is on screen.
fn focused(clients: &str, windows: &str) -> Vec<String> {
    let mut out = Vec::new();
    for l in clients.lines() {
        let mut f = l.split(' ');
        let (Some(window), Some(flags), Some(session)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        if crate::desk::is_column(session) || !flags.split(',').any(|f| f == "focused") {
            continue;
        }
        if session == crate::desk::SESSION {
            out.extend(
                windows
                    .lines()
                    .filter_map(|l| l.split_once(' '))
                    .filter(|(s, _)| crate::desk::is_column(s))
                    .map(|(_, w)| w.to_string()),
            );
        } else {
            out.push(window.to_string());
        }
    }
    out
}

fn check(out: &Output, args: &[&str]) -> Result<String> {
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!(
            "tmux {} failed: {}",
            args.first().unwrap_or(&""),
            err.trim()
        )
    }
}

/// `s` as text tmux shows unchanged: `#` starts formats (`#{}`, `#()`)
/// in messages, so it's doubled.
pub fn literal(s: &str) -> String {
    s.replace('#', "##")
}

/// Parses `tmux -V` output such as "tmux 3.6a" or "tmux next-3.7".
pub fn parse_version(s: &str) -> Option<(u32, u32)> {
    let v = s.split_whitespace().nth(1)?;
    let v = v.rsplit('-').next()?;
    let mut parts = v.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor: String = parts
        .next()?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    Some((major, minor.parse().ok()?))
}

pub fn version() -> Result<(u32, u32)> {
    let out = Command::new("tmux")
        .arg("-V")
        .output()
        .context("tmux is not installed or not on PATH")?;
    let text = String::from_utf8_lossy(&out.stdout);
    parse_version(&text).ok_or_else(|| anyhow!("can't parse tmux version from {text:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway tmux server, killed on drop.
    struct TestServer(Tmux);
    impl Drop for TestServer {
        fn drop(&mut self) {
            self.0.kill_server();
        }
    }
    fn server(tag: &str) -> TestServer {
        TestServer(Tmux::new(format!(
            "grove-test-{}-{tag}",
            std::process::id()
        )))
    }
    fn sh(cmd: &str) -> Vec<String> {
        vec!["sh".into(), "-c".into(), cmd.into()]
    }

    #[test]
    fn the_desks_focus_counts_for_every_window_it_shows() {
        let windows = "grove @1\ngrove @5\ngrove @6\ndesk @9\ndesk-aaaa @5\ndesk-bbbb @6\n";
        // On the dashboard: its window. The desk columns' own clients
        // never count, focused or not.
        let clients = "@1 attached,focused grove\n@5 attached,focused desk-aaaa\n";
        assert_eq!(focused(clients, windows), ["@1"]);
        // On the desk: every window in a column.
        let clients = "@9 attached,focused desk\n@5 attached desk-aaaa\n@6 attached desk-bbbb\n";
        assert_eq!(focused(clients, windows), ["@5", "@6"]);
        // The desk in an unfocused terminal: nothing.
        assert!(focused("@9 attached desk\n", windows).is_empty());
    }

    #[test]
    fn literal_text_survives_tmux_format_expansion() {
        let t = server("literal");
        let d = tempfile::tempdir().unwrap();
        t.0.new_session_detached("s", "w", d.path(), &sh("sleep 30"))
            .unwrap();
        // A file name shown in a message must not expand or run anything.
        let name = "sent #{session_name} #(touch x) ##.png";
        let out =
            t.0.run(&["display-message", "-p", "-t", "s", &literal(name)])
                .unwrap();
        assert_eq!(out.trim(), name);
    }

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("tmux 3.6a\n"), Some((3, 6)));
        assert_eq!(parse_version("tmux 3.2"), Some((3, 2)));
        assert_eq!(parse_version("tmux next-3.7"), Some((3, 7)));
        assert_eq!(parse_version("garbage"), None);
    }

    #[test]
    fn installed_tmux_is_new_enough() {
        assert!(version().unwrap() >= (3, 2));
    }

    #[test]
    fn session_and_window_lifecycle() {
        let s = server("life");
        let t = &s.0;
        let dir = tempfile::tempdir().unwrap();
        assert!(!t.has_session("g"));
        t.new_session_detached("g", "dash", dir.path(), &sh("sleep 30"))
            .unwrap();
        assert!(t.has_session("g"));

        let cwd = dir.path().join("with space");
        std::fs::create_dir_all(&cwd).unwrap();
        let out = cwd.join("env.txt");
        let id = t
            .new_window(
                "g",
                "proj:s1",
                &cwd,
                &[("CLAUDE_CONFIG_DIR".into(), "/x/.claude-play".into())],
                &sh(&format!(
                    "echo \"$CLAUDE_CONFIG_DIR|$PWD\" > '{}'; sleep 30",
                    out.display()
                )),
            )
            .unwrap();
        assert!(id.starts_with('@'), "{id}");
        assert_eq!(t.list_windows("g").unwrap().len(), 2);
        assert!(t.list_windows("g").unwrap().contains(&id));

        // The window got its env and cwd.
        for _ in 0..50 {
            if out.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let got = std::fs::read_to_string(&out).unwrap();
        let (account, pwd) = got.trim().split_once('|').unwrap();
        assert_eq!(account, "/x/.claude-play");
        // $PWD may or may not resolve macOS's /var → /private/var symlink.
        assert_eq!(
            Path::new(pwd).canonicalize().unwrap(),
            cwd.canonicalize().unwrap()
        );

        t.set_window_option(&id, "window-status-style", "fg=red")
            .unwrap();
        t.select_window(&id).unwrap();
        t.kill_window(&id).unwrap();
        assert!(!t.list_windows("g").unwrap().contains(&id));
    }

    #[test]
    fn list_windows_of_missing_session_errors() {
        let s = server("missing");
        assert!(s.0.list_windows("nope").is_err());
    }
}
