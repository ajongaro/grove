//! Text the MCP tools show about a project: session counts, git, docs.

use std::path::Path;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use crate::model::{Session, State};

/// Sessions that haven't ended, most recently active first.
pub fn live(mut sessions: Vec<Session>) -> Vec<Session> {
    sessions.retain(|s| s.state != State::Ended);
    sessions.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
    sessions
}

/// `1 needs you, 2 your turn, 1 working`, or `no sessions`. Starting
/// counts as working, as on the dashboard.
pub fn counts(sessions: &[Session]) -> String {
    let n = |f: &dyn Fn(State) -> bool| sessions.iter().filter(|s| f(s.state)).count();
    let parts: Vec<String> = [
        (n(&|s| s == State::NeedsYou), "needs you"),
        (n(&|s| s == State::YourTurn), "your turn"),
        (
            n(&|s| matches!(s, State::Working | State::Starting)),
            "working",
        ),
    ]
    .into_iter()
    .filter(|(c, _)| *c > 0)
    .map(|(c, l)| format!("{c} {l}"))
    .collect();
    if parts.is_empty() {
        "no sessions".into()
    } else {
        parts.join(", ")
    }
}

pub fn ago(t: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let m = (now - t).num_minutes().max(0);
    match m {
        0 => "just now".into(),
        1..=59 => format!("{m}m ago"),
        60..=2879 => format!("{}h ago", m / 60),
        _ => format!("{}d ago", m / 1440),
    }
}

/// `git -C dir <args>`, never taking optional locks: a status that runs past
/// the timeout is killed, and a killed git would leave `index.lock` behind.
fn git_cmd(dir: &Path, args: &[&str]) -> std::process::Command {
    let mut c = std::process::Command::new("git");
    c.arg("--no-optional-locks").arg("-C").arg(dir).args(args);
    c
}

/// `git -C dir <args>`'s stdout, or None on failure or after 2 seconds.
fn git_out(dir: &Path, args: &[&str]) -> Option<String> {
    use std::process::Stdio;
    let mut child = git_cmd(dir, args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait().ok()? {
            Some(st) if st.success() => break,
            Some(_) => return None,
            None if Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let mut s = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut s).ok()?;
    Some(s)
}

/// Branch, uncommitted file count and the last 5 commits, or `git: none`.
pub fn git(dir: &Path) -> String {
    let Some(branch) = git_out(dir, &["rev-parse", "--abbrev-ref", "HEAD"]) else {
        return "git: none".into();
    };
    let changed = match git_out(dir, &["status", "--porcelain"]) {
        Some(s) => {
            let n = s.lines().count();
            format!("{n} file{}", if n == 1 { "" } else { "s" })
        }
        None => "unknown".into(),
    };
    let log = git_out(dir, &["log", "-5", "--format=  %h %s (%cr)"]).unwrap_or_default();
    format!(
        "branch: {}\nuncommitted: {changed}\nrecent commits:\n{}",
        branch.trim(),
        log.trim_end()
    )
}

/// The first `lines` lines of a text file, noting how many were left out.
pub fn excerpt(path: &Path, lines: usize) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let all: Vec<&str> = text.lines().collect();
    let mut out = all[..all.len().min(lines)].join("\n");
    if all.len() > lines {
        out.push_str(&format!("\n({} more lines)", all.len() - lines));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Session, State};

    fn session(state: State, mins_ago: i64) -> Session {
        let t = chrono::Utc::now() - chrono::Duration::minutes(mins_ago);
        Session {
            id: format!("id-{mins_ago}"),
            project: "alpha".into(),
            name: format!("s{mins_ago}"),
            host: "local".into(),
            tmux_window: None,
            state,
            message: None,
            created_at: t,
            updated_at: t,
        }
    }

    #[test]
    fn counts_by_state_with_starting_as_working() {
        let s = vec![
            session(State::NeedsYou, 1),
            session(State::YourTurn, 2),
            session(State::Starting, 3),
            session(State::Working, 4),
        ];
        assert_eq!(counts(&s), "1 needs you, 1 your turn, 2 working");
        assert_eq!(counts(&[]), "no sessions");
    }

    #[test]
    fn live_drops_ended_and_sorts_newest_first() {
        let s = live(vec![
            session(State::Working, 9),
            session(State::Ended, 1),
            session(State::YourTurn, 2),
        ]);
        let names: Vec<_> = s.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["s2", "s9"]);
    }

    #[test]
    fn ago_buckets() {
        let now = chrono::Utc::now();
        let m = |n| now - chrono::Duration::minutes(n);
        assert_eq!(ago(m(0), now), "just now");
        assert_eq!(ago(m(5), now), "5m ago");
        assert_eq!(ago(m(180), now), "3h ago");
        assert_eq!(ago(m(60 * 50), now), "2d ago");
    }

    #[test]
    fn git_reports_branch_changes_and_commits() {
        let d = tempfile::tempdir().unwrap();
        let g = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(d.path())
                .args(args)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap();
            assert!(out.status.success(), "{out:?}");
        };
        g(&["init", "-q", "-b", "main"]);
        std::fs::write(d.path().join("a"), "1").unwrap();
        g(&["add", "a"]);
        g(&["commit", "-q", "-m", "first commit"]);
        std::fs::write(d.path().join("b"), "2").unwrap();
        let out = git(d.path());
        assert!(out.contains("branch: main"), "{out}");
        assert!(out.contains("uncommitted: 1 file"), "{out}");
        assert!(out.contains("first commit"), "{out}");
    }

    #[test]
    fn git_outside_a_repo_is_none() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(git(d.path()), "git: none");
    }

    #[test]
    fn excerpt_takes_the_first_lines() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("README.md");
        std::fs::write(
            &f,
            (1..=50).map(|i| format!("line {i}\n")).collect::<String>(),
        )
        .unwrap();
        let e = excerpt(&f, 40).unwrap();
        assert!(e.contains("line 40") && !e.contains("line 41"), "{e}");
        assert!(e.ends_with("(10 more lines)"), "{e}");
        assert_eq!(excerpt(&d.path().join("nope.md"), 40), None);
    }

    #[test]
    fn git_never_takes_optional_locks() {
        let c = git_cmd(Path::new("/x"), &["status", "--porcelain"]);
        let args: Vec<_> = c
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "--no-optional-locks", "{args:?}");
    }
}
