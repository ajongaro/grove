//! A conversation's title, for finding sessions: Claude's own `ai-title`
//! from its transcript, else its first prompt. Transcripts can run to many
//! megabytes, so the answer is cached under `run/titles/`: a title for good,
//! a first prompt (or nothing) until the transcript grows, looked at again
//! at most every few minutes.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::config::{Config, Paths, account_dir};
use crate::model::Session;

/// How long a title-less answer stands before the transcript is read again.
const RECHECK: Duration = Duration::from_secs(300);
/// Titles are cut to this many characters.
const LONGEST: usize = 120;

/// Where Claude saved conversation `id` under account folder `account`:
/// `projects/<folder>/<id>.jsonl`.
pub fn transcript(account: &Path, id: &str) -> Option<PathBuf> {
    let file = format!("{id}.jsonl");
    std::fs::read_dir(account.join("projects"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|d| d.path().join(&file))
        .find(|p| p.is_file())
}

/// The title of local session `s` in the project at `project_path`.
pub fn for_session(
    paths: &Paths,
    cfg: &Config,
    project_path: &Path,
    s: &Session,
) -> Option<String> {
    let account = account_dir(&cfg.account_for(project_path)).ok()?;
    title(paths, &account, s.conversation_id())
}

/// Conversation `id`'s title, from the cache when it can be.
pub fn title(paths: &Paths, account: &Path, id: &str) -> Option<String> {
    let cache = paths.run.join("titles").join(id);
    let cached = std::fs::read_to_string(&cache).ok();
    if let Some(t) = cached.as_deref().and_then(|c| c.strip_prefix("t:")) {
        return Some(t.to_string());
    }
    let prompt = cached
        .as_deref()
        .and_then(|c| c.strip_prefix("p:"))
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    if cached.is_some() {
        let age = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let looked = age(&cache).unwrap_or(SystemTime::UNIX_EPOCH);
        let fresh = SystemTime::now()
            .duration_since(looked)
            .is_ok_and(|d| d < RECHECK);
        let grown = transcript(account, id)
            .and_then(|t| age(&t))
            .is_some_and(|m| m > looked);
        if fresh || !grown {
            return prompt;
        }
    }
    let found = transcript(account, id)
        .map(|t| read(&t))
        .unwrap_or_default();
    let line = match &found {
        Found::Title(t) => format!("t:{t}"),
        Found::Prompt(p) => format!("p:{p}"),
        Found::Nothing => "p:".to_string(),
    };
    if let Some(dir) = cache.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = crate::store::write_atomic(&cache, line.as_bytes());
    match found {
        Found::Title(t) | Found::Prompt(t) => Some(t),
        Found::Nothing => None,
    }
}

#[derive(Debug, Default, PartialEq)]
enum Found {
    Title(String),
    Prompt(String),
    #[default]
    Nothing,
}

/// The transcript's first `ai-title`, else its first prompt typed by a
/// person. Stops at the title, which comes early.
fn read(path: &Path) -> Found {
    let Ok(file) = std::fs::File::open(path) else {
        return Found::Nothing;
    };
    let mut prompt = None;
    for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
        if line.contains("\"ai-title\"") {
            let v: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
            if let Some(t) = v["aiTitle"].as_str().and_then(clean) {
                return Found::Title(t);
            }
        } else if prompt.is_none() && line.contains("\"type\":\"user\"") {
            let v: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
            prompt = typed(&v["message"]["content"]).and_then(clean);
        }
    }
    prompt.map_or(Found::Nothing, Found::Prompt)
}

/// What a person typed in a user message: a string, or the first text of a
/// list (a prompt with images); a list with tool results is Claude's own
/// work. Slash commands and system notes start with "<".
fn typed(content: &serde_json::Value) -> Option<&str> {
    let text = match content {
        serde_json::Value::String(s) => s.as_str(),
        serde_json::Value::Array(blocks) => {
            if blocks.iter().any(|b| b["type"] == "tool_result") {
                return None;
            }
            blocks.iter().find(|b| b["type"] == "text")?["text"].as_str()?
        }
        _ => return None,
    };
    (!text.trim_start().starts_with('<')).then_some(text)
}

/// One line, trimmed and cut to `LONGEST`; None when empty.
fn clean(s: &str) -> Option<String> {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.is_empty() {
        return None;
    }
    let mut out: String = one.chars().take(LONGEST).collect();
    if one.chars().count() > LONGEST {
        out.pop();
        out.push('…');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct T {
        _d: tempfile::TempDir,
        paths: Paths,
        account: PathBuf,
    }

    fn t() -> T {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path().join("grove"));
        paths.ensure().unwrap();
        let account = d.path().join("claude");
        std::fs::create_dir_all(account.join("projects/-x")).unwrap();
        T {
            _d: d,
            paths,
            account,
        }
    }

    fn write(t: &T, id: &str, lines: &[serde_json::Value]) {
        let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(t.account.join(format!("projects/-x/{id}.jsonl")), body).unwrap();
    }

    fn user(text: &str) -> serde_json::Value {
        serde_json::json!({"type":"user","message":{"role":"user","content":text}})
    }

    #[test]
    fn claudes_title_wins() {
        let t = t();
        write(
            &t,
            "c1",
            &[
                user("<command-name>/clear</command-name>"),
                user("build the GA schema"),
                serde_json::json!({"type":"ai-title","aiTitle":"GA schema for iOS and Web"}),
            ],
        );
        assert_eq!(
            title(&t.paths, &t.account, "c1").as_deref(),
            Some("GA schema for iOS and Web")
        );
        // Cached for good: the transcript isn't needed again.
        std::fs::remove_file(t.account.join("projects/-x/c1.jsonl")).unwrap();
        assert_eq!(
            title(&t.paths, &t.account, "c1").as_deref(),
            Some("GA schema for iOS and Web")
        );
    }

    #[test]
    fn first_typed_prompt_until_there_is_a_title() {
        let t = t();
        let tool =
            serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result"}]}});
        write(
            &t,
            "c2",
            &[tool, user("  fix the\nkeno   flow  "), user("and more")],
        );
        assert_eq!(
            title(&t.paths, &t.account, "c2").as_deref(),
            Some("fix the keno flow")
        );
    }

    #[test]
    fn a_prompt_with_images_still_counts() {
        let t = t();
        let with_image = serde_json::json!({"type":"user","message":{"content":[
            {"type":"text","text":"can we do something about this toolbar"},
            {"type":"image","source":{}}
        ]}});
        write(&t, "c4", &[with_image, user("a later prompt")]);
        assert_eq!(
            title(&t.paths, &t.account, "c4").as_deref(),
            Some("can we do something about this toolbar")
        );
    }

    #[test]
    fn no_transcript_no_title() {
        let t = t();
        assert_eq!(title(&t.paths, &t.account, "nope"), None);
        assert!(transcript(&t.account, "nope").is_none());
    }

    #[test]
    fn long_titles_are_cut() {
        let t = t();
        write(&t, "c3", &[user(&"word ".repeat(100))]);
        let got = title(&t.paths, &t.account, "c3").unwrap();
        assert_eq!(got.chars().count(), LONGEST);
        assert!(got.ends_with('…'));
    }
}
