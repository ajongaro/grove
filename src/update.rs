//! Whether a newer Grove is out: GitHub's latest release, asked at most
//! once a day (the answer is kept in `run/update.json`). `update_check =
//! false` in config.toml turns it off.

use std::cmp::Ordering;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::Paths;

/// The newest full release (not a draft or prerelease): docs.github.com,
/// "Get the latest release". Unauthenticated, so 60 requests an hour per IP;
/// a day between checks keeps far below that.
const LATEST: &str = "https://api.github.com/repos/ajongaro/grove/releases/latest";
/// How long one answer is trusted.
const EVERY: chrono::Duration = chrono::Duration::hours(24);

pub fn this_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Dotted versions compared number by number (`0.1.10` > `0.1.9`); a
/// leading `v` is ignored and a missing part counts as 0.
pub fn cmp(a: &str, b: &str) -> Ordering {
    let parts = |v: &str| -> Vec<u64> {
        v.trim()
            .trim_start_matches('v')
            .split(['.', '-', '+'])
            .take(3)
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parts(a), parts(b));
    (0..3)
        .map(|i| a.get(i).unwrap_or(&0).cmp(b.get(i).unwrap_or(&0)))
        .find(|o| o.is_ne())
        .unwrap_or(Ordering::Equal)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Cache {
    checked_at: DateTime<Utc>,
    /// The latest release's version, without the `v`; empty if the check
    /// failed (so an offline machine doesn't ask on every start either).
    latest: String,
}

fn cache_file(paths: &Paths) -> std::path::PathBuf {
    paths.run.join("update.json")
}

fn read_cache(file: &Path) -> Option<Cache> {
    serde_json::from_slice(&std::fs::read(file).ok()?).ok()
}

/// The newer version that's out, if any: from the cache while it's under a
/// day old, otherwise from `fetch` (which then refreshes the cache).
pub fn check(
    paths: &Paths,
    now: DateTime<Utc>,
    current: &str,
    fetch: &dyn Fn() -> Result<String>,
) -> Option<String> {
    let file = cache_file(paths);
    let latest = match read_cache(&file).filter(|c| now - c.checked_at < EVERY) {
        Some(c) => c.latest,
        None => {
            let latest = fetch().unwrap_or_default();
            let cache = Cache {
                checked_at: now,
                latest: latest.clone(),
            };
            if let Ok(bytes) = serde_json::to_vec(&cache) {
                let _ = crate::store::write_atomic(&file, &bytes);
            }
            latest
        }
    };
    (!latest.is_empty() && cmp(&latest, current) == Ordering::Greater).then_some(latest)
}

/// Asks GitHub for the latest release's version (curl, as the ticker does).
pub fn fetch_latest() -> Result<String> {
    let out = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "10",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "X-GitHub-Api-Version: 2026-03-10",
            LATEST,
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .context("running curl")?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    parse_latest(&out.stdout)
}

fn parse_latest(body: &[u8]) -> Result<String> {
    let v: serde_json::Value = serde_json::from_slice(body).context("GitHub's answer")?;
    let tag = v["tag_name"].as_str().context("no tag_name")?;
    Ok(tag.trim_start_matches('v').to_string())
}

/// How to upgrade, from where this grove was installed.
pub fn how_to_upgrade(exe: &Path) -> &'static str {
    let s = exe.to_string_lossy();
    if s.contains("/Cellar/grove/") || s.contains("/opt/grove/") {
        "brew upgrade grove"
    } else if s.contains("/.cargo/bin/") {
        "cargo install again"
    } else {
        "re-run install.sh"
    }
}

/// Checks now and then every hour (the cache decides when GitHub is asked),
/// sending the newer version once it's known.
pub fn start(paths: Paths) -> std::sync::mpsc::Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        loop {
            if let Some(v) = check(&paths, Utc::now(), this_version(), &fetch_latest)
                && tx.send(v).is_err()
            {
                return;
            }
            std::thread::sleep(Duration::from_secs(3600));
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn versions_compare_by_number() {
        assert_eq!(cmp("0.1.10", "0.1.9"), Ordering::Greater);
        assert_eq!(cmp("v1.0.0", "0.9.99"), Ordering::Greater);
        assert_eq!(cmp("0.1.42", "v0.1.42"), Ordering::Equal);
        assert_eq!(cmp("0.2", "0.2.0"), Ordering::Equal);
        assert_eq!(cmp("0.1.40", "0.1.42"), Ordering::Less);
    }

    #[test]
    fn reads_the_tag_from_githubs_answer() {
        let body = br#"{"tag_name":"v0.1.43","html_url":"https://github.com/x","draft":false}"#;
        assert_eq!(parse_latest(body).unwrap(), "0.1.43");
        assert!(parse_latest(br#"{"message":"Not Found"}"#).is_err());
    }

    #[test]
    fn asks_at_most_daily_and_only_reports_newer() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path().join("home"));
        let asked = Cell::new(0);
        let fetch = || {
            asked.set(asked.get() + 1);
            Ok("0.1.43".to_string())
        };
        let t = Utc::now();
        assert_eq!(
            check(&paths, t, "0.1.42", &fetch).as_deref(),
            Some("0.1.43")
        );
        // Within the day the cache answers, for this version or a newer one.
        assert_eq!(
            check(&paths, t + chrono::Duration::hours(23), "0.1.42", &fetch).as_deref(),
            Some("0.1.43")
        );
        assert_eq!(check(&paths, t, "0.1.43", &fetch), None);
        assert_eq!(asked.get(), 1);
        assert!(check(&paths, t + chrono::Duration::hours(25), "0.1.42", &fetch).is_some());
        assert_eq!(asked.get(), 2);
    }

    #[test]
    fn a_failed_check_waits_a_day_too() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path().join("home"));
        let asked = Cell::new(0);
        let fetch = || -> Result<String> {
            asked.set(asked.get() + 1);
            bail!("offline")
        };
        let t = Utc::now();
        assert_eq!(check(&paths, t, "0.1.42", &fetch), None);
        assert_eq!(
            check(&paths, t + chrono::Duration::hours(1), "0.1.42", &fetch),
            None
        );
        assert_eq!(asked.get(), 1);
    }

    #[test]
    fn upgrade_hint_follows_the_install() {
        assert_eq!(
            how_to_upgrade(Path::new("/opt/homebrew/opt/grove/bin/grove")),
            "brew upgrade grove"
        );
        assert_eq!(
            how_to_upgrade(Path::new("/Users/a/.cargo/bin/grove")),
            "cargo install again"
        );
        assert_eq!(
            how_to_upgrade(Path::new("/Users/a/.local/bin/grove")),
            "re-run install.sh"
        );
    }
}
