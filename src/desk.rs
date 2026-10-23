//! The pinned desk (`V`): a tmux session with a column per pinned session,
//! each a live view of that session's own window, here or on its finca, so
//! you can work in several at once. A column runs `grove desk-attach <id>`
//! (over ssh for a finca's), which shows the session's window in a
//! one-window session of its own and attaches to it. The window itself
//! stays where it was: closing the desk never ends a session.

use std::path::Path;

use anyhow::{Result, bail};

use crate::model::State;
use crate::pins::Pin;
use crate::store::Store;
use crate::tmux::Tmux;

/// The desk's tmux session.
pub const SESSION: &str = "desk";
/// Each column's own session is this plus the Grove session's id.
const COLUMN: &str = "desk-";
/// Narrower than this Claude gets cramped, so the columns tile instead.
const COLUMN_MIN_W: usize = 60;

/// Whether tmux session `name` is a desk column's.
pub fn is_column(name: &str) -> bool {
    name.starts_with(COLUMN)
}

/// A column: which pin it shows and what it runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub key: String,
    pub argv: Vec<String>,
}

/// The columns for `pins` (each with the finca destination that last
/// answered); `exe` is this grove.
pub fn columns(store: &Store, pins: &[(Pin, Option<String>)], exe: &str) -> Result<Vec<Column>> {
    pins.iter()
        .map(|(pin, via)| {
            let argv = match &pin.finca {
                None => vec![exe.to_string(), "desk-attach".into(), pin.id.clone()],
                Some(slug) => {
                    let f = store.get_finca(slug)?;
                    let via = via.as_deref().filter(|v| f.destinations().any(|d| d == *v));
                    let cmd = format!(
                        "{} desk-attach {}",
                        crate::finca::sh_quote(&f.grove_bin),
                        crate::finca::sh_quote(&pin.id)
                    );
                    crate::finca::remote_argv(&f, via.unwrap_or(&f.ssh), &cmd)
                }
            };
            Ok(Column {
                key: format!("{}/{}", pin.finca.as_deref().unwrap_or(""), pin.id),
                argv,
            })
        })
        .collect()
}

/// Puts the client on the desk, rebuilt first if the pins changed or a
/// column has closed.
pub fn open(tmux: &Tmux, columns: &[Column], cwd: &Path) -> Result<()> {
    if columns.is_empty() {
        bail!("Nothing is pinned yet: p on a session pins it");
    }
    let key = keys(columns);
    let fresh = tmux.has_session(SESSION)
        && tmux.session_option(SESSION, "@grove_pins").as_deref() == Some(key.as_str())
        && tmux.pane_count(&format!("={SESSION}:")) == columns.len();
    if !fresh {
        build(tmux, columns, cwd, tmux.client_width())?;
    }
    tmux.switch_client(&format!("={SESSION}"))
}

fn keys(columns: &[Column]) -> String {
    columns
        .iter()
        .map(|c| c.key.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A new desk with `columns`, for a client `width` wide.
fn build(tmux: &Tmux, columns: &[Column], cwd: &Path, width: Option<u16>) -> Result<()> {
    let _ = tmux.kill_session(SESSION);
    // Columns an old desk left behind.
    for s in tmux.list_sessions().iter().filter(|s| is_column(s)) {
        let _ = tmux.kill_session(s);
    }
    let target = format!("={SESSION}:");
    let layout = layout(columns.len(), width);
    tmux.new_session_detached(SESSION, "pinned", cwd, &columns[0].argv)?;
    for c in &columns[1..] {
        tmux.split_window(&target, cwd, &c.argv)?;
        // Evened out each time, so the next split has room.
        tmux.select_layout(&target, layout)?;
    }
    tmux.set_option(SESSION, "@grove_pins", &keys(columns))?;
    // When the last column closes, go back to the dashboard.
    tmux.set_option(SESSION, "detach-on-destroy", "off")?;
    Ok(())
}

/// Side by side, unless that makes columns too narrow.
fn layout(n: usize, width: Option<u16>) -> &'static str {
    match width {
        Some(w) if usize::from(w) < n * COLUMN_MIN_W => "tiled",
        _ => "even-horizontal",
    }
}

/// `grove desk-attach`, in a column: makes session `id`'s window a column
/// session's only window and returns that session's name.
pub fn prepare(tmux: &Tmux, store: &Store, id: &str) -> Result<String> {
    let Some(s) = store.get_session(id) else {
        bail!("That session is no longer here");
    };
    let window = match &s.tmux_window {
        Some(w) if s.state != State::Ended => w.clone(),
        _ => bail!("{}:{} has ended", s.project, s.name),
    };
    let name = format!("{COLUMN}{id}");
    let _ = tmux.kill_session(&name);
    let placeholder = tmux.new_session_window(&name, &["sleep", "60"])?;
    let linked = tmux.link_window(&window, &name);
    let _ = tmux.kill_window(&placeholder);
    if let Err(e) = linked {
        let _ = tmux.kill_session(&name);
        bail!("{}:{} has no window: {e:#}", s.project, s.name);
    }
    // The desk has a status bar; one per column would be noise.
    tmux.set_option(&name, "status", "off")?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use crate::model::Session;

    struct Server(Tmux);
    impl Drop for Server {
        fn drop(&mut self) {
            self.0.kill_server();
        }
    }

    fn server(tag: &str) -> Server {
        Server(Tmux::new(format!(
            "grove-test-{}-desk-{tag}",
            std::process::id()
        )))
    }

    fn sh(cmd: &str) -> Vec<String> {
        vec!["sh".into(), "-c".into(), cmd.into()]
    }

    fn store_with(window: Option<&str>, state: State) -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let store = Store::new(Paths::new(d.path().join("home")));
        store
            .save_session(&Session {
                id: "s1".into(),
                project: "alpha".into(),
                name: "fix".into(),
                host: "local".into(),
                tmux_window: window.map(str::to_string),
                state,
                message: None,
                conversation: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .unwrap();
        (d, store)
    }

    #[test]
    fn a_column_shows_the_sessions_own_window() {
        let t = server("prepare");
        let d = tempfile::tempdir().unwrap();
        t.0.new_session_detached("grove", "dashboard", d.path(), &sh("sleep 60"))
            .unwrap();
        let w =
            t.0.new_window("grove", "fix", d.path(), &[], &sh("sleep 60"))
                .unwrap();
        let (_s, store) = store_with(Some(&w), State::Working);
        let name = prepare(&t.0, &store, "s1").unwrap();
        assert_eq!(name, "desk-s1");
        assert!(is_column(&name) && !is_column(SESSION));
        // Its only window is the session's, which stays in Grove's too.
        assert_eq!(t.0.list_windows(&name).unwrap(), std::slice::from_ref(&w));
        assert!(t.0.list_windows("grove").unwrap().contains(&w));
        assert_eq!(t.0.session_option(&name, "status").as_deref(), Some("off"));
        // Again (a desk rebuilt): replaced, not doubled.
        prepare(&t.0, &store, "s1").unwrap();
        assert_eq!(t.0.list_windows(&name).unwrap(), std::slice::from_ref(&w));
        // Closing the column leaves the session running.
        t.0.kill_session(&name).unwrap();
        assert!(t.0.list_windows("grove").unwrap().contains(&w));
    }

    #[test]
    fn ended_or_missing_sessions_say_so() {
        let t = server("ended");
        let (_s, store) = store_with(Some("@1"), State::Ended);
        let e = prepare(&t.0, &store, "s1").unwrap_err();
        assert!(format!("{e:#}").contains("alpha:fix has ended"), "{e:#}");
        let e = prepare(&t.0, &store, "nope").unwrap_err();
        assert!(format!("{e:#}").contains("no longer here"), "{e:#}");
        // Its window gone: nothing left behind.
        let (_s, store) = store_with(Some("@99"), State::Working);
        let d = tempfile::tempdir().unwrap();
        t.0.new_session_detached("grove", "dashboard", d.path(), &sh("sleep 60"))
            .unwrap();
        assert!(prepare(&t.0, &store, "s1").is_err());
        assert!(!t.0.has_session("desk-s1"));
    }

    #[test]
    fn the_desk_has_a_column_per_pin_and_is_rebuilt_when_they_change() {
        let t = server("build");
        let d = tempfile::tempdir().unwrap();
        let col = |k: &str| Column {
            key: k.into(),
            argv: sh("sleep 60"),
        };
        let three = [col("/a"), col("pi/b"), col("/c")];
        build(&t.0, &three, d.path(), Some(200)).unwrap();
        assert_eq!(t.0.pane_count("=desk:"), 3);
        assert_eq!(
            t.0.session_option(SESSION, "@grove_pins").as_deref(),
            Some("/a pi/b /c")
        );
        // A leftover column session from an old desk goes on a rebuild.
        t.0.new_session_window("desk-old", &["sleep", "60"])
            .unwrap();
        build(&t.0, &three[..2], d.path(), Some(200)).unwrap();
        assert_eq!(t.0.pane_count("=desk:"), 2);
        assert!(!t.0.has_session("desk-old"));
        assert!(t.0.has_session(SESSION));
        // Many columns still fit a small window.
        let many: Vec<_> = (0..8).map(|i| col(&format!("/{i}"))).collect();
        build(&t.0, &many, d.path(), Some(80)).unwrap();
        assert_eq!(t.0.pane_count("=desk:"), 8);
    }

    #[test]
    fn columns_stack_when_they_would_be_too_narrow() {
        assert_eq!(layout(3, Some(200)), "even-horizontal");
        assert_eq!(layout(4, Some(200)), "tiled");
        assert_eq!(layout(1, Some(40)), "tiled");
        assert_eq!(layout(4, None), "even-horizontal");
    }

    #[test]
    fn nothing_pinned_explains() {
        let t = server("empty");
        let d = tempfile::tempdir().unwrap();
        let e = open(&t.0, &[], d.path()).unwrap_err();
        assert!(format!("{e}").contains("p on a session pins it"), "{e}");
    }

    #[test]
    fn local_columns_attach_here_and_finca_ones_over_ssh() {
        let d = tempfile::tempdir().unwrap();
        let store = Store::new(Paths::new(d.path().join("home")));
        store.paths.ensure().unwrap();
        std::fs::write(
            store.paths.fincas.join("pi.toml"),
            "name = \"pi\"\nssh = \"me@pi\"\ngrove_bin = \"/opt/grove\"\nbackup = \"me@pi-wifi\"\n",
        )
        .unwrap();
        let pins = [
            (
                Pin {
                    finca: None,
                    id: "s1".into(),
                },
                None,
            ),
            (
                Pin {
                    finca: Some("pi".into()),
                    id: "r2".into(),
                },
                Some("me@pi-wifi".into()),
            ),
        ];
        let cols = columns(&store, &pins, "/bin/grove").unwrap();
        assert_eq!(cols[0].key, "/s1");
        assert_eq!(cols[0].argv, ["/bin/grove", "desk-attach", "s1"]);
        assert_eq!(cols[1].key, "pi/r2");
        let script = cols[1].argv.join(" ");
        // The destination that last answered, and the remote grove.
        assert!(script.contains("me@pi-wifi"), "{script}");
        assert!(
            script.contains("desk-attach") && script.contains("r2"),
            "{script}"
        );
    }
}
