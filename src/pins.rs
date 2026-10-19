//! Pinned sessions: the sessions you're working in, from any project,
//! account or finca, kept at the top of the dashboard. Stored in Grove's
//! own home (`pins.json`), not a Claude account's, so pins span accounts.

use serde::{Deserialize, Serialize};

use crate::config::Paths;

const PINS_VERSION: u32 = 1;

/// A pinned session: on this machine (`finca` None) or on a finca, by slug.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pin {
    pub finca: Option<String>,
    pub id: String,
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
            Pin {
                finca: None,
                id: "a".into(),
            },
            Pin {
                finca: Some("pi".into()),
                id: "b".into(),
            },
        ];
        save(&paths, &pins).unwrap();
        assert_eq!(load(&paths), pins);
        std::fs::write(paths.home.join("pins.json"), "nope").unwrap();
        assert!(load(&paths).is_empty());
        std::fs::write(paths.home.join("pins.json"), r#"{"version":9,"pins":[]}"#).unwrap();
        assert!(load(&paths).is_empty());
    }
}
