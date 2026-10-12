//! Desktop alerts when a session starts waiting on you. The dashboard sends
//! them (not the hook), so sessions on fincas alert on this machine too.

use std::collections::{HashMap, HashSet};
use std::process::{Command, Stdio};

use crate::model::State;

/// One session as the dashboard currently sees it.
#[derive(Debug, Clone)]
pub struct Observed {
    /// Stable across checks: the session id, or finca/project/session.
    pub key: String,
    /// "local" or the finca's slug. A source's first check only records.
    pub source: String,
    pub state: State,
    /// The tmux window it runs in on this machine (local sessions only).
    pub window: Option<String>,
    /// e.g. "grove:2" or "homebox · grove:2".
    pub label: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub title: String,
    pub body: String,
}

/// Remembers each session's last state so only changes alert.
#[derive(Debug, Default)]
pub struct Alerts {
    states: HashMap<String, State>,
    primed: HashSet<String>,
}

impl Alerts {
    /// Alerts for sessions whose state changed since the last check.
    /// `watched` lists the windows on screen in a focused terminal; it's only
    /// called if some session changed.
    pub fn check(
        &mut self,
        now: Vec<Observed>,
        watched: impl FnOnce() -> Vec<String>,
    ) -> Vec<Alert> {
        let mut watched = Some(watched);
        let mut watched_list: Option<Vec<String>> = None;
        let mut out = Vec::new();
        let mut next = HashMap::new();
        let mut sources = HashSet::new();
        for o in now {
            let prev = self.states.get(&o.key).copied();
            // Already-waiting sessions at startup (or a finca's first poll)
            // aren't news.
            let primed = self.primed.contains(&o.source);
            if primed && prev != Some(o.state) {
                let list = watched_list
                    .get_or_insert_with(|| watched.take().map(|f| f()).unwrap_or_default());
                let watching = o.window.as_ref().is_some_and(|w| list.contains(w));
                if worth_alerting(prev, o.state, watching) {
                    out.push(Alert {
                        title: format!("{} {}", o.label, o.state.label()),
                        body: o
                            .message
                            .clone()
                            .unwrap_or_else(|| "Claude is waiting on you".into()),
                    });
                }
            }
            sources.insert(o.source);
            next.insert(o.key, o.state);
        }
        self.primed.extend(sources);
        self.states = next;
        out
    }
}

/// Whether a session going from `prev` to `now` should alert. `prev` is None
/// for a session seen for the first time; `watching` means its window is on
/// screen in a focused terminal.
pub fn worth_alerting(prev: Option<State>, now: State, watching: bool) -> bool {
    if watching {
        return false;
    }
    match now {
        State::NeedsYou => true,
        // Claude finished a turn. Not after a permission prompt: that's the
        // same wait you were already alerted to.
        State::YourTurn => prev == Some(State::Working),
        State::Starting | State::Working | State::Ended => false,
    }
}

/// Shows the alert with the system's notifier, without waiting for it.
/// Failures (no notifier, no desktop over SSH) are silent.
pub fn send(a: &Alert) {
    let mut cmd = if cfg!(target_os = "macos") {
        // Text goes in as arguments, so quotes in a message need no escaping.
        let mut c = Command::new("osascript");
        c.args([
            "-e",
            "on run argv",
            "-e",
            "display notification (item 2 of argv) with title \"Grove\" subtitle (item 1 of argv)",
            "-e",
            "end run",
        ]);
        c
    } else {
        let mut c = Command::new("notify-send");
        c.args(["--app-name=Grove", "--"]);
        c
    };
    cmd.args([&a.title, &a.body])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Ok(mut child) = cmd.spawn() {
        std::thread::spawn(move || child.wait());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(key: &str, source: &str, state: State, window: Option<&str>) -> Observed {
        Observed {
            key: key.into(),
            source: source.into(),
            state,
            window: window.map(str::to_string),
            label: format!("p:{key}"),
            message: None,
        }
    }

    fn none() -> Vec<String> {
        Vec::new()
    }

    #[test]
    fn needing_you_or_finishing_a_turn_alerts_unless_watched() {
        use State::*;
        assert!(worth_alerting(Some(Working), NeedsYou, false));
        assert!(worth_alerting(None, NeedsYou, false));
        assert!(worth_alerting(Some(Working), YourTurn, false));
        assert!(!worth_alerting(Some(Working), NeedsYou, true));
        assert!(!worth_alerting(Some(Working), YourTurn, true));
        // Leaving a permission prompt idle is the same wait, not news.
        assert!(!worth_alerting(Some(NeedsYou), YourTurn, false));
        for quiet in [Starting, Working, Ended] {
            assert!(!worth_alerting(Some(YourTurn), quiet, false), "{quiet:?}");
        }
    }

    #[test]
    fn first_check_of_a_source_only_records() {
        let mut a = Alerts::default();
        let alerts = a.check(vec![obs("1", "local", State::NeedsYou, Some("@1"))], none);
        assert!(alerts.is_empty());
        // Still waiting: no change, no alert.
        let alerts = a.check(vec![obs("1", "local", State::NeedsYou, Some("@1"))], none);
        assert!(alerts.is_empty());
    }

    #[test]
    fn starting_to_need_you_alerts_with_the_message() {
        let mut a = Alerts::default();
        a.check(vec![obs("1", "local", State::Working, Some("@1"))], none);
        let mut o = obs("1", "local", State::NeedsYou, Some("@1"));
        o.message = Some("Permission needed: Bash".into());
        let alerts = a.check(vec![o], none);
        assert_eq!(
            alerts,
            vec![Alert {
                title: "p:1 needs you".into(),
                body: "Permission needed: Bash".into()
            }]
        );
    }

    #[test]
    fn a_fincas_first_poll_only_records() {
        let mut a = Alerts::default();
        a.check(vec![obs("1", "local", State::Working, Some("@1"))], none);
        let alerts = a.check(
            vec![
                obs("1", "local", State::Working, Some("@1")),
                obs("homebox/p/1", "homebox", State::NeedsYou, None),
            ],
            none,
        );
        assert!(alerts.is_empty());
    }

    #[test]
    fn watched_windows_are_only_asked_for_on_a_change() {
        let mut a = Alerts::default();
        a.check(vec![obs("1", "local", State::Working, Some("@1"))], none);
        a.check(vec![obs("1", "local", State::Working, Some("@1"))], || {
            panic!("asked for watched windows with nothing changed")
        });
    }
}
