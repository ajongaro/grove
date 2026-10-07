//! The `grove status --json` document: what a finca reports to the laptop.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::State;
use crate::store::Store;

pub const STATUS_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub version: u32,
    pub host: String,
    pub generated_at: DateTime<Utc>,
    pub needs_you: usize,
    /// Sessions that aren't ended.
    pub live: usize,
    pub projects: Vec<ProjectStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectStatus {
    pub name: String,
    pub slug: String,
    pub sessions: Vec<SessionStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionStatus {
    pub name: String,
    pub state: State,
    #[serde(default)]
    pub message: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, PartialEq)]
pub enum ParseError {
    Invalid(String),
    WrongVersion(u32),
}

/// Snapshot of the local store. Ended sessions are left out.
pub fn build(store: &Store, host: &str) -> Status {
    let sessions = store.list_sessions(None);
    let projects: Vec<ProjectStatus> = store
        .list_projects()
        .into_iter()
        .filter_map(Result::ok)
        .map(|p| {
            let slug = p.slug();
            ProjectStatus {
                sessions: sessions
                    .iter()
                    .filter(|s| s.project == slug && s.state != State::Ended)
                    .map(|s| SessionStatus {
                        name: s.name.clone(),
                        state: s.state,
                        message: s.message.clone(),
                        updated_at: s.updated_at,
                    })
                    .collect(),
                name: p.name,
                slug,
            }
        })
        .collect();
    let all = projects.iter().flat_map(|p| &p.sessions);
    Status {
        version: STATUS_VERSION,
        host: host.to_string(),
        generated_at: Utc::now(),
        needs_you: all.clone().filter(|s| s.state == State::NeedsYou).count(),
        live: all.count(),
        projects,
    }
}

pub fn parse(json: &str) -> Result<Status, ParseError> {
    // Check the version before the full shape, so a future format with
    // different fields reports "needs update" rather than "invalid".
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| ParseError::Invalid(e.to_string()))?;
    match v.get("version").and_then(serde_json::Value::as_u64) {
        Some(n) if n == STATUS_VERSION as u64 => {}
        Some(n) => return Err(ParseError::WrongVersion(n as u32)),
        None => return Err(ParseError::Invalid("missing version".into())),
    }
    serde_json::from_value(v).map_err(|e| ParseError::Invalid(e.to_string()))
}

/// This machine's short host name, for `Status::host`.
pub fn hostname() -> String {
    std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use crate::model::{Project, Session};

    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::new(Paths::new(d.path().join("home")));
        for name in ["alpha", "beta"] {
            s.add_project(&Project {
                name: name.into(),
                path: "/x".into(),
                color: None,
                summary: None,
            })
            .unwrap();
        }
        let mk = |id: &str, project: &str, state: State, msg: Option<&str>| Session {
            id: id.into(),
            project: project.into(),
            name: format!("s-{id}"),
            host: "local".into(),
            tmux_window: Some("@1".into()),
            state,
            message: msg.map(str::to_string),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        s.save_session(&mk(
            "1",
            "alpha",
            State::NeedsYou,
            Some("Permission needed: Bash"),
        ))
        .unwrap();
        s.save_session(&mk("2", "alpha", State::Working, None))
            .unwrap();
        s.save_session(&mk("3", "alpha", State::Ended, None))
            .unwrap();
        (d, s)
    }

    #[test]
    fn build_counts_and_omits_ended() {
        let (_d, s) = store();
        let st = build(&s, "therver");
        assert_eq!(st.version, STATUS_VERSION);
        assert_eq!(st.host, "therver");
        assert_eq!(st.needs_you, 1);
        assert_eq!(st.live, 2);
        let alpha = st.projects.iter().find(|p| p.slug == "alpha").unwrap();
        assert_eq!(alpha.sessions.len(), 2);
        assert!(alpha.sessions.iter().all(|x| x.state != State::Ended));
        let beta = st.projects.iter().find(|p| p.slug == "beta").unwrap();
        assert!(beta.sessions.is_empty());
    }

    #[test]
    fn empty_store_is_a_valid_status() {
        let d = tempfile::tempdir().unwrap();
        let st = build(&Store::new(Paths::new(d.path().join("nope"))), "h");
        assert_eq!((st.live, st.needs_you, st.projects.len()), (0, 0, 0));
    }

    #[test]
    fn round_trips_through_json() {
        let (_d, s) = store();
        let st = build(&s, "therver");
        assert_eq!(parse(&serde_json::to_string(&st).unwrap()).unwrap(), st);
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        let (_d, s) = store();
        let mut v = serde_json::to_value(build(&s, "h")).unwrap();
        v["future_field"] = serde_json::json!({"x": 1});
        v["projects"][0]["also_new"] = serde_json::json!(true);
        assert!(parse(&v.to_string()).is_ok());
    }

    #[test]
    fn other_versions_are_detected() {
        let (_d, s) = store();
        let mut v = serde_json::to_value(build(&s, "h")).unwrap();
        v["version"] = serde_json::json!(2);
        assert_eq!(parse(&v.to_string()), Err(ParseError::WrongVersion(2)));
    }

    #[test]
    fn garbage_is_invalid() {
        assert!(matches!(
            parse("error: unrecognized subcommand 'status'"),
            Err(ParseError::Invalid(_))
        ));
        assert!(matches!(parse("{}"), Err(ParseError::Invalid(_))));
    }
}
