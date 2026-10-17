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
            // A window marked as another session's is a stale record's.
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
    Screens {
        version: SCREENS_VERSION,
        host: host.to_string(),
        sessions,
    }
}

/// A finca's screens, or why there are none.
#[derive(Debug)]
#[allow(dead_code)] // used by the session grid (plan task 9)
pub enum Fetched {
    Ok(Screens),
    /// Its Grove predates `grove screens`.
    TooOld,
    /// Why, for debugging: the grid just lets the finca's tiles go stale.
    #[allow(dead_code)]
    Failed(String),
}

/// clap's answer to a subcommand an older Grove doesn't have.
#[allow(dead_code)] // used by the session grid (plan task 9)
fn unknown_command(out: &crate::finca::RemoteOut) -> bool {
    out.code == Some(2) && out.stderr.contains("unrecognized subcommand")
}

/// `grove screens --json` on the finca at `dest`.
#[allow(dead_code)] // used by the session grid (plan task 9)
pub fn fetch(
    f: &crate::finca::Finca,
    dest: &str,
    lines: usize,
    timeout: std::time::Duration,
) -> Fetched {
    use crate::finca::{login_shell_cmd, sh_quote};
    let cmd = login_shell_cmd(&format!(
        "{} screens --json --lines {lines}",
        sh_quote(&f.grove_bin)
    ));
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
/// Returns the session's handle.
#[allow(dead_code)] // used by the session grid (plan task 9)
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
    let out = crate::finca::run_timed(
        dest,
        &cmd,
        Some(text.as_bytes().to_vec()),
        std::time::Duration::from_secs(15),
    );
    if unknown_command(&out) {
        return Err(format!(
            "{}'s Grove is too old to reply from here: update it (grove finca update)",
            f.name
        ));
    }
    match out.code {
        Some(0) => Ok(out.stdout.trim().lines().last().unwrap_or("").to_string()),
        _ => Err(out
            .stderr
            .trim()
            .lines()
            .last()
            .unwrap_or("couldn't reach it")
            .to_string()),
    }
}

/// One finca's fetch, as the grid receives it.
#[derive(Debug)]
#[allow(dead_code)] // used by the session grid (plan task 9)
pub struct FetchResult {
    pub slug: String,
    pub at: DateTime<Utc>,
    pub fetched: Fetched,
}

/// Fetches fincas' screens in the background while the grid is open;
/// stops when dropped.
#[allow(dead_code)] // used by the session grid (plan task 9)
pub struct Fetcher {
    pub results: std::sync::mpsc::Receiver<FetchResult>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[allow(dead_code)] // used by the session grid (plan task 9)
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
                            tx.send(FetchResult {
                                slug: f.slug(),
                                at: Utc::now(),
                                fetched,
                            })
                            .is_ok()
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
            &[
                "sh".into(),
                "-c".into(),
                "printf 'hello\\nfrom beta\\n'; sleep 30".into(),
            ],
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let w = tmux.list_windows_as("grove", "#{window_id}").unwrap()[0].clone();
        store
            .save_session(&session("b1", "beta", State::YourTurn, Some(&w), 5))
            .unwrap();
        tmux.set_window_option(&w, "@grove_session", "b1").unwrap();
        store
            .save_session(&session("a1", "alpha", State::Working, None, 9))
            .unwrap();
        store
            .save_session(&session("a2", "alpha", State::Ended, None, 1))
            .unwrap();
        // A stale record pointing at someone else's window shows nothing.
        store
            .save_session(&session("b2", "beta", State::Working, Some(&w), 1))
            .unwrap();
        let out = local(&store, &tmux, 40, "homebox");
        tmux.kill_server();
        let ids: Vec<_> = out.sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["a1", "b1", "b2"]);
        assert_eq!(out.version, SCREENS_VERSION);
        assert_eq!(out.host, "homebox");
        assert!(
            out.sessions[1].screen.ends_with("hello\nfrom beta"),
            "{:?}",
            out.sessions[1].screen
        );
        assert_eq!(out.sessions[0].screen, "");
        assert_eq!(out.sessions[2].screen, "");
    }

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

    const SECS5: std::time::Duration = std::time::Duration::from_secs(5);

    #[test]
    fn fetch_parses_the_finca_document() {
        let d = tempfile::tempdir().unwrap();
        let doc = r#"{"version":1,"host":"pi","sessions":[{"id":"x","project":"blog","name":"s2","state":"your_turn","updated_at":"2026-10-16T12:00:00Z","window":"@3","screen":"hi"}]}"#;
        let bin = crate::finca::tests::script(
            d.path(),
            "grove",
            &format!("echo 'login noise'; echo '{doc}'"),
        );
        let got = via_fake_ssh(d.path(), || fetch(&finca_at(&bin), "me@pi", 40, SECS5));
        let Fetched::Ok(s) = got else {
            panic!("{got:?}")
        };
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
        let got = via_fake_ssh(d.path(), || fetch(&finca_at(&bin), "me@pi", 40, SECS5));
        assert!(matches!(got, Fetched::TooOld), "{got:?}");
    }

    #[test]
    fn fetch_that_times_out_fails() {
        let d = tempfile::tempdir().unwrap();
        let bin = crate::finca::tests::script(d.path(), "grove", "sleep 5");
        let got = via_fake_ssh(d.path(), || {
            fetch(
                &finca_at(&bin),
                "me@pi",
                40,
                std::time::Duration::from_millis(300),
            )
        });
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
                "[ \"$1\" = type-prompt ] || exit 9; cat > '{}'; if [ \"$2\" = sid ]; then echo blog:s2; else echo 'blog:s9 is working; wait' >&2; exit 1; fi",
                got.display()
            ),
        );
        let f = finca_at(&bin);
        let ok = via_fake_ssh(d.path(), || {
            type_prompt_remote(&f, "me@pi", "sid", "hello there")
        });
        assert_eq!(ok, Ok("blog:s2".into()));
        assert_eq!(std::fs::read_to_string(&got).unwrap(), "hello there");
        let err =
            via_fake_ssh(d.path(), || type_prompt_remote(&f, "me@pi", "other", "x")).unwrap_err();
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
        let err = via_fake_ssh(d.path(), || {
            type_prompt_remote(&finca_at(&bin), "me@pi", "sid", "x")
        })
        .unwrap_err();
        assert!(
            err.contains("too old") && err.contains("grove finca update"),
            "{err}"
        );
    }

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
                SECS5,
            );
            let r = fetcher.results.recv_timeout(SECS5).unwrap();
            assert_eq!(r.slug, "pi");
            assert!(matches!(r.fetched, Fetched::Ok(_)), "{:?}", r.fetched);
            // A second round arrives on the interval.
            assert!(fetcher.results.recv_timeout(SECS5).is_ok());
            let stop = fetcher.stop.clone();
            drop(fetcher);
            assert!(stop.load(std::sync::atomic::Ordering::Relaxed));
        });
    }
}
