//! Account boxes hidden from the dashboard (`H`), by account name, so a
//! day can be all work and an evening all play. Kept in Grove's home
//! (`hidden.json`) so a restart doesn't bring them back.

use std::collections::BTreeSet;

use crate::config::Paths;

fn file(paths: &Paths) -> std::path::PathBuf {
    paths.home.join("hidden.json")
}

/// The hidden accounts. A missing or unreadable file hides nothing.
pub fn load(paths: &Paths) -> BTreeSet<String> {
    std::fs::read_to_string(file(paths))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(paths: &Paths, accounts: &BTreeSet<String>) -> anyhow::Result<()> {
    let text = serde_json::to_string_pretty(accounts)?;
    crate::store::write_atomic(&file(paths), text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads_and_a_bad_file_hides_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().to_path_buf());
        assert!(load(&paths).is_empty());
        save(&paths, &BTreeSet::from(["work".to_string()])).unwrap();
        assert_eq!(load(&paths), BTreeSet::from(["work".to_string()]));
        std::fs::write(file(&paths), "nope").unwrap();
        assert!(load(&paths).is_empty());
    }
}
