//! The dashboard's grove: the trees in the header, leaves that drift while
//! sessions work, and the day's harvest. Anything that moves is a function of
//! the time the frame is drawn at, so it adds no redraws.

use chrono::{DateTime, Utc};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};

use super::render::parse_hex;
use crate::config::Config;
use crate::stats::{Harvest, Totals};

pub(super) const LEAF: Color = Color::Rgb(0x8f, 0xb3, 0x5a);
pub(super) const STEM: Color = Color::Rgb(0x8d, 0x6e, 0x4c);
pub(super) const OLIVE: Color = Color::Rgb(0x6b, 0x4a, 0x7a);
/// A paler leaf, so neighbouring trees read as two.
const SAGE: Color = Color::Rgb(0x6f, 0x9a, 0x6a);

/// How long a drifting leaf stays in one place.
const LEAF_STEP_MS: i64 = 400;

/// Three small trees standing on the branch's stem line: crowns on the top
/// row, trunks on the bottom, in half blocks like the wordmark.
pub fn grove() -> [Vec<(&'static str, Color)>; 2] {
    [
        vec![
            ("  ▟", LEAF),
            ("●", OLIVE),
            ("▙ ", LEAF),
            ("▗▆▖", SAGE),
            ("  ▟██▙", LEAF),
        ],
        // Each trunk under its crown's middle (the stem row starts one
        // column further right than the crowns).
        vec![("▀▀▐▀▀▀▐▀▀▀▀▐▌", STEM)],
    ]
}

/// One row, `width` cells wide, with two leaves tumbling across it while
/// anything is working; blank otherwise.
pub fn leaves(width: usize, now: DateTime<Utc>, working: bool) -> String {
    let mut row = vec![' '; width];
    if working && width > 0 {
        let step = now.timestamp_millis().div_euclid(LEAF_STEP_MS) as usize;
        // The second leaf is half a row behind and a little slower.
        for (x, c) in [
            (step % width, if step.is_multiple_of(2) { '❦' } else { '❧' }),
            (
                (step * 2 / 3 + width / 2) % width,
                if step.is_multiple_of(3) { '❧' } else { ',' },
            ),
        ] {
            row[x] = c;
        }
    }
    row.into_iter().collect()
}

/// A quip shows for `QUIP_FOR` seconds every `QUIP_EVERY`.
pub const QUIP_EVERY: i64 = 240;
const QUIP_FOR: i64 = 25;

pub const QUIPS: &[&str] = &[
    "the olives are listening",
    "photosynthesis: pending",
    "no trees were harmed in this commit",
    "roots: deep. context: deeper",
    "a watched session never finishes",
    "the grove does not judge your diffs",
    "somewhere, a tree is compiling",
    "leaves fall. tests pass. mostly.",
    "it's always olive season somewhere",
    "every session was once a seed",
    "bark is worse than its bite",
    "the trees have opinions on tabs",
    "nothing grows in a merge conflict",
    "patience. sap moves slowly.",
    "a grove is just a forest with a plan",
    "the wind carries no tokens",
    "shade is free. compute is not.",
    "this branch is going places",
    "moss grows on idle sessions",
    "the soil remembers every rebase",
];

/// The footer's quip at `now`, if one is due.
pub fn quip(now: DateTime<Utc>) -> Option<&'static str> {
    let t = now.timestamp();
    (t.rem_euclid(QUIP_EVERY) < QUIP_FOR)
        .then(|| QUIPS[t.div_euclid(QUIP_EVERY) as usize % QUIPS.len()])
}

/// What the Sessions pane shows when nothing is running anywhere here.
pub fn quiet_grove() -> Vec<Line<'static>> {
    let row = |runs: &[(&'static str, Color)]| {
        Line::from(
            runs.iter()
                .map(|(t, c)| Span::styled(*t, Style::new().fg(*c)))
                .collect::<Vec<_>>(),
        )
    };
    vec![
        row(&[("      ▄▆▄          ", LEAF), ("▗▄▖", SAGE)]),
        row(&[
            ("     ▟█", LEAF),
            ("●", OLIVE),
            ("█▙   ", LEAF),
            ("▗▆▖", SAGE),
            ("  ▟███▙", SAGE),
        ]),
        row(&[("       ▐▌     ▐▌    ▐▌", STEM)]),
        row(&[("   ▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔", STEM)]),
        Line::from(Span::raw("     the grove is quiet").dark_gray()),
    ]
}

/// 340 → "340", 2100 → "2.1k", 12000 → "12k".
pub fn short_count(n: u64) -> String {
    let short = |v: f64, unit: &str| {
        if v < 10.0 {
            format!("{:.1}{unit}", (v * 10.0).floor() / 10.0)
        } else {
            format!("{}{unit}", v.floor())
        }
    };
    match n {
        0..1000 => n.to_string(),
        1000..1_000_000 => short(n as f64 / 1000.0, "k"),
        _ => short(n as f64 / 1_000_000.0, "M"),
    }
}

/// Today's harvest in at most `max` columns. Several accounts each get their
/// own totals in their colour; detail goes before accounts do.
pub fn harvest_line(harvests: &[Harvest], cfg: &Config, max: usize) -> Line<'static> {
    if harvests.is_empty() {
        let l = Line::from(Span::raw("🧺 nothing harvested yet today").dark_gray());
        return if l.width() <= max { l } else { Line::default() };
    }
    let lines = |t: &Totals| format!("+{} −{}", short_count(t.added), short_count(t.removed));
    let sep = || Span::raw(" · ").dark_gray();
    let candidates: Vec<Vec<Span<'static>>> = if cfg.accounts.is_empty() {
        let t = harvests.iter().fold(Totals::default(), |mut a, h| {
            a.cost += h.totals.cost;
            a.added += h.totals.added;
            a.removed += h.totals.removed;
            a
        });
        let n: usize = harvests.iter().map(|h| h.sessions).sum();
        let cost = Span::raw(format!("${:.2}", t.cost));
        let sessions = Span::raw(format!(
            "{n} session{} today",
            if n == 1 { "" } else { "s" }
        ))
        .dark_gray();
        vec![
            vec![
                cost.clone(),
                sep(),
                // No ✏️: older tmux draws its variation selector a column
                // short and shifts the rest of the row.
                Span::raw(lines(&t)),
                sep(),
                sessions.clone(),
            ],
            vec![cost.clone(), sep(), sessions],
            vec![cost],
        ]
    } else {
        let per = |detail: bool| {
            let mut spans = Vec::new();
            for h in harvests {
                if !spans.is_empty() {
                    spans.push(sep());
                }
                let color = cfg
                    .account(Some(&h.account))
                    .ok()
                    .and_then(|a| parse_hex(a.color.as_deref()));
                let mut text = format!("{} ${:.2}", h.account, h.totals.cost);
                if detail {
                    text.push_str(&format!(" {}", lines(&h.totals)));
                }
                spans.push(match color {
                    Some(c) => Span::styled(text, Style::new().fg(c)),
                    None => Span::raw(text),
                });
            }
            spans
        };
        vec![per(true), per(false)]
    };
    candidates
        .into_iter()
        .map(|spans| {
            let mut all = vec![Span::raw("🧺 ")];
            all.extend(spans);
            Line::from(all)
        })
        .find(|l| l.width() <= max)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap()
    }

    fn text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn h(account: &str, cost: f64, added: u64, removed: u64, sessions: usize) -> Harvest {
        Harvest {
            account: account.into(),
            totals: Totals {
                cost,
                added,
                removed,
            },
            sessions,
        }
    }

    fn two_accounts() -> Config {
        toml::from_str(
            "account_name = \"personal\"\n[accounts.work]\ndir = \"~/.claude-work\"\ncolor = \"#ff8800\"\n",
        )
        .unwrap()
    }

    #[test]
    fn short_counts() {
        let got: Vec<_> = [0, 340, 999, 2100, 12_000, 1_500_000]
            .into_iter()
            .map(short_count)
            .collect();
        assert_eq!(got, ["0", "340", "999", "2.1k", "12k", "1.5M"]);
    }

    #[test]
    fn leaves_still_when_idle() {
        assert_eq!(leaves(30, t(), false), " ".repeat(30));
    }

    #[test]
    fn leaves_drift() {
        let a = leaves(30, t(), true);
        let b = leaves(30, t() + chrono::Duration::milliseconds(400), true);
        assert_ne!(a, b);
        for l in [&a, &b] {
            assert_eq!(Span::raw(l.as_str()).width(), 30, "{l:?}");
            assert!(l.chars().any(|c| c != ' '), "{l:?}");
        }
        assert_eq!(leaves(0, t(), true), "");
    }

    #[test]
    fn grove_is_two_small_rows() {
        let rows = grove();
        for r in &rows {
            let w: usize = r.iter().map(|(t, _)| Span::raw(*t).width()).sum();
            assert!(w > 0 && w <= 18, "{w}");
        }
    }

    #[test]
    fn harvest_one_account() {
        let l = harvest_line(&[h("default", 14.2, 2100, 340, 6)], &Config::default(), 80);
        assert_eq!(text(&l), "🧺 $14.20 · +2.1k −340 · 6 sessions today");
        let l = harvest_line(&[h("default", 1.0, 1, 0, 1)], &Config::default(), 80);
        assert!(text(&l).ends_with("1 session today"), "{}", text(&l));
    }

    #[test]
    fn harvest_splits_accounts_in_colour() {
        let hs = [
            h("personal", 4.4, 700, 140, 1),
            h("work", 9.8, 1400, 200, 2),
        ];
        let l = harvest_line(&hs, &two_accounts(), 80);
        assert_eq!(
            text(&l),
            "🧺 personal $4.40 +700 −140 · work $9.80 +1.4k −200"
        );
        let work = l.spans.iter().find(|s| s.content.contains("work")).unwrap();
        assert_eq!(work.style.fg, Some(Color::Rgb(255, 136, 0)));
    }

    #[test]
    fn harvest_drops_line_counts_when_narrow() {
        let hs = [
            h("personal", 4.4, 700, 140, 1),
            h("work", 9.8, 1400, 200, 2),
        ];
        let l = harvest_line(&hs, &two_accounts(), 36);
        assert_eq!(text(&l), "🧺 personal $4.40 · work $9.80");
        assert!(text(&harvest_line(&hs, &two_accounts(), 10)).is_empty());
        let one = [h("default", 14.2, 2100, 340, 6)];
        assert_eq!(
            text(&harvest_line(&one, &Config::default(), 30)),
            "🧺 $14.20 · 6 sessions today"
        );
        assert_eq!(
            text(&harvest_line(&one, &Config::default(), 12)),
            "🧺 $14.20"
        );
    }

    #[test]
    fn quip_shows_briefly_every_few_minutes() {
        let at = |secs: i64| Utc.timestamp_opt(secs, 0).unwrap();
        let base = 1_800_000_000 - 1_800_000_000 % QUIP_EVERY;
        assert!(quip(at(base)).is_some());
        assert!(quip(at(base + 24)).is_some());
        assert!(quip(at(base + 30)).is_none());
        assert_ne!(quip(at(base)), quip(at(base + QUIP_EVERY)));
        assert!(QUIPS.len() >= 15);
        assert!(QUIPS.iter().all(|q| q.chars().count() <= 40), "{QUIPS:?}");
    }

    #[test]
    fn quiet_grove_is_small() {
        let scene = quiet_grove();
        assert!(scene.len() <= 6);
        assert!(scene.iter().all(|l| l.width() <= 34));
        assert!(scene.iter().any(|l| text(l).contains("the grove is quiet")));
    }

    #[test]
    fn harvest_empty_is_dim_placeholder() {
        let l = harvest_line(&[], &Config::default(), 80);
        assert_eq!(text(&l), "🧺 nothing harvested yet today");
        assert!(l.spans.iter().all(|s| s.style.fg == Some(Color::DarkGray)));
    }
}
