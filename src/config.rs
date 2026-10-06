//! Grove settings (`config.toml`) and the on-disk layout under `GROVE_HOME`.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Injected as CLAUDE_CONFIG_DIR into every session Grove launches.
    pub claude_config_dir: String,
    pub tmux_session: String,
    /// Empty means `$EDITOR`, then `vi`.
    pub editor: String,
    /// Extra arguments appended to every `claude` launch.
    pub claude_args: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            claude_config_dir: "~/.claude-play".into(),
            tmux_session: "grove".into(),
            editor: String::new(),
            claude_args: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
    pub projects: PathBuf,
    pub wiki: PathBuf,
    pub sessions: PathBuf,
    pub run: PathBuf,
    pub log: PathBuf,
}

impl Paths {
    pub fn new(home: PathBuf) -> Self {
        Self {
            projects: home.join("projects"),
            wiki: home.join("wiki"),
            sessions: home.join("sessions"),
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
        for d in [&self.projects, &self.wiki, &self.sessions, &self.run] {
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

/// The expanded Claude config dir. Errors if it doesn't exist: Grove never
/// falls back to the default `~/.claude` account.
pub fn claude_dir(cfg: &Config) -> Result<PathBuf> {
    let dir = expand_tilde(&cfg.claude_config_dir);
    if !dir.is_dir() {
        bail!(
            "Claude config dir {} does not exist (claude_config_dir in config.toml); refusing to launch on another account",
            dir.display()
        );
    }
    Ok(dir)
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
    fn defaults_point_at_claude_play() {
        let c = Config::default();
        assert_eq!(c.claude_config_dir, "~/.claude-play");
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
        assert_eq!(c.claude_config_dir, "~/.claude-play");
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
            expand_tilde("~/.claude-play"),
            PathBuf::from(home()).join(".claude-play")
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
