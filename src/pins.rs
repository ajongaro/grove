//! Pinned sessions: the sessions you're working in, from any project,
//! account or finca, kept at the top of the dashboard. Stored in Grove's
//! own home (`pins.json`), not a Claude account's, so pins span accounts.

use serde::{Deserialize, Serialize};

use crate::config::Paths;

const PINS_VERSION: u32 = 1;

/// A pinned session: on this machine (`finca` None) or on a finca, by slug.
/// Or a pinned shell (`shell` set): `id` is then its tmux window here, a
/// project's shell or a finca's (those run here too, over ssh).
#[derive(Debug, Clone, Eq, Serialize, Deserialize)]
pub struct Pin {
    pub finca: Option<String>,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<ShellPin>,
}

/// What a pinned shell needs to be reopened after a reboot.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ShellPin {
    /// The project's slug, for a project's shell; None for a finca's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// The name it had, given back to the reopened shell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// The same pin: a shell's name can change without making it another.
impl PartialEq for Pin {
    fn eq(&self, other: &Self) -> bool {
        self.finca == other.finca
            && self.id == other.id
            && self.shell.as_ref().map(|s| &s.project) == other.shell.as_ref().map(|s| &s.project)
    }
}

impl Pin {
    pub fn session(finca: Option<String>, id: impl Into<String>) -> Self {
        Pin {
            finca,
            id: id.into(),
            shell: None,
        }
    }

    pub fn is_shell(&self) -> bool {
        self.shell.is_some()
    }
}

#[derive(Serialize, Deserialize)]
struct PinsFile {
    version: u32,
    pins: Vec<Pin>,
}

fn file(paths: &Paths) -> std::path::PathBuf {
    paths.home.join("pins.json")
}

/// The pins in the order they were pinned. A missing, unreadable or
/// unknown-version file is no pins.
pub fn load(paths: &Paths) -> Vec<Pin> {
    std::fs::read_to_string(file(paths))
        .ok()
        .and_then(|t| serde_json::from_str::<PinsFile>(&t).ok())
        .filter(|f| f.version == PINS_VERSION)
        .map(|f| f.pins)
        .unwrap_or_default()
}

pub fn save(paths: &Paths, pins: &[Pin]) -> anyhow::Result<()> {
    let text = serde_json::to_string_pretty(&PinsFile {
        version: PINS_VERSION,
        pins: pins.to_vec(),
    })?;
    crate::store::write_atomic(&file(paths), text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;

    #[test]
    fn pins_round_trip_and_bad_files_are_empty() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path().to_path_buf());
        assert!(load(&paths).is_empty());
        let pins = vec![
            Pin::session(None, "a"),
            Pin::session(Some("pi".into()), "b"),
            Pin {
                finca: None,
                id: "@4".into(),
                shell: Some(ShellPin {
                    project: Some("alpha".into()),
                    name: Some("logs".into()),
                }),
            },
        ];
        save(&paths, &pins).unwrap();
        assert_eq!(load(&paths), pins);
        assert_eq!(load(&paths)[2].shell, pins[2].shell);
        // Pins saved before shells could be pinned still load.
        std::fs::write(
            paths.home.join("pins.json"),
            r#"{"version":1,"pins":[{"finca":null,"id":"a"}]}"#,
        )
        .unwrap();
        assert_eq!(load(&paths), vec![Pin::session(None, "a")]);
        std::fs::write(paths.home.join("pins.json"), "nope").unwrap();
        assert!(load(&paths).is_empty());
        std::fs::write(paths.home.join("pins.json"), r#"{"version":9,"pins":[]}"#).unwrap();
        assert!(load(&paths).is_empty());
    }
}
