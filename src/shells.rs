//! Shells that stay open (`S` on the dashboard): one window each in
//! Grove's tmux session, tagged with its project. tmux is the record:
//! nothing is stored, so a shell lasts exactly as long as its window.

use anyhow::{Result, bail};

use crate::model::Project;
use crate::tmux::Tmux;

/// The window option holding the shell's project slug.
pub const TAG: &str = "@grove_shell";

#[derive(Debug, Clone, PartialEq)]
pub struct Shell {
    /// The window id (`@N`).
    pub id: String,
    /// The project's slug.
    pub project: String,
    /// `sh1`, `sh2`, …
    pub name: String,
    /// What's running in it now (the shell itself, or vim, …).
    pub command: String,
}

/// Every open shell in `session`, in window order; none without tmux.
pub fn list(tmux: &Tmux, session: &str) -> Vec<Shell> {
    let format = format!("#{{window_id}}\t#{{{TAG}}}\t#{{window_name}}\t#{{pane_current_command}}");
    tmux.list_windows_as(session, &format)
        .map(|lines| parse(&lines.join("\n")))
        .unwrap_or_default()
}

/// Parses `id \t project \t window name \t command` lines; windows
/// without a project (Claude sessions, the dashboard) aren't shells.
fn parse(out: &str) -> Vec<Shell> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let (id, project, window, command) = (f.next()?, f.next()?, f.next()?, f.next()?);
            if project.is_empty() {
                return None;
            }
            let name = window
                .strip_prefix(&format!("{project}:"))
                .unwrap_or(window);
            Some(Shell {
                id: id.to_string(),
                project: project.to_string(),
                name: name.to_string(),
                command: command.to_string(),
            })
        })
        .collect()
}

/// `sh1`, `sh2`, …: the lowest number none of the project's shells has.
pub fn next_name(shells: &[Shell], project: &str) -> String {
    (1..)
        .map(|n| format!("sh{n}"))
        .find(|n| !shells.iter().any(|s| s.project == project && &s.name == n))
        .expect("unbounded range")
}

/// Opens a shell window running `argv` in the project's folder, without
/// switching to it.
pub fn open(tmux: &Tmux, session: &str, project: &Project, argv: &[String]) -> Result<Shell> {
    let slug = project.slug();
    let name = next_name(&list(tmux, session), &slug);
    let id = tmux.new_window(session, &format!("{slug}:{name}"), &project.path, &[], argv)?;
    tmux.set_window_option(&id, TAG, &slug)?;
    let command = argv.first().map(|c| c.rsplit('/').next().unwrap_or(c));
    Ok(Shell {
        id,
        project: slug,
        name,
        command: command.unwrap_or_default().to_string(),
    })
}

/// Renames the shell in `window`: refused when another of its project's
/// shells has that name. Returns its old name.
pub fn rename(tmux: &Tmux, session: &str, window: &str, name: &str) -> Result<String> {
    let shells = list(tmux, session);
    let Some(sh) = shells.iter().find(|s| s.id == window) else {
        bail!("that shell has closed");
    };
    if shells
        .iter()
        .any(|s| s.id != sh.id && s.project == sh.project && s.name == name)
    {
        bail!("another shell in {} is already called {name}", sh.project);
    }
    tmux.rename_window(window, &format!("{}:{name}", sh.project))?;
    Ok(sh.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(project: &str, name: &str) -> Shell {
        Shell {
            id: format!("@{name}"),
            project: project.into(),
            name: name.into(),
            command: "zsh".into(),
        }
    }

    #[test]
    fn only_tagged_windows_are_shells() {
        let out = "@0\t\tdashboard\tgrove\n\
                   @1\t\tgrove:s1\tclaude\n\
                   @2\tgrove\tgrove:sh1\tzsh\n\
                   @3\tblog\tblog:sh2\tvim\n";
        assert_eq!(
            parse(out),
            vec![
                Shell {
                    id: "@2".into(),
                    project: "grove".into(),
                    name: "sh1".into(),
                    command: "zsh".into(),
                },
                Shell {
                    id: "@3".into(),
                    project: "blog".into(),
                    name: "sh2".into(),
                    command: "vim".into(),
                },
            ]
        );
    }

    #[test]
    fn a_renamed_window_keeps_its_whole_name() {
        let out = "@4\tgrove\tbuild\tzsh\n";
        assert_eq!(parse(out)[0].name, "build");
    }

    #[test]
    fn names_fill_the_lowest_gap_per_project() {
        let shells = [shell("a", "sh1"), shell("a", "sh3"), shell("b", "sh2")];
        assert_eq!(next_name(&shells, "a"), "sh2");
        assert_eq!(next_name(&shells, "b"), "sh1");
        assert_eq!(next_name(&[], "c"), "sh1");
    }

    #[test]
    fn open_tags_a_window_in_the_project_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let tmux = Tmux::new(format!("grove-test-{}-shells", std::process::id()));
        let sleeper = vec!["sleep".to_string(), "30".to_string()];
        tmux.new_session_detached("grove", "dash", &root, &sleeper)
            .unwrap();
        let project = Project {
            name: "My App".into(),
            path: root.clone(),
            color: None,
            summary: None,
        };
        let first = open(&tmux, "grove", &project, &sleeper).unwrap();
        let second = open(&tmux, "grove", &project, &sleeper).unwrap();
        let shells = list(&tmux, "grove");
        let cwd = tmux
            .cmd()
            .args(["display", "-p", "-t", &first.id, "#{pane_current_path}"])
            .output()
            .unwrap();
        tmux.kill_server();

        assert_eq!(
            String::from_utf8_lossy(&cwd.stdout).trim(),
            root.to_string_lossy()
        );

        assert_eq!(shells, vec![first.clone(), second.clone()]);
        assert_eq!((first.name.as_str(), second.name.as_str()), ("sh1", "sh2"));
        assert_eq!(first.project, "my-app");
        assert_eq!(first.command, "sleep");
    }

    #[test]
    fn no_tmux_means_no_shells() {
        let tmux = Tmux::new(format!("grove-test-{}-noshells", std::process::id()));
        assert!(list(&tmux, "grove").is_empty());
    }
}
