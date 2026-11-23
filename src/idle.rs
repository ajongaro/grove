//! Ending idle sessions (`idle_end_after`, and `X` on the dashboard): a
//! Claude session waiting on you with nothing happening in it still holds a
//! few hundred MB. Its conversation is saved, and Enter resumes it.

use chrono::{DateTime, Duration, Utc};

use crate::model::{Agent, Session, State};

/// How long `X` waits for when `idle_end_after` is off.
pub const MANUAL_LIMIT: Duration = Duration::hours(1);

/// `idle_end_after`: `"off"` (or empty), or a number and `m`, `h` or `d`.
pub fn parse_limit(s: &str) -> Result<Option<Duration>, String> {
    let s = s.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("off") {
        return Ok(None);
    }
    let bad = || {
        format!("idle_end_after = \"{s}\" isn't a time: try \"off\", \"30m\", \"24h\" or \"2d\"")
    };
    let (n, unit) = s.split_at(s.len() - 1);
    let n: i64 = n.trim().parse().map_err(|_| bad())?;
    let d = match unit {
        "m" => Duration::minutes(n),
        "h" => Duration::hours(n),
        "d" => Duration::days(n),
        _ => return Err(bad()),
    };
    if n < 10 && unit == "m" || n <= 0 {
        return Err(format!(
            "idle_end_after = \"{s}\" is too short: at least 10m"
        ));
    }
    Ok(Some(d))
}

/// "24h", "2d", "90m": a limit as written back to people.
pub fn show(d: Duration) -> String {
    let m = d.num_minutes();
    if m % (24 * 60) == 0 {
        format!("{}d", m / (24 * 60))
    } else if m % 60 == 0 {
        format!("{}h", m / 60)
    } else {
        format!("{m}m")
    }
}

/// A window of the Grove tmux session: its id, its `@grove_session` tag and
/// when tmux last saw output in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub id: String,
    pub session: String,
    pub activity: Option<DateTime<Utc>>,
}

/// Parses `list-windows -F '#{window_id} #{window_activity} #{@grove_session}'`.
pub fn parse_windows(lines: &[String]) -> Vec<Window> {
    lines
        .iter()
        .filter_map(|l| {
            let mut parts = l.splitn(3, ' ');
            let id = parts.next()?.to_string();
            let activity = parts
                .next()
                .and_then(|t| t.parse::<i64>().ok())
                .and_then(|t| DateTime::from_timestamp(t, 0));
            let session = parts.next().unwrap_or("").trim().to_string();
            Some(Window {
                id,
                session,
                activity,
            })
        })
        .collect()
}

/// Sessions idle for at least `limit`: Claude sessions waiting on you (or
/// still saying starting), unpinned, in a window that's theirs, with no
/// hook event and no output in the window for that long. The input box is
/// checked separately (`input_box_empty`), on each one's screen.
pub fn candidates<'a>(
    sessions: &'a [Session],
    windows: &[Window],
    pinned: &[String],
    limit: Duration,
    now: DateTime<Utc>,
) -> Vec<&'a Session> {
    sessions
        .iter()
        .filter(|s| matches!(s.state, State::YourTurn | State::Starting))
        .filter(|s| s.agent == Agent::Claude)
        .filter(|s| !pinned.contains(&s.id))
        .filter(|s| {
            let Some(w) = s
                .tmux_window
                .as_ref()
                .and_then(|w| windows.iter().find(|x| &x.id == w))
                .filter(|w| w.session == s.id)
            else {
                return false;
            };
            let last = w.activity.map_or(s.updated_at, |a| a.max(s.updated_at));
            now - last >= limit
        })
        .collect()
}

/// Whether Claude's input box on `screen` (a capture of its pane) is empty,
/// so ending the session loses no draft. The box is the text between the
/// last two horizontal rules, after its `❯` prompt. No box found: false.
pub fn input_box_empty(screen: &str) -> bool {
    let lines: Vec<&str> = screen.lines().collect();
    let rule = |l: &&str| {
        let t = l.trim();
        t.chars().count() >= 10 && t.chars().all(|c| c == '─')
    };
    let rules: Vec<usize> = (0..lines.len()).filter(|&i| rule(&lines[i])).collect();
    let [.., top, bottom] = rules[..] else {
        return false;
    };
    let inside = lines[top + 1..bottom].join("\n");
    let Some(rest) = inside.trim_start().strip_prefix('❯') else {
        return false;
    };
    rest.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-15T08:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn limits_parse() {
        assert_eq!(parse_limit("off"), Ok(None));
        assert_eq!(parse_limit(""), Ok(None));
        assert_eq!(parse_limit("24h"), Ok(Some(Duration::hours(24))));
        assert_eq!(parse_limit("2d"), Ok(Some(Duration::days(2))));
        assert_eq!(parse_limit("30m"), Ok(Some(Duration::minutes(30))));
        for bad in ["24", "h", "1w", "-2h", "0d", "5m", "soon"] {
            assert!(parse_limit(bad).is_err(), "{bad}");
        }
        assert_eq!(show(Duration::hours(24)), "1d");
        assert_eq!(show(Duration::hours(12)), "12h");
        assert_eq!(show(Duration::minutes(90)), "90m");
    }

    #[test]
    fn windows_parse() {
        let w = parse_windows(&[
            "@3 1763193600 abc".to_string(),
            "@4 1763193600 ".to_string(),
        ]);
        assert_eq!(w[0].id, "@3");
        assert_eq!(w[0].session, "abc");
        assert_eq!(w[0].activity, DateTime::from_timestamp(1763193600, 0));
        assert_eq!(w[1].session, "");
    }

    fn sess(id: &str, state: State, idle_h: i64) -> Session {
        Session {
            id: id.into(),
            project: "p".into(),
            name: id.into(),
            host: "local".into(),
            tmux_window: Some(format!("@{id}")),
            state,
            message: None,
            conversation: None,
            launch: None,
            agent: Agent::Claude,
            unseen: false,
            created_at: t0() - Duration::days(9),
            updated_at: t0() - Duration::hours(idle_h),
        }
    }

    fn win(id: &str, idle_h: i64) -> Window {
        Window {
            id: format!("@{id}"),
            session: id.into(),
            activity: Some(t0() - Duration::hours(idle_h)),
        }
    }

    #[test]
    fn only_quiet_unpinned_sessions_waiting_on_you() {
        let mut opencode = sess("7", State::YourTurn, 48);
        opencode.agent = Agent::OpenCode;
        let sessions = vec![
            sess("1", State::YourTurn, 48),
            sess("2", State::Starting, 48),
            sess("3", State::Working, 48),
            sess("4", State::NeedsYou, 48),
            sess("5", State::YourTurn, 2),
            sess("6", State::YourTurn, 48),
            opencode,
            sess("8", State::YourTurn, 48),
            sess("9", State::YourTurn, 48),
        ];
        let mut windows: Vec<Window> = ["1", "2", "3", "4", "5", "6", "7"]
            .iter()
            .map(|id| win(id, 48))
            .collect();
        // Output in its window an hour ago.
        windows.push(win("8", 1));
        // Its window is another session's now.
        windows.push(Window {
            session: "other".into(),
            ..win("9", 48)
        });
        let ids: Vec<&str> = candidates(
            &sessions,
            &windows,
            &["6".to_string()],
            Duration::hours(24),
            t0(),
        )
        .iter()
        .map(|s| s.id.as_str())
        .collect();
        assert_eq!(ids, ["1", "2"]);
    }

    const EMPTY: &str = "\
  some reply text
────────────────────────────────────────
❯ \u{a0}
────────────────────────────────────────
    status line
";

    #[test]
    fn the_input_box_is_read_from_the_screen() {
        assert!(input_box_empty(EMPTY));
        assert!(!input_box_empty(
            &EMPTY.replace("❯ \u{a0}", "❯ half a thought")
        ));
        let multi = EMPTY.replace("❯ \u{a0}", "❯ first line\n  second");
        assert!(!input_box_empty(&multi));
        // A rule in the reply above doesn't count: the last two do.
        let above = format!("──────────────────\n{EMPTY}");
        assert!(input_box_empty(&above));
        // A dialog instead of the box, or nothing at all: can't tell.
        assert!(!input_box_empty("Do you trust this folder?\n 1. Yes\n"));
        assert!(!input_box_empty(""));
    }
}
