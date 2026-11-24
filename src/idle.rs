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
    let (n, make): (&str, fn(i64) -> Option<Duration>) = if let Some(n) = s.strip_suffix('m') {
        (n, Duration::try_minutes)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, Duration::try_hours)
    } else if let Some(n) = s.strip_suffix('d') {
        (n, Duration::try_days)
    } else {
        return Err(bad());
    };
    let n: i64 = n.trim().parse().map_err(|_| bad())?;
    if n <= 0 {
        return Err(bad());
    }
    let d = make(n).ok_or_else(bad)?;
    if d < Duration::minutes(10) {
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
    /// More than one is a side shell beside Claude, which ending would kill.
    pub panes: usize,
    pub session: String,
    pub activity: Option<DateTime<Utc>>,
}

/// `list-windows`'s format for `parse_windows`.
pub const WINDOW_FORMAT: &str = "#{window_id} #{window_activity} #{window_panes} #{@grove_session}";

/// Parses `list-windows -F WINDOW_FORMAT`.
pub fn parse_windows(lines: &[String]) -> Vec<Window> {
    lines
        .iter()
        .filter_map(|l| {
            let mut parts = l.splitn(4, ' ');
            let id = parts.next()?.to_string();
            let activity = parts
                .next()
                .and_then(|t| t.parse::<i64>().ok())
                .and_then(|t| DateTime::from_timestamp(t, 0));
            let panes = parts.next().and_then(|n| n.parse().ok()).unwrap_or(1);
            let session = parts.next().unwrap_or("").trim().to_string();
            Some(Window {
                id,
                panes,
                session,
                activity,
            })
        })
        .collect()
}

/// Sessions idle for at least `limit`: Claude sessions waiting on you (or
/// still saying starting), unpinned, with no background task from their last
/// turn, alone in a window that's theirs, with no
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
        // A job its last turn left running in the background.
        .filter(|s| !s.background)
        .filter(|s| {
            let Some(w) = s
                .tmux_window
                .as_ref()
                .and_then(|w| windows.iter().find(|x| &x.id == w))
                .filter(|w| w.session == s.id && w.panes == 1)
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
    // A capture with styles (`capture-pane -e`): dim text in the box is a
    // hint ("Try …", a suggested prompt), not something typed.
    let lines: Vec<String> = screen.lines().map(undimmed).collect();
    let rule = |l: &str| {
        // A named session's top rule has its name in it: "──── name ─".
        let t = l.trim();
        t.starts_with('─') && t.ends_with('─') && t.chars().filter(|&c| c == '─').count() >= 10
    };
    let rules: Vec<usize> = (0..lines.len()).filter(|&i| rule(&lines[i])).collect();
    let [.., top, bottom] = rules[..] else {
        return false;
    };
    // Under the box Claude Code counts what still runs: "1 shell",
    // "1 background task". Not "← 1 agent", which stays once the agent
    // is done: agents come from the Stop hook (`Session::background`).
    let under = lines[bottom + 1..].join(" ").to_lowercase();
    let words: Vec<&str> = under.split_whitespace().collect();
    let running = under.contains("background")
        || words.windows(2).any(|w| {
            w[0].chars().all(|c| c.is_ascii_digit())
                && ["shell", "task"].iter().any(|k| w[1].starts_with(k))
        });
    if running {
        return false;
    }
    let inside = lines[top + 1..bottom].join("\n");
    let Some(rest) = inside.trim_start().strip_prefix('❯') else {
        return false;
    };
    rest.trim().is_empty()
}

/// A captured line's text without its escape codes, and without the text
/// drawn dim (SGR 2, until 0 or 22).
fn undimmed(line: &str) -> String {
    let mut out = String::new();
    let mut dim = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            if !dim {
                out.push(c);
            }
            continue;
        }
        if chars.peek() != Some(&'[') {
            continue;
        }
        chars.next();
        let mut params = String::new();
        let mut end = None;
        for c in chars.by_ref() {
            if c.is_ascii_alphabetic() {
                end = Some(c);
                break;
            }
            params.push(c);
        }
        if end != Some('m') {
            continue;
        }
        let mut ps = params.split(';');
        while let Some(p) = ps.next() {
            match p {
                "" | "0" | "22" => dim = false,
                "2" => dim = true,
                // A color's numbers: 38;5;n or 38;2;r;g;b.
                "38" | "48" | "58" => match ps.next() {
                    Some("5") => {
                        ps.next();
                    }
                    Some("2") => {
                        ps.next();
                        ps.next();
                        ps.next();
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
    out
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
        for bad in [
            "24",
            "h",
            "1w",
            "-2h",
            "0d",
            "5m",
            "soon",
            "1天",
            "24 ч",
            "99999999999999999d",
        ] {
            assert!(parse_limit(bad).is_err(), "{bad}");
        }
        assert_eq!(show(Duration::hours(24)), "1d");
        assert_eq!(show(Duration::hours(12)), "12h");
        assert_eq!(show(Duration::minutes(90)), "90m");
    }

    #[test]
    fn windows_parse() {
        let w = parse_windows(&[
            "@3 1763193600 1 abc".to_string(),
            "@4 1763193600 2 ".to_string(),
        ]);
        assert_eq!(w[0].id, "@3");
        assert_eq!(w[0].session, "abc");
        assert_eq!(w[0].panes, 1);
        assert_eq!(w[1].panes, 2);
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
            reply: None,
            background: false,
            agent: Agent::Claude,
            unseen: false,
            created_at: t0() - Duration::days(9),
            updated_at: t0() - Duration::hours(idle_h),
        }
    }

    fn win(id: &str, idle_h: i64) -> Window {
        Window {
            id: format!("@{id}"),
            panes: 1,
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
        // A side shell open beside Claude (C-\\): its window holds more.
        let mut sessions = sessions;
        sessions.push(sess("10", State::YourTurn, 48));
        windows.push(Window {
            panes: 2,
            ..win("10", 48)
        });
        // A background job from its last turn.
        let mut bg = sess("11", State::YourTurn, 48);
        bg.background = true;
        sessions.push(bg);
        windows.push(win("11", 48));
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
    fn a_background_task_under_the_box_isnt_idle() {
        let busy = EMPTY.replace("    status line", "  ⏵⏵ auto mode on · 1 background task");
        assert!(!input_box_empty(&busy));
        let shell = EMPTY.replace("    status line", "  1 shell · ↓ to view");
        assert!(!input_box_empty(&shell));
        // Real ones, from Claude Code 2.1.329.
        let shell = EMPTY.replace(
            "    status line",
            "  ⏵⏵ auto mode on · 1 shell · ↓ to manage",
        );
        assert!(!input_box_empty(&shell));
        // "← 1 agent" stays after the agent is done (it shows on sessions
        // with nothing running), so agents come from the Stop hook's
        // background_tasks instead (`Session::background`).
        let agent = EMPTY.replace(
            "    status line",
            "  -- INSERT -- ⏵⏵ auto mode on (shift+tab to cycle) · ← 1 agent",
        );
        assert!(input_box_empty(&agent));
        // A word in the status line alone isn't something running.
        let words = EMPTY.replace("    status line", "    📁 shell-scripts:main | agent notes");
        assert!(input_box_empty(&words));
    }

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

    #[test]
    fn a_named_sessions_box_has_its_name_in_the_top_rule() {
        // Claude Code 2.1.330, a session with a name.
        let named = EMPTY.replacen(
            "────────────────────────────────────────",
            "─────────────────────────── spain:trip ─",
            1,
        );
        assert!(input_box_empty(&named));
        let draft = named.replace("❯ \u{a0}", "❯ half a thought");
        assert!(!input_box_empty(&draft));
        // A table border in the reply isn't a rule.
        let table = format!("  └──────────┴──────────┘\n{EMPTY}");
        assert!(input_box_empty(&table));
    }

    #[test]
    fn dim_hint_text_in_the_box_isnt_a_draft() {
        // From `capture-pane -e`, Claude Code 2.1.330: a hint is drawn dim.
        let hint = EMPTY.replace(
            "❯ \u{a0}",
            "\x1b[39m❯ \x1b[2mOnce fixed, resend your message\x1b[0m",
        );
        assert!(input_box_empty(&hint));
        let typed = EMPTY.replace(
            "❯ \u{a0}",
            "\x1b[38;5;246m\x1b[48;5;237m❯ \x1b[38;5;231msay hi\x1b[39m",
        );
        assert!(!input_box_empty(&typed));
        // A color's numbers aren't a style: 38;5;2 is green, not dim.
        let green = EMPTY.replace("❯ \u{a0}", "❯ \x1b[38;5;2mdraft\x1b[39m");
        assert!(!input_box_empty(&green));
        let styled_rules = EMPTY.replace(
            "────────────────────────────────────────",
            "\x1b[38;5;244m────────────────────────────────────────\x1b[39m",
        );
        assert!(input_box_empty(&styled_rules));
    }
}
