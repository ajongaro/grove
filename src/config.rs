//! Grove settings (`config.toml`) and the on-disk layout under `GROVE_HOME`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
    /// Desktop alerts from the dashboard when a session needs you (here or
    /// on a finca). Turn off on a finca nobody sits at.
    pub alerts: bool,
    /// Grove's status line in its sessions (statusline.rs). Off leaves your
    /// own Claude Code status line.
    pub statusline: bool,
    /// The dashboard's stock ticker (ticker.rs). Off without a Finnhub key.
    pub ticker: TickerConfig,
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
            alerts: true,
            statusline: true,
            ticker: TickerConfig::default(),
        }
    }
}

/// `[ticker]` in config.toml.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TickerConfig {
    /// A Finnhub API key (finnhub.io, free). Empty means `$FINNHUB_API_KEY`;
    /// with neither, there's no ticker.
    pub key: String,
    /// Shown first on every page, change only. SPY, DIA and QQQ show as
    /// S&P, Dow and Nasdaq, whose ETFs they are.
    pub indexes: Vec<String>,
    /// The rest, a page at a time.
    pub symbols: Vec<String>,
}

impl Default for TickerConfig {
    fn default() -> Self {
        Self {
            key: String::new(),
            indexes: ["SPY", "DIA", "QQQ"].map(String::from).to_vec(),
            symbols: Vec::new(),
        }
    }
}

impl TickerConfig {
    /// The key to fetch with, if any: the config's, else `$FINNHUB_API_KEY`.
    pub fn key(&self) -> Option<String> {
        self.key_or(std::env::var("FINNHUB_API_KEY").ok())
    }

    fn key_or(&self, env: Option<String>) -> Option<String> {
        Some(self.key.trim().to_string())
            .filter(|k| !k.is_empty())
            .or_else(|| env.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()))
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
    /// Projects under this folder run on this account, e.g. `~/Work`.
    /// Projects under no account's folder run on the default account.
    #[serde(default)]
    pub folder: Option<String>,
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

    /// The account a project at `path` runs on: the one whose `folder`
    /// contains it, else the default.
    pub fn account_for(&self, path: &Path) -> Account {
        let path = resolve(path);
        self.accounts
            .iter()
            .find(|(_, a)| {
                a.folder
                    .as_deref()
                    .is_some_and(|f| folder_contains(&resolve(&expand_tilde(f)), &path))
            })
            .map(|(name, a)| Account {
                name: name.clone(),
                dir: expand_tilde(&a.dir),
                color: a.color.clone(),
            })
            .unwrap_or_else(|| self.default_account())
    }

    /// Every account: the default first, then the named ones.
    pub fn all_accounts(&self) -> Vec<Account> {
        std::iter::once(None)
            .chain(self.accounts.keys().map(|k| Some(k.as_str())))
            .filter_map(|n| self.account(n).ok())
            .collect()
    }

    /// An account by name, or the default for `None`.
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

/// Symlinks resolved, so a folder and a project path compare however each
/// was written; as given when it doesn't exist.
fn resolve(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// Whether `path` is `folder` or somewhere inside it.
/// Compares whole components (`~/Workshop` isn't in `~/Work`), ignoring case
/// on macOS, where the filesystem does.
fn folder_contains(folder: &Path, path: &Path) -> bool {
    if cfg!(target_os = "macos") {
        let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
        lower(path).starts_with(lower(folder))
    } else {
        path.starts_with(folder)
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
    pub sessions: PathBuf,
    pub fincas: PathBuf,
    pub todos: PathBuf,
    pub run: PathBuf,
    /// What each session's status line recorded (stats.rs).
    pub stats: PathBuf,
    pub log: PathBuf,
}

impl Paths {
    pub fn new(home: PathBuf) -> Self {
        Self {
            projects: home.join("projects"),
            sessions: home.join("sessions"),
            fincas: home.join("fincas"),
            todos: home.join("todos"),
            run: home.join("run"),
            stats: home.join("stats"),
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
            &self.sessions,
            &self.run,
            &self.stats,
            &self.fincas,
            &self.todos,
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
    fn statusline_defaults_on() {
        assert!(Config::default().statusline);
        let c: Config = toml::from_str("statusline = false").unwrap();
        assert!(!c.statusline);
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
    fn ticker_is_off_without_a_key() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.ticker.key_or(None), None);
        assert_eq!(cfg.ticker.indexes, ["SPY", "DIA", "QQQ"]);
        assert!(cfg.ticker.symbols.is_empty());
        // The environment's key will do.
        assert_eq!(
            cfg.ticker.key_or(Some(" k1 ".into())).as_deref(),
            Some("k1")
        );
        assert_eq!(cfg.ticker.key_or(Some("".into())), None);
        let cfg: Config = toml::from_str(
            "[ticker]\nkey = \"k2\"\nsymbols = [\"AAPL\", \"BRK.B\"]\nindexes = []\n",
        )
        .unwrap();
        // The config's wins.
        assert_eq!(cfg.ticker.key_or(Some("k1".into())).as_deref(), Some("k2"));
        assert_eq!(cfg.ticker.symbols, ["AAPL", "BRK.B"]);
        assert!(cfg.ticker.indexes.is_empty());
        assert!(toml::from_str::<Config>("[ticker]\nsymbol = []\n").is_err());
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

    fn work_and_play() -> Config {
        toml::from_str(
            "claude_config_dir = \"/c/play\"\n\
             [accounts.work]\ndir = \"/c/work\"\nfolder = \"/u/Work\"\n",
        )
        .unwrap()
    }

    #[test]
    fn a_project_under_an_accounts_folder_uses_that_account() {
        let c = work_and_play();
        assert_eq!(c.account_for(Path::new("/u/Work/acme")).name, "work");
        assert_eq!(c.account_for(Path::new("/u/Work/a/b/c")).name, "work");
        assert_eq!(c.account_for(Path::new("/u/Work")).name, "work");
    }

    #[test]
    fn a_project_anywhere_else_uses_the_default_account() {
        let c = work_and_play();
        assert_eq!(c.account_for(Path::new("/u/Play/grove")).name, "play");
        assert_eq!(c.account_for(Path::new("/tmp/x")).name, "play");
        // A sibling that merely starts with the same letters isn't inside.
        assert_eq!(c.account_for(Path::new("/u/Workshop/x")).name, "play");
    }

    #[test]
    fn folder_match_ignores_a_trailing_slash() {
        let c: Config = toml::from_str(
            "claude_config_dir = \"/c/play\"\n\
             [accounts.work]\ndir = \"/c/work\"\nfolder = \"/u/Work/\"\n",
        )
        .unwrap();
        assert_eq!(c.account_for(Path::new("/u/Work/acme")).name, "work");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn folder_match_ignores_case_on_macos() {
        // APFS is case-insensitive: ~/work/acme is the same folder as ~/Work/acme.
        let c = work_and_play();
        assert_eq!(c.account_for(Path::new("/u/work/acme")).name, "work");
    }

    #[test]
    fn folder_reached_through_a_symlink_still_matches() {
        // Project paths are stored canonicalized (`grove add`), so a folder
        // behind a symlink must be resolved too.
        let d = tempfile::tempdir().unwrap();
        let real = d.path().canonicalize().unwrap().join("real-work");
        std::fs::create_dir_all(real.join("acme")).unwrap();
        let link = d.path().join("Work");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let c: Config = toml::from_str(&format!(
            "claude_config_dir = \"/c/play\"\n\
             [accounts.work]\ndir = \"/c/work\"\nfolder = \"{}\"\n",
            link.display()
        ))
        .unwrap();
        assert_eq!(c.account_for(&real.join("acme")).name, "work");
    }

    #[test]
    fn without_folders_everything_uses_the_default_account() {
        // A finca: no account names a folder.
        let c: Config = toml::from_str("claude_config_dir = \"/c/play\"\n").unwrap();
        assert_eq!(c.account_for(Path::new("/u/Work/acme")).name, "play");
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
        assert_eq!(p.sessions, PathBuf::from("/g/sessions"));
        assert_eq!(p.run, PathBuf::from("/g/run"));
        assert_eq!(p.todos, PathBuf::from("/g/todos"));
        assert_eq!(p.log, PathBuf::from("/g/grove.log"));
        assert_eq!(p.config_file(), PathBuf::from("/g/config.toml"));
    }

    #[test]
    fn ensure_creates_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path().join("nested home"));
        p.ensure().unwrap();
        for d in [&p.projects, &p.sessions, &p.run, &p.stats, &p.todos] {
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
