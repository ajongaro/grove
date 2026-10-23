//! The ticker line above the footer: the indexes, then a page of symbols
//! that turns every few seconds.

use chrono::{DateTime, Utc};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};

use crate::ticker::{Market, Quote, Tape};

const UP: Color = Color::Rgb(0x9e, 0xce, 0x6a);
const DOWN: Color = Color::Rgb(0xf7, 0x76, 0x8e);
/// Seconds each page shows.
const PAGE_SECS: i64 = 8;
/// Older than this, the line says how old.
const STALE_MINS: i64 = 20;

/// What an index ETF stands for.
fn label(symbol: &str) -> &str {
    match symbol {
        "SPY" => "S&P",
        "DIA" => "Dow",
        "QQQ" => "Nasdaq",
        s => s,
    }
}

/// Up, down or flat as it shows, to one decimal.
fn change(pct: f64) -> Span<'static> {
    if pct >= 0.05 {
        Span::styled(format!("▲{pct:.1}%"), Style::new().fg(UP))
    } else if pct <= -0.05 {
        Span::styled(format!("▼{:.1}%", -pct), Style::new().fg(DOWN))
    } else {
        Span::raw("·0.0%").dark_gray()
    }
}

fn index(q: &Quote) -> Vec<Span<'static>> {
    vec![
        Span::raw(format!("{} ", label(&q.symbol))),
        change(q.change_pct),
    ]
}

fn symbol(q: &Quote) -> Vec<Span<'static>> {
    vec![
        Span::raw(q.symbol.clone()).bold(),
        Span::raw(format!(" {:.2} ", q.price)).gray(),
        change(q.change_pct),
    ]
}

fn width(spans: &[Span]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// `items` joined with two spaces.
fn join(items: Vec<Vec<Span<'static>>>) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        if i > 0 {
            out.push(Span::raw("  "));
        }
        out.extend(item);
    }
    out
}

/// The line for `width` columns at `now`; None when even the indexes
/// don't fit.
pub fn line(tape: &Tape, cols: usize, now: DateTime<Utc>) -> Option<Line<'static>> {
    let mut note = match tape.market {
        Market::Pre => "pre-market".to_string(),
        Market::After => "after hours".to_string(),
        Market::Closed => "closed".to_string(),
        Market::Open | Market::Unknown => String::new(),
    };
    if (now - tape.at).num_minutes() >= STALE_MINS {
        let age = format!("as of {} ago", super::app::format_age(tape.at, now));
        note = if note.is_empty() {
            age
        } else {
            format!("{note}, {age}")
        };
    }
    let lead = join(tape.indexes.iter().map(index).collect());
    let sep = if lead.is_empty() { 0 } else { 3 };
    // Room for the page count at its widest.
    let n = tape.symbols.len();
    let count_w = format!(" {n}/{n}").len();
    let note_w = if note.is_empty() { 0 } else { note.len() + 2 };
    let room = cols.checked_sub(1 + width(&lead) + note_w)?;
    // Pack the symbols into pages that fit.
    let room = room.saturating_sub(sep + count_w);
    let mut pages: Vec<Vec<Vec<Span<'static>>>> = Vec::new();
    let mut used = 0;
    for q in &tape.symbols {
        let s = symbol(q);
        let w = width(&s);
        match pages.last_mut() {
            Some(page) if used + 2 + w <= room => {
                page.push(s);
                used += 2 + w;
            }
            _ if w <= room => {
                pages.push(vec![s]);
                used = w;
            }
            _ => {}
        }
    }
    if lead.is_empty() && pages.is_empty() {
        return None;
    }
    let mut spans = vec![Span::raw(" ")];
    spans.extend(lead);
    let total = pages.len();
    if total > 0 {
        let at = (now.timestamp().div_euclid(PAGE_SECS) as usize) % total;
        if sep > 0 {
            spans.push(Span::raw(" │ ").dark_gray());
        }
        spans.extend(join(pages.swap_remove(at)));
        if total > 1 {
            spans.push(Span::raw(format!(" {}/{total}", at + 1)).dark_gray());
        }
    }
    if !note.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::raw(note).dark_gray().italic());
    }
    Some(Line::from(spans))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn q(symbol: &str, price: f64, change_pct: f64) -> Quote {
        Quote {
            symbol: symbol.into(),
            price,
            change_pct,
        }
    }

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 23, 15, 0, 0).unwrap()
    }

    fn tape(market: Market) -> Tape {
        Tape {
            indexes: vec![
                q("SPY", 500.0, 0.4),
                q("DIA", 420.0, -0.2),
                q("QQQ", 480.0, 0.03),
            ],
            symbols: [
                "AAPL", "AMD", "AMZN", "ANET", "ASML", "BE", "GME", "GOOG", "GS",
            ]
            .iter()
            .enumerate()
            .map(|(i, s)| q(s, 100.0 + i as f64, if i % 2 == 0 { 1.25 } else { -0.8 }))
            .collect(),
            market,
            at: at(),
        }
    }

    fn text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn indexes_lead_then_a_page_of_symbols() {
        let l = line(&tape(Market::Open), 100, at()).unwrap();
        let t = text(&l);
        assert!(
            t.starts_with(
                " S&P ▲0.4%  Dow ▼0.2%  Nasdaq ·0.0% │ AAPL 100.00 ▲1.2%  AMD 101.00 ▼0.8%"
            ),
            "{t}"
        );
        assert!(t.ends_with(" 1/3"), "{t}");
        assert!(l.width() <= 100, "{}", l.width());
        let up = l.spans.iter().find(|s| s.content == "▲0.4%").unwrap();
        assert_eq!(up.style.fg, Some(UP));
        let down = l.spans.iter().find(|s| s.content == "▼0.2%").unwrap();
        assert_eq!(down.style.fg, Some(DOWN));
    }

    #[test]
    fn pages_turn_with_the_clock() {
        let t = tape(Market::Open);
        let page = |secs| text(&line(&t, 100, at() + chrono::Duration::seconds(secs)).unwrap());
        assert!(page(0).contains("AAPL"));
        assert!(page(7).contains("AAPL"));
        let second = page(8);
        assert!(
            !second.contains("AAPL") && second.ends_with(" 2/3"),
            "{second}"
        );
        assert_eq!(page(24), page(0));
        // Every symbol shows on some page.
        let all: String = (0..3).map(|p| page(p * 8)).collect();
        for s in &t.symbols {
            assert!(all.contains(&s.symbol), "{} missing", s.symbol);
        }
    }

    #[test]
    fn wide_enough_is_one_page_without_a_count() {
        let t = text(&line(&tape(Market::Open), 400, at()).unwrap());
        assert!(t.contains("GS 108.00") && !t.contains("1/1"), "{t}");
    }

    #[test]
    fn the_market_and_old_prices_say_so() {
        let t = text(&line(&tape(Market::Closed), 100, at()).unwrap());
        assert!(t.ends_with("  closed"), "{t}");
        let t = text(&line(&tape(Market::Pre), 100, at()).unwrap());
        assert!(t.ends_with("  pre-market"), "{t}");
        let later = at() + chrono::Duration::hours(3);
        let t = text(&line(&tape(Market::Open), 100, later).unwrap());
        assert!(t.ends_with("as of 3h ago"), "{t}");
        assert!(line(&tape(Market::Open), 100, later).unwrap().width() <= 100);
    }

    #[test]
    fn too_narrow_shows_nothing() {
        assert!(line(&tape(Market::Open), 20, at()).is_none());
        // No indexes: symbols alone.
        let mut t = tape(Market::Open);
        t.indexes.clear();
        assert!(text(&line(&t, 60, at()).unwrap()).starts_with(" AAPL"));
    }
}
