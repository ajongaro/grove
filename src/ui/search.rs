//! `/`: find a session anywhere, here or on a finca, live or ended, by
//! fuzzy-matching its project, name, machine, title and last message.

use chrono::{DateTime, Utc};

use super::app::{Action, App};
use crate::model::State;

/// Where Enter takes a found session.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Local {
        id: String,
        window: Option<String>,
        live: bool,
    },
    Finca {
        slug: String,
        via: Option<String>,
        /// Older Groves don't send ids.
        id: Option<String>,
        window: Option<String>,
        live: bool,
    },
}

/// One session as the search lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// `project:name`.
    pub handle: String,
    /// The machine it's on.
    pub host: String,
    pub state: State,
    /// What it's about: its conversation's title, else its last message.
    pub about: Option<String>,
    pub message: Option<String>,
    pub updated_at: DateTime<Utc>,
    pub target: Target,
}

/// A match: the item and which characters of its handle and `about`
/// matched, as char indices.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub item: Item,
    pub handle_hl: Vec<usize>,
    pub about_hl: Vec<usize>,
}

/// Every session Grove knows of, here and on the fincas' last reports.
pub fn items(app: &App) -> Vec<Item> {
    let mut out: Vec<Item> = app
        .sessions
        .iter()
        .map(|s| Item {
            handle: format!("{}:{}", s.project, s.name),
            host: app.host.clone(),
            state: s.state,
            about: app.titles.get(&s.id).cloned().or_else(|| s.message.clone()),
            message: s.message.clone(),
            updated_at: s.updated_at,
            target: Target::Local {
                id: s.id.clone(),
                window: s.tmux_window.clone(),
                live: s.state != State::Ended,
            },
        })
        .collect();
    for f in &app.fincas {
        let Some(st) = &f.last_status else { continue };
        for p in &st.projects {
            let live = p.sessions.iter().map(|s| (s, true));
            for (s, live) in live.chain(p.ended.iter().map(|s| (s, false))) {
                out.push(Item {
                    handle: format!("{}:{}", p.slug, s.name),
                    host: f.name.clone(),
                    state: if live { s.state } else { State::Ended },
                    about: s.title.clone().or_else(|| s.message.clone()),
                    message: s.message.clone(),
                    updated_at: s.updated_at,
                    target: Target::Finca {
                        slug: f.slug.clone(),
                        via: f.via.clone(),
                        id: s.id.clone(),
                        window: s.window.clone(),
                        live,
                    },
                });
            }
        }
    }
    out
}

/// Waiting sessions first, then the rest; ended ones last.
fn urgency(s: State) -> u8 {
    match s {
        State::NeedsYou => 0,
        State::YourTurn => 1,
        State::Working => 2,
        State::Starting => 3,
        State::Ended => 4,
    }
}

/// The sessions matching every word of `query`, best first. An empty query
/// lists them all, those waiting on you first, then by how recent.
pub fn search(app: &App, query: &str) -> Vec<Hit> {
    let terms: Vec<&str> = query.split_whitespace().collect();
    let mut hits: Vec<(i64, Hit)> = items(app)
        .into_iter()
        .filter_map(|item| {
            let about = item.about.clone().unwrap_or_default();
            // The handle, then the rest, with gaps a word can't span.
            let hay = format!(
                "{}  {}  {}  {}",
                item.handle,
                item.host,
                about,
                item.message
                    .as_deref()
                    .filter(|m| *m != about)
                    .unwrap_or("")
            );
            let handle_end = item.handle.chars().count();
            let about_at = handle_end + 2 + item.host.chars().count() + 2;
            let about_end = about_at + about.chars().count();
            let mut score = 0;
            let mut handle_hl = Vec::new();
            let mut about_hl = Vec::new();
            for t in &terms {
                let (s, pos) = fuzzy(t, &hay)?;
                score += s;
                for p in pos {
                    if p < handle_end {
                        handle_hl.push(p);
                    } else if (about_at..about_end).contains(&p) {
                        about_hl.push(p - about_at);
                    }
                }
            }
            Some((
                score,
                Hit {
                    item,
                    handle_hl,
                    about_hl,
                },
            ))
        })
        .collect();
    hits.sort_by(|(a, x), (b, y)| {
        b.cmp(a)
            .then(urgency(x.item.state).cmp(&urgency(y.item.state)))
            .then(y.item.updated_at.cmp(&x.item.updated_at))
    });
    hits.into_iter().map(|(_, h)| h).collect()
}

/// fzf-style: `term`'s characters in order in `text`, ignoring case. The
/// score favours word starts and runs of consecutive characters; the
/// positions are char indices into `text`.
pub fn fuzzy(term: &str, text: &str) -> Option<(i64, Vec<usize>)> {
    let lower = |c: char| c.to_lowercase().next().unwrap_or(c);
    let t: Vec<char> = text.chars().map(lower).collect();
    let q: Vec<char> = term.chars().map(lower).collect();
    let Some(&first) = q.first() else {
        return Some((0, Vec::new()));
    };
    let word_start = |i: usize| i == 0 || !t[i - 1].is_alphanumeric();
    let mut best: Option<(i64, Vec<usize>)> = None;
    for start in (0..t.len()).filter(|&i| t[i] == first) {
        let mut pos = vec![start];
        let mut next = start + 1;
        for &c in &q[1..] {
            match (next..t.len()).find(|&k| t[k] == c) {
                Some(k) => {
                    pos.push(k);
                    next = k + 1;
                }
                None => break,
            }
        }
        if pos.len() < q.len() {
            // Later starts can only find less.
            break;
        }
        let mut score = 0;
        for (n, &p) in pos.iter().enumerate() {
            score += 16;
            if word_start(p) {
                score += 10;
            }
            if n > 0 {
                let gap = p - pos[n - 1] - 1;
                score += if gap == 0 { 12 } else { -(gap.min(10) as i64) };
            }
        }
        if best.as_ref().is_none_or(|(b, _)| score > *b) {
            best = Some((score, pos));
        }
    }
    best
}

/// What Enter on `hit` does; Err is the footer's explanation instead.
pub fn enter(hit: &Hit) -> Result<Action, String> {
    match &hit.item.target {
        Target::Local {
            window: Some(w),
            live: true,
            ..
        } => Ok(Action::Jump { window: w.clone() }),
        Target::Local { live: true, .. } => Err(format!("{} is still starting", hit.item.handle)),
        Target::Local { id, .. } => Ok(Action::Resume {
            session_id: id.clone(),
        }),
        Target::Finca {
            slug,
            via,
            window,
            live: true,
            ..
        } => Ok(match window {
            Some(w) => Action::FincaJump {
                slug: slug.clone(),
                via: via.clone(),
                window: w.clone(),
            },
            None => Action::ConnectFinca {
                slug: slug.clone(),
                via: via.clone(),
            },
        }),
        Target::Finca {
            slug,
            via,
            id: Some(id),
            ..
        } => Ok(Action::FincaResume {
            slug: slug.clone(),
            via: via.clone(),
            session_id: id.clone(),
        }),
        Target::Finca { .. } => Err(format!(
            "{}'s Grove is too old to resume from here: update it (grove finca update)",
            hit.item.host
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_finds_letters_in_order() {
        let (_, pos) = fuzzy("gaschema", "msl:google-analytics  GA schema").unwrap();
        assert_eq!(pos.len(), 8);
        assert!(fuzzy("xyz", "google-analytics").is_none());
        assert!(fuzzy("scg", "google schema").is_none());
        assert_eq!(fuzzy("", "anything"), Some((0, vec![])));
        // Case is ignored.
        assert!(fuzzy("KENO", "club-keno-extras").is_some());
    }

    #[test]
    fn fuzzy_prefers_runs_and_word_starts() {
        let run = fuzzy("keno", "club-keno-extras").unwrap().0;
        let scattered = fuzzy("keno", "kale entree onion").unwrap().0;
        assert!(run > scattered, "{run} {scattered}");
        // The best of several starts: the run, not the first k.
        let (_, pos) = fuzzy("keno", "k club-keno").unwrap();
        assert_eq!(pos, [7, 8, 9, 10]);
        let start = fuzzy("an", "msl:analytics").unwrap().0;
        let middle = fuzzy("an", "msl:planet").unwrap().0;
        assert!(start > middle, "{start} {middle}");
    }
}
