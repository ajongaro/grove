//! Due dates for TODO items: reading what you type (`dec 1`, `fri`, `+3`)
//! and showing them (`Dec 1`). Pure: the caller passes today.

use chrono::NaiveDate;

/// How close a due date is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    Later,
    /// Today or within the next 3 days.
    Soon,
    Overdue,
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
/// How far ahead `+N` may reach, in days.
const MAX_AHEAD: i64 = 3650;

/// What you typed as a due date; `Ok(None)` (clear it) when blank. Month
/// and day without a year mean the next such date; a weekday the next one.
pub fn parse(input: &str, today: NaiveDate) -> Result<Option<NaiveDate>, String> {
    let s = input.trim().to_lowercase();
    if s.is_empty() {
        return Ok(None);
    }
    read(&s, today).map(Some).ok_or_else(|| {
        format!(
            "\"{}\" isn't a date: try 2026-12-01, 12/1, dec 1, fri, tomorrow or +3",
            input.trim()
        )
    })
}

fn read(s: &str, today: NaiveDate) -> Option<NaiveDate> {
    use chrono::{Datelike, Duration};
    match s {
        "today" => return Some(today),
        "tomorrow" => return today.succ_opt(),
        _ => {}
    }
    if let Some(n) = s.strip_prefix('+') {
        let n: i64 = n.trim().parse().ok().filter(|n| (0..=MAX_AHEAD).contains(n))?;
        return today.checked_add_signed(Duration::days(n));
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d);
    }
    if let Some(w) = prefix_of(s, &WEEKDAYS) {
        let ahead = (w as i64 - today.weekday().num_days_from_monday() as i64 - 1).rem_euclid(7) + 1;
        return today.checked_add_signed(Duration::days(ahead));
    }
    let (month, day) = month_day(s)?;
    let this = NaiveDate::from_ymd_opt(today.year(), month, day);
    match this {
        Some(d) if d >= today => Some(d),
        // Passed (or Feb 29 not this year): the next year that has it.
        _ => (1..=4).find_map(|y| NaiveDate::from_ymd_opt(today.year() + y, month, day)),
    }
}

/// The index of the name `s` starts (3 letters at least) or spells out.
fn prefix_of(s: &str, names: &[&str]) -> Option<usize> {
    if s.len() < 3 || !s.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    names.iter().position(|n| s.starts_with(n))
}

/// `12/1`, `12-1`, `dec 1`, `1 dec`, `december 1st` → (month, day).
fn month_day(s: &str) -> Option<(u32, u32)> {
    if let Some((m, d)) = s.split_once(['/', '-']) {
        return Some((m.trim().parse().ok()?, d.trim().parse().ok()?));
    }
    let words: Vec<&str> = s.split([' ', ',']).filter(|w| !w.is_empty()).collect();
    let [a, b] = words.as_slice() else {
        return None;
    };
    let day = |w: &str| -> Option<u32> {
        w.trim_end_matches(|c: char| c.is_ascii_alphabetic()).parse().ok()
    };
    match (prefix_of(a, &MONTHS), prefix_of(b, &MONTHS)) {
        (Some(m), None) => Some((m as u32 + 1, day(b)?)),
        (None, Some(m)) => Some((m as u32 + 1, day(a)?)),
        _ => None,
    }
}

/// `Dec 1`, with the year when it isn't this one: `Jan 5 2027`.
pub fn show(d: NaiveDate, today: NaiveDate) -> String {
    use chrono::Datelike;
    if d.year() == today.year() {
        d.format("%b %-d").to_string()
    } else {
        d.format("%b %-d %Y").to_string()
    }
}

pub fn urgency(d: NaiveDate, today: NaiveDate) -> Urgency {
    let days = (d - today).num_days();
    if days < 0 {
        Urgency::Overdue
    } else if days <= 3 {
        Urgency::Soon
    } else {
        Urgency::Later
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    /// A Thursday.
    fn t() -> NaiveDate {
        d(2026, 11, 19)
    }

    #[test]
    fn reads_dates_the_ways_people_type_them() {
        let t = t();
        assert_eq!(parse("2026-12-01", t), Ok(Some(d(2026, 12, 1))));
        assert_eq!(parse("12/1", t), Ok(Some(d(2026, 12, 1))));
        // Already passed this year: next year's.
        assert_eq!(parse("11-1", t), Ok(Some(d(2027, 11, 1))));
        assert_eq!(parse("Dec 1", t), Ok(Some(d(2026, 12, 1))));
        assert_eq!(parse("1 december", t), Ok(Some(d(2026, 12, 1))));
        assert_eq!(parse("fri", t), Ok(Some(d(2026, 11, 20))));
        // A weekday is the next one, not today.
        assert_eq!(parse("thursday", t), Ok(Some(d(2026, 11, 26))));
        assert_eq!(parse("today", t), Ok(Some(t)));
        assert_eq!(parse("Tomorrow", t), Ok(Some(d(2026, 11, 20))));
        assert_eq!(parse("+3", t), Ok(Some(d(2026, 11, 22))));
        assert_eq!(parse("  ", t), Ok(None));
    }

    #[test]
    fn nonsense_is_refused_with_an_example() {
        let t = t();
        assert!(parse("next week", t).unwrap_err().contains("2026-12-01"));
        assert!(parse("2/30", t).is_err());
        assert!(parse("2026-13-01", t).is_err());
        assert!(parse("+99999", t).is_err());
    }

    #[test]
    fn shows_the_year_only_when_it_isnt_this_one() {
        assert_eq!(show(d(2026, 12, 1), t()), "Dec 1");
        assert_eq!(show(d(2027, 1, 5), t()), "Jan 5 2027");
    }

    #[test]
    fn soon_is_today_to_three_days_out() {
        let t = t();
        assert_eq!(urgency(d(2026, 11, 18), t), Urgency::Overdue);
        assert_eq!(urgency(t, t), Urgency::Soon);
        assert_eq!(urgency(d(2026, 11, 22), t), Urgency::Soon);
        assert_eq!(urgency(d(2026, 11, 23), t), Urgency::Later);
    }
}
