# Account-scoped Grove MCP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let Claude see and act on Grove's projects, sessions and to-dos
through `grove mcp`, limited to its own Claude account's projects.

**Architecture:** `GroveMcp` gains the config, the Grove tmux server and a
`scope` (the account name, or the error every tool returns). Every lookup
goes through scoped helpers, so a project outside the account looks
exactly like a project that doesn't exist. The account comes from
`--account` or `$GROVE_ACCOUNT` (set on Grove's windows). If
`$CLAUDE_CONFIG_DIR` names a different account, the server refuses.
Overview text (git, README excerpts) lives in a new `mcp_overview.rs`.
To-do edits are a pure `todo::apply_ops`.

**Tech Stack:** Rust 2024, rmcp 3.5 (`#[tool_router]`, `Parameters`,
`schemars`), tmux CLI, git CLI, clap.

**Spec:** `docs/superpowers/specs/2026-10-16-account-scoped-mcp-design.md`

## Global Constraints

- Out-of-scope projects and sessions give the same error text as missing
  ones. Their names never appear in any output, including the lists of
  valid names in error messages.
- Grove never edits Claude's configuration: `grove mcp setup` prints
  commands, it doesn't run them.
- The registration command uses `runfiles::grove_exe()` (the brew-stable
  path).
- `send_prompt` only acts on sessions in state `YourTurn`, never on the
  caller's own session (`$GROVE_SESSION`).
- `read_session`: `lines` defaults to 60, max 500.
- `get_project`: README/CLAUDE.md excerpts are the first 40 lines; git
  runs with a 2-second timeout.
- Tests: `cargo test --bin grove <filter>` (not `--lib`), and the full
  `cargo test`. Before each commit: `cargo fmt` and
  `cargo clippy --all-targets -- -D warnings`. Commits end with
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- `cargo fmt` reflows code, so check that every edit landed (re-grep)
  rather than assuming.
- Examples in code, tests and docs use generic names (`alpha`, `beta`,
  `work`, `play`, `homebox`), never the user's real hosts.

## Review Focus

- **Session names repeat across projects** (`s1` in alpha and in beta):
  a bare `s1` must be ambiguous within the account (an error listing
  `alpha:s1, beta:s1`), never silently pick one. A bare name that's
  unique within the account must work, even if another account has the
  same name. Tests in Task 5.
- **The default account at `~/.claude`:** with no `CLAUDE_CONFIG_DIR`,
  Claude keeps user-scope servers in `~/.claude.json`, not
  `~/.claude/.claude.json`. So `mcp setup` omits the prefix for it, and
  doctor reads `~/.claude.json`. Tests in Task 7.
- **A session whose tmux window is gone** (`tmux_window` None, or the
  window killed): `read_session` and `send_prompt` return a clear error,
  not a tmux stderr dump or a panic. Tests in Task 5 and Task 6.
- **config.toml fails to parse when the server starts:** the server
  still starts, and every tool returns the config error. Test in Task 2.
- **Symlinked or differently cased project paths:** scope uses
  `cfg.account_for`, which already resolves symlinks and ignores case on
  macOS. Don't compare paths by hand anywhere. Test in Task 1 (project
  path through a symlink into the work folder).

---

### Task 1: Scope: which account, and filtering the existing tools

**Files:**
- Modify: `src/config.rs` (add `all_accounts`)
- Modify: `src/mcp.rs`
- Modify: `src/mcp_tests.rs`

**Interfaces:**
- Produces:
  - `Config::all_accounts(&self) -> Vec<Account>`: the default first,
    then named ones in key order.
  - `mcp::scope_for(cfg: &Config, flag: Option<&str>, grove_account: Option<&str>, claude_config_dir: Option<&str>) -> Result<String, String>`
  - `GroveMcp` fields:
    - `store: Store`, `cfg: Config`, `tmux: crate::tmux::Tmux`;
    - `scope: Result<String, String>`;
    - `default_project: Option<String>`, `own_session: Option<String>`.
  - Private helpers later tasks use:
    - `fn account(&self) -> Result<&str, String>`
    - `fn in_scope(&self, p: &Project) -> bool`
    - `fn projects(&self) -> Result<Vec<Project>, String>`: scoped, sorted
      as the store returns them.
    - `fn project(&self, project: Option<String>) -> Result<Project, String>`

- [ ] **Step 1: Write the failing tests**

Replace the `mcp()` helper at the top of `src/mcp_tests.rs`, and add the
scope tests. The existing tests keep calling `mcp(default)` and `add(&m, name)`.

```rust
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
    assert_eq!(scope_for(&cfg, Some("work"), Some("play"), None), Ok("work".into()));
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
    assert_eq!(scope_for(&cfg, Some("work"), None, work_dir.to_str()), Ok("work".into()));
    assert_eq!(scope_for(&cfg, Some("work"), None, Some("/elsewhere")), Ok("work".into()));
}

#[test]
fn scope_error_is_returned_by_every_tool() {
    let mut t = mcp(None);
    add(&t, "alpha");
    t.m.scope = Err("no account".into());
    assert_eq!(t.do_list_projects(), Err("no account".into()));
    assert_eq!(t.do_get_project(Some("alpha".into())), Err("no account".into()));
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
        .add_project(&Project { name: "linked".into(), path: link, color: None, summary: None })
        .unwrap();
    assert!(t.do_list_projects().unwrap().contains("linked"));
}
```

Fix up the existing tests that use `m.store` or `/code/{name}`: `m` is now
a `T`, which derefs to `GroveMcp`. The path assertion in
`list_projects_shows_name_path_summary` becomes
`out.contains(&format!("code/alpha"))`.

- [ ] **Step 2: Run the tests to watch them fail**

Run: `cargo test --bin grove mcp::tests 2>&1 | grep -E "^error" | head`
Expected: compile errors. `GroveMcp` has no field `cfg`/`tmux`/`scope`/`own_session`, and `scope_for` isn't found.

- [ ] **Step 3: Implement**

In `src/config.rs`, inside `impl Config`, after `account`:

```rust
    /// Every account: the default first, then the named ones.
    pub fn all_accounts(&self) -> Vec<Account> {
        std::iter::once(None)
            .chain(self.accounts.keys().map(|k| Some(k.as_str())))
            .filter_map(|n| self.account(n).ok())
            .collect()
    }
```

In `src/mcp.rs`, replace the struct and `project`, and add the scope
helpers:

```rust
/// Tool logic, separate from the rmcp wrappers so tests can call it directly.
/// `Err` text is returned to Claude as a tool error.
#[derive(Clone)]
pub struct GroveMcp {
    pub store: Store,
    pub cfg: Config,
    /// Grove's own tmux server, for sessions.
    pub tmux: Tmux,
    /// The account every tool is limited to, or the error each one returns.
    pub scope: Result<String, String>,
    /// From `$GROVE_PROJECT`, set on every Grove session's window.
    pub default_project: Option<String>,
    /// From `$GROVE_SESSION`: this Claude's own session.
    pub own_session: Option<String>,
}

/// The account a server serves: `--account`, else `$GROVE_ACCOUNT`. When
/// `$CLAUDE_CONFIG_DIR` is another account's folder, it was registered in
/// the wrong place, so it refuses rather than show that account this one's
/// projects.
pub fn scope_for(
    cfg: &Config,
    flag: Option<&str>,
    grove_account: Option<&str>,
    claude_config_dir: Option<&str>,
) -> Result<String, String> {
    let name = flag
        .or(grove_account)
        .filter(|n| !n.is_empty())
        .ok_or_else(|| {
            "Grove's tools don't know which Claude account this is. In a terminal, run `grove mcp setup` and use the command it prints for this account.".to_string()
        })?;
    let account = cfg.account(Some(name)).map_err(|e| format!("{e:#}"))?;
    if let Some(dir) = claude_config_dir.filter(|d| !d.is_empty()) {
        let canon = |p: &std::path::Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        let here = canon(&crate::config::expand_tilde(dir));
        if let Some(other) = cfg
            .all_accounts()
            .into_iter()
            .find(|a| a.name != account.name && canon(&a.dir) == here)
        {
            return Err(format!(
                "these Grove tools are registered for the {} account but are running in {}'s Claude config ({dir}); run `grove mcp setup` for the right command",
                account.name, other.name
            ));
        }
    }
    Ok(account.name)
}

impl GroveMcp {
    fn account(&self) -> Result<&str, String> {
        self.scope.as_deref().map_err(Clone::clone)
    }

    fn in_scope(&self, p: &Project) -> bool {
        self.account()
            .is_ok_and(|a| self.cfg.account_for(&p.path).name == a)
    }

    /// This account's projects (unreadable project files are skipped).
    fn projects(&self) -> Result<Vec<Project>, String> {
        self.account()?;
        Ok(self
            .store
            .list_projects()
            .into_iter()
            .filter_map(Result::ok)
            .filter(|p| self.in_scope(p))
            .collect())
    }

    fn names(&self) -> String {
        let names: Vec<String> = self
            .projects()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.name)
            .collect();
        if names.is_empty() {
            "(none)".into()
        } else {
            names.join(", ")
        }
    }

    /// A project of this account by name or slug, else the session's own.
    /// One outside the account gets the same error as one that doesn't exist.
    fn project(&self, project: Option<String>) -> Result<Project, String> {
        self.account()?;
        let name = project
            .filter(|p| !p.trim().is_empty())
            .or_else(|| self.default_project.clone());
        match name {
            Some(n) => self
                .store
                .get_project(&n)
                .ok()
                .filter(|p| self.in_scope(p))
                .ok_or_else(|| format!("no project \"{n}\"; projects: {}", self.names())),
            None => Err(format!(
                "no project given and this isn't a Grove session; pass `project` (one of: {})",
                self.names()
            )),
        }
    }
```

`do_list_projects` iterates `self.projects()?` instead of
`self.store.list_projects()...filter_map(Result::ok)`. Add the imports
`use crate::config::Config;` and `use crate::tmux::Tmux;`.

In `serve`, construct the new fields temporarily so it compiles (Task 2
wires them properly):

```rust
pub async fn serve(store: Store) -> anyhow::Result<()> {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let cfg = crate::config::load_config(&store.paths).unwrap_or_default();
    let scope = scope_for(&cfg, None, env("GROVE_ACCOUNT").as_deref(), env("CLAUDE_CONFIG_DIR").as_deref());
    let service = GroveMcp {
        store,
        cfg,
        tmux: Tmux::new("grove"),
        scope,
        default_project: env("GROVE_PROJECT"),
        own_session: env("GROVE_SESSION"),
    }
    .serve(rmcp::transport::stdio())
    .await?;
    service.waiting().await?;
    Ok(())
}
```

- [ ] **Step 4: Run the tests to watch them pass**

Run: `cargo test --bin grove mcp::tests 2>&1 | grep "test result"`
Expected: `ok`, with every mcp test passing (existing plus 8 new).

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/config.rs src/mcp.rs src/mcp_tests.rs
git commit -m "feat(mcp): limit tools to one Claude account's projects

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Wiring: `--account`, `GROVE_ACCOUNT`, `grove mcp setup`

**Files:**
- Modify: `src/main.rs` (Mcp command, setup subcommand)
- Modify: `src/mcp.rs` (`serve` signature, `setup_commands`)
- Modify: `src/launch.rs` (`GROVE_ACCOUNT` in env)
- Modify: `src/launch_tests.rs`, `src/mcp_tests.rs`

**Interfaces:**
- Consumes: `scope_for`, `GroveMcp` (Task 1); `runfiles::grove_exe()`; `main.rs::grove_tmux(paths)`.
- Produces:
  - `mcp::serve(store: Store, tmux: Tmux, account_flag: Option<String>) -> anyhow::Result<()>`
  - `mcp::setup_commands(cfg: &Config, exe: &Path, home: &Path) -> String`
  - `mcp::claude_json(account_dir: &Path, home: &Path) -> PathBuf`: the
    file Claude keeps user-scope MCP servers in for that account.
  - A launched window's env includes `GROVE_ACCOUNT=<account name>`.

- [ ] **Step 1: Write the failing tests**

In `src/launch_tests.rs`, extend the existing test that checks the build
env (find the test asserting `env_val(&spec, "GROVE_PROJECT")`) with:

```rust
    assert_eq!(env_val(&spec, "GROVE_ACCOUNT"), Some("claude play"));
```

In `src/mcp_tests.rs`:

```rust
#[test]
fn setup_prints_one_command_per_account() {
    let d = tempfile::tempdir().unwrap();
    let cfg = two_accounts(d.path());
    let out = setup_commands(&cfg, std::path::Path::new("/opt/homebrew/opt/grove/bin/grove"), d.path());
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
    assert!(out.contains("\nclaude mcp add --scope user grove -- '/b/grove' mcp --account 'default'"), "{out}");
    assert!(!out.contains("CLAUDE_CONFIG_DIR="), "{out}");
}

#[test]
fn claude_json_location() {
    let home = std::path::Path::new("/home/me");
    assert_eq!(claude_json(&home.join(".claude"), home), home.join(".claude.json"));
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
    let m = GroveMcp::new(Store::new(paths), crate::tmux::Tmux::new("x"), Some("work".into()), &|_| None);
    let e = m.do_list_projects().unwrap_err();
    assert!(e.contains("config"), "{e}");
}
```

- [ ] **Step 2: Run the tests to watch them fail**

Run: `cargo test --bin grove -- mcp::tests launch::tests 2>&1 | grep -E "^error|FAILED|panicked" | head`
Expected: compile errors for `setup_commands`, `claude_json` and `GroveMcp::new`.

- [ ] **Step 3: Implement**

`src/launch.rs` `build_with_bin`: add to `env`, after `GROVE_PROJECT`:

```rust
            ("GROVE_ACCOUNT".into(), account.name.clone()),
```

(`account` is moved into the `LaunchSpec` below it, so take the name
first, e.g. `let account_name = account.name.clone();` before
`Ok(LaunchSpec {`.)

`src/mcp.rs`: add a constructor that does the environment reading, and
use it in `serve`:

```rust
impl GroveMcp {
    /// `env` reads environment variables (tests pass their own).
    pub fn new(
        store: Store,
        tmux: Tmux,
        account_flag: Option<String>,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Self {
        let env = |k: &str| env(k).filter(|v| !v.is_empty());
        let (cfg, scope) = match crate::config::load_config(&store.paths) {
            Ok(cfg) => {
                let scope = scope_for(
                    &cfg,
                    account_flag.as_deref(),
                    env("GROVE_ACCOUNT").as_deref(),
                    env("CLAUDE_CONFIG_DIR").as_deref(),
                );
                (cfg, scope)
            }
            Err(e) => (Config::default(), Err(format!("Grove's config.toml: {e:#}"))),
        };
        Self {
            store,
            cfg,
            tmux,
            scope,
            default_project: env("GROVE_PROJECT"),
            own_session: env("GROVE_SESSION"),
        }
    }
}

pub async fn serve(store: Store, tmux: Tmux, account_flag: Option<String>) -> anyhow::Result<()> {
    let service = GroveMcp::new(store, tmux, account_flag, &|k| std::env::var(k).ok())
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}

/// Where Claude keeps user-scope MCP servers for the account in `dir`:
/// `~/.claude.json` for the default `~/.claude` (used without
/// CLAUDE_CONFIG_DIR), else `<dir>/.claude.json`.
pub fn claude_json(dir: &std::path::Path, home: &std::path::Path) -> std::path::PathBuf {
    if dir == home.join(".claude") {
        home.join(".claude.json")
    } else {
        dir.join(".claude.json")
    }
}

/// `grove mcp setup`: one `claude mcp add` per account, to paste.
pub fn setup_commands(cfg: &Config, exe: &std::path::Path, home: &std::path::Path) -> String {
    let q = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
    let mut out = String::from(
        "Give Claude Grove's tools in every session, not only Grove's own.\nRun each line once (Grove doesn't change Claude's settings itself):\n",
    );
    for a in cfg.all_accounts() {
        let prefix = if a.dir == home.join(".claude") {
            String::new()
        } else {
            format!("CLAUDE_CONFIG_DIR={} ", q(&a.dir.to_string_lossy()))
        };
        out.push_str(&format!(
            "\n# {} ({})\n{prefix}claude mcp add --scope user grove -- {} mcp --account {}\n",
            a.name,
            a.dir.display(),
            q(&exe.to_string_lossy()),
            q(&a.name),
        ));
    }
    out.push_str("\nCheck with: grove doctor\n");
    out
}
```

Note that the default-dir test expects a `\n` before `claude mcp add`.
That holds because the `# name (dir)` comment line ends with `\n`.

`src/main.rs`: make `Mcp` a visible command with an optional subcommand:

```rust
    /// MCP server giving Claude Grove's projects, sessions and to-dos.
    /// `grove mcp setup` prints how to add it to each Claude account.
    Mcp {
        /// The Claude account these tools are for (a name from config.toml).
        #[arg(long)]
        account: Option<String>,
        #[command(subcommand)]
        cmd: Option<McpCmd>,
    },
```

```rust
#[derive(Subcommand)]
enum McpCmd {
    /// Print the command that adds Grove's tools to each Claude account.
    Setup,
}
```

```rust
        Some(Cmd::Mcp { cmd: Some(McpCmd::Setup), .. }) => exit_on_err((|| {
            let cfg = load_config(&paths)?;
            let home = std::path::PathBuf::from(std::env::var("HOME")?);
            print!("{}", mcp::setup_commands(&cfg, &runfiles::grove_exe()?, &home));
            Ok(())
        })()),
        Some(Cmd::Mcp { account, cmd: None }) => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            let tmux = grove_tmux(&paths);
            if let Err(e) = rt.block_on(mcp::serve(Store::new(paths.clone()), tmux, account)) {
                log::error(&paths, "mcp", &format!("{e:#}"));
                std::process::exit(1);
            }
        }
```

Match `exit_on_err`'s and `load_config`'s actual signatures in main.rs.
If `exit_on_err` takes `anyhow::Result<()>`, the closure above fits.

- [ ] **Step 4: Run the tests to watch them pass**

Run: `cargo test 2>&1 | grep "test result"`
Expected: every suite `ok`.

Manual: `cargo run -q -- mcp setup` prints one block per account in your
config, with your real account folders.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add -A src
git commit -m "feat(mcp): grove mcp --account and grove mcp setup

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Project overview: counts, git, README and CLAUDE.md, sessions, to-dos

**Files:**
- Create: `src/mcp_overview.rs`
- Modify: `src/main.rs` (`mod mcp_overview;`)
- Modify: `src/mcp.rs` (`do_list_projects`, `do_get_project`)
- Modify: `src/todo.rs` (make `parse` pub)
- Test: `src/mcp_tests.rs`, and unit tests at the bottom of `mcp_overview.rs`

**Interfaces:**
- Consumes: `GroveMcp::projects`, `project` (Task 1); `Store::list_sessions(Some(slug))`, `Store::read_todos(slug)`, `todo::open_count`.
- Produces:
  - `mcp_overview::live(sessions: Vec<Session>) -> Vec<Session>`: not
    ended, newest activity first.
  - `mcp_overview::counts(sessions: &[Session]) -> String`: e.g.
    `1 needs you, 2 your turn, 1 working`, or `no sessions`. Starting
    counts as working.
  - `mcp_overview::ago(t: DateTime<Utc>, now: DateTime<Utc>) -> String`:
    `just now`, `5m ago`, `3h ago`, `2d ago`.
  - `mcp_overview::git(dir: &Path) -> String`: multi-line, or
    `git: none`.
  - `mcp_overview::excerpt(path: &Path, lines: usize) -> Option<String>`
  - `pub fn todo::parse(text: &str) -> Vec<Item>`

- [ ] **Step 1: Write the failing tests**

At the bottom of the new `src/mcp_overview.rs` (with the module body
still empty apart from this):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Session, State};

    fn session(state: State, mins_ago: i64) -> Session {
        let t = chrono::Utc::now() - chrono::Duration::minutes(mins_ago);
        Session {
            id: format!("id-{mins_ago}"),
            project: "alpha".into(),
            name: format!("s{mins_ago}"),
            host: "local".into(),
            tmux_window: None,
            state,
            message: None,
            created_at: t,
            updated_at: t,
        }
    }

    #[test]
    fn counts_by_state_with_starting_as_working() {
        let s = vec![
            session(State::NeedsYou, 1),
            session(State::YourTurn, 2),
            session(State::Starting, 3),
            session(State::Working, 4),
        ];
        assert_eq!(counts(&s), "1 needs you, 1 your turn, 2 working");
        assert_eq!(counts(&[]), "no sessions");
    }

    #[test]
    fn live_drops_ended_and_sorts_newest_first() {
        let s = live(vec![session(State::Working, 9), session(State::Ended, 1), session(State::YourTurn, 2)]);
        let names: Vec<_> = s.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["s2", "s9"]);
    }

    #[test]
    fn ago_buckets() {
        let now = chrono::Utc::now();
        let m = |n| now - chrono::Duration::minutes(n);
        assert_eq!(ago(m(0), now), "just now");
        assert_eq!(ago(m(5), now), "5m ago");
        assert_eq!(ago(m(180), now), "3h ago");
        assert_eq!(ago(m(60 * 50), now), "2d ago");
    }

    #[test]
    fn git_reports_branch_changes_and_commits() {
        let d = tempfile::tempdir().unwrap();
        let g = |args: &[&str]| {
            assert!(std::process::Command::new("git").arg("-C").arg(d.path()).args(args)
                .env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t")
                .output().unwrap().status.success());
        };
        g(&["init", "-q", "-b", "main"]);
        std::fs::write(d.path().join("a"), "1").unwrap();
        g(&["add", "a"]);
        g(&["commit", "-q", "-m", "first commit"]);
        std::fs::write(d.path().join("b"), "2").unwrap();
        let out = git(d.path());
        assert!(out.contains("branch: main"), "{out}");
        assert!(out.contains("uncommitted: 1 file"), "{out}");
        assert!(out.contains("first commit"), "{out}");
    }

    #[test]
    fn git_outside_a_repo_is_none() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(git(d.path()), "git: none");
    }

    #[test]
    fn excerpt_takes_the_first_lines() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("README.md");
        std::fs::write(&f, (1..=50).map(|i| format!("line {i}\n")).collect::<String>()).unwrap();
        let e = excerpt(&f, 40).unwrap();
        assert!(e.contains("line 40") && !e.contains("line 41"), "{e}");
        assert!(e.ends_with("(10 more lines)"), "{e}");
        assert_eq!(excerpt(&d.path().join("nope.md"), 40), None);
    }
}
```

In `src/mcp_tests.rs`:

```rust
#[test]
fn get_project_shows_readme_sessions_and_todos() {
    let t = mcp(None);
    add(&t, "alpha");
    let dir = t.dir.path().join("code/alpha");
    std::fs::write(dir.join("README.md"), "# Alpha\nDoes alpha things.\n").unwrap();
    std::fs::write(dir.join("CLAUDE.md"), "Use tabs.\n").unwrap();
    t.store.write_todos("alpha", "- [ ] ship it\n- [x] start\n").unwrap();
    let out = t.do_get_project(Some("alpha".into())).unwrap();
    for want in ["Does alpha things.", "Use tabs.", "git: none", "1. [ ] ship it", "2. [x] start", "no sessions"] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
}

#[test]
fn list_projects_shows_counts_and_open_todos() {
    let t = mcp(None);
    add(&t, "alpha");
    t.store.write_todos("alpha", "- [ ] a\n- [ ] b\n- [x] c\n").unwrap();
    let out = t.do_list_projects().unwrap();
    assert!(out.contains("no sessions") && out.contains("2 open to-dos"), "{out}");
}
```

- [ ] **Step 2: Run the tests to watch them fail**

Run: `cargo test --bin grove -- mcp 2>&1 | grep -E "^error|FAILED" | head`
Expected: compile errors for `counts`, `live`, `ago`, `git` and `excerpt`.
Before compiling, add `mod mcp_overview;` to main.rs.

- [ ] **Step 3: Implement**

`src/mcp_overview.rs`, above the tests:

```rust
//! Text the MCP tools show about a project: session counts, git, docs.

use std::path::Path;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use crate::model::{Session, State};

/// Sessions that haven't ended, most recently active first.
pub fn live(mut sessions: Vec<Session>) -> Vec<Session> {
    sessions.retain(|s| s.state != State::Ended);
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    sessions
}

pub fn counts(sessions: &[Session]) -> String {
    let n = |f: &dyn Fn(State) -> bool| sessions.iter().filter(|s| f(s.state)).count();
    let parts: Vec<String> = [
        (n(&|s| s == State::NeedsYou), "needs you"),
        (n(&|s| s == State::YourTurn), "your turn"),
        (n(&|s| matches!(s, State::Working | State::Starting)), "working"),
    ]
    .into_iter()
    .filter(|(c, _)| *c > 0)
    .map(|(c, l)| format!("{c} {l}"))
    .collect();
    if parts.is_empty() {
        "no sessions".into()
    } else {
        parts.join(", ")
    }
}

pub fn ago(t: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let m = (now - t).num_minutes().max(0);
    match m {
        0 => "just now".into(),
        1..=59 => format!("{m}m ago"),
        60..=2879 => format!("{}h ago", m / 60),
        _ => format!("{}d ago", m / 1440),
    }
}

/// `git -C dir <args>`'s stdout, or None on failure or after 2 seconds.
fn git_out(dir: &Path, args: &[&str]) -> Option<String> {
    use std::process::{Command, Stdio};
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait().ok()? {
            Some(st) if st.success() => break,
            Some(_) => return None,
            None if Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let mut s = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut s).ok()?;
    Some(s)
}

pub fn git(dir: &Path) -> String {
    let Some(branch) = git_out(dir, &["rev-parse", "--abbrev-ref", "HEAD"]) else {
        return "git: none".into();
    };
    let changed = git_out(dir, &["status", "--porcelain"])
        .map(|s| s.lines().count())
        .unwrap_or(0);
    let log = git_out(dir, &["log", "-5", "--format=  %h %s (%cr)"]).unwrap_or_default();
    format!(
        "branch: {}\nuncommitted: {changed} file{}\nrecent commits:\n{}",
        branch.trim(),
        if changed == 1 { "" } else { "s" },
        log.trim_end()
    )
}

/// The first `lines` lines of a text file, noting how many were left out.
pub fn excerpt(path: &Path, lines: usize) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let all: Vec<&str> = text.lines().collect();
    let mut out = all[..all.len().min(lines)].join("\n");
    if all.len() > lines {
        out.push_str(&format!("\n({} more lines)", all.len() - lines));
    }
    Some(out)
}
```

Note: the git-output pipe could fill while we wait. `log -5` and
`rev-parse` are tiny. A huge `status --porcelain` in a giant dirty repo
could exceed the 64 KB pipe buffer and hit the timeout. That's
acceptable: it reports `uncommitted: 0`.

`src/todo.rs`: change `fn parse(` to `pub fn parse(`.

`src/mcp.rs`, numbered to-dos (shared with Task 4):

```rust
fn numbered(items: &[crate::todo::Item]) -> String {
    if items.is_empty() {
        return "(no to-dos)".into();
    }
    items
        .iter()
        .enumerate()
        .map(|(i, it)| format!("{}. [{}] {}", i + 1, if it.done { 'x' } else { ' ' }, it.text))
        .collect::<Vec<_>>()
        .join("\n")
}
```

Rewrite `do_list_projects`'s line as:

```rust
            .map(|p| {
                let summary = p.summary.as_deref().unwrap_or("(no summary)");
                let sessions = mcp_overview::live(self.store.list_sessions(Some(&p.slug())));
                let open = crate::todo::open_count(&self.store.read_todos(&p.slug()));
                format!(
                    "- {} — {} — {summary}\n  sessions: {}; {open} open to-do{}",
                    p.name,
                    p.path.display(),
                    mcp_overview::counts(&sessions),
                    if open == 1 { "" } else { "s" }
                )
            })
```

Rewrite `do_get_project`:

```rust
    pub fn do_get_project(&self, project: Option<String>) -> Result<String, String> {
        let p = self.project(project)?;
        let slug = p.slug();
        let now = chrono::Utc::now();
        let sessions = mcp_overview::live(self.store.list_sessions(Some(&slug)));
        let mut out = format!(
            "Name: {}\nPath: {}\nAccount: {}\nColor: {}\nSummary: {}\n\nSessions: {}\n",
            p.name,
            p.path.display(),
            self.cfg.account_for(&p.path).name,
            p.color.as_deref().unwrap_or("(none)"),
            p.summary.as_deref().unwrap_or("(none)"),
            mcp_overview::counts(&sessions),
        );
        for s in &sessions {
            out.push_str(&format!("  {slug}:{} — {} — {}\n", s.name, s.state.label(), mcp_overview::ago(s.updated_at, now)));
        }
        out.push_str(&format!("\nTo-dos:\n{}\n\n{}\n", numbered(&crate::todo::parse(&self.store.read_todos(&slug))), mcp_overview::git(&p.path)));
        for doc in ["README.md", "CLAUDE.md"] {
            if let Some(e) = mcp_overview::excerpt(&p.path.join(doc), 40) {
                out.push_str(&format!("\n--- {doc} ---\n{e}\n"));
            }
        }
        Ok(out)
    }
```

`do_update_project` returns `self.do_get_project(Some(p.name))`. That's
fine, but its output is now long. Leave it.

Check `get_project_defaults_to_session_project`: it asserts the output
doesn't contain "alpha". The new output includes paths under
`code/beta` only, so it still holds.

- [ ] **Step 4: Run the tests to watch them pass**

Run: `cargo test --bin grove -- mcp 2>&1 | grep "test result"`
Expected: `ok`.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add -A src
git commit -m "feat(mcp): project overview with sessions, git, docs and to-dos

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: To-dos: `get_todos` and `edit_todos`

**Files:**
- Modify: `src/todo.rs` (`Op`, `apply_ops`, `serialize_items`)
- Modify: `src/todo_tests.rs`
- Modify: `src/mcp.rs`, `src/mcp_tests.rs`

**Interfaces:**
- Consumes: `todo::parse`, `numbered` (Task 3); `GroveMcp::project` (Task 1).
- Produces:
  - `pub enum todo::Op { Add(String), Check(usize), Uncheck(usize), Reword(usize, String), Remove(usize) }`:
    numbers are 1-based, into the original list.
  - `pub fn todo::apply_ops(items: &[Item], ops: &[Op]) -> Result<Vec<Item>, String>`
  - `pub fn todo::serialize_items(items: &[Item]) -> String` (`Todos::serialize`
    calls it).
  - `GroveMcp::do_get_todos(project) -> Result<String, String>`,
    `do_edit_todos(project, Vec<TodoOpArg>) -> Result<String, String>`

- [ ] **Step 1: Write the failing tests**

`src/todo_tests.rs`:

```rust
fn items(spec: &[(&str, bool)]) -> Vec<Item> {
    spec.iter().map(|(t, d)| Item { text: t.to_string(), done: *d }).collect()
}

#[test]
fn ops_number_the_original_list() {
    let start = items(&[("a", false), ("b", false), ("c", false)]);
    let out = apply_ops(
        &start,
        &[Op::Remove(1), Op::Check(3), Op::Reword(2, "B".into()), Op::Add("d".into())],
    )
    .unwrap();
    assert_eq!(out, items(&[("B", false), ("c", true), ("d", false)]));
}

#[test]
fn uncheck_and_out_of_range() {
    let start = items(&[("a", true)]);
    assert_eq!(apply_ops(&start, &[Op::Uncheck(1)]).unwrap(), items(&[("a", false)]));
    let e = apply_ops(&start, &[Op::Check(2)]).unwrap_err();
    assert!(e.contains('2') && e.contains("1 item"), "{e}");
    assert!(apply_ops(&start, &[Op::Check(0)]).is_err());
}

#[test]
fn an_op_on_a_removed_item_is_an_error() {
    let start = items(&[("a", false)]);
    assert!(apply_ops(&start, &[Op::Remove(1), Op::Check(1)]).is_err());
}

#[test]
fn empty_text_is_refused() {
    assert!(apply_ops(&[], &[Op::Add("  ".into())]).is_err());
}

#[test]
fn serialize_items_matches_the_dashboard_format() {
    let it = items(&[("a", false), ("b", true)]);
    assert_eq!(serialize_items(&it), "- [ ] a\n- [x] b\n");
    assert_eq!(parse(&serialize_items(&it)), it);
}
```

`src/mcp_tests.rs`:

```rust
fn op(op: &str, n: Option<usize>, text: Option<&str>) -> TodoOpArg {
    TodoOpArg { op: op.into(), n, text: text.map(str::to_string) }
}

#[test]
fn edit_todos_saves_and_returns_the_list() {
    let t = mcp(None);
    add(&t, "alpha");
    t.store.write_todos("alpha", "- [ ] a\n").unwrap();
    let out = t
        .do_edit_todos(Some("alpha".into()), vec![op("check", Some(1), None), op("add", None, Some("b"))])
        .unwrap();
    assert!(out.contains("1. [x] a") && out.contains("2. [ ] b"), "{out}");
    assert_eq!(t.store.read_todos("alpha"), "- [x] a\n- [ ] b\n");
}

#[test]
fn a_bad_op_saves_nothing() {
    let t = mcp(None);
    add(&t, "alpha");
    t.store.write_todos("alpha", "- [ ] a\n").unwrap();
    for bad in [op("check", Some(5), None), op("frob", Some(1), None), op("reword", Some(1), None)] {
        assert!(t.do_edit_todos(Some("alpha".into()), vec![op("add", None, Some("x")), bad]).is_err());
    }
    assert_eq!(t.store.read_todos("alpha"), "- [ ] a\n");
}

#[test]
fn todos_of_another_account_are_hidden() {
    let t = scoped("work", None);
    add_in(&t, "beta", false);
    t.store.write_todos("beta", "- [ ] secret\n").unwrap();
    assert!(t.do_get_todos(Some("beta".into())).is_err());
    assert!(t.do_edit_todos(Some("beta".into()), vec![op("add", None, Some("x"))]).is_err());
    assert_eq!(t.store.read_todos("beta"), "- [ ] secret\n");
}
```

- [ ] **Step 2: Run the tests to watch them fail**

Run: `cargo test --bin grove -- todo mcp 2>&1 | grep -E "^error" | head`
Expected: compile errors for `apply_ops`, `Op`, `serialize_items`, `TodoOpArg` and `do_edit_todos`.

- [ ] **Step 3: Implement**

`src/todo.rs`:

```rust
/// One edit to a list, from outside the dashboard (the MCP tools).
/// Numbers are 1-based positions in the list before any edit.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Add(String),
    Check(usize),
    Uncheck(usize),
    Reword(usize, String),
    Remove(usize),
}

/// Applies `ops` in order; any bad op fails the whole batch.
pub fn apply_ops(items: &[Item], ops: &[Op]) -> Result<Vec<Item>, String> {
    let mut slots: Vec<Option<Item>> = items.iter().cloned().map(Some).collect();
    let mut added = Vec::new();
    let len = items.len();
    let mut at = |n: usize| -> Result<usize, String> {
        if n == 0 || n > len {
            Err(format!("there's no item {n}; the list has {len} item{}", if len == 1 { "" } else { "s" }))
        } else {
            Ok(n - 1)
        }
    };
    let text_ok = |t: &str| -> Result<String, String> {
        let t = t.trim();
        if t.is_empty() { Err("a to-do needs some text".into()) } else { Ok(t.to_string()) }
    };
    for op in ops {
        match op {
            Op::Add(t) => added.push(Item { text: text_ok(t)?, done: false }),
            Op::Remove(n) => {
                let i = at(*n)?;
                slots[i].take().ok_or_else(|| format!("item {n} was already removed"))?;
            }
            Op::Check(n) | Op::Uncheck(n) | Op::Reword(n, _) => {
                let i = at(*n)?;
                let item = slots[i].as_mut().ok_or_else(|| format!("item {n} was removed"))?;
                match op {
                    Op::Check(_) => item.done = true,
                    Op::Uncheck(_) => item.done = false,
                    Op::Reword(_, t) => item.text = text_ok(t)?,
                    _ => unreachable!(),
                }
            }
        }
    }
    Ok(slots.into_iter().flatten().chain(added).collect())
}

pub fn serialize_items(items: &[Item]) -> String {
    items
        .iter()
        .map(|i| format!("- [{}] {}\n", if i.done { 'x' } else { ' ' }, i.text))
        .collect()
}
```

`at` doesn't need `mut`. Drop it if clippy complains. Make
`Todos::serialize` call `serialize_items(&self.items)`.

`src/mcp.rs`:

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TodoOpArg {
    /// add, check, uncheck, reword or remove.
    pub op: String,
    /// The item's number as get_todos shows it (before this call's edits). Not for add.
    pub n: Option<usize>,
    /// The text, for add and reword.
    pub text: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EditTodosArgs {
    /// Project name or slug. Defaults to the current Grove session's project.
    pub project: Option<String>,
    /// Edits, applied in order and saved together; one bad edit saves nothing.
    pub ops: Vec<TodoOpArg>,
}

impl TodoOpArg {
    fn to_op(&self) -> Result<crate::todo::Op, String> {
        use crate::todo::Op;
        let n = || self.n.ok_or_else(|| format!("{} needs `n`", self.op));
        let text = || self.text.clone().ok_or_else(|| format!("{} needs `text`", self.op));
        Ok(match self.op.as_str() {
            "add" => Op::Add(text()?),
            "check" => Op::Check(n()?),
            "uncheck" => Op::Uncheck(n()?),
            "reword" => Op::Reword(n()?, text()?),
            "remove" => Op::Remove(n()?),
            other => return Err(format!("unknown op \"{other}\"; use add, check, uncheck, reword or remove")),
        })
    }
}
```

In `impl GroveMcp`:

```rust
    pub fn do_get_todos(&self, project: Option<String>) -> Result<String, String> {
        let p = self.project(project)?;
        Ok(numbered(&crate::todo::parse(&self.store.read_todos(&p.slug()))))
    }

    pub fn do_edit_todos(&self, project: Option<String>, ops: Vec<TodoOpArg>) -> Result<String, String> {
        let p = self.project(project)?;
        let ops = ops.iter().map(TodoOpArg::to_op).collect::<Result<Vec<_>, _>>()?;
        let items = crate::todo::parse(&self.store.read_todos(&p.slug()));
        let new = crate::todo::apply_ops(&items, &ops)?;
        self.store
            .write_todos(&p.slug(), &crate::todo::serialize_items(&new))
            .map_err(|e| e.to_string())?;
        Ok(numbered(&new))
    }
```

Tool wrappers, in the `#[tool_router]` impl:

```rust
    #[tool(description = "A Grove project's to-do list, numbered.")]
    fn get_todos(&self, Parameters(a): Parameters<ProjectArg>) -> CallToolResult {
        result(self.do_get_todos(a.project))
    }

    #[tool(description = "Edit a Grove project's to-do list: add, check, uncheck, reword or remove items by number. Returns the new list.")]
    fn edit_todos(&self, Parameters(a): Parameters<EditTodosArgs>) -> CallToolResult {
        result(self.do_edit_todos(a.project, a.ops))
    }
```

- [ ] **Step 4: Run the tests to watch them pass**

Run: `cargo test --bin grove -- todo mcp 2>&1 | grep "test result"`
Expected: `ok`.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add -A src
git commit -m "feat(mcp): read and edit to-do lists

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Sessions: `list_sessions` and `read_session`

**Files:**
- Modify: `src/tmux.rs` (`capture`)
- Modify: `src/mcp.rs`, `src/mcp_tests.rs`

**Interfaces:**
- Consumes: `mcp_overview::{live, ago}` (Task 3); scope helpers (Task 1).
- Produces:
  - `Tmux::capture(&self, target: &str, lines: usize) -> Result<String>`:
    `capture-pane -p -J -t <target> -S -<lines>`.
  - `GroveMcp::session(&self, key: &str) -> Result<(Session, Project), String>`:
    matches an id, `slug:name`, or a bare name that's unique within the
    account.
  - `do_list_sessions(project: Option<String>)`, `do_read_session(session: String, lines: Option<usize>)`

- [ ] **Step 1: Write the failing tests**

`src/mcp_tests.rs`:

```rust
use crate::model::{Session, State};

fn add_session(t: &T, project: &str, name: &str, state: State, window: Option<&str>) -> Session {
    let now = chrono::Utc::now();
    let s = Session {
        id: format!("id-{project}-{name}"),
        project: project.into(),
        name: name.into(),
        host: "local".into(),
        tmux_window: window.map(str::to_string),
        state,
        message: None,
        created_at: now,
        updated_at: now,
    };
    t.store.save_session(&s).unwrap();
    s
}

#[test]
fn list_sessions_is_scoped_and_skips_ended() {
    let t = scoped("work", None);
    add_in(&t, "alpha", true);
    add_in(&t, "beta", false);
    add_session(&t, "alpha", "s1", State::Working, None);
    add_session(&t, "alpha", "old", State::Ended, None);
    add_session(&t, "beta", "s1", State::YourTurn, None);
    let out = t.do_list_sessions(None).unwrap();
    assert!(out.contains("alpha:s1") && out.contains("working"), "{out}");
    assert!(!out.contains("beta") && !out.contains("old"), "{out}");
}

#[test]
fn session_names_resolve_within_the_account() {
    let t = mcp(None);
    add(&t, "alpha");
    add(&t, "gamma");
    add_session(&t, "alpha", "s1", State::Working, None);
    add_session(&t, "gamma", "s1", State::Working, None);
    add_session(&t, "alpha", "fix", State::Working, None);
    assert_eq!(t.session("fix").unwrap().0.project, "alpha");
    assert_eq!(t.session("gamma:s1").unwrap().0.project, "gamma");
    assert_eq!(t.session("id-alpha-s1").unwrap().0.project, "alpha");
    let e = t.session("s1").unwrap_err();
    assert!(e.contains("alpha:s1") && e.contains("gamma:s1"), "{e}");
}

#[test]
fn another_accounts_session_is_unknown() {
    let t = scoped("work", None);
    add_in(&t, "alpha", true);
    add_in(&t, "beta", false);
    add_session(&t, "alpha", "s1", State::Working, None);
    add_session(&t, "beta", "secret", State::Working, None);
    let hidden = t.session("secret").unwrap_err();
    let missing = t.session("nope").unwrap_err();
    assert_eq!(hidden.replace("secret", "X"), missing.replace("nope", "X"));
    assert!(t.session("beta:secret").is_err());
    assert!(t.session("id-beta-secret").is_err());
    // A bare name shared with another account's session isn't ambiguous.
    add_session(&t, "beta", "s1", State::Working, None);
    assert_eq!(t.session("s1").unwrap().0.project, "alpha");
}

#[test]
fn read_session_shows_the_screen() {
    let t = mcp(None);
    add(&t, "alpha");
    let session = &t.cfg.tmux_session;
    t.tmux
        .new_session_detached(session, "w", t.dir.path(), &["sh".into(), "-c".into(), "printf 'hello\\nfrom alpha\\n'; sleep 30".into()])
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let window = t.tmux.list_windows_as(session, "#{window_id}").unwrap()[0].clone();
    add_session(&t, "alpha", "s1", State::YourTurn, Some(&window));
    let out = t.do_read_session("s1".into(), None);
    let gone = {
        add_session(&t, "alpha", "s2", State::YourTurn, None);
        t.do_read_session("s2".into(), None)
    };
    t.tmux.kill_server();
    let out = out.unwrap();
    assert!(out.contains("hello") && out.contains("from alpha"), "{out}");
    assert!(!out.ends_with("\n\n"), "{out:?}");
    assert!(gone.unwrap_err().contains("window"));
}
```

Check `list_windows_as`'s actual return in tmux.rs (it takes a format)
and that `new_session_detached` takes
`(name, window_name, cwd, argv)`. Adjust the call if the signature
differs.

- [ ] **Step 2: Run the tests to watch them fail**

Run: `cargo test --bin grove -- mcp 2>&1 | grep -E "^error" | head`
Expected: compile errors for `do_list_sessions`, `session` and `do_read_session`.

- [ ] **Step 3: Implement**

`src/tmux.rs`, in `impl Tmux`:

```rust
    /// The last `lines` lines of a pane's screen and scrollback, wrapped
    /// lines joined.
    pub fn capture(&self, target: &str, lines: usize) -> Result<String> {
        let start = format!("-{lines}");
        self.run(&["capture-pane", "-p", "-J", "-t", target, "-S", &start])
    }
```

Check what `self.run` returns. If it's `Result<String>` of stdout, this
is right. If it returns `Output`, convert with
`String::from_utf8_lossy(&o.stdout).into_owned()`.

`src/mcp.rs`, in `impl GroveMcp`:

```rust
    /// Live sessions of this account's projects (all, or one project's).
    fn sessions(&self, project: Option<&Project>) -> Result<Vec<(Session, Project)>, String> {
        let projects = match project {
            Some(p) => vec![p.clone()],
            None => self.projects()?,
        };
        let mut out: Vec<(Session, Project)> = projects
            .into_iter()
            .flat_map(|p| {
                mcp_overview::live(self.store.list_sessions(Some(&p.slug())))
                    .into_iter()
                    .map(move |s| (s, p.clone()))
            })
            .collect();
        out.sort_by(|a, b| b.0.updated_at.cmp(&a.0.updated_at));
        Ok(out)
    }

    /// A live session of this account by id, `project:name`, or a name
    /// unique within the account.
    fn session(&self, key: &str) -> Result<(Session, Project), String> {
        let all = self.sessions(None)?;
        let handle = |s: &Session, p: &Project| format!("{}:{}", p.slug(), s.name);
        let key = key.trim();
        if let Some(hit) = all.iter().find(|(s, p)| s.id == key || handle(s, p) == key) {
            return Ok(hit.clone());
        }
        let named: Vec<_> = all.iter().filter(|(s, _)| s.name == key).collect();
        match named.as_slice() {
            [one] => Ok((*one).clone()),
            [] => {
                let names: Vec<String> = all.iter().map(|(s, p)| handle(s, p)).collect();
                Err(format!(
                    "no live session \"{key}\"; sessions: {}",
                    if names.is_empty() { "(none)".into() } else { names.join(", ") }
                ))
            }
            many => Err(format!(
                "\"{key}\" is more than one session: {}; use one of those",
                many.iter().map(|(s, p)| handle(s, p)).collect::<Vec<_>>().join(", ")
            )),
        }
    }

    pub fn do_list_sessions(&self, project: Option<String>) -> Result<String, String> {
        let p = match project.filter(|p| !p.trim().is_empty()) {
            Some(name) => Some(self.project(Some(name))?),
            None => None,
        };
        let now = chrono::Utc::now();
        let lines: Vec<String> = self
            .sessions(p.as_ref())?
            .iter()
            .map(|(s, p)| {
                let msg = s.message.as_deref().map(|m| format!(" — {m}")).unwrap_or_default();
                format!("- {}:{} — {} — {}{msg}", p.slug(), s.name, s.state.label(), mcp_overview::ago(s.updated_at, now))
            })
            .collect();
        Ok(if lines.is_empty() { "No live sessions.".into() } else { lines.join("\n") })
    }

    pub fn do_read_session(&self, session: String, lines: Option<usize>) -> Result<String, String> {
        let (s, p) = self.session(&session)?;
        let n = lines.unwrap_or(60).clamp(1, 500);
        let window = s
            .tmux_window
            .as_deref()
            .ok_or_else(|| format!("{}:{} has no window (it may still be starting)", p.slug(), s.name))?;
        let screen = self
            .tmux
            .capture(window, n)
            .map_err(|_| format!("{}:{}'s window is gone", p.slug(), s.name))?;
        let kept: Vec<&str> = screen.trim_end().lines().collect();
        Ok(kept[kept.len().saturating_sub(n)..].join("\n"))
    }
```

`Session` must be imported: `use crate::model::{Project, Session};`.

Arg structs and wrappers:

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListSessionsArgs {
    /// Only this project's sessions (name or slug). All of them if left out.
    pub project: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadSessionArgs {
    /// The session: project:name as list_sessions shows it, a name, or an id.
    pub session: String,
    /// How many lines from the bottom of its screen (default 60, max 500).
    pub lines: Option<usize>,
}
```

```rust
    #[tool(description = "Live Claude sessions in Grove projects: state (working, needs you, your turn) and when each last moved.")]
    fn list_sessions(&self, Parameters(a): Parameters<ListSessionsArgs>) -> CallToolResult {
        result(self.do_list_sessions(a.project))
    }

    #[tool(description = "The bottom of a Grove session's screen: what it's doing or asking.")]
    fn read_session(&self, Parameters(a): Parameters<ReadSessionArgs>) -> CallToolResult {
        result(self.do_read_session(a.session, a.lines))
    }
```

- [ ] **Step 4: Run the tests to watch them pass**

Run: `cargo test --bin grove -- mcp 2>&1 | grep "test result"`
Expected: `ok`.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add -A src
git commit -m "feat(mcp): list sessions and read a session's screen

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Acting: `start_session` and `send_prompt`

**Files:**
- Modify: `src/launch.rs` (optional first prompt)
- Modify: `src/ui/run.rs` (caller passes `None`)
- Modify: `src/launch_tests.rs`
- Modify: `src/mcp.rs`, `src/mcp_tests.rs`

**Interfaces:**
- Consumes: `session`, `project` (Tasks 1 and 5); `Tmux::paste_text`, `send_keys`, `has_session`.
- Produces:
  - `launch::start(tmux, store, cfg, project, name, prompt: Option<String>) -> Result<Session>`
  - `launch::start_with_bin(tmux, store, cfg, project, name, prompt: Option<String>, bin)`:
    a `prompt` is the last argv element.
  - `GroveMcp::do_start_session(project, prompt, name)`,
    `do_start_session_with_bin(project, prompt, name, bin)` (tests),
    `do_send_prompt(session, text)`.

- [ ] **Step 1: Write the failing tests**

`src/launch_tests.rs`: in `start_opens_window_and_records_session`, pass
`Some("fix the build".into())` as the new `prompt` argument. The stub
records its args, so change it to:

```rust
    std::fs::write(&stub, format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nsleep 30\n", e.proj_dir.join("args").display())).unwrap();
```

After `kill_server`, add:

```rust
    let args = std::fs::read_to_string(e.proj_dir.join("args")).unwrap();
    assert_eq!(args.lines().last(), Some("fix the build"), "{args}");
```

(If the window hasn't run yet when it's read, poll for the file for up
to 2 seconds before killing the server.)

`src/mcp_tests.rs`:

```rust
#[test]
fn start_session_needs_grove_running() {
    let t = mcp(None);
    add(&t, "alpha");
    let e = t.do_start_session(Some("alpha".into()), None, None).unwrap_err();
    assert!(e.contains("Grove isn't running"), "{e}");
}

#[test]
fn start_session_of_another_account_is_refused() {
    let t = scoped("work", None);
    add_in(&t, "beta", false);
    assert!(t.do_start_session(Some("beta".into()), None, None).is_err());
}

#[test]
fn send_prompt_types_into_a_waiting_session_only() {
    let mut t = mcp(None);
    add(&t, "alpha");
    let session = t.cfg.tmux_session.clone();
    // `cat` echoes what's typed, standing in for Claude's input box.
    t.tmux.new_session_detached(&session, "w", t.dir.path(), &["cat".into()]).unwrap();
    let window = t.tmux.list_windows_as(&session, "#{window_id}").unwrap()[0].clone();
    add_session(&t, "alpha", "wait", State::YourTurn, Some(&window));
    add_session(&t, "alpha", "busy", State::Working, Some(&window));
    add_session(&t, "alpha", "ask", State::NeedsYou, Some(&window));
    let me = add_session(&t, "alpha", "me", State::YourTurn, Some(&window));
    let sent = t.do_send_prompt("wait".into(), "hello there".into());
    let busy = t.do_send_prompt("busy".into(), "x".into());
    let ask = t.do_send_prompt("ask".into(), "x".into());
    t.m.own_session = Some(me.id.clone());
    let own = t.do_send_prompt("me".into(), "x".into());
    std::thread::sleep(std::time::Duration::from_millis(300));
    let screen = t.tmux.capture(&window, 20).unwrap();
    t.tmux.kill_server();
    sent.unwrap();
    assert!(screen.contains("hello there"), "{screen}");
    assert!(busy.unwrap_err().contains("working"));
    assert!(ask.unwrap_err().contains("needs you"));
    assert!(own.unwrap_err().contains("own"));
}
```

- [ ] **Step 2: Run the tests to watch them fail**

Run: `cargo test --bin grove -- mcp launch 2>&1 | grep -E "^error" | head`
Expected: compile errors: wrong argument count for `start_with_bin`, and missing `do_start_session` and `do_send_prompt`.

- [ ] **Step 3: Implement**

`src/launch.rs`:
- `start` and `start_with_bin` gain `prompt: Option<String>` after `name`.
- `build_with_bin` gains `prompt: Option<&str>` and, after
  `argv.extend(cfg.claude_args...)`, does
  `if let Some(p) = prompt.filter(|p| !p.trim().is_empty()) { argv.push(p.to_string()); }`.
- `src/ui/run.rs:48` passes `None`.
- Fix every other `build_with_bin(` call in the tests by adding `None`.

`src/mcp.rs`:

```rust
    pub fn do_start_session(&self, project: Option<String>, prompt: Option<String>, name: Option<String>) -> Result<String, String> {
        self.do_start_session_with_bin(project, prompt, name, &crate::launch::claude_bin())
    }

    pub fn do_start_session_with_bin(
        &self,
        project: Option<String>,
        prompt: Option<String>,
        name: Option<String>,
        bin: &str,
    ) -> Result<String, String> {
        let p = self.project(project)?;
        if !self.tmux.has_session(&self.cfg.tmux_session) {
            return Err("Grove isn't running: start it with `grove` in a terminal".into());
        }
        let s = crate::launch::start_with_bin(&self.tmux, &self.store, &self.cfg, &p, name, prompt, bin)
            .map_err(|e| format!("{e:#}"))?;
        Ok(format!("Started {}:{} on the {} account.", p.slug(), s.name, self.cfg.account_for(&p.path).name))
    }

    pub fn do_send_prompt(&self, session: String, text: String) -> Result<String, String> {
        let (s, p) = self.session(&session)?;
        let handle = format!("{}:{}", p.slug(), s.name);
        if self.own_session.as_deref() == Some(s.id.as_str()) {
            return Err("that's this session's own window; just answer here".into());
        }
        match s.state {
            State::YourTurn => {}
            State::NeedsYou => return Err(format!("{handle} needs you: answer its prompt on the dashboard first")),
            other => return Err(format!("{handle} is {}; wait until it's your turn", other.label())),
        }
        if text.trim().is_empty() {
            return Err("nothing to send".into());
        }
        let window = s.tmux_window.as_deref().ok_or_else(|| format!("{handle} has no window"))?;
        self.tmux.paste_text(window, &text).map_err(|_| format!("{handle}'s window is gone"))?;
        // Trial and error (spec): Claude's input may need a beat after a paste.
        std::thread::sleep(std::time::Duration::from_millis(150));
        self.tmux.send_keys(window, &["Enter"]).map_err(|e| format!("{e:#}"))?;
        Ok(format!("Sent to {handle}."))
    }
```

Import `crate::model::State`.

Arg structs and wrappers:

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StartSessionArgs {
    /// Project name or slug. Defaults to the current Grove session's project.
    pub project: Option<String>,
    /// The new session's first message.
    pub prompt: Option<String>,
    /// A short name for it (default s1, s2, …).
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SendPromptArgs {
    /// The session: project:name as list_sessions shows it, a name, or an id.
    pub session: String,
    /// What to type into it. It must be waiting for you ("your turn").
    pub text: String,
}
```

```rust
    #[tool(description = "Start a new Claude session in a Grove project, on that project's account, optionally with a first prompt. It appears on the Grove dashboard.")]
    fn start_session(&self, Parameters(a): Parameters<StartSessionArgs>) -> CallToolResult {
        result(self.do_start_session(a.project, a.prompt, a.name))
    }

    #[tool(description = "Send a prompt to a Grove session that's waiting for its next message (state: your turn). Refused while it's working or needs you.")]
    fn send_prompt(&self, Parameters(a): Parameters<SendPromptArgs>) -> CallToolResult {
        result(self.do_send_prompt(a.session, a.text))
    }
```

- [ ] **Step 4: Run the tests to watch them pass**

Run: `cargo test 2>&1 | grep "test result"`
Expected: every suite `ok`.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add -A src
git commit -m "feat(mcp): start sessions and send prompts to waiting ones

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Doctor line, help, README

**Files:**
- Modify: `src/doctor.rs`, plus its tests (find them with `grep -n "mod tests\|#\[test\]" src/doctor.rs`)
- Modify: `README.md`

**Interfaces:**
- Consumes: `mcp::claude_json` (Task 2); `Config::all_accounts` (Task 1).
- Produces: `doctor::mcp_registered(claude_json: &Path) -> bool`, and one
  `mcp <account>` check per account: Ok `registered`, or Warn
  `not registered; grove mcp setup prints the command`.

- [ ] **Step 1: Write the failing tests**

In doctor's tests:

```rust
#[test]
fn mcp_registration_is_read_from_claude_json() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join(".claude.json");
    assert!(!mcp_registered(&f));
    std::fs::write(&f, r#"{"mcpServers":{"other":{}}}"#).unwrap();
    assert!(!mcp_registered(&f));
    std::fs::write(&f, r#"{"mcpServers":{"grove":{"command":"/b/grove","args":["mcp","--account","work"]}}}"#).unwrap();
    assert!(mcp_registered(&f));
    std::fs::write(&f, "not json").unwrap();
    assert!(!mcp_registered(&f));
}
```

- [ ] **Step 2: Run the test to watch it fail**

Run: `cargo test --bin grove mcp_registration 2>&1 | grep -E "^error" | head`
Expected: `cannot find function mcp_registered`.

- [ ] **Step 3: Implement**

`src/doctor.rs`:

```rust
/// Whether Claude's user-scope config at `path` has a `grove` MCP server.
pub fn mcp_registered(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .is_some_and(|v| v["mcpServers"]["grove"].is_object())
}

fn mcp(cfg: &Config, out: &mut Vec<Check>) {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else { return };
    for a in cfg.all_accounts() {
        let what = format!("mcp {}", a.name);
        if mcp_registered(&crate::mcp::claude_json(&a.dir, &home)) {
            out.push(check(Level::Ok, what, "registered"));
        } else {
            out.push(check(
                Level::Warn,
                what,
                "not registered, so only Grove's own sessions have its tools; grove mcp setup prints the command",
            ));
        }
    }
}
```

Call `mcp(&cfg, &mut out);` right after `accounts(&cfg, &mut out);` in
`run`. If an existing doctor test asserts an exact list of checks, add
the expected `mcp …` entries there.

`README.md`: add a section after "To-do lists":

````markdown
## Claude's view of your projects (MCP)

Every Claude Grove starts can see your projects through Grove's MCP
tools, limited to that session's account. A work Claude sees only
projects under the work folder; a personal one sees the rest.

| Tool | What Claude can do |
|---|---|
| `list_projects`, `get_project` | see each project: summary, sessions, to-dos, git branch and recent commits, README and CLAUDE.md |
| `list_sessions`, `read_session` | see which sessions are working, need you or wait for you, and read one's screen |
| `get_todos`, `edit_todos` | read and edit a project's to-do list |
| `update_project` | set a project's summary or color |
| `start_session` | start a session in a project, with a first prompt |
| `send_prompt` | send a prompt to a session that's waiting for you |

Claude asks before using a tool unless you've allowed it. Nothing answers
permission prompts for you.

To give the tools to every Claude session, including a plain `claude`
in a terminal, run `grove mcp setup` and paste the command it prints for
each account. Grove doesn't change Claude's settings itself.
`grove doctor` shows which accounts have it.
````

Add `grove mcp setup` to the README's command list, if it has one near
`grove doctor`. Add `src/mcp_overview.rs` to the file table in
"Extending it":
`| src/mcp_overview.rs | what the MCP tools say about a project: sessions, git, docs |`.
Update the `src/mcp.rs` row to "MCP tools, scoped to one Claude account".

- [ ] **Step 4: Run all the checks**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test 2>&1 | grep "test result"`
Expected: clippy clean; every suite `ok`.

- [ ] **Step 5: Commit, install, and do the live check**

```bash
git add -A
git commit -m "feat(mcp): doctor shows registration; README section

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
cargo install --path .
```

Live check (with the user):
1. Press `R` on the dashboard. In a new Grove session on each account,
   ask Claude to list projects. Each sees only its own account's.
2. Run `grove mcp setup`, then run the printed command for each account.
3. Open a plain `claude` in a terminal for each account (with that
   account's `CLAUDE_CONFIG_DIR`). It lists only that account's
   projects.
4. Start a session with a prompt from one Claude, and check it appears
   on the dashboard and starts on the prompt.
5. Send a prompt to a waiting session, and check that it submits (if it
   doesn't, tune the 150 ms pause in `do_send_prompt`).
6. Read a session's screen.
