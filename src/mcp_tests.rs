use super::*;
use crate::config::Paths;
use crate::model::Project;

fn mcp(default: Option<&str>) -> (tempfile::TempDir, GroveMcp) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(Paths::new(dir.path().join("home")));
    (
        dir,
        GroveMcp {
            store,
            default_project: default.map(str::to_string),
        },
    )
}

fn add(m: &GroveMcp, name: &str) {
    m.store
        .add_project(&Project {
            name: name.into(),
            path: format!("/code/{name}").into(),
            color: None,
            summary: None,
        })
        .unwrap();
}

#[test]
fn list_projects_empty_home() {
    let (_d, m) = mcp(None);
    let out = m.do_list_projects().unwrap();
    assert!(out.contains("No projects"), "{out}");
}

#[test]
fn list_projects_shows_name_path_summary() {
    let (_d, m) = mcp(None);
    add(&m, "alpha");
    m.do_update_project(Some("alpha".into()), Some("Alpha app.".into()), None)
        .unwrap();
    let out = m.do_list_projects().unwrap();
    assert!(
        out.contains("alpha") && out.contains("/code/alpha") && out.contains("Alpha app."),
        "{out}"
    );
}

#[test]
fn get_project_defaults_to_session_project() {
    let (_d, m) = mcp(Some("beta"));
    add(&m, "alpha");
    add(&m, "beta");
    m.store
        .write_wiki("beta", "Commands", "cargo test")
        .unwrap();
    let out = m.do_get_project(None).unwrap();
    assert!(out.contains("beta") && !out.contains("alpha"), "{out}");
    assert!(out.contains("Commands"), "{out}");
}

#[test]
fn no_project_and_no_default_is_an_error() {
    let (_d, m) = mcp(None);
    add(&m, "alpha");
    let err = m.do_get_project(None).unwrap_err();
    assert!(err.contains("project") && err.contains("alpha"), "{err}");
}

#[test]
fn unknown_project_lists_valid_names() {
    let (_d, m) = mcp(None);
    add(&m, "alpha");
    let err = m.do_read_wiki(Some("nope".into()), None).unwrap_err();
    assert!(err.contains("alpha"), "{err}");
}

#[test]
fn update_project_persists_summary_and_color() {
    let (_d, m) = mcp(Some("alpha"));
    add(&m, "alpha");
    m.do_update_project(None, Some("New summary".into()), Some("#AABBCC".into()))
        .unwrap();
    let p = m.store.get_project("alpha").unwrap();
    assert_eq!(p.summary.as_deref(), Some("New summary"));
    assert_eq!(p.color.as_deref(), Some("#AABBCC"));
}

#[test]
fn update_project_rejects_bad_color_and_changes_nothing() {
    let (_d, m) = mcp(Some("alpha"));
    add(&m, "alpha");
    for bad in ["red", "#12345", "#GGGGGG", "123456"] {
        let err = m
            .do_update_project(None, Some("x".into()), Some(bad.into()))
            .unwrap_err();
        assert!(err.contains("#RRGGBB"), "{bad}: {err}");
    }
    assert_eq!(m.store.get_project("alpha").unwrap().summary, None);
}

#[test]
fn update_project_with_nothing_to_change_errors() {
    let (_d, m) = mcp(Some("alpha"));
    add(&m, "alpha");
    assert!(m.do_update_project(None, None, None).is_err());
}

#[test]
fn write_then_read_wiki() {
    let (_d, m) = mcp(Some("alpha"));
    add(&m, "alpha");
    let out = m
        .do_write_wiki_section(None, "Gotchas".into(), "Watch out.".into())
        .unwrap();
    assert!(out.contains("gotchas.md"), "{out}");
    assert_eq!(
        m.do_read_wiki(None, Some("gotchas".into())).unwrap(),
        "# Gotchas\n\nWatch out.\n"
    );
    assert!(m.do_read_wiki(None, None).unwrap().contains("Watch out."));
}

#[test]
fn read_empty_wiki_says_so() {
    let (_d, m) = mcp(Some("alpha"));
    add(&m, "alpha");
    assert!(
        m.do_read_wiki(None, None)
            .unwrap()
            .contains("no wiki sections")
    );
}

#[test]
fn write_wiki_to_unknown_project_errors_without_writing() {
    let (_d, m) = mcp(None);
    assert!(
        m.do_write_wiki_section(Some("ghost".into()), "T".into(), "c".into())
            .is_err()
    );
    assert!(!m.store.paths.wiki.join("ghost").exists());
}
