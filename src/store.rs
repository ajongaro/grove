//! The only module that touches disk. All writes are atomic.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use crate::config::Paths;
use crate::model::{Project, Session, slugify};

/// A project file that failed to parse: (path, error).
pub type BadFile = (PathBuf, String);

#[derive(Debug, Clone)]
pub struct Store {
    pub paths: Paths,
}

impl Store {
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }

    fn project_file(&self, slug: &str) -> PathBuf {
        self.paths.projects.join(format!("{slug}.toml"))
    }

    fn session_file(&self, id: &str) -> PathBuf {
        self.paths.sessions.join(format!("{id}.json"))
    }

    /// Every `projects/*.toml`, sorted by file name. Unparseable files are
    /// returned as `Err` so the dashboard can show them instead of crashing.
    pub fn list_projects(&self) -> Vec<Result<Project, BadFile>> {
        files_with_ext(&self.paths.projects, "toml")
            .into_iter()
            .map(|f| {
                std::fs::read_to_string(&f)
                    .map_err(|e| e.to_string())
                    .and_then(|t| toml::from_str::<Project>(&t).map_err(|e| e.to_string()))
                    .map_err(|e| (f, e))
            })
            .collect()
    }

    pub fn get_project(&self, name_or_slug: &str) -> Result<Project> {
        let slug = slugify(name_or_slug);
        let all: Vec<Project> = self
            .list_projects()
            .into_iter()
            .filter_map(Result::ok)
            .collect();
        all.iter()
            .find(|p| p.slug() == slug)
            .cloned()
            .ok_or_else(|| {
                let names: Vec<_> = all.iter().map(|p| p.name.as_str()).collect();
                anyhow!(
                    "no project named {name_or_slug:?}; projects: {}",
                    list_or_none(&names)
                )
            })
    }

    /// Registers a new project; errors if its slug is already taken.
    pub fn add_project(&self, p: &Project) -> Result<()> {
        let slug = p.slug();
        if slug.is_empty() {
            bail!("project name {:?} has no letters or digits", p.name);
        }
        if self.project_file(&slug).exists() {
            bail!("a project with slug {slug:?} already exists");
        }
        self.save_project(p)
    }

    pub fn save_project(&self, p: &Project) -> Result<()> {
        let text = toml::to_string_pretty(p)?;
        write_atomic(&self.project_file(&p.slug()), text.as_bytes())
    }

    /// Sessions (optionally for one project slug), oldest first.
    pub fn list_sessions(&self, project: Option<&str>) -> Vec<Session> {
        let mut out: Vec<Session> = files_with_ext(&self.paths.sessions, "json")
            .into_iter()
            .filter_map(|f| serde_json::from_str(&std::fs::read_to_string(f).ok()?).ok())
            .filter(|s: &Session| project.is_none_or(|p| s.project == p))
            .collect();
        out.sort_by_key(|s| s.created_at);
        out
    }

    pub fn get_session(&self, id: &str) -> Option<Session> {
        serde_json::from_str(&std::fs::read_to_string(self.session_file(id)).ok()?).ok()
    }

    pub fn save_session(&self, s: &Session) -> Result<()> {
        let text = serde_json::to_string_pretty(s)?;
        write_atomic(&self.session_file(&s.id), text.as_bytes())
    }

    pub fn delete_session(&self, id: &str) -> Result<()> {
        match std::fs::remove_file(self.session_file(id)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    /// (title, path) for each wiki section, sorted by title. The title is the
    /// file's first `# ` heading, else its file stem.
    pub fn wiki_sections(&self, slug: &str) -> Vec<(String, PathBuf)> {
        let mut out: Vec<_> = files_with_ext(&self.paths.wiki.join(slug), "md")
            .into_iter()
            .map(|f| {
                let title = std::fs::read_to_string(&f)
                    .ok()
                    .and_then(|t| {
                        t.lines()
                            .find_map(|l| l.strip_prefix("# ").map(|h| h.trim().to_string()))
                    })
                    .unwrap_or_else(|| {
                        f.file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    });
                (title, f)
            })
            .collect();
        out.sort_by_key(|(title, _)| title.to_lowercase());
        out
    }

    /// One section (matched by title, case-insensitively) or, with `None`,
    /// every section joined in title order.
    pub fn read_wiki(&self, slug: &str, section: Option<&str>) -> Result<String> {
        let sections = self.wiki_sections(slug);
        let read = |p: &Path| {
            std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))
        };
        match section {
            None => Ok(sections
                .iter()
                .map(|(_, p)| read(p))
                .collect::<Result<Vec<_>>>()?
                .join("\n")),
            Some(want) => {
                let want_slug = slugify(want);
                match sections.iter().find(|(t, _)| slugify(t) == want_slug) {
                    Some((_, p)) => read(p),
                    None => {
                        let titles: Vec<_> = sections.iter().map(|(t, _)| t.as_str()).collect();
                        bail!(
                            "no wiki section {want:?}; sections: {}",
                            list_or_none(&titles)
                        )
                    }
                }
            }
        }
    }

    /// Creates or replaces `wiki/<slug>/<slugified title>.md`. The file always
    /// starts with `# <title>`; a matching heading in `content` isn't doubled.
    pub fn write_wiki(&self, slug: &str, title: &str, content: &str) -> Result<PathBuf> {
        let file_slug = slugify(title);
        if file_slug.is_empty() {
            bail!("wiki title {title:?} has no letters or digits");
        }
        let heading = format!("# {}", title.trim());
        let body = content.trim();
        let body = body
            .strip_prefix(&heading)
            .map(str::trim_start)
            .unwrap_or(body);
        let text = format!("{heading}\n\n{body}\n");
        let path = self.paths.wiki.join(slug).join(format!("{file_slug}.md"));
        write_atomic(&path, text.as_bytes())?;
        Ok(path)
    }
}

/// Writes to a temp file in the same directory, then renames over `path`.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent", path.display()))?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))
}

fn files_with_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|e| e == ext)
                && !p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .starts_with('.')
        })
        .collect();
    out.sort();
    out
}

fn list_or_none(items: &[&str]) -> String {
    if items.is_empty() {
        "(none)".into()
    } else {
        items.join(", ")
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
