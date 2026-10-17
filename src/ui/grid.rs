//! The session grid (`v`): every live session as a tile showing the bottom
//! of its screen. Layout and movement are pure; drawing is at the bottom.

use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout as Split, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use super::app::App;
use super::render::{ACCENT, FINCAS_COLOR, SESSIONS_COLOR, state_color};
use crate::model::State;

/// Seconds without a good fetch before a finca's tiles go stale.
pub const STALE_SECS: i64 = 6;

#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    /// The finca's (slug, name), or None for this machine.
    pub finca: Option<(String, String)>,
    /// The finca destination that last answered.
    pub via: Option<String>,
    pub project: String,
    pub name: String,
    /// None for a finca tile from the status poll (its Grove is too old to
    /// reply to).
    pub session_id: Option<String>,
    pub state: State,
    pub updated_at: DateTime<Utc>,
    pub window: Option<String>,
    /// None: no live screen for this tile.
    pub screen: Option<String>,
    pub stale: bool,
}

impl Tile {
    pub fn handle(&self) -> String {
        format!("{}:{}", self.project, self.name)
    }
}

/// Every live session: this machine's (projects in pane order, oldest
/// first), then each finca's in the Fincas pane order.
pub fn tiles(app: &App, now: DateTime<Utc>) -> Vec<Tile> {
    let mut out = Vec::new();
    for p in &app.projects {
        for s in app
            .sessions
            .iter()
            .filter(|s| s.project == p.slug && s.state != State::Ended)
        {
            out.push(Tile {
                finca: None,
                via: None,
                project: s.project.clone(),
                name: s.name.clone(),
                session_id: Some(s.id.clone()),
                state: s.state,
                updated_at: s.updated_at,
                window: s.tmux_window.clone(),
                screen: Some(app.local_screens.get(&s.id).cloned().unwrap_or_default()),
                stale: false,
            });
        }
    }
    for f in &app.fincas {
        let finca = Some((f.slug.clone(), f.name.clone()));
        let screens = app
            .finca_screens
            .get(&f.slug)
            .filter(|_| !app.too_old.contains(&f.slug));
        if let Some((at, screens)) = screens {
            let stale = (now - *at).num_seconds() > STALE_SECS;
            for s in &screens.sessions {
                out.push(Tile {
                    finca: finca.clone(),
                    via: f.via.clone(),
                    project: s.project.clone(),
                    name: s.name.clone(),
                    session_id: Some(s.id.clone()),
                    state: s.state,
                    updated_at: s.updated_at,
                    window: s.window.clone(),
                    screen: Some(s.screen.clone()),
                    stale,
                });
            }
        } else if let Some(st) = f
            .last_status
            .as_ref()
            // Not one that stopped answering: its sessions may be long gone.
            .filter(|_| !matches!(f.reach, Some(crate::finca::Reach::Unreachable { .. })))
        {
            for p in &st.projects {
                for s in p.sessions.iter().filter(|s| s.state != State::Ended) {
                    out.push(Tile {
                        finca: finca.clone(),
                        via: f.via.clone(),
                        project: p.slug.clone(),
                        name: s.name.clone(),
                        session_id: None,
                        state: s.state,
                        updated_at: s.updated_at,
                        window: s.window.clone(),
                        screen: None,
                        stale: false,
                    });
                }
            }
        }
    }
    out
}

/// The smallest tile worth drawing (border included).
pub const MIN_W: u16 = 40;
pub const MIN_H: u16 = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub cols: usize,
    /// Rows on screen (more may be scrolled to).
    pub rows: usize,
    pub tile_w: u16,
    pub tile_h: u16,
}

/// Tiles of at least MIN_W × MIN_H, filling `width` × `height`; extra rows
/// scroll rather than shrink.
pub fn layout(n: usize, width: u16, height: u16) -> Layout {
    let cols = usize::from(width / MIN_W).clamp(1, n.max(1));
    let rows = n
        .div_ceil(cols)
        .clamp(1, usize::from(height / MIN_H).max(1));
    Layout {
        cols,
        rows,
        tile_w: width / cols as u16,
        tile_h: height / rows as u16,
    }
}

/// The top visible row that keeps `sel` on screen.
pub fn first_row(sel: usize, cols: usize, rows: usize) -> usize {
    (sel / cols.max(1)).saturating_sub(rows.saturating_sub(1))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// The neighbour of `sel` in `dir`, staying put at an edge.
pub fn step(sel: usize, n: usize, cols: usize, dir: Dir) -> usize {
    if n == 0 {
        return 0;
    }
    let cols = cols.max(1);
    let sel = sel.min(n - 1);
    let col = sel % cols;
    match dir {
        Dir::Left if col > 0 => sel - 1,
        Dir::Right if col + 1 < cols && sel + 1 < n => sel + 1,
        Dir::Up if sel >= cols => sel - cols,
        // Onto a short last row: its last tile.
        Dir::Down if (sel / cols + 1) * cols < n => (sel + cols).min(n - 1),
        _ => sel,
    }
}

/// The next tile (wrapping) that needs you or is your turn; `from` if none.
pub fn next_waiting(states: &[State], from: usize, forward: bool) -> usize {
    let n = states.len();
    (1..n)
        .map(|k| {
            if forward {
                (from + k) % n
            } else {
                (from + n - k) % n
            }
        })
        .find(|&i| matches!(states[i], State::NeedsYou | State::YourTurn))
        .unwrap_or(from)
}

/// The grid: a top line, the tiles, and the reply line or hints, in `area`.
pub fn render_grid(
    frame: &mut Frame,
    app: &App,
    sel: usize,
    reply: Option<&super::app::Reply>,
    area: Rect,
    now: DateTime<Utc>,
) {
    let tiles = tiles(app, now);
    let [top, body, bottom] = Split::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(if reply.is_some() { 2 } else { 1 }),
    ])
    .areas(area);
    let l = layout(tiles.len(), body.width, body.height);
    app.grid_cols.set(l.cols);
    let sel = sel.min(tiles.len().saturating_sub(1));
    let first = first_row(sel, l.cols, l.rows);
    let shown = (first * l.cols).min(tiles.len())..((first + l.rows) * l.cols).min(tiles.len());

    // Top line: totals, what's shown, unreachable fincas, the way back.
    let count = |f: &dyn Fn(State) -> bool| tiles.iter().filter(|t| f(t.state)).count();
    let mut spans = vec![
        Span::raw(" SESSIONS").fg(SESSIONS_COLOR),
        Span::raw(format!(" · {} live", tiles.len())),
    ];
    if shown.len() < tiles.len() {
        spans.push(Span::raw(format!(" · showing {}–{}", shown.start + 1, shown.end)).dark_gray());
    }
    for (icon, n, c) in [
        ("⚑", count(&|s| s == State::NeedsYou), Color::Red),
        ("●", count(&|s| s == State::YourTurn), Color::Yellow),
        (
            "●",
            count(&|s| matches!(s, State::Working | State::Starting)),
            Color::Green,
        ),
    ] {
        spans.push(Span::raw(format!("  {icon}{n}")).fg(if n == 0 { Color::DarkGray } else { c }));
    }
    for f in &app.fincas {
        if matches!(f.reach, Some(crate::finca::Reach::Unreachable { .. })) {
            spans.push(Span::raw(format!(" · {} unreachable", f.name)).dark_gray());
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), top);
    frame.render_widget(
        Paragraph::new("v back · ? help ")
            .alignment(Alignment::Right)
            .dark_gray(),
        top,
    );

    if tiles.is_empty() {
        frame.render_widget(
            Paragraph::new(" No live sessions. Esc back, then Enter on a project to start one.")
                .dark_gray(),
            body,
        );
    }
    for (k, i) in shown.clone().enumerate() {
        let (r, c) = ((k / l.cols) as u16, (k % l.cols) as u16);
        let (x, y) = (body.x + c * l.tile_w, body.y + r * l.tile_h);
        // The last row and column take what division leaves over.
        let last_col = usize::from(c) + 1 == l.cols;
        let last_row = usize::from(r) + 1 == l.rows;
        let rect = Rect {
            x,
            y,
            width: if last_col {
                body.right().saturating_sub(x)
            } else {
                l.tile_w
            },
            height: if last_row {
                body.bottom().saturating_sub(y)
            } else {
                l.tile_h
            },
        }
        .intersection(body);
        draw_tile(frame, &tiles[i], i == sel, rect, now);
    }

    // Bottom: the reply line, or a message, or the hints.
    match reply {
        Some(r) => {
            // A failed send's error takes the hint's place.
            let hint = app
                .footer
                .clone()
                .unwrap_or_else(|| "Enter send · Esc cancel".into());
            let lines = vec![
                Line::from(vec![
                    Span::raw(format!(" reply to {} ▸ ", r.handle)).fg(ACCENT),
                    Span::raw(r.text.clone()),
                    Span::raw("_").dark_gray(),
                ]),
                Line::from(format!(" {hint}")).dark_gray(),
            ];
            frame.render_widget(Paragraph::new(lines), bottom);
        }
        None => {
            let text = app.footer.clone().unwrap_or_else(|| {
                "Enter jump · i reply · x kill · Tab next waiting · hjkl move".into()
            });
            frame.render_widget(Paragraph::new(format!(" {text}")).dark_gray(), bottom);
        }
    }
}

fn draw_tile(frame: &mut Frame, t: &Tile, selected: bool, rect: Rect, now: DateTime<Utc>) {
    if rect.width < 3 || rect.height < 2 {
        return;
    }
    let dim = |c: Color| if t.stale { Color::DarkGray } else { c };
    // A session asking for permission shows its whole screen, so the
    // question is visible.
    let (lines, draft) = match &t.screen {
        Some(s) if t.state != State::NeedsYou => trim_prompt(s),
        Some(s) => (s.lines().collect(), false),
        None => (Vec::new(), false),
    };
    let icon = if t.state == State::NeedsYou {
        "⚑"
    } else {
        "●"
    };
    let mut title = vec![Span::raw(" ")];
    if let Some((_, name)) = &t.finca {
        title.push(Span::raw(name.clone()).fg(dim(FINCAS_COLOR)));
        title.push(Span::raw(" · ").dark_gray());
    }
    title.push(Span::raw(t.handle()).fg(dim(Color::White)));
    title.push(Span::raw(format!(" {icon} {}", t.state.label())).fg(dim(state_color(t.state))));
    title.push(Span::raw(format!("  {}", super::app::format_age(t.updated_at, now))).dark_gray());
    if draft {
        title.push(Span::raw("  ✎ draft").fg(dim(Color::Cyan)));
    }
    if t.stale {
        title.push(Span::raw("  stale").dark_gray());
    }
    title.push(Span::raw(" "));
    let border = if selected {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else if t.finca.is_some() && !t.stale {
        Style::default().fg(FINCAS_COLOR)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if selected {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
        .border_style(border)
        .title(Line::from(title));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let body: Vec<Line> = match &t.screen {
        None => vec![Line::from(t.state.label().to_string()).dark_gray()],
        Some(_) => {
            let keep = &lines[lines.len().saturating_sub(usize::from(inner.height))..];
            keep.iter()
                .map(|l| Line::from(fit(l, usize::from(inner.width))))
                .collect()
        }
    };
    let style = if t.stale {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
    };
    frame.render_widget(Paragraph::new(body).style(style), inner);
}

/// A screen's lines without Claude's input box and whatever sits under it
/// (status bar, mode line), and trailing blank lines. The box is a `❯`
/// line with a rule above it and a rule closing it below; screens without
/// one come back whole. True when the box held typed, unsent text.
fn trim_prompt(screen: &str) -> (Vec<&str>, bool) {
    let mut lines: Vec<&str> = screen.lines().collect();
    let rule = |l: &str| l.trim_start().starts_with("───");
    let mut draft = false;
    let boxed = (1..lines.len()).rev().find(|&i| {
        lines[i].trim_start().starts_with('❯')
            && rule(lines[i - 1])
            && lines[i + 1..].iter().any(|l| rule(l))
    });
    if let Some(i) = boxed {
        let close = i + 1 + lines[i + 1..].iter().position(|l| rule(l)).unwrap_or(0);
        let typed = lines[i].trim_start().trim_start_matches('❯');
        draft =
            !typed.trim().is_empty() || lines[i + 1..close].iter().any(|l| !l.trim().is_empty());
        lines.truncate(i - 1);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    (lines, draft)
}

/// `s` with tabs as spaces, cut to `width` display columns.
fn fit(s: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for ch in s.replace('\t', "    ").chars() {
        let w = Span::raw(ch.to_string()).width();
        if used + w > width {
            break;
        }
        used += w;
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;

    #[test]
    fn layout_fills_wide_and_tall_screens() {
        assert_eq!(
            layout(4, 110, 28),
            Layout {
                cols: 2,
                rows: 2,
                tile_w: 55,
                tile_h: 14
            }
        );
        assert_eq!(
            layout(1, 200, 60),
            Layout {
                cols: 1,
                rows: 1,
                tile_w: 200,
                tile_h: 60
            }
        );
        // 200 wide fits 5 columns; 7 tiles is 2 rows.
        assert_eq!(
            layout(7, 200, 60),
            Layout {
                cols: 5,
                rows: 2,
                tile_w: 40,
                tile_h: 30
            }
        );
    }

    #[test]
    fn layout_scrolls_instead_of_shrinking() {
        // 80x24: 2 columns, 3 rows of 8; 10 tiles need 5 rows.
        let l = layout(10, 80, 24);
        assert_eq!((l.cols, l.rows, l.tile_h), (2, 3, 8));
    }

    #[test]
    fn tiny_terminals_get_one_clipped_tile() {
        assert_eq!(
            layout(3, 30, 6),
            Layout {
                cols: 1,
                rows: 1,
                tile_w: 30,
                tile_h: 6
            }
        );
        assert_eq!(
            layout(0, 0, 0),
            Layout {
                cols: 1,
                rows: 1,
                tile_w: 0,
                tile_h: 0
            }
        );
    }

    #[test]
    fn first_row_keeps_the_selection_visible() {
        assert_eq!(first_row(0, 2, 3), 0);
        assert_eq!(first_row(5, 2, 3), 0); // row 2
        assert_eq!(first_row(6, 2, 3), 1); // row 3
        assert_eq!(first_row(9, 2, 3), 2); // row 4
        assert_eq!(first_row(4, 2, 0), 2); // no visible rows: no panic
    }

    #[test]
    fn step_moves_within_the_grid() {
        // 5 tiles, 3 columns: [0 1 2] [3 4]
        assert_eq!(step(0, 5, 3, Dir::Left), 0);
        assert_eq!(step(1, 5, 3, Dir::Right), 2);
        assert_eq!(step(2, 5, 3, Dir::Right), 2);
        assert_eq!(step(4, 5, 3, Dir::Right), 4);
        assert_eq!(step(1, 5, 3, Dir::Down), 4);
        assert_eq!(step(2, 5, 3, Dir::Down), 4); // onto a short last row
        assert_eq!(step(4, 5, 3, Dir::Down), 4);
        assert_eq!(step(4, 5, 3, Dir::Up), 1);
        assert_eq!(step(0, 0, 3, Dir::Down), 0);
    }

    #[test]
    fn next_waiting_wraps_and_skips_working() {
        use State::*;
        let s = [Working, NeedsYou, Working, YourTurn, Starting];
        assert_eq!(next_waiting(&s, 0, true), 1);
        assert_eq!(next_waiting(&s, 1, true), 3);
        assert_eq!(next_waiting(&s, 3, true), 1);
        assert_eq!(next_waiting(&s, 1, false), 3);
        assert_eq!(next_waiting(&[Working, Working], 1, true), 1);
        assert_eq!(next_waiting(&[], 0, true), 0);
    }

    const RULE: &str = "────────────────────────────────────────";

    #[test]
    fn trim_prompt_cuts_the_input_box_and_status_bar() {
        let screen = format!(
            "  Enjoy digging into it.\n\n✻ Cogitated for 5s · done 9:51 PM\n\n{RULE}\n❯\u{a0}\n{RULE}\n    🤖 Opus 4.8 | PLAY | 📁 generic | 🌿 main\n  -- INSERT -- ⏵⏵ auto mode on\n"
        );
        let (lines, draft) = trim_prompt(&screen);
        assert_eq!(
            lines,
            [
                "  Enjoy digging into it.",
                "",
                "✻ Cogitated for 5s · done 9:51 PM"
            ]
        );
        assert!(!draft);
    }

    #[test]
    fn trim_prompt_reports_typed_text_as_a_draft() {
        let screen = format!("done\n{RULE}\n❯ fix the\n  failing test\n{RULE}\n  status\n");
        let (lines, draft) = trim_prompt(&screen);
        assert_eq!(lines, ["done"]);
        assert!(draft);
    }

    #[test]
    fn trim_prompt_leaves_screens_without_a_box() {
        let screen = "one\ntwo\n\nthree\n\n\n";
        assert_eq!(
            trim_prompt(screen),
            (vec!["one", "two", "", "three"], false)
        );
    }

    #[test]
    fn trim_prompt_keeps_a_permission_prompt() {
        // Its ❯ marks a choice, with no closing rule under it.
        let screen = format!(
            "{RULE}\n Bash command\n   rm -rf build\n Do you want to proceed?\n❯ 1. Yes\n  2. No\n\n Esc to cancel\n"
        );
        let (lines, draft) = trim_prompt(&screen);
        assert_eq!(lines.len(), 8);
        assert_eq!(lines[4], "❯ 1. Yes");
        assert!(!draft);
    }
}
