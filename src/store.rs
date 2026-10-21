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
                    .and_then(|p| {
                        // Sessions are keyed by the file's slug,
                        // so a renamed `name` would orphan them.
                        let stem = f.file_stem().unwrap_or_default().to_string_lossy();
                        if p.slug() == stem {
                            Ok(p)
                        } else {
                            Err(format!(
                                "name {:?} doesn't match file {stem}.toml; rename the file to {}.toml or change the name back",
                                p.name,
                                p.slug()
                            ))
                        }
                    })
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
    /// Deletes the project file. Its session files are left alone.
    pub fn remove_project(&self, name_or_slug: &str) -> Result<Project> {
        let p = self.get_project(name_or_slug)?;
        std::fs::remove_file(self.project_file(&p.slug()))?;
        Ok(p)
    }

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
        crate::stats::prune_old(&self.paths);
        match std::fs::remove_file(self.session_file(id)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    /// The project's to-do list (a Markdown checklist, see todo.rs).
    pub fn todos_file(&self, slug: &str) -> PathBuf {
        self.paths.todos.join(format!("{slug}.md"))
    }
    /// The to-do file's text; empty when there isn't one yet.
    pub fn read_todos(&self, slug: &str) -> String {
        std::fs::read_to_string(self.todos_file(slug)).unwrap_or_default()
    }
    pub fn write_todos(&self, slug: &str, text: &str) -> Result<()> {
        write_atomic(&self.todos_file(slug), text.as_bytes())
    }

    pub fn list_fincas(&self) -> Vec<Result<crate::finca::Finca, BadFile>> {
        files_with_ext(&self.paths.fincas, "toml")
            .into_iter()
            .map(|f| {
                std::fs::read_to_string(&f)
                    .map_err(|e| e.to_string())
                    .and_then(|t| {
                        toml::from_str::<crate::finca::Finca>(&t).map_err(|e| e.to_string())
                    })
                    .and_then(|x| {
                        let stem = f.file_stem().unwrap_or_default().to_string_lossy();
                        if x.slug() == stem {
                            Ok(x)
                        } else {
                            Err(format!("name {:?} doesn't match file {stem}.toml", x.name))
                        }
                    })
                    .map_err(|e| (f, e))
            })
            .collect()
    }
    pub fn get_finca(&self, name_or_slug: &str) -> Result<crate::finca::Finca> {
        let slug = slugify(name_or_slug);
        let all: Vec<_> = self
            .list_fincas()
            .into_iter()
            .filter_map(Result::ok)
            .collect();
        all.iter()
            .find(|f| f.slug() == slug)
            .cloned()
            .ok_or_else(|| {
                let names: Vec<_> = all.iter().map(|f| f.name.as_str()).collect();
                anyhow!(
                    "no finca named {name_or_slug:?}; fincas: {}",
                    list_or_none(&names)
                )
            })
    }
    /// Writes the finca's file, replacing one with the same slug.
    pub fn save_finca(&self, f: &crate::finca::Finca) -> Result<()> {
        let slug = f.slug();
        if slug.is_empty() {
            bail!("finca name {:?} has no letters or digits", f.name);
        }
        write_atomic(
            &self.paths.fincas.join(format!("{slug}.toml")),
            toml::to_string_pretty(f)?.as_bytes(),
        )
    }

    pub fn add_finca(&self, f: &crate::finca::Finca) -> Result<()> {
        let slug = f.slug();
        if slug.is_empty() {
            bail!("finca name {:?} has no letters or digits", f.name);
        }
        let path = self.paths.fincas.join(format!("{slug}.toml"));
        if path.exists() {
            bail!("a finca named {slug:?} already exists");
        }
        write_atomic(&path, toml::to_string_pretty(f)?.as_bytes())
    }
    pub fn remove_finca(&self, name_or_slug: &str) -> Result<crate::finca::Finca> {
        let f = self.get_finca(name_or_slug)?;
        std::fs::remove_file(self.paths.fincas.join(format!("{}.toml", f.slug())))?;
        Ok(f)
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
