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
        return Err(
            "a line starting with \"!\" would run a shell command there; rephrase it".into(),
        );
    }
    if starts('/') {
        return Err(
            "a line starting with \"/\" would run a slash command there; rephrase it".into(),
        );
    }
    Ok(text)
}

/// The session's tmux window, if it's still that session's. Window ids
/// restart on a new tmux server, so a stale record could name someone
/// else's window: Grove marks each window with its session's id.
pub fn window(tmux: &Tmux, s: &Session, account: &str, handle: &str) -> Result<String, String> {
    let w = s
        .tmux_window
        .clone()
        .ok_or_else(|| format!("{handle} has no window yet (it may still be starting)"))?;
    let ours = match tmux.window_option(&w, "@grove_session") {
        Some(id) => id == s.id,
        // Windows from before Grove marked them: at least the account.
        None => tmux.window_option(&w, "@grove_account").as_deref() == Some(account),
    };
    if ours {
        Ok(w)
    } else {
        Err(format!("{handle}'s window is gone"))
    }
}

/// Types `text` into `s` (its project on `account`) and presses Enter.
/// Returns the session's handle, `project:name`.
pub fn type_into(tmux: &Tmux, s: &Session, account: &str, text: &str) -> Result<String, String> {
    let handle = format!("{}:{}", s.project, s.name);
    match s.state {
        State::YourTurn => {}
        State::NeedsYou => {
            return Err(format!(
                "{handle} needs you: answer its prompt on the dashboard first"
            ));
        }
        other => {
            return Err(format!(
                "{handle} is {}; wait until it's your turn",
                other.label()
            ));
        }
    }
    let text = clean(text)?;
    let w = window(tmux, s, account, &handle)?;
    tmux.paste_text(&w, &text)
        .map_err(|e| format!("couldn't type into {handle}: {e:#}"))?;
    // Claude's input may need a beat after a paste before Enter.
    std::thread::sleep(std::time::Duration::from_millis(150));
    tmux.send_keys(&w, &["Enter"])
        .map_err(|e| format!("{e:#}"))?;
    Ok(handle)
}

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
        for bad in [
            "!ls",
            "  /help",
            "ok\n!rm -rf ~",
            "ok\u{2028}/x",
            "ok\u{2029}!x",
        ] {
            let e = clean(bad).unwrap_err();
            assert!(e.contains('!') || e.contains('/'), "{bad}: {e}");
        }
        assert!(clean("why does a/b fail? use x!").is_ok());
    }
}
