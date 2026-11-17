//! Pure mapping from an agent's event (a Claude Code hook, an OpenCode
//! plugin event) to a session state change.

use serde::Deserialize;

use crate::model::State;

/// The subset of a hook's stdin JSON that Grove reads. Unknown fields are
/// ignored, so new Claude Code versions don't break parsing.
#[derive(Debug, Clone, Deserialize)]
pub struct HookEvent {
    pub session_id: String,
    pub hook_event_name: String,
    #[serde(default)]
    pub notification_type: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    /// SessionEnd only: "clear", "resume", "logout", "prompt_input_exit", "other".
    #[serde(default)]
    pub reason: Option<String>,
}

/// `None` means "no change". The string is the message to store.
pub fn next(current: State, ev: &HookEvent) -> Option<(State, Option<String>)> {
    if current == State::Ended {
        return None;
    }
    let (state, msg) = match ev.hook_event_name.as_str() {
        "UserPromptSubmit" | "PostToolUse" | "PostToolUseFailure" => (State::Working, None),
        "PermissionRequest" => (
            State::NeedsYou,
            Some(format!(
                "Permission needed: {}",
                ev.tool_name.as_deref().unwrap_or("a tool")
            )),
        ),
        "Notification" => match ev.notification_type.as_deref() {
            Some("permission_prompt") => (State::NeedsYou, ev.message.clone()),
            // An idle reminder while a permission prompt is open is still a
            // permission prompt; don't downgrade it.
            Some("idle_prompt") if current == State::NeedsYou => return None,
            Some("idle_prompt") => (State::YourTurn, ev.message.clone()),
            _ => return None,
        },
        "Stop" => (State::YourTurn, None),
        "StopFailure" => (
            State::YourTurn,
            Some(
                ev.message
                    .clone()
                    .unwrap_or_else(|| "Turn ended with an error".into()),
            ),
        ),
        // /clear and /resume end the conversation, not the window: Claude is
        // still running and waiting for input.
        "SessionEnd" if matches!(ev.reason.as_deref(), Some("clear" | "resume")) => {
            (State::YourTurn, None)
        }
        "SessionEnd" => (State::Ended, None),
        _ => return None,
    };
    if state == current {
        return None;
    }
    Some((state, msg))
}

/// An OpenCode event as Grove's plugin passes it on (runfiles.rs
/// `opencode_plugin`): its `type` and `data`. Unknown fields are ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct OpenCodeEvent {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub data: OpenCodeData,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct OpenCodeData {
    #[serde(rename = "sessionID", default)]
    pub session_id: Option<String>,
    /// permission.asked: what it asks to do (`shell`, `edit`, …).
    #[serde(default)]
    pub action: Option<String>,
    /// session.renamed.
    #[serde(default)]
    pub title: Option<String>,
    /// session.execution.failed.
    #[serde(default)]
    pub error: Option<OpenCodeError>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct OpenCodeError {
    #[serde(default)]
    pub message: Option<String>,
}

/// `next` for an OpenCode event. A turn ("execution") runs from started to
/// succeeded, failed or interrupted; a permission ask pauses it.
pub fn next_opencode(current: State, ev: &OpenCodeEvent) -> Option<(State, Option<String>)> {
    if current == State::Ended {
        return None;
    }
    let (state, msg) = match ev.kind.as_str() {
        "session.execution.started" | "permission.replied" => (State::Working, None),
        "permission.asked" => (
            State::NeedsYou,
            Some(format!(
                "Permission needed: {}",
                ev.data.action.as_deref().unwrap_or("a tool")
            )),
        ),
        "session.execution.succeeded" | "session.execution.interrupted" => (State::YourTurn, None),
        "session.execution.failed" => (
            State::YourTurn,
            Some(
                ev.data
                    .error
                    .as_ref()
                    .and_then(|e| e.message.clone())
                    .unwrap_or_else(|| "Turn ended with an error".into()),
            ),
        ),
        _ => return None,
    };
    if state == current {
        return None;
    }
    Some((state, msg))
}

#[cfg(test)]
mod opencode_tests {
    use super::*;

    fn fixture(name: &str) -> OpenCodeEvent {
        let path = format!(
            "{}/tests/fixtures/opencode/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
    }

    fn after(current: State, name: &str) -> Option<(State, Option<String>)> {
        next_opencode(current, &fixture(name))
    }

    #[test]
    fn a_turn_runs_from_started_to_its_end() {
        assert_eq!(
            after(State::Starting, "execution_started"),
            Some((State::Working, None))
        );
        for end in ["execution_succeeded", "execution_interrupted"] {
            assert_eq!(
                after(State::Working, end),
                Some((State::YourTurn, None)),
                "{end}"
            );
        }
        assert_eq!(
            after(State::Working, "execution_failed"),
            Some((
                State::YourTurn,
                Some("Response still truncated after two output token limit continuations".into())
            ))
        );
    }

    #[test]
    fn a_permission_ask_needs_you_until_answered() {
        assert_eq!(
            after(State::Working, "permission_asked"),
            Some((State::NeedsYou, Some("Permission needed: shell".into())))
        );
        assert_eq!(
            after(State::NeedsYou, "permission_replied"),
            Some((State::Working, None))
        );
    }

    #[test]
    fn other_events_and_ended_sessions_change_nothing() {
        assert_eq!(after(State::Working, "session_renamed"), None);
        assert_eq!(after(State::Working, "execution_started"), None);
        assert_eq!(after(State::Ended, "execution_started"), None);
    }

    #[test]
    fn every_fixture_names_its_session() {
        for name in [
            "execution_started",
            "execution_succeeded",
            "execution_interrupted",
            "execution_failed",
            "permission_asked",
            "permission_replied",
            "session_renamed",
        ] {
            assert!(
                crate::runfiles::OPENCODE_EVENTS.contains(&fixture(name).kind.as_str()),
                "{name} isn't passed on by the plugin"
            );
            assert_eq!(
                fixture(name).data.session_id.as_deref(),
                Some("ses_11111111222233334444555555555555")
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> HookEvent {
        let path = format!(
            "{}/tests/fixtures/hooks/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
    }

    fn state_after(current: State, name: &str) -> Option<State> {
        next(current, &fixture(name)).map(|(s, _)| s)
    }

    #[test]
    fn all_fixtures_parse() {
        for name in [
            "session_start",
            "user_prompt_submit",
            "post_tool_use",
            "post_tool_use_failure",
            "permission_request",
            "notification_permission",
            "notification_idle",
            "notification_auth",
            "stop",
            "stop_failure",
            "session_end",
        ] {
            fixture(name);
        }
    }

    #[test]
    fn working_events() {
        for name in [
            "user_prompt_submit",
            "post_tool_use",
            "post_tool_use_failure",
        ] {
            assert_eq!(
                state_after(State::YourTurn, name),
                Some(State::Working),
                "{name}"
            );
        }
    }

    #[test]
    fn approving_a_tool_clears_needs_you() {
        assert_eq!(
            state_after(State::NeedsYou, "post_tool_use"),
            Some(State::Working)
        );
    }

    #[test]
    fn needs_you_events() {
        assert_eq!(
            state_after(State::Working, "permission_request"),
            Some(State::NeedsYou)
        );
        assert_eq!(
            state_after(State::Working, "notification_permission"),
            Some(State::NeedsYou)
        );
    }

    #[test]
    fn needs_you_message_is_kept() {
        let (_, msg) = next(State::Working, &fixture("notification_permission")).unwrap();
        assert_eq!(
            msg.as_deref(),
            Some("Claude needs your permission to use Bash")
        );
        let (_, msg) = next(State::Working, &fixture("permission_request")).unwrap();
        assert_eq!(msg.as_deref(), Some("Permission needed: Bash"));
    }

    #[test]
    fn your_turn_events() {
        for name in ["stop", "stop_failure", "notification_idle"] {
            assert_eq!(
                state_after(State::Working, name),
                Some(State::YourTurn),
                "{name}"
            );
        }
    }

    #[test]
    fn stop_failure_keeps_error_as_message() {
        let mut ev = fixture("stop_failure");
        ev.message = None;
        let (_, msg) = next(State::Working, &ev).unwrap();
        assert_eq!(msg.as_deref(), Some("Turn ended with an error"));
    }

    #[test]
    fn idle_prompt_does_not_override_needs_you() {
        assert_eq!(state_after(State::NeedsYou, "notification_idle"), None);
    }

    #[test]
    fn clear_or_resume_does_not_end_a_live_session() {
        assert_eq!(
            state_after(State::Working, "session_end_clear"),
            Some(State::YourTurn)
        );
        let mut ev = fixture("session_end_clear");
        ev.reason = Some("resume".into());
        assert_eq!(next(State::YourTurn, &ev), None);
    }

    #[test]
    fn session_end_ends() {
        assert_eq!(
            state_after(State::Working, "session_end"),
            Some(State::Ended)
        );
    }

    #[test]
    fn no_change_events() {
        assert_eq!(state_after(State::Starting, "session_start"), None);
        assert_eq!(state_after(State::Working, "notification_auth"), None);
        let mut ev = fixture("stop");
        ev.hook_event_name = "PreCompact".into();
        assert_eq!(next(State::Working, &ev), None);
    }

    #[test]
    fn ended_sessions_stay_ended() {
        for name in ["user_prompt_submit", "permission_request", "stop"] {
            assert_eq!(state_after(State::Ended, name), None, "{name}");
        }
    }

    #[test]
    fn same_state_is_no_change() {
        assert_eq!(state_after(State::Working, "post_tool_use"), None);
    }
}
