//! What `grove statusline` learns about each session: its last running
//! totals, and what it added on each local day, per account. Each Grove
//! session has one file, written only by its own status line, so nothing
//! races; the daily harvest is the sum of those files.

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::config::Paths;
use crate::store::write_atomic;

/// Daily tallies older than this are dropped.
const KEEP_DAYS: i64 = 30;

/// The part of Claude Code's status line JSON Grove reads
/// (https://code.claude.com/docs/en/statusline). Any field can be missing.
#[derive(Debug, Default, Deserialize)]
pub struct StatusInput {
    pub session_id: Option<String>,
    pub model: Option<Model>,
    pub workspace: Option<Workspace>,
    pub cost: Option<Cost>,
    pub context_window: Option<ContextWindow>,
    pub rate_limits: Option<RateLimits>,
    pub agent: Option<Agent>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Model {
    pub id: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Workspace {
    pub current_dir: Option<String>,
    pub project_dir: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Cost {
    #[serde(default, deserialize_with = "number")]
    pub total_cost_usd: Option<f64>,
    #[serde(default, deserialize_with = "count")]
    pub total_duration_ms: Option<u64>,
    #[serde(default, deserialize_with = "count")]
    pub total_lines_added: Option<u64>,
    #[serde(default, deserialize_with = "count")]
    pub total_lines_removed: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ContextWindow {
    #[serde(default, deserialize_with = "number")]
    pub used_percentage: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RateLimits {
    pub five_hour: Option<Window>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Window {
    #[serde(default, deserialize_with = "number")]
    pub used_percentage: Option<f64>,
}

/// Numbers are read leniently: a field of an unexpected type is just
/// missing, rather than failing the whole payload and blanking the line.
fn number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    Ok(serde_json::Value::deserialize(d)?.as_f64())
}

/// A count sent as `10` or `10.0`.
fn count<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_u64()
        .or_else(|| v.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64)))
}

#[derive(Debug, Default, Deserialize)]
pub struct Agent {
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Totals {
    pub cost: f64,
    pub added: u64,
    pub removed: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionStats {
    pub account: String,
    /// The conversation's running totals as last seen.
    pub last: Totals,
    pub context_pct: Option<f64>,
    pub five_hour_pct: Option<f64>,
    pub updated_at: DateTime<Utc>,
    /// Local `YYYY-MM-DD` → what the session added that day.
    #[serde(default)]
    pub days: BTreeMap<String, Totals>,
}

/// One account's share of a day.
// The dashboard header shows these in the next release (the swagger design).
#[allow(dead_code)]
#[derive(Debug, PartialEq)]
pub struct Harvest {
    pub account: String,
    pub totals: Totals,
    /// Sessions that added anything that day.
    pub sessions: usize,
}

/// The next stats after seeing `input`. Totals only grow within a
/// conversation, so a drop means /clear or /resume started a new one: its
/// whole value is new.
pub fn accrue(
    prev: Option<&SessionStats>,
    account: &str,
    input: &StatusInput,
    today: &str,
    now: DateTime<Utc>,
) -> SessionStats {
    let old = prev.map(|p| p.last.clone()).unwrap_or_default();
    let cost = input.cost.as_ref();
    let cur = Totals {
        cost: cost.and_then(|c| c.total_cost_usd).unwrap_or(old.cost),
        added: cost.and_then(|c| c.total_lines_added).unwrap_or(old.added),
        removed: cost
            .and_then(|c| c.total_lines_removed)
            .unwrap_or(old.removed),
    };
    let grew_f = |cur: f64, old: f64| if cur >= old { cur - old } else { cur };
    let grew = |cur: u64, old: u64| if cur >= old { cur - old } else { cur };
    let delta = Totals {
        cost: grew_f(cur.cost, old.cost),
        added: grew(cur.added, old.added),
        removed: grew(cur.removed, old.removed),
    };
    let mut days = prev.map(|p| p.days.clone()).unwrap_or_default();
    if delta.cost > 0.0 || delta.added > 0 || delta.removed > 0 {
        let day = days.entry(today.to_string()).or_default();
        day.cost += delta.cost;
        day.added += delta.added;
        day.removed += delta.removed;
    }
    if let Ok(t) = NaiveDate::parse_from_str(today, "%Y-%m-%d") {
        days.retain(|d, _| {
            NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok_and(|d| (t - d).num_days() <= KEEP_DAYS)
        });
    }
    SessionStats {
        account: account.to_string(),
        last: cur,
        context_pct: input
            .context_window
            .as_ref()
            .and_then(|c| c.used_percentage)
            .or(prev.and_then(|p| p.context_pct)),
        five_hour_pct: input
            .rate_limits
            .as_ref()
            .and_then(|r| r.five_hour.as_ref())
            .and_then(|w| w.used_percentage)
            .or(prev.and_then(|p| p.five_hour_pct)),
        updated_at: now,
        days,
    }
}

fn file(paths: &Paths, key: &str) -> std::path::PathBuf {
    paths.stats.join(format!("{key}.json"))
}

/// Folds `input` into session `key`'s stats file and returns the result.
pub fn record(
    paths: &Paths,
    key: &str,
    account: &str,
    input: &StatusInput,
) -> Result<SessionStats> {
    let path = file(paths, key);
    let prev: Option<SessionStats> = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let next = accrue(prev.as_ref(), account, input, &today, Utc::now());
    write_atomic(&path, &serde_json::to_vec(&next)?)?;
    Ok(next)
}

/// Every session's stats by key; unreadable files are skipped.
pub fn load_all(paths: &Paths) -> HashMap<String, SessionStats> {
    let Ok(rd) = std::fs::read_dir(&paths.stats) else {
        return HashMap::new();
    };
    rd.filter_map(|e| {
        let path = e.ok()?.path();
        if path.extension()? != "json" {
            return None;
        }
        let key = path.file_stem()?.to_str()?.to_string();
        let stats = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
        Some((key, stats))
    })
    .collect()
}

/// Removes stats nobody has updated for `KEEP_DAYS`: their days are past
/// every harvest. A deleted session's stats stay until then, so clearing
/// ended sessions doesn't take their spend out of the day's totals.
pub fn prune_old(paths: &Paths) {
    let cutoff = Utc::now() - chrono::Duration::days(KEEP_DAYS);
    for (key, s) in load_all(paths) {
        if s.updated_at < cutoff {
            let _ = std::fs::remove_file(file(paths, &key));
        }
    }
}

/// Each account's totals for `day`, by account name.
#[allow(dead_code)]
pub fn harvest(all: &HashMap<String, SessionStats>, day: &str) -> Vec<Harvest> {
    let mut by: BTreeMap<&str, Harvest> = BTreeMap::new();
    for s in all.values() {
        let Some(t) = s.days.get(day) else { continue };
        let h = by.entry(&s.account).or_insert_with(|| Harvest {
            account: s.account.clone(),
            totals: Totals::default(),
            sessions: 0,
        });
        h.totals.cost += t.cost;
        h.totals.added += t.added;
        h.totals.removed += t.removed;
        h.sessions += 1;
    }
    by.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input(cost: f64, added: u64, removed: u64) -> StatusInput {
        serde_json::from_value(json!({ "cost": {
            "total_cost_usd": cost,
            "total_lines_added": added,
            "total_lines_removed": removed,
        }}))
        .unwrap()
    }

    fn totals(cost: f64, added: u64, removed: u64) -> Totals {
        Totals {
            cost,
            added,
            removed,
        }
    }

    fn stats(account: &str, last: Totals, days: &[(&str, Totals)]) -> SessionStats {
        SessionStats {
            account: account.into(),
            last,
            context_pct: None,
            five_hour_pct: None,
            updated_at: Utc::now(),
            days: days
                .iter()
                .map(|(d, t)| (d.to_string(), t.clone()))
                .collect(),
        }
    }

    fn close(a: &Totals, b: &Totals) -> bool {
        (a.cost - b.cost).abs() < 1e-9 && a.added == b.added && a.removed == b.removed
    }

    const TODAY: &str = "2026-10-20";

    #[test]
    fn first_snapshot_accrues_everything() {
        let s = accrue(None, "work", &input(1.5, 10, 2), TODAY, Utc::now());
        assert_eq!(s.account, "work");
        assert_eq!(s.days[TODAY], totals(1.5, 10, 2));
        assert_eq!(s.last, totals(1.5, 10, 2));
    }

    #[test]
    fn increase_lands_on_todays_date() {
        let prev = stats(
            "work",
            totals(1.0, 5, 0),
            &[("2026-10-19", totals(1.0, 5, 0))],
        );
        let s = accrue(Some(&prev), "work", &input(1.4, 9, 1), TODAY, Utc::now());
        assert_eq!(s.days["2026-10-19"], totals(1.0, 5, 0));
        assert!(close(&s.days[TODAY], &totals(0.4, 4, 1)), "{:?}", s.days);
    }

    #[test]
    fn reset_counts_new_value() {
        let prev = stats("work", totals(3.0, 100, 0), &[]);
        let s = accrue(Some(&prev), "work", &input(0.2, 3, 0), TODAY, Utc::now());
        assert!(close(&s.days[TODAY], &totals(0.2, 3, 0)), "{:?}", s.days);
        assert_eq!(s.last, totals(0.2, 3, 0));
    }

    #[test]
    fn no_change_adds_no_day() {
        let prev = stats("work", totals(1.0, 5, 0), &[]);
        let s = accrue(Some(&prev), "work", &input(1.0, 5, 0), TODAY, Utc::now());
        assert!(!s.days.contains_key(TODAY));
    }

    #[test]
    fn missing_fields_change_nothing() {
        let prev = stats("work", totals(1.0, 5, 0), &[]);
        let empty: StatusInput = serde_json::from_str("{}").unwrap();
        let s = accrue(Some(&prev), "work", &empty, TODAY, Utc::now());
        assert_eq!(s.last, totals(1.0, 5, 0));
        assert!(s.days.is_empty());
    }

    #[test]
    fn old_days_pruned() {
        let prev = stats(
            "work",
            totals(1.0, 5, 0),
            &[
                ("2026-09-01", totals(1.0, 5, 0)),
                ("2026-10-01", totals(0.0, 1, 0)),
            ],
        );
        let s = accrue(Some(&prev), "work", &input(1.0, 5, 0), TODAY, Utc::now());
        assert!(!s.days.contains_key("2026-09-01"));
        assert!(s.days.contains_key("2026-10-01"));
    }

    #[test]
    fn harvest_splits_by_account() {
        let all: HashMap<String, SessionStats> = [
            (
                "a",
                stats("work", totals(1.0, 10, 0), &[(TODAY, totals(1.0, 10, 0))]),
            ),
            (
                "b",
                stats("work", totals(2.0, 0, 5), &[(TODAY, totals(2.0, 0, 5))]),
            ),
            (
                "c",
                stats("personal", totals(0.5, 1, 1), &[(TODAY, totals(0.5, 1, 1))]),
            ),
            (
                "d",
                stats(
                    "work",
                    totals(9.0, 9, 9),
                    &[("2026-10-19", totals(9.0, 9, 9))],
                ),
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        assert_eq!(
            harvest(&all, TODAY),
            vec![
                Harvest {
                    account: "personal".into(),
                    totals: totals(0.5, 1, 1),
                    sessions: 1
                },
                Harvest {
                    account: "work".into(),
                    totals: totals(3.0, 10, 5),
                    sessions: 2
                },
            ]
        );
    }

    #[test]
    fn record_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path().join("grove home"));
        record(&p, "abc", "work", &input(1.0, 1, 0)).unwrap();
        std::fs::write(p.stats.join("bad.json"), "nope").unwrap();
        let all = load_all(&p);
        assert_eq!(all["abc"].last.cost, 1.0);
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn whole_numbers_sent_as_floats_still_parse() {
        let v: StatusInput = serde_json::from_value(json!({
            "cost": { "total_cost_usd": 1, "total_duration_ms": 1000.5,
                      "total_lines_added": 10.0, "total_lines_removed": 2 }
        }))
        .unwrap();
        let c = v.cost.unwrap();
        assert_eq!(c.total_duration_ms, Some(1000));
        assert_eq!(c.total_lines_added, Some(10));
        assert_eq!(c.total_cost_usd, Some(1.0));
    }

    #[test]
    fn parses_real_status_json() {
        let v: StatusInput = serde_json::from_value(json!({
            "hook_event_name": "Status",
            "session_id": "abc123",
            "model": { "id": "claude-opus", "display_name": "Opus" },
            "workspace": { "current_dir": "/a/b", "project_dir": "/a" },
            "cost": {
                "total_cost_usd": 0.01234,
                "total_duration_ms": 45000,
                "total_api_duration_ms": 2300,
                "total_lines_added": 156,
                "total_lines_removed": 23
            },
            "context_window": { "used_percentage": 8, "context_window_size": 200000 },
            "rate_limits": { "five_hour": { "used_percentage": 23.5, "resets_at": 1738425600 } },
            "exceeds_200k_tokens": false
        }))
        .unwrap();
        assert_eq!(v.session_id.as_deref(), Some("abc123"));
        assert_eq!(v.model.unwrap().display_name.as_deref(), Some("Opus"));
        assert_eq!(v.cost.unwrap().total_lines_added, Some(156));
        assert_eq!(v.context_window.unwrap().used_percentage, Some(8.0));
        assert_eq!(
            v.rate_limits.unwrap().five_hour.unwrap().used_percentage,
            Some(23.5)
        );
        let _: StatusInput = serde_json::from_str("{}").unwrap();
        // Null where the docs say "may be null early on".
        let _: StatusInput =
            serde_json::from_str(r#"{"context_window":{"used_percentage":null}}"#).unwrap();
    }
}
