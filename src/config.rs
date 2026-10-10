//! Grove settings (`config.toml`) and the on-disk layout under `GROVE_HOME`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Config {
    /// The default account: injected as CLAUDE_CONFIG_DIR into every session
    /// whose project doesn't name another account.
    pub claude_config_dir: String,
    /// The default account's name in the dashboard and status bar. Empty
    /// means one derived from the directory (`~/.claude-play` → `play`).
    pub account_name: String,
    #[serde(default)]
    pub account_color: Option<String>,
    /// Other Claude accounts, by name. A project picks one with `account`.
    pub accounts: BTreeMap<String, AccountConfig>,
    pub tmux_session: String,
    /// Empty means `$EDITOR`, then `vi`.
    pub editor: String,
    /// Extra arguments appended to every `claude` launch.
    pub claude_args: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            claude_config_dir: "~/.claude".into(),
            account_name: String::new(),
            account_color: None,
            accounts: BTreeMap::new(),
            tmux_session: "grove".into(),
            editor: String::new(),
            claude_args: Vec::new(),
        }
    }
}

/// `[accounts.<name>]` in config.toml.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountConfig {
    /// Its CLAUDE_CONFIG_DIR, e.g. `~/.claude-work`.
    pub dir: String,
    #[serde(default)]
    pub color: Option<String>,
}

/// A Claude account a session runs on.
#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub name: String,
    /// Expanded; not checked to exist (see `account_dir`).
    pub dir: PathBuf,
    pub color: Option<String>,
}

impl Config {
    pub fn default_account(&self) -> Account {
        let name = if self.account_name.is_empty() {
            derive_account_name(&self.claude_config_dir)
        } else {
            self.account_name.clone()
        };
        Account {
            name,
            dir: expand_tilde(&self.claude_config_dir),
            color: self.account_color.clone(),
        }
    }

    /// The account a project names, or the default for `None`.
    pub fn account(&self, name: Option<&str>) -> Result<Account> {
        let default = self.default_account();
        let Some(name) = name.filter(|n| *n != default.name) else {
            return Ok(default);
        };
        match self.accounts.get(name) {
            Some(a) => Ok(Account {
                name: name.to_string(),
                dir: expand_tilde(&a.dir),
                color: a.color.clone(),
            }),
            None => {
                let known: Vec<&str> = std::iter::once(default.name.as_str())
                    .chain(self.accounts.keys().map(String::as_str))
                    .collect();
                bail!(
                    "unknown account \"{name}\"; config.toml has: {}",
                    known.join(", ")
                )
            }
        }
    }
}

/// `~/.claude-work` → `work`; plain `~/.claude` → `default`.
fn derive_account_name(dir: &str) -> String {
    let base = dir.trim_end_matches('/').rsplit('/').next().unwrap_or(dir);
    let base = base.trim_start_matches('.');
    let name = base.strip_prefix("claude-").unwrap_or(base);
    if name.is_empty() || name == "claude" {
        "default".into()
    } else {
        name.to_string()
    }
}

#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
    pub projects: PathBuf,
    pub wiki: PathBuf,
    pub sessions: PathBuf,
    pub fincas: PathBuf,
    pub run: PathBuf,
    pub log: PathBuf,
}

impl Paths {
    pub fn new(home: PathBuf) -> Self {
        Self {
            projects: home.join("projects"),
            wiki: home.join("wiki"),
            sessions: home.join("sessions"),
            fincas: home.join("fincas"),
            run: home.join("run"),
            log: home.join("grove.log"),
            home,
        }
    }

    /// `$GROVE_HOME`, else `~/.grove`.
    pub fn from_env() -> Self {
        match std::env::var_os("GROVE_HOME") {
            Some(h) if !h.is_empty() => Self::new(PathBuf::from(h)),
            _ => Self::new(home_dir().join(".grove")),
        }
    }

    pub fn ensure(&self) -> Result<()> {
        for d in [
            &self.projects,
            &self.wiki,
            &self.sessions,
            &self.run,
            &self.fincas,
        ] {
            std::fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
        }
        Ok(())
    }

    pub fn config_file(&self) -> PathBuf {
        self.home.join("config.toml")
    }
}

pub fn load_config(paths: &Paths) -> Result<Config> {
    let file = paths.config_file();
    match std::fs::read_to_string(&file) {
        Ok(text) => toml::from_str(&text).with_context(|| format!("parsing {}", file.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", file.display())),
    }
}

pub fn expand_tilde(p: &str) -> PathBuf {
    if p == "~" {
        home_dir()
    } else if let Some(rest) = p.strip_prefix("~/") {
        home_dir().join(rest)
    } else {
        PathBuf::from(p)
    }
}

/// The default account's config dir. Errors if it doesn't exist.
pub fn claude_dir(cfg: &Config) -> Result<PathBuf> {
    account_dir(&cfg.default_account())
}

/// The account's config dir. Errors if it doesn't exist: Grove never lets
/// Claude fall back to some other account.
pub fn account_dir(a: &Account) -> Result<PathBuf> {
    if !a.dir.is_dir() {
        bail!(
            "Claude config dir {} for account \"{}\" does not exist (config.toml); refusing to launch on another account",
            a.dir.display(),
            a.name
        );
    }
    Ok(a.dir.clone())
}

pub fn editor(cfg: &Config) -> String {
    if !cfg.editor.is_empty() {
        return cfg.editor.clone();
    }
    std::env::var("EDITOR")
        .ok()
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| "vi".into())
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> String {
        std::env::var("HOME").unwrap()
    }

    #[test]
    fn defaults_point_at_claude_codes_own_dir() {
        let c = Config::default();
        assert_eq!(c.claude_config_dir, "~/.claude");
        assert_eq!(c.tmux_session, "grove");
        assert!(c.claude_args.is_empty());
    }

    #[test]
    fn missing_config_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path().to_path_buf());
        assert_eq!(load_config(&p).unwrap(), Config::default());
    }

    #[test]
    fn partial_config_merges_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path().to_path_buf());
        std::fs::write(p.config_file(), "tmux_session = \"g2\"\n").unwrap();
        let c = load_config(&p).unwrap();
        assert_eq!(c.tmux_session, "g2");
        assert_eq!(c.claude_config_dir, "~/.claude");
    }

    #[test]
    fn invalid_config_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path().to_path_buf());
        std::fs::write(p.config_file(), "tmux_session = [").unwrap();
        assert!(load_config(&p).is_err());
    }

    #[test]
    fn tilde_expands_to_home() {
        assert_eq!(
            expand_tilde("~/.claude-work"),
            PathBuf::from(home()).join(".claude-work")
        );
        assert_eq!(expand_tilde("~"), PathBuf::from(home()));
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
    }

    #[test]
    fn claude_dir_errors_when_missing() {
        let c = Config {
            claude_config_dir: "/definitely/not/here".into(),
            ..Config::default()
        };
        let err = claude_dir(&c).unwrap_err().to_string();
        assert!(err.contains("/definitely/not/here"), "{err}");
    }

    #[test]
    fn account_names_derive_from_the_dir() {
        assert_eq!(derive_account_name("~/.claude-play"), "play");
        assert_eq!(derive_account_name("~/.claude-work/"), "work");
        assert_eq!(derive_account_name("~/.claude"), "default");
        assert_eq!(derive_account_name("/srv/acme"), "acme");
    }

    #[test]
    fn accounts_from_config_toml() {
        let c: Config = toml::from_str(
            "claude_config_dir = \"/c/play\"\naccount_name = \"personal\"\n\
             [accounts.work]\ndir = \"/c/work\"\ncolor = \"#e8a33d\"\n",
        )
        .unwrap();
        let d = c.account(None).unwrap();
        assert_eq!(
            (d.name.as_str(), d.dir.to_str().unwrap()),
            ("personal", "/c/play")
        );
        assert_eq!(c.account(Some("personal")).unwrap(), d);
        let w = c.account(Some("work")).unwrap();
        assert_eq!(w.dir, PathBuf::from("/c/work"));
        assert_eq!(w.color.as_deref(), Some("#e8a33d"));
        let err = c.account(Some("wrok")).unwrap_err().to_string();
        assert!(
            err.contains("wrok") && err.contains("personal, work"),
            "{err}"
        );
    }

    #[test]
    fn account_dir_names_the_account_when_missing() {
        let a = Account {
            name: "work".into(),
            dir: "/definitely/not/here".into(),
            color: None,
        };
        let err = account_dir(&a).unwrap_err().to_string();
        assert!(
            err.contains("\"work\"") && err.contains("/definitely/not/here"),
            "{err}"
        );
    }

    #[test]
    fn claude_dir_ok_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let c = Config {
            claude_config_dir: dir.path().to_string_lossy().into(),
            ..Config::default()
        };
        assert_eq!(claude_dir(&c).unwrap(), dir.path());
    }

    #[test]
    fn paths_layout_under_home() {
        let p = Paths::new(PathBuf::from("/g"));
        assert_eq!(p.projects, PathBuf::from("/g/projects"));
        assert_eq!(p.wiki, PathBuf::from("/g/wiki"));
        assert_eq!(p.sessions, PathBuf::from("/g/sessions"));
        assert_eq!(p.run, PathBuf::from("/g/run"));
        assert_eq!(p.log, PathBuf::from("/g/grove.log"));
        assert_eq!(p.config_file(), PathBuf::from("/g/config.toml"));
    }

    #[test]
    fn ensure_creates_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path().join("nested home"));
        p.ensure().unwrap();
        for d in [&p.projects, &p.wiki, &p.sessions, &p.run] {
            assert!(d.is_dir(), "{d:?}");
        }
    }

    #[test]
    fn editor_prefers_config() {
        let c = Config {
            editor: "nvim".into(),
            ..Config::default()
        };
        assert_eq!(editor(&c), "nvim");
    }
}
