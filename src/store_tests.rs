use super::*;
use crate::model::State;
use chrono::Utc;

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::new(Paths::new(dir.path().join("grove home")));
    (dir, s)
}

fn proj(name: &str) -> Project {
    Project {
        name: name.into(),
        path: "/tmp/x".into(),
        color: None,
        summary: None,
    }
}

fn sess(id: &str, project: &str) -> Session {
    Session {
        id: id.into(),
        project: project.into(),
        name: "s1".into(),
        host: "local".into(),
        tmux_window: Some("@3".into()),
        state: State::Starting,
        message: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn atomic_write_creates_parent_and_leaves_no_tmp() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a/b/c.json");
    write_atomic(&f, b"hi").unwrap();
    write_atomic(&f, b"there").unwrap();
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "there");
    let names: Vec<_> = std::fs::read_dir(f.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("c.json")]);
}

#[test]
fn empty_home_lists_nothing() {
    let (_d, s) = store();
    assert!(s.list_projects().is_empty());
    assert!(s.list_sessions(None).is_empty());
    assert!(s.wiki_sections("nope").is_empty());
    assert!(s.get_session("x").is_none());
}

#[test]
fn project_roundtrip_by_name_or_slug() {
    let (_d, s) = store();
    let mut p = proj("My App");
    p.color = Some("#123456".into());
    s.add_project(&p).unwrap();
    assert_eq!(s.get_project("My App").unwrap(), p);
    assert_eq!(s.get_project("my-app").unwrap(), p);
    assert_eq!(s.list_projects().len(), 1);
    assert!(s.paths.projects.join("my-app.toml").is_file());
}

#[test]
fn adding_colliding_slug_errors_and_keeps_original() {
    let (_d, s) = store();
    s.add_project(&proj("my-app")).unwrap();
    let err = s
        .add_project(&Project {
            path: "/other".into(),
            ..proj("My App")
        })
        .unwrap_err();
    assert!(err.to_string().contains("my-app"), "{err}");
    assert_eq!(
        s.get_project("my-app").unwrap().path,
        PathBuf::from("/tmp/x")
    );
}

#[test]
fn empty_slug_is_rejected() {
    let (_d, s) = store();
    assert!(s.add_project(&proj("!!!")).is_err());
}

#[test]
fn unknown_project_error_lists_valid_names() {
    let (_d, s) = store();
    s.add_project(&proj("alpha")).unwrap();
    let err = s.get_project("beta").unwrap_err().to_string();
    assert!(err.contains("alpha"), "{err}");
}

#[test]
fn malformed_project_file_is_reported_not_fatal() {
    let (_d, s) = store();
    s.add_project(&proj("good")).unwrap();
    std::fs::write(s.paths.projects.join("bad.toml"), "name = [").unwrap();
    let all = s.list_projects();
    assert_eq!(all.len(), 2);
    assert_eq!(all.iter().filter(|r| r.is_ok()).count(), 1);
    let bad = all.iter().find_map(|r| r.as_ref().err()).unwrap();
    assert!(bad.0.ends_with("bad.toml"));
}

#[test]
fn save_project_updates_existing() {
    let (_d, s) = store();
    s.add_project(&proj("alpha")).unwrap();
    let mut p = s.get_project("alpha").unwrap();
    p.summary = Some("does things".into());
    s.save_project(&p).unwrap();
    assert_eq!(
        s.get_project("alpha").unwrap().summary.as_deref(),
        Some("does things")
    );
}

#[test]
fn sessions_save_get_filter_delete() {
    let (_d, s) = store();
    s.save_session(&sess("a", "p1")).unwrap();
    s.save_session(&sess("b", "p2")).unwrap();
    assert_eq!(s.get_session("a").unwrap().project, "p1");
    assert_eq!(s.list_sessions(None).len(), 2);
    let p2 = s.list_sessions(Some("p2"));
    assert_eq!(p2.len(), 1);
    assert_eq!(p2[0].id, "b");
    s.delete_session("a").unwrap();
    assert!(s.get_session("a").is_none());
    s.delete_session("a").unwrap(); // idempotent
}

#[test]
fn sessions_listed_oldest_first() {
    let (_d, s) = store();
    let mut a = sess("a", "p");
    let mut b = sess("b", "p");
    a.created_at = Utc::now();
    b.created_at = a.created_at - chrono::Duration::minutes(5);
    s.save_session(&a).unwrap();
    s.save_session(&b).unwrap();
    let ids: Vec<_> = s
        .list_sessions(Some("p"))
        .into_iter()
        .map(|x| x.id)
        .collect();
    assert_eq!(ids, vec!["b", "a"]);
}

#[test]
fn wiki_write_list_read() {
    let (_d, s) = store();
    let p = s
        .write_wiki("proj", "Common Tasks", "Do the thing.")
        .unwrap();
    assert!(p.ends_with("proj/common-tasks.md"));
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        "# Common Tasks\n\nDo the thing.\n"
    );
    s.write_wiki("proj", "Architecture", "Boxes.").unwrap();
    let titles: Vec<_> = s
        .wiki_sections("proj")
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    assert_eq!(titles, vec!["Architecture", "Common Tasks"]);
    assert_eq!(
        s.read_wiki("proj", Some("common tasks")).unwrap(),
        "# Common Tasks\n\nDo the thing.\n"
    );
    let all = s.read_wiki("proj", None).unwrap();
    assert!(all.find("# Architecture").unwrap() < all.find("# Common Tasks").unwrap());
}

#[test]
fn wiki_write_replaces_and_keeps_existing_heading() {
    let (_d, s) = store();
    s.write_wiki("proj", "Gotchas", "old").unwrap();
    s.write_wiki("proj", "Gotchas", "# Gotchas\n\nnew").unwrap();
    assert_eq!(
        s.read_wiki("proj", Some("Gotchas")).unwrap(),
        "# Gotchas\n\nnew\n"
    );
    assert_eq!(s.wiki_sections("proj").len(), 1);
}

#[test]
fn wiki_title_falls_back_to_filename_without_heading() {
    let (_d, s) = store();
    let dir = s.paths.wiki.join("proj");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.md"), "no heading here").unwrap();
    assert_eq!(s.wiki_sections("proj")[0].0, "notes");
}

#[test]
fn read_missing_wiki_section_errors_with_titles() {
    let (_d, s) = store();
    s.write_wiki("proj", "Architecture", "x").unwrap();
    let err = s.read_wiki("proj", Some("Nope")).unwrap_err().to_string();
    assert!(err.contains("Architecture"), "{err}");
}
