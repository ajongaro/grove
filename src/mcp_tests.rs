use super::*;
use crate::config::{AccountConfig, Config, Paths};
use crate::model::Project;

/// Projects live in <tmp>/code/<name>; <tmp>/work is the work account's folder.
struct T {
    dir: tempfile::TempDir,
    m: GroveMcp,
}

impl std::ops::Deref for T {
    type Target = GroveMcp;
    fn deref(&self) -> &GroveMcp {
        &self.m
    }
}

fn two_accounts(dir: &std::path::Path) -> Config {
    let mut accounts = std::collections::BTreeMap::new();
    accounts.insert(
        "work".to_string(),
        AccountConfig {
            dir: dir.join("claude-work").to_string_lossy().into(),
            color: None,
            folder: Some(dir.join("work").to_string_lossy().into()),
        },
    );
    Config {
        claude_config_dir: dir.join("claude-play").to_string_lossy().into(),
        accounts,
        tmux_session: format!("grove-test-{}-mcp", std::process::id()),
        ..Config::default()
    }
}

/// A server scoped to `account` ("play" or "work").
fn scoped(account: &str, default: Option<&str>) -> T {
    let dir = tempfile::tempdir().unwrap();
    let cfg = two_accounts(dir.path());
    let store = Store::new(Paths::new(dir.path().join("home")));
    let m = GroveMcp {
        store,
        cfg,
        tmux: crate::tmux::Tmux::new(format!("grove-test-{}-mcp", std::process::id())),
        scope: Ok(account.to_string()),
        default_project: default.map(str::to_string),
        own_session: None,
    };
    T { dir, m }
}

/// The play account, as the pre-scope tests assume.
fn mcp(default: Option<&str>) -> T {
    scoped("play", default)
}

/// Adds a project at <tmp>/code/<name> (play) or <tmp>/work/<name> (work).
fn add_in(t: &T, name: &str, work: bool) {
    let base = if work { "work" } else { "code" };
    let path = t.dir.path().join(base).join(name);
    std::fs::create_dir_all(&path).unwrap();
    t.store
        .add_project(&Project {
            name: name.into(),
            path,
            color: None,
            summary: None,
        })
        .unwrap();
}

fn add(t: &T, name: &str) {
    add_in(t, name, false);
}

#[test]
fn scope_flag_beats_grove_account() {
    let d = tempfile::tempdir().unwrap();
    let cfg = two_accounts(d.path());
    assert_eq!(
        scope_for(&cfg, Some("work"), Some("play"), None),
        Ok("work".into())
    );
    assert_eq!(scope_for(&cfg, None, Some("work"), None), Ok("work".into()));
}

#[test]
fn scope_without_an_account_says_how_to_register() {
    let d = tempfile::tempdir().unwrap();
    let e = scope_for(&two_accounts(d.path()), None, None, None).unwrap_err();
    assert!(e.contains("grove mcp setup"), "{e}");
}

#[test]
fn scope_unknown_account_names_the_real_ones() {
    let d = tempfile::tempdir().unwrap();
    let e = scope_for(&two_accounts(d.path()), Some("nope"), None, None).unwrap_err();
    assert!(e.contains("nope") && e.contains("work"), "{e}");
}

#[test]
fn scope_refuses_another_accounts_claude_config() {
    let d = tempfile::tempdir().unwrap();
    let cfg = two_accounts(d.path());
    let play_dir = d.path().join("claude-play");
    std::fs::create_dir_all(&play_dir).unwrap();
    let e = scope_for(&cfg, Some("work"), None, play_dir.to_str()).unwrap_err();
    assert!(e.contains("work") && e.contains("claude-play"), "{e}");
    // Its own config dir, or an unrelated one, is fine.
    let work_dir = d.path().join("claude-work");
    assert_eq!(
        scope_for(&cfg, Some("work"), None, work_dir.to_str()),
        Ok("work".into())
    );
    assert_eq!(
        scope_for(&cfg, Some("work"), None, Some("/elsewhere")),
        Ok("work".into())
    );
}

#[test]
fn scope_error_is_returned_by_every_tool() {
    let mut t = mcp(None);
    add(&t, "alpha");
    t.m.scope = Err("no account".into());
    assert_eq!(t.do_list_projects(), Err("no account".into()));
    assert_eq!(
        t.do_get_project(Some("alpha".into())),
        Err("no account".into())
    );
}

#[test]
fn work_scope_sees_only_work_projects() {
    let t = scoped("work", None);
    add_in(&t, "alpha", true);
    add_in(&t, "beta", false);
    let list = t.do_list_projects().unwrap();
    assert!(list.contains("alpha") && !list.contains("beta"), "{list}");
    assert!(t.do_get_project(Some("alpha".into())).is_ok());
}

#[test]
fn out_of_scope_project_looks_like_a_missing_one() {
    let t = scoped("work", None);
    add_in(&t, "alpha", true);
    add_in(&t, "beta", false);
    let hidden = t.do_get_project(Some("beta".into())).unwrap_err();
    let missing = t.do_get_project(Some("gamma".into())).unwrap_err();
    assert_eq!(hidden.replace("beta", "X"), missing.replace("gamma", "X"));
    assert!(!missing.contains("beta"), "{missing}");
    let upd = t
        .do_update_project(Some("beta".into()), Some("pwned".into()), None)
        .unwrap_err();
    assert!(!upd.contains("pwned"));
    assert_eq!(t.store.get_project("beta").unwrap().summary, None);
}

#[test]
fn symlinked_path_into_the_work_folder_counts_as_work() {
    let t = scoped("work", None);
    let real = t.dir.path().join("work").join("real");
    std::fs::create_dir_all(&real).unwrap();
    let link = t.dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    t.store
        .add_project(&Project {
            name: "linked".into(),
            path: link,
            color: None,
            summary: None,
        })
        .unwrap();
    assert!(t.do_list_projects().unwrap().contains("linked"));
}

#[test]
fn list_projects_empty_home() {
    let m = mcp(None);
    let out = m.do_list_projects().unwrap();
    assert!(out.contains("No projects"), "{out}");
}

#[test]
fn list_projects_shows_name_path_summary() {
    let m = mcp(None);
    add(&m, "alpha");
    m.do_update_project(Some("alpha".into()), Some("Alpha app.".into()), None)
        .unwrap();
    let out = m.do_list_projects().unwrap();
    assert!(
        out.contains("alpha") && out.contains("code/alpha") && out.contains("Alpha app."),
        "{out}"
    );
}

#[test]
fn get_project_defaults_to_session_project() {
    let m = mcp(Some("beta"));
    add(&m, "alpha");
    add(&m, "beta");
    let out = m.do_get_project(None).unwrap();
    assert!(out.contains("beta") && !out.contains("alpha"), "{out}");
    assert!(!out.contains("Wiki"), "{out}");
}

#[test]
fn no_project_and_no_default_is_an_error() {
    let m = mcp(None);
    add(&m, "alpha");
    let err = m.do_get_project(None).unwrap_err();
    assert!(err.contains("project") && err.contains("alpha"), "{err}");
}

#[test]
fn unknown_project_lists_valid_names() {
    let m = mcp(None);
    add(&m, "alpha");
    let err = m.do_get_project(Some("nope".into())).unwrap_err();
    assert!(err.contains("alpha"), "{err}");
}

#[test]
fn update_project_persists_summary_and_color() {
    let m = mcp(Some("alpha"));
    add(&m, "alpha");
    m.do_update_project(None, Some("New summary".into()), Some("#AABBCC".into()))
        .unwrap();
    let p = m.store.get_project("alpha").unwrap();
    assert_eq!(p.summary.as_deref(), Some("New summary"));
    assert_eq!(p.color.as_deref(), Some("#AABBCC"));
}

#[test]
fn update_project_rejects_bad_color_and_changes_nothing() {
    let m = mcp(Some("alpha"));
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
    let m = mcp(Some("alpha"));
    add(&m, "alpha");
    assert!(m.do_update_project(None, None, None).is_err());
}

#[test]
fn setup_prints_one_command_per_account() {
    let d = tempfile::tempdir().unwrap();
    let cfg = two_accounts(d.path());
    let out = setup_commands(
        &cfg,
        std::path::Path::new("/opt/homebrew/opt/grove/bin/grove"),
        d.path(),
    );
    let work = d.path().join("claude-work");
    assert!(
        out.contains(&format!(
            "CLAUDE_CONFIG_DIR='{}' claude mcp add --scope user grove -- '/opt/homebrew/opt/grove/bin/grove' mcp --account 'work'",
            work.display()
        )),
        "{out}"
    );
    assert!(out.contains("--account 'play'"), "{out}");
}

#[test]
fn setup_for_the_default_claude_dir_has_no_prefix() {
    let d = tempfile::tempdir().unwrap();
    let cfg = Config {
        claude_config_dir: d.path().join(".claude").to_string_lossy().into(),
        ..Config::default()
    };
    let out = setup_commands(&cfg, std::path::Path::new("/b/grove"), d.path());
    assert!(
        out.contains("\nclaude mcp add --scope user grove -- '/b/grove' mcp --account 'default'"),
        "{out}"
    );
    assert!(!out.contains("CLAUDE_CONFIG_DIR="), "{out}");
}

#[test]
fn claude_json_location() {
    let home = std::path::Path::new("/home/me");
    assert_eq!(
        claude_json(&home.join(".claude"), home),
        home.join(".claude.json")
    );
    assert_eq!(
        claude_json(&home.join(".claude-work"), home),
        home.join(".claude-work/.claude.json")
    );
}

#[test]
fn a_config_error_reaches_every_tool() {
    let d = tempfile::tempdir().unwrap();
    let paths = Paths::new(d.path().join("home"));
    paths.ensure().unwrap();
    std::fs::write(paths.config_file(), "this is = = not toml").unwrap();
    let m = GroveMcp::new(
        Store::new(paths),
        crate::tmux::Tmux::new("x"),
        Some("work".into()),
        &|_| None,
    );
    let e = m.do_list_projects().unwrap_err();
    assert!(e.contains("config"), "{e}");
}

#[test]
fn new_reads_account_project_and_session_from_env() {
    let d = tempfile::tempdir().unwrap();
    let paths = Paths::new(d.path().join("home"));
    paths.ensure().unwrap();
    let env = |k: &str| match k {
        "GROVE_ACCOUNT" => Some("default".to_string()),
        "GROVE_PROJECT" => Some("alpha".to_string()),
        "GROVE_SESSION" => Some("sid".to_string()),
        _ => None,
    };
    let m = GroveMcp::new(Store::new(paths), crate::tmux::Tmux::new("x"), None, &env);
    assert_eq!(m.scope, Ok("default".into()));
    assert_eq!(m.default_project.as_deref(), Some("alpha"));
    assert_eq!(m.own_session.as_deref(), Some("sid"));
}
