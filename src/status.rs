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
    /// The session's id, for pinning it from another machine. Older Groves
    /// don't send it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub state: State,
    #[serde(default)]
    pub message: Option<String>,
    pub updated_at: DateTime<Utc>,
    /// Its tmux window on the finca, for jumping straight in. Older Groves
    /// don't send it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum ParseError {
    Invalid(String),
    WrongVersion(u32),
}

/// Snapshot of the local store. Only sessions whose tmux window is in
/// `live_windows` count: session files can say needs_you long after a
/// reboot or a dead tmux server.
pub fn build(store: &Store, host: &str, live_windows: &[String]) -> Status {
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
                    .filter(|s| {
                        s.tmux_window
                            .as_ref()
                            .is_some_and(|w| live_windows.contains(w))
                    })
                    .map(|s| SessionStatus {
                        id: Some(s.id.clone()),
                        name: s.name.clone(),
                        state: s.state,
                        message: s.message.clone(),
                        updated_at: s.updated_at,
                        window: s.tmux_window.clone(),
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

    /// Every test session lives in window @1.
    fn live() -> Vec<String> {
        vec!["@1".to_string()]
    }

    #[test]
    fn sessions_whose_window_is_gone_are_not_reported() {
        // After a reboot or tmux-server death the session files still say
        // needs_you, but nothing is running.
        let (_d, s) = store();
        let st = build(&s, "pi", &[]);
        assert_eq!((st.live, st.needs_you), (0, 0));
        assert!(st.projects.iter().all(|p| p.sessions.is_empty()));
        let st = build(&s, "pi", &["@9".to_string()]);
        assert_eq!(st.live, 0);
    }

    #[test]
    fn build_counts_and_omits_ended() {
        let (_d, s) = store();
        let st = build(&s, "homebox", &live());
        assert_eq!(st.version, STATUS_VERSION);
        assert_eq!(st.host, "homebox");
        assert_eq!(st.needs_you, 1);
        assert_eq!(st.live, 2);
        let alpha = st.projects.iter().find(|p| p.slug == "alpha").unwrap();
        assert_eq!(alpha.sessions.len(), 2);
        assert!(alpha.sessions.iter().all(|x| x.state != State::Ended));
        let beta = st.projects.iter().find(|p| p.slug == "beta").unwrap();
        assert!(beta.sessions.is_empty());
    }

    #[test]
    fn status_sessions_carry_their_id_and_old_status_still_parses() {
        let (_d, s) = store();
        let v = serde_json::to_value(build(&s, "h", &live())).unwrap();
        let alpha = v["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["slug"] == "alpha")
            .unwrap();
        let ids: Vec<&str> = alpha["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"1") && ids.contains(&"2"));
        let old: SessionStatus = serde_json::from_str(
            r#"{"name":"s1","state":"working","updated_at":"2026-10-07T10:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(old.id, None);
    }

    #[test]
    fn empty_store_is_a_valid_status() {
        let d = tempfile::tempdir().unwrap();
        let st = build(&Store::new(Paths::new(d.path().join("nope"))), "h", &[]);
        assert_eq!((st.live, st.needs_you, st.projects.len()), (0, 0, 0));
    }

    #[test]
    fn round_trips_through_json() {
        let (_d, s) = store();
        let st = build(&s, "homebox", &live());
        assert_eq!(parse(&serde_json::to_string(&st).unwrap()).unwrap(), st);
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        let (_d, s) = store();
        let mut v = serde_json::to_value(build(&s, "h", &live())).unwrap();
        v["future_field"] = serde_json::json!({"x": 1});
        v["projects"][0]["also_new"] = serde_json::json!(true);
        assert!(parse(&v.to_string()).is_ok());
    }

    #[test]
    fn other_versions_are_detected() {
        let (_d, s) = store();
        let mut v = serde_json::to_value(build(&s, "h", &live())).unwrap();
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
