//! `grove doctor`: checks everything Grove depends on and says what's wrong,
//! all at once instead of stopping at the first problem.

use std::path::Path;

use crate::config::{Config, Paths, load_config};
use crate::finca::{Finca, Reach};
use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    /// Worth knowing, but Grove works (e.g. a finca away from home).
    Warn,
    /// Sessions won't start, or something is broken.
    Fail,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub level: Level,
    pub what: String,
    pub detail: String,
}

/// The outside world, so tests don't depend on the machine.
pub struct Probes<'a> {
    pub tmux_version: &'a dyn Fn() -> anyhow::Result<(u32, u32)>,
    /// `claude`'s absolute path, or None if it's not on PATH.
    pub claude: &'a dyn Fn() -> Option<String>,
    /// Whether a program is on PATH.
    pub has: &'a dyn Fn(&str) -> bool,
    pub poll: &'a dyn Fn(&Finca) -> (Reach, Option<String>),
}

fn check(level: Level, what: impl Into<String>, detail: impl Into<String>) -> Check {
    Check {
        level,
        what: what.into(),
        detail: detail.into(),
    }
}

pub fn run(paths: &Paths, probes: &Probes) -> Vec<Check> {
    let mut out = Vec::new();
    match (probes.tmux_version)() {
        Ok(v) if v >= (3, 2) => out.push(check(Level::Ok, "tmux", format!("{}.{}", v.0, v.1))),
        Ok(v) => out.push(check(
            Level::Fail,
            "tmux",
            format!("{}.{} is too old; Grove needs 3.2 or newer", v.0, v.1),
        )),
        Err(e) => out.push(check(Level::Fail, "tmux", format!("{e:#}"))),
    }
    match (probes.claude)() {
        Some(p) => out.push(check(Level::Ok, "claude", p)),
        None => out.push(check(
            Level::Fail,
            "claude",
            "not found on PATH; install Claude Code",
        )),
    }

    let cfg = match load_config(paths) {
        Ok(c) => {
            out.push(check(Level::Ok, "config", show(&paths.config_file())));
            c
        }
        Err(e) => {
            out.push(check(Level::Fail, "config", format!("{e:#}")));
            Config::default()
        }
    };
    accounts(&cfg, &mut out);
    let store = Store::new(paths.clone());
    projects(&store, &cfg, &mut out);
    fincas(&store, probes, &mut out);

    let notifier = if cfg!(target_os = "macos") {
        "osascript"
    } else {
        "notify-send"
    };
    if !cfg.alerts {
        out.push(check(Level::Ok, "alerts", "off (config.toml)"));
    } else if (probes.has)(notifier) {
        out.push(check(Level::Ok, "alerts", format!("via {notifier}")));
    } else {
        out.push(check(
            Level::Warn,
            "alerts",
            format!(
                "{notifier} not found, so no desktop alerts; set alerts = false to silence this"
            ),
        ));
    }
    out
}

fn accounts(cfg: &Config, out: &mut Vec<Check>) {
    let names = std::iter::once(None).chain(cfg.accounts.keys().map(|k| Some(k.as_str())));
    for name in names {
        let Ok(a) = cfg.account(name) else { continue };
        let what = format!("account {}", a.name);
        if a.dir.is_dir() {
            out.push(check(Level::Ok, what, show(&a.dir)));
        } else {
            out.push(check(
                Level::Fail,
                what,
                format!(
                    "{} doesn't exist; log in with CLAUDE_CONFIG_DIR={} claude",
                    show(&a.dir),
                    show(&a.dir)
                ),
            ));
        }
    }
}

fn projects(store: &Store, cfg: &Config, out: &mut Vec<Check>) {
    for r in store.list_projects() {
        match r {
            Err((path, e)) => out.push(check(
                Level::Fail,
                format!("project file {}", show(&path)),
                e,
            )),
            Ok(p) => {
                let what = format!("project {}", p.name);
                if !p.path.is_dir() {
                    out.push(check(
                        Level::Fail,
                        what,
                        format!(
                            "{} is gone; grove rm {} to forget it",
                            show(&p.path),
                            p.slug()
                        ),
                    ));
                } else if let Err(e) = cfg.account(p.account.as_deref()) {
                    out.push(check(Level::Fail, what, format!("{e:#}")));
                } else {
                    out.push(check(Level::Ok, what, show(&p.path)));
                }
            }
        }
    }
}

fn fincas(store: &Store, probes: &Probes, out: &mut Vec<Check>) {
    for r in store.list_fincas() {
        match r {
            Err((path, e)) => {
                out.push(check(Level::Fail, format!("finca file {}", show(&path)), e))
            }
            Ok(f) => {
                let what = format!("finca {}", f.name);
                match (probes.poll)(&f) {
                    (Reach::Reachable(st), via) => out.push(check(
                        Level::Ok,
                        what,
                        format!(
                            "via {}: {} live, {} need you",
                            via.unwrap_or(f.ssh),
                            st.live,
                            st.needs_you
                        ),
                    )),
                    (Reach::NeedsUpdate, _) => out.push(check(
                        Level::Fail,
                        what,
                        "its grove is a different version; update it (scripts/deploy-fincas.sh)",
                    )),
                    (Reach::Unreachable { error }, _) => out.push(check(Level::Warn, what, error)),
                }
            }
        }
    }
}

/// A path with the home folder shown as `~`.
fn show(p: &Path) -> String {
    let home = crate::config::expand_tilde("~");
    match p.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Project;

    fn ok_probes<'a>() -> Probes<'a> {
        Probes {
            tmux_version: &|| Ok((3, 5)),
            claude: &|| Some("/bin/claude".into()),
            has: &|_| true,
            poll: &|_| {
                (
                    Reach::Unreachable {
                        error: "timed out".into(),
                    },
                    None,
                )
            },
        }
    }

    fn level_of(checks: &[Check], what: &str) -> Level {
        checks
            .iter()
            .find(|c| c.what == what)
            .unwrap_or_else(|| panic!("no check {what:?} in {checks:?}"))
            .level
    }

    #[test]
    fn reports_every_problem_at_once() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path().join("home"));
        paths.ensure().unwrap();
        let play = d.path().join("claude-play");
        std::fs::create_dir(&play).unwrap();
        std::fs::write(
            paths.config_file(),
            format!(
                "claude_config_dir = \"{}\"\n[accounts.work]\ndir = \"{}\"\n",
                play.display(),
                d.path().join("missing").display()
            ),
        )
        .unwrap();
        let store = Store::new(paths.clone());
        let proj = |name: &str, path: &Path, account: Option<&str>| Project {
            name: name.into(),
            path: path.into(),
            color: None,
            summary: None,
            account: account.map(str::to_string),
        };
        store.add_project(&proj("here", d.path(), None)).unwrap();
        store
            .add_project(&proj("gone", &d.path().join("nope"), None))
            .unwrap();
        store
            .add_project(&proj("typo", d.path(), Some("wrok")))
            .unwrap();
        store
            .add_finca(&Finca {
                name: "away".into(),
                ssh: "me@away".into(),
                grove_bin: "grove".into(),
                color: None,
                backup: None,
            })
            .unwrap();
        let old_tmux = Probes {
            tmux_version: &|| Ok((3, 1)),
            claude: &|| None,
            ..ok_probes()
        };

        let checks = run(&paths, &old_tmux);
        assert_eq!(level_of(&checks, "tmux"), Level::Fail);
        assert_eq!(level_of(&checks, "claude"), Level::Fail);
        assert_eq!(level_of(&checks, "account play"), Level::Ok);
        assert_eq!(level_of(&checks, "account work"), Level::Fail);
        assert_eq!(level_of(&checks, "project here"), Level::Ok);
        assert_eq!(level_of(&checks, "project gone"), Level::Fail);
        assert_eq!(level_of(&checks, "project typo"), Level::Fail);
        // Away from home isn't broken.
        assert_eq!(level_of(&checks, "finca away"), Level::Warn);
        assert_eq!(level_of(&checks, "alerts"), Level::Ok);
    }

    #[test]
    fn a_missing_notifier_only_warns() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path().join("home"));
        let probes = Probes {
            has: &|_| false,
            ..ok_probes()
        };
        assert_eq!(level_of(&run(&paths, &probes), "alerts"), Level::Warn);
    }
}
