//! The dashboard's grove: a sky in the header with olive trees, clouds and
//! leaves that blow while sessions work, and the day's harvest. Anything that
//! moves is a function of the time the frame is drawn at, so it adds no
//! redraws.

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
const STAR: Color = Color::Rgb(0xf0, 0xe6, 0xa0);
const SUN: Color = Color::Rgb(0xff, 0xd8, 0x4a);
const MOON: Color = Color::Rgb(0xe8, 0xe4, 0xd0);
const RAIN: Color = Color::Rgb(0xa8, 0xc8, 0xe8);
const OVERCAST: (u8, u8, u8) = (0x70, 0x78, 0x84);

/// How long a drifting leaf stays in one place.
const LEAF_STEP_MS: i64 = 400;
/// How long a raindrop stays in one place.
const RAIN_STEP_MS: i64 = 300;
/// How long a cloud stays in one place.
const CLOUD_STEP_S: i64 = 3;

/// The sky's width bounds: below the minimum there is no room for a tree.
pub const SKY_MIN: usize = 12;
pub const SKY_MAX: usize = 34;

/// The sky and its clouds at a local hour.
struct Palette {
    sky: Color,
    cloud: Color,
    stars: bool,
}

fn palette(hour: u32) -> Palette {
    let (sky, cloud, stars) = match hour {
        6..8 => (
            Color::Rgb(0xee, 0xb8, 0xa8),
            Color::Rgb(0xff, 0xee, 0xf0),
            false,
        ),
        8..18 => (
            Color::Rgb(0x5f, 0xa8, 0xe0),
            Color::Rgb(0xff, 0xff, 0xff),
            false,
        ),
        18..20 => (
            Color::Rgb(0xe0, 0x84, 0x4c),
            Color::Rgb(0xc9, 0xb4, 0xb8),
            false,
        ),
        _ => (
            Color::Rgb(0x16, 0x20, 0x40),
            Color::Rgb(0x4a, 0x55, 0x70),
            true,
        ),
    };
    Palette { sky, cloud, stars }
}

/// The grove's weather, from what its sessions are doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    /// Nothing working, nothing waiting: the sun (or moon) is out.
    Calm,
    /// Something is working: leaves blow through.
    Busy,
    /// Something needs you: it rains.
    Rain,
}

/// `c` moved `t` of the way towards `to`.
fn blend(c: Color, to: (u8, u8, u8), t: f32) -> Color {
    let Color::Rgb(r, g, b) = c else { return c };
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color::Rgb(mix(r, to.0), mix(g, to.1), mix(b, to.2))
}

/// One cell of the sky; `None` is open sky.
type Cell = Option<(char, Style)>;

/// Writes `s` into `row` from column `x` on, clipped to the row; spaces are
/// see-through.
fn put(row: &mut [Cell], x: isize, s: &str, style: impl Fn(char) -> Style) {
    for (i, c) in s.chars().enumerate() {
        let at = x + i as isize;
        if c != ' ' && at >= 0 && (at as usize) < row.len() {
            row[at as usize] = Some((c, style(c)));
        }
    }
}

/// A window of sky `width` columns wide over a strip of grass `ground`
/// wide: clouds drifting, olive trees and stars at night. Its colours follow
/// the local `hour`, its weather the grove's `mood`.
pub fn sky(
    width: usize,
    ground: usize,
    hour: u32,
    now: DateTime<Utc>,
    mood: Mood,
) -> [Line<'static>; 3] {
    let mut p = palette(hour);
    if mood == Mood::Rain {
        p.sky = blend(p.sky, OVERCAST, 0.6);
        p.cloud = blend(p.cloud, OVERCAST, 0.5);
        p.stars = false;
    }
    let on_sky = |fg: Color| Style::new().fg(fg).bg(p.sky);
    let mut rows = [vec![None; width], vec![None; width]];
    let secs = now.timestamp();
    let w = width as isize;

    // Clouds pass in front of the sun and moon.
    if mood == Mood::Calm && width >= 4 {
        let light = if p.stars { MOON } else { SUN };
        for (row, part) in rows.iter_mut().zip(["▗▄▖", "▝▀▘"]) {
            put(row, w - 4, part, |_| on_sky(light));
        }
    }

    // Two clouds, the small one slower and half a sky behind.
    for (shape, speed, offset) in [([" ▄▄▄▄ ", "▀▀▀▀▀▀"], 1, 0), (["▗▄▄▖", ""], 2, w / 2)]
    {
        let span = w + 6;
        let x = (secs / (CLOUD_STEP_S * speed)).rem_euclid(span as i64) as isize;
        let x = (x + offset).rem_euclid(span) - 6;
        for (row, part) in rows.iter_mut().zip(shape) {
            put(row, x, part, |_| on_sky(p.cloud));
        }
    }

    if p.stars {
        for (i, (x, y)) in [(2, 0), (7, 1), (17, 0), (29, 0), (33, 1)]
            .into_iter()
            .enumerate()
        {
            let twinkle = (secs / 2 + i as i64).rem_euclid(5) == 0;
            if x < width && rows[y][x].is_none() && !twinkle {
                rows[y][x] = Some(('✦', on_sky(STAR)));
            }
        }
    }

    if mood == Mood::Rain {
        let step = now.timestamp_millis().div_euclid(RAIN_STEP_MS) as usize;
        for (y, row) in rows.iter_mut().enumerate() {
            for (x, cell) in row.iter_mut().enumerate() {
                if cell.is_none() && (x + y + step).is_multiple_of(3) {
                    *cell = Some(('╷', on_sky(RAIN)));
                }
            }
        }
    }

    // Olive trees stand in front of the clouds; each trunk sits under its
    // crown's middle.
    let mut trunks = Vec::new();
    let big = width * 2 / 5;
    let small = width * 3 / 4;
    for (x, crown, leaf, trunk, fits) in [
        (big, ["▗▟█●▙▖", "▜●██●▛"], LEAF, 2, big + 6 <= width),
        (
            small,
            [" ▄▄ ", "▟●█▙"],
            SAGE,
            1,
            small >= big + 7 && small + 4 <= width,
        ),
    ] {
        if !fits {
            continue;
        }
        for (row, part) in rows.iter_mut().zip(crown) {
            put(row, x as isize, part, |c| match c {
                '●' => Style::new().fg(OLIVE).bg(leaf),
                _ => on_sky(leaf),
            });
        }
        trunks.push(x + trunk);
    }

    if mood == Mood::Busy {
        let step = now.timestamp_millis().div_euclid(LEAF_STEP_MS) as usize;
        for (y, x, c) in [
            (
                1,
                step % width,
                if step.is_multiple_of(2) { '❦' } else { '❧' },
            ),
            (
                0,
                (step * 2 / 3 + width / 2) % width,
                if step.is_multiple_of(3) { '❧' } else { ',' },
            ),
        ] {
            if rows[y][x].is_none() {
                rows[y][x] = Some((c, on_sky(LEAF)));
            }
        }
    }

    let line = |row: &[Cell]| {
        Line::from(
            row.iter()
                .map(|c| match c {
                    Some((c, st)) => Span::styled(c.to_string(), *st),
                    None => Span::styled(" ", Style::new().bg(p.sky)),
                })
                .collect::<Vec<_>>(),
        )
    };
    let mut grass = vec!["▔"; ground.min(width)];
    for x in trunks {
        if x + 1 < grass.len() {
            grass[x] = "▐";
            grass[x + 1] = "▌";
        }
    }
    let grass = Line::from(
        grass
            .into_iter()
            .map(|c| Span::styled(c, Style::new().fg(if c == "▔" { LEAF } else { STEM })))
            .collect::<Vec<_>>(),
    );
    [line(&rows[0]), line(&rows[1]), grass]
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

    fn cells(l: &Line) -> Vec<(String, Style)> {
        l.spans
            .iter()
            .map(|s| (s.content.to_string(), s.style))
            .collect()
    }

    #[test]
    fn sky_fills_its_width_and_the_ground_its_own() {
        for w in [SKY_MIN, 20, SKY_MAX] {
            let rows = sky(w, w - 3, 12, t(), Mood::Busy);
            assert_eq!(rows[0].width(), w);
            assert_eq!(rows[1].width(), w);
            assert_eq!(rows[2].width(), w - 3);
        }
    }

    #[test]
    fn olive_trees_and_white_clouds_by_day() {
        let rows = sky(SKY_MAX, SKY_MAX, 12, t(), Mood::Calm);
        let top = text(&rows[0]) + &text(&rows[1]);
        assert!(top.contains("▟█●▙") && top.contains('●'), "{top}");
        assert!(
            cells(&rows[0])
                .iter()
                .chain(cells(&rows[1]).iter())
                .any(|(c, st)| (c == "▀" || c == "▄") && st.fg == Some(Color::Rgb(255, 255, 255))),
            "no cloud: {top}"
        );
        assert!(!top.contains('✦'), "{top}");
        assert_eq!(
            text(&rows[2]).matches("▐▌").count(),
            2,
            "{}",
            text(&rows[2])
        );
    }

    #[test]
    fn sky_follows_the_clock() {
        let bg = |hour| sky(20, 20, hour, t(), Mood::Calm)[0].spans[0].style.bg;
        let [dawn, day, sunset, night] = [7, 12, 19, 23].map(bg);
        assert!(day.is_some());
        assert!(dawn != day && day != sunset && sunset != night && night != dawn);
        assert_eq!(bg(3), night);
        let rows = sky(SKY_MAX, SKY_MAX, 23, t(), Mood::Calm);
        assert!((text(&rows[0]) + &text(&rows[1])).contains('✦'));
    }

    #[test]
    fn leaves_blow_only_while_working() {
        let blown = |mood| {
            (0..6).any(|i| {
                let at = t() + chrono::Duration::milliseconds(LEAF_STEP_MS * i);
                let rows = sky(SKY_MAX, SKY_MAX, 12, at, mood);
                (text(&rows[0]) + &text(&rows[1])).contains(['❦', '❧', ','])
            })
        };
        assert!(blown(Mood::Busy));
        assert!(!blown(Mood::Calm));
        assert!(!blown(Mood::Rain));
    }

    fn has_fg(rows: &[Line], fg: Color) -> bool {
        rows[..2]
            .iter()
            .flat_map(|l| l.spans.iter())
            .any(|s| s.style.fg == Some(fg))
    }

    #[test]
    fn rain_when_someone_needs_you() {
        let at = |ms| t() + chrono::Duration::milliseconds(ms);
        let rain = sky(SKY_MAX, SKY_MAX, 12, at(0), Mood::Rain);
        let busy = sky(SKY_MAX, SKY_MAX, 12, at(0), Mood::Busy);
        assert!(text(&rain[1]).contains('╷'), "{}", text(&rain[1]));
        assert!(!text(&busy[1]).contains('╷'));
        // A greyer sky, and the drops fall.
        assert_ne!(rain[0].spans[0].style.bg, busy[0].spans[0].style.bg);
        let later = sky(SKY_MAX, SKY_MAX, 12, at(RAIN_STEP_MS), Mood::Rain);
        assert_ne!(text(&rain[1]), text(&later[1]));
    }

    #[test]
    fn sun_when_calm_and_moon_at_night() {
        assert!(has_fg(&sky(SKY_MAX, SKY_MAX, 12, t(), Mood::Calm), SUN));
        assert!(!has_fg(&sky(SKY_MAX, SKY_MAX, 12, t(), Mood::Busy), SUN));
        assert!(!has_fg(&sky(SKY_MAX, SKY_MAX, 12, t(), Mood::Rain), SUN));
        let night = sky(SKY_MAX, SKY_MAX, 23, t(), Mood::Calm);
        assert!(has_fg(&night, MOON) && !has_fg(&night, SUN));
    }

    #[test]
    fn clouds_drift() {
        let at = |secs| {
            text(
                &sky(
                    SKY_MAX,
                    SKY_MAX,
                    12,
                    t() + chrono::Duration::seconds(secs),
                    Mood::Calm,
                )[1],
            )
        };
        assert_ne!(at(0), at(CLOUD_STEP_S * 2));
    }

    #[test]
    fn small_sky_keeps_one_tree() {
        let rows = sky(SKY_MIN, SKY_MIN, 12, t(), Mood::Calm);
        assert_eq!(
            text(&rows[2]).matches("▐▌").count(),
            1,
            "{}",
            text(&rows[2])
        );
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
