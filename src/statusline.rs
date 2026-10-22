//! `grove statusline`: Claude Code's status line for Grove's sessions. Reads
//! the status JSON on stdin, prints a coloured line (two when the terminal
//! is too narrow for one), and records the
//! session's stats (stats.rs). Must never fail or block.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};

use crate::config::{Paths, load_config};
use crate::log;
use crate::stats::{self, StatusInput};
use crate::store::{Store, write_atomic};

const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const DIM: &str = "\x1b[90m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

/// Columns kept free beside the line: Claude's padding of 2, plus slack for
/// emoji a terminal draws wider than expected.
const MARGIN: usize = 4;
/// Names shorten no further than this.
const SHORTEST: usize = 8;

/// How long a directory's git summary is reused.
const GIT_CACHE_FOR: Duration = Duration::from_secs(5);

/// What the line shows beyond Claude's JSON. A None drops its segment.
pub struct Context {
    /// Grove account name and its `#rrggbb` colour.
    pub account: Option<(String, Option<String>)>,
    /// `project:session` for Grove's sessions, else the folder's name.
    pub place: String,
    pub git: Option<GitInfo>,
    /// The terminal's width (Claude sets `COLUMNS`); None never wraps.
    pub columns: Option<usize>,
}

pub struct GitInfo {
    pub branch: String,
    pub staged: usize,
    pub modified: usize,
}

/// Who and where on the first line, the numbers after; when the terminal
/// is too narrow for one line the numbers move to a second, and long
/// folder and branch names shorten until the first fits.
pub fn render(input: &StatusInput, ctx: &Context) -> String {
    let sep = format!(" {DIM}|{RESET} ");
    let numbers = numbers(input);
    let mut caps = (usize::MAX, usize::MAX);
    let mut who = identity(input, ctx, caps);
    let one = [who.clone(), numbers.clone()].concat().join(&sep);
    let Some(room) = ctx.columns.map(|c| c.saturating_sub(MARGIN)) else {
        return one;
    };
    if width(&one) <= room {
        return one;
    }
    let place = ctx.place.chars().count();
    let branch = ctx.git.as_ref().map_or(0, |g| g.branch.chars().count());
    while width(&who.join(&sep)) > room {
        // Take a character off the longer of the two names.
        let (p, b) = (caps.0.min(place), caps.1.min(branch));
        if p.max(b) <= SHORTEST {
            break;
        }
        caps = if p >= b { (p - 1, b) } else { (p, b - 1) };
        who = identity(input, ctx, caps);
    }
    format!("{}\n{}", who.join(&sep), numbers.join(&sep))
}

/// `s` cut to `max` characters, ending in "…" when cut.
fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Display width of `s`, colour codes aside.
fn width(s: &str) -> usize {
    let mut plain = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            chars.by_ref().find(|&c| c == 'm');
        } else {
            plain.push(c);
        }
    }
    ratatui::text::Span::raw(plain).width()
}

/// Model, account, place and git, with the place and branch cut to `caps`.
fn identity(input: &StatusInput, ctx: &Context, caps: (usize, usize)) -> Vec<String> {
    let mut segs: Vec<String> = Vec::new();
    if let Some(m) = &input.model
        && let Some(name) = m.display_name.as_ref().or(m.id.as_ref())
    {
        segs.push(format!("🤖 {CYAN}{name}{RESET}"));
    }
    if let Some((name, color)) = &ctx.account {
        let style = match color.as_deref().and_then(rgb) {
            Some((r, g, b)) => format!("\x1b[1;38;2;{r};{g};{b}m"),
            None => format!("{BOLD}{GREEN}"),
        };
        segs.push(format!("{style}{}{RESET}", name.to_uppercase()));
    }
    if let Some(agent) = input.agent.as_ref().and_then(|a| a.name.as_ref()) {
        let tag = format!("{DIM}[{YELLOW}{agent}{DIM}]{RESET}");
        match segs.last_mut() {
            Some(last) => *last = format!("{last} {tag}"),
            None => segs.push(tag),
        }
    }
    segs.push(format!("📁 {BOLD}{}{RESET}", shorten(&ctx.place, caps.0)));
    if let Some(g) = &ctx.git {
        let mut s = format!("🌿 {}", shorten(&g.branch, caps.1));
        if g.staged > 0 {
            s.push_str(&format!(" {GREEN}+{}{RESET}", g.staged));
        }
        if g.modified > 0 {
            s.push_str(&format!(" {YELLOW}~{}{RESET}", g.modified));
        }
        segs.push(s);
    }
    segs
}

/// Context, cost, time, lines changed and the five-hour limit.
fn numbers(input: &StatusInput) -> Vec<String> {
    let mut segs = Vec::new();
    let pct = input
        .context_window
        .as_ref()
        .and_then(|c| c.used_percentage)
        .unwrap_or(0.0)
        .clamp(0.0, 100.0) as usize;
    let color = match pct {
        90.. => RED,
        70.. => YELLOW,
        _ => GREEN,
    };
    let filled = pct * 10 / 100;
    segs.push(format!(
        "🧠 {color}{}{DIM}{}{RESET} {color}{pct}%{RESET}",
        "▓".repeat(filled),
        "░".repeat(10 - filled)
    ));
    let cost = input.cost.as_ref();
    segs.push(format!(
        "💰 ${:.2}",
        cost.and_then(|c| c.total_cost_usd).unwrap_or(0.0)
    ));
    let secs = cost.and_then(|c| c.total_duration_ms).unwrap_or(0) / 1000;
    segs.push(if secs >= 60 {
        format!("⏱️ {}m {}s", secs / 60, secs % 60)
    } else {
        format!("⏱️ {secs}s")
    });
    segs.push(format!(
        "✏️ {GREEN}+{}{RESET} {RED}-{}{RESET}",
        cost.and_then(|c| c.total_lines_added).unwrap_or(0),
        cost.and_then(|c| c.total_lines_removed).unwrap_or(0)
    ));
    if let Some(p) = input
        .rate_limits
        .as_ref()
        .and_then(|r| r.five_hour.as_ref())
        .and_then(|w| w.used_percentage)
    {
        segs.push(format!("📊 {}%", p.round() as i64));
    }
    segs
}

fn rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    Some((v(0)?, v(2)?, v(4)?))
}

/// The status line for `stdin`, recording stats when it's a Grove session.
pub fn run(paths: &Paths, stdin: &str) -> String {
    run_with(
        paths,
        stdin,
        std::env::var("GROVE_SESSION").ok(),
        std::env::var("GROVE_ACCOUNT").ok(),
    )
}

fn run_with(
    paths: &Paths,
    stdin: &str,
    grove_session: Option<String>,
    grove_account: Option<String>,
) -> String {
    let input: StatusInput = serde_json::from_str(stdin).unwrap_or_default();
    let store = Store::new(paths.clone());
    // Prefer the id Grove put on the window, as hooks do: /clear and
    // /resume start a new conversation id in the same window.
    let session = grove_session
        .and_then(|id| store.get_session(&id))
        .or_else(|| store.get_session(input.session_id.as_deref()?));
    let dir = input
        .workspace
        .as_ref()
        .and_then(|w| w.project_dir.clone().or(w.current_dir.clone()));
    let (place, account) = match &session {
        Some(s) => {
            let cfg = load_config(paths).ok();
            let name = grove_account
                .or_else(|| Some(cfg.as_ref()?.default_account().name))
                .unwrap_or_else(|| "default".into());
            let color = cfg
                .as_ref()
                .and_then(|c| c.account(Some(&name)).ok())
                .and_then(|a| a.color);
            if let Err(e) = stats::record(paths, &s.id, &name, &input) {
                log::error(paths, "statusline", &format!("recording {}: {e:#}", s.id));
            }
            // With one account there's nothing to tell apart.
            let several = cfg.as_ref().is_some_and(|c| !c.accounts.is_empty());
            (
                format!("{}:{}", s.project, s.name),
                several.then_some((name, color)),
            )
        }
        None => {
            let base = dir
                .as_deref()
                .and_then(|d| Path::new(d).file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "?".into());
            (base, None)
        }
    };
    let git = dir
        .as_deref()
        .and_then(|d| git_info(Path::new(d), &paths.run.join("git-cache")));
    render(
        &input,
        &Context {
            account,
            place,
            git,
            columns: std::env::var("COLUMNS").ok().and_then(|c| c.parse().ok()),
        },
    )
}

/// Branch and staged/modified file counts, reused for a few seconds per
/// directory: the status line runs after every message.
fn git_info(dir: &Path, cache_dir: &Path) -> Option<GitInfo> {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    dir.hash(&mut h);
    let cache = cache_dir.join(format!("{:x}", h.finish()));
    let fresh = std::fs::metadata(&cache)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < GIT_CACHE_FOR);
    let line = match fresh
        .then(|| std::fs::read_to_string(&cache).ok())
        .flatten()
    {
        Some(line) => line,
        None => {
            let line = git_summary(dir).unwrap_or_default();
            let _ = write_atomic(&cache, line.as_bytes());
            line
        }
    };
    let mut parts = line.trim().rsplitn(3, ':');
    let modified = parts.next()?.parse().ok()?;
    let staged = parts.next()?.parse().ok()?;
    let branch = parts.next()?.to_string();
    Some(GitInfo {
        branch,
        staged,
        modified,
    })
}

/// `branch:staged:modified`, or None outside a work tree.
fn git_summary(dir: &Path) -> Option<String> {
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    git(&["rev-parse", "--is-inside-work-tree"])?;
    let branch = git(&["symbolic-ref", "--short", "HEAD"])
        .or_else(|| git(&["rev-parse", "--short", "HEAD"]))
        .filter(|b| !b.is_empty())?;
    let count = |args: &[&str]| git(args).map_or(0, |s| s.lines().count());
    Some(format!(
        "{branch}:{}:{}",
        count(&["diff", "--cached", "--numstat"]),
        count(&["diff", "--numstat"])
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Session, State};
    use serde_json::json;

    /// The text without its colours.
    fn plain(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c in chars.by_ref() {
                    if c == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    fn parse(v: serde_json::Value) -> StatusInput {
        serde_json::from_value(v).unwrap()
    }

    fn ctx(account: Option<(&str, Option<&str>)>, place: &str, git: Option<GitInfo>) -> Context {
        Context {
            account: account.map(|(n, c)| (n.to_string(), c.map(str::to_string))),
            place: place.into(),
            git,
            columns: None,
        }
    }

    fn long() -> (StatusInput, Context) {
        let input = parse(json!({
            "model": { "display_name": "Opus 5.5" },
            "cost": { "total_cost_usd": 2.78, "total_duration_ms": 125_000 },
            "context_window": { "used_percentage": 15.0 },
            "rate_limits": { "five_hour": { "used_percentage": 4.0 } }
        }));
        let git = GitInfo {
            branch: "feature/mi-7803/club-keno-extras".into(),
            staged: 0,
            modified: 3,
        };
        let c = ctx(
            Some(("work", None)),
            "msl-gambyt-product-middleware:google-analytics",
            Some(git),
        );
        (input, c)
    }

    #[test]
    fn one_line_when_it_fits() {
        let (input, mut c) = long();
        c.columns = Some(300);
        let out = render(&input, &c);
        assert!(!out.contains('\n'), "{out}");
        assert!(plain(&out).contains("feature/mi-7803/club-keno-extras ~3 | 🧠"));
    }

    #[test]
    fn numbers_move_to_a_second_line_when_narrow() {
        let (input, mut c) = long();
        c.columns = Some(120);
        let out = plain(&render(&input, &c));
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2, "{out}");
        assert!(
            lines[0].starts_with("🤖 Opus 5.5 | WORK | 📁 msl-gambyt"),
            "{out}"
        );
        assert!(lines[0].contains("club-keno-extras"), "{out}");
        assert!(lines[1].starts_with("🧠 ▓░░"), "{out}");
        assert!(
            lines[1].contains("💰 $2.78") && lines[1].ends_with("📊 4%"),
            "{out}"
        );
        assert!(lines.iter().all(|l| width(l) <= 120 - MARGIN), "{out}");
    }

    #[test]
    fn long_names_shorten_to_fit() {
        let (input, mut c) = long();
        c.columns = Some(70);
        let out = plain(&render(&input, &c));
        let first = out.lines().next().unwrap();
        assert!(width(first) <= 70 - MARGIN, "{first}");
        assert!(
            first.contains('…') && first.contains("WORK") && first.contains("~3"),
            "{first}"
        );
        assert!(
            first.contains("📁 msl-") && first.contains("🌿 feat"),
            "{first}"
        );
    }

    fn with_ctx(pct: f64) -> String {
        render(
            &parse(json!({ "context_window": { "used_percentage": pct } })),
            &ctx(None, "x", None),
        )
    }

    #[test]
    fn renders_every_segment() {
        let input = parse(json!({
            "model": { "display_name": "Opus" },
            "cost": {
                "total_cost_usd": 1.234,
                "total_duration_ms": 125_000,
                "total_lines_added": 10,
                "total_lines_removed": 2
            },
            "context_window": { "used_percentage": 23.0 },
            "rate_limits": { "five_hour": { "used_percentage": 12.4 } }
        }));
        let git = GitInfo {
            branch: "main".into(),
            staged: 1,
            modified: 2,
        };
        let out = render(
            &input,
            &ctx(Some(("work", Some("#ff8800"))), "api:s1", Some(git)),
        );
        assert_eq!(
            plain(&out),
            "🤖 Opus | WORK | 📁 api:s1 | 🌿 main +1 ~2 | 🧠 ▓▓░░░░░░░░ 23% | 💰 $1.23 | ⏱️ 2m 5s | ✏️ +10 -2 | 📊 12%"
        );
        assert!(out.contains("38;2;255;136;0"), "{out:?}");
    }

    #[test]
    fn agent_follows_its_account_without_a_bar() {
        let input = parse(json!({ "agent": { "name": "reviewer" } }));
        let out = plain(&render(&input, &ctx(Some(("work", None)), "x", None)));
        assert!(out.starts_with("WORK [reviewer] | 📁 x"), "{out}");
    }

    #[test]
    fn missing_fields_drop_segments() {
        let out = render(&parse(json!({})), &ctx(None, "x", None));
        assert_eq!(
            plain(&out),
            "📁 x | 🧠 ░░░░░░░░░░ 0% | 💰 $0.00 | ⏱️ 0s | ✏️ +0 -0"
        );
    }

    #[test]
    fn context_colours() {
        assert!(with_ctx(69.0).contains("\x1b[32m69%"));
        assert!(with_ctx(70.0).contains("\x1b[33m70%"));
        assert!(with_ctx(95.0).contains("\x1b[31m95%"));
        assert!(plain(&with_ctx(150.0)).contains("▓▓▓▓▓▓▓▓▓▓ 100%"));
    }

    fn home() -> (tempfile::TempDir, Paths) {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::new(d.path().join("grove home"));
        p.ensure().unwrap();
        (d, p)
    }

    #[test]
    fn garbage_input_still_prints_a_line() {
        let (_d, p) = home();
        let out = run_with(&p, "not json", None, None);
        assert!(plain(&out).contains("📁"), "{out}");
    }

    #[test]
    fn non_grove_session_prints_and_records_nothing() {
        let (_d, p) = home();
        let stdin = json!({ "session_id": "nope", "workspace": { "project_dir": "/tmp/foo" } });
        let out = run_with(&p, &stdin.to_string(), None, None);
        assert!(plain(&out).contains("📁 foo"), "{out}");
        assert!(crate::stats::load_all(&p).is_empty());
    }

    fn save(p: &Paths, id: &str) {
        let now = chrono::Utc::now();
        Store::new(p.clone())
            .save_session(&Session {
                id: id.into(),
                project: "api".into(),
                name: "s2".into(),
                host: "local".into(),
                tmux_window: None,
                state: State::Working,
                message: None,
                conversation: None,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
    }

    #[test]
    fn one_account_shows_no_account_name() {
        let (_d, p) = home();
        save(&p, "g1");
        let out = plain(&run_with(
            &p,
            "{}",
            Some("g1".into()),
            Some("default".into()),
        ));
        assert!(out.starts_with("📁 api:s2"), "{out}");
        // But its stats are still filed under it.
        assert_eq!(crate::stats::load_all(&p)["g1"].account, "default");
    }

    #[test]
    fn grove_session_records_and_names_window() {
        let (_d, p) = home();
        save(&p, "g1");
        std::fs::write(
            p.config_file(),
            "[accounts.work]\ndir = \"~/.claude-work\"\n",
        )
        .unwrap();
        // After /clear the payload's id is a new one; GROVE_SESSION wins.
        let stdin = json!({ "session_id": "other", "cost": { "total_cost_usd": 0.5 } });
        let out = run_with(
            &p,
            &stdin.to_string(),
            Some("g1".into()),
            Some("work".into()),
        );
        let out = plain(&out);
        assert!(out.contains("📁 api:s2") && out.contains("WORK"), "{out}");
        let all = crate::stats::load_all(&p);
        assert_eq!(all["g1"].account, "work");
        assert_eq!(all["g1"].last.cost, 0.5);
    }
}
