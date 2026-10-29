//! `C-\` in a session's window: a shell below it, in the session's folder,
//! for running what Claude suggests. `C-\` again moves between the two, and
//! `exit` closes the shell. The shell pane is tagged, so Grove keeps
//! reading and typing into the Claude pane (tmux.rs `main_pane`).

use std::path::Path;

use anyhow::Result;

use crate::tmux::Tmux;

/// The pane option marking a side shell.
pub const TAG: &str = "@grove_side";
/// The shell's share of the window's height.
const HEIGHT: &str = "35%";

#[derive(Debug, PartialEq)]
pub enum Did {
    Opened(String),
    Focused(String),
    /// Why nothing happened, for the status line.
    Refused(&'static str),
}

/// What `C-\` does in `pane`: opens a shell running `argv` below it, or
/// moves to the shell or back to the session when there is one.
pub fn toggle(tmux: &Tmux, pane: &str, argv: &[String]) -> Result<Did> {
    let info = tmux.display(pane, "#{window_id}\t#{window_name}\t#{pane_current_path}")?;
    let mut f = info.splitn(3, '\t');
    let (window, name, cwd) = (
        f.next().unwrap_or_default(),
        f.next().unwrap_or_default(),
        f.next().unwrap_or_default(),
    );
    if name == "dashboard" {
        return Ok(Did::Refused(
            "C-\\ opens a shell below a session, not the dashboard",
        ));
    }
    let panes = tmux.list_panes_as(window, &format!("#{{pane_id}}\t#{{{TAG}}}"))?;
    let (sides, mains): (Vec<_>, Vec<_>) = panes
        .iter()
        .filter_map(|l| l.split_once('\t'))
        .partition(|(_, tag)| !tag.is_empty());
    let here_is_side = sides.iter().any(|(id, _)| *id == pane);
    let to = if here_is_side {
        mains.first()
    } else {
        sides.first()
    };
    if let Some((id, _)) = to {
        tmux.select_pane(id)?;
        return Ok(Did::Focused(id.to_string()));
    }
    if here_is_side {
        // Only side shells left: nothing to go back to.
        return Ok(Did::Refused("this window has no session pane"));
    }
    let id = tmux.split_below(pane, Path::new(cwd), HEIGHT, argv)?;
    tmux.set_pane_option(&id, TAG, "1")?;
    Ok(Did::Opened(id))
}

/// `grove side-shell`, from the key binding: reports through tmux, never
/// stdout (run-shell would show it).
pub fn run(tmux: &Tmux, pane: &str) {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".into());
    match toggle(tmux, pane, &[shell]) {
        Ok(Did::Refused(why)) => tmux.display_message(pane, why),
        Ok(_) => {}
        Err(e) => tmux.display_message(pane, &format!("couldn't open a shell: {e:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Server(Tmux);
    impl Drop for Server {
        fn drop(&mut self) {
            self.0.kill_server();
        }
    }

    fn server(tag: &str) -> Server {
        Server(Tmux::new(format!(
            "grove-test-{}-side-{tag}",
            std::process::id()
        )))
    }

    fn sh(cmd: &str) -> Vec<String> {
        vec!["sh".into(), "-c".into(), cmd.into()]
    }

    fn active(t: &Tmux, window: &str) -> String {
        t.display(window, "#{pane_id}").unwrap()
    }

    #[test]
    fn opens_below_in_the_sessions_folder_then_moves_between_them() {
        let t = server("toggle");
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().canonicalize().unwrap();
        t.0.new_session_detached("grove", "dashboard", &dir, &sh("sleep 60"))
            .unwrap();
        let w =
            t.0.new_window("grove", "alpha:fix", &dir, &[], &sh("sleep 60"))
                .unwrap();
        let claude = active(&t.0, &w);
        let Did::Opened(shell) = toggle(&t.0, &claude, &sh("sleep 60")).unwrap() else {
            panic!("no shell")
        };
        assert_eq!(t.0.pane_count(&w), 2);
        assert_eq!(active(&t.0, &w), shell);
        // Below the session, in its folder.
        let at =
            t.0.display(&shell, "#{pane_at_bottom} #{pane_current_path}")
                .unwrap();
        assert_eq!(at, format!("1 {}", dir.display()));
        // Grove still reads and types into the session.
        assert_eq!(t.0.main_pane(&w), claude);
        // Back and forth, never a second shell.
        assert_eq!(
            toggle(&t.0, &shell, &[]).unwrap(),
            Did::Focused(claude.clone())
        );
        assert_eq!(active(&t.0, &w), claude);
        assert_eq!(
            toggle(&t.0, &claude, &[]).unwrap(),
            Did::Focused(shell.clone())
        );
        assert_eq!(t.0.pane_count(&w), 2);
        // Closing the shell leaves the session alone.
        t.0.kill_pane(&shell).unwrap();
        assert_eq!(t.0.pane_count(&w), 1);
        assert_eq!(t.0.main_pane(&w), claude);
    }

    #[test]
    fn not_on_the_dashboard() {
        let t = server("dash");
        let d = tempfile::tempdir().unwrap();
        t.0.new_session_detached("grove", "dashboard", d.path(), &sh("sleep 60"))
            .unwrap();
        let pane = active(&t.0, "=grove:");
        assert!(matches!(
            toggle(&t.0, &pane, &sh("sleep 60")).unwrap(),
            Did::Refused(_)
        ));
        assert_eq!(t.0.pane_count("=grove:"), 1);
    }
}
