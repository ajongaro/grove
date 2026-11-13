//! Draws the dashboard.

use super::theme::pal;
use chrono::{DateTime, Timelike, Utc};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Wrap};

use super::app::{App, FincaItem, FincaRow, Mode, Pane, PromptKind, format_age};
use crate::model::State;

/// Width of each session-state count on a project row: a two-column glyph,
/// two digits, a space.
const SLOT_W: usize = 5;

pub fn render(frame: &mut Frame, app: &App, now: DateTime<Utc>) {
    // The theme's own background and text (grove's are the terminal's).
    frame.render_widget(Block::new().style(base_style()), frame.area());
    // The header gives its rows back to the lists in a short window: the
    // full one shrinks to the compact line, which then goes.
    let height = frame.area().height;
    let header_h = match app.cfg.header {
        Header::Full if height >= HEADER_MIN_HEIGHT => HEADER_ROWS,
        Header::Full | Header::Compact if height >= TICKER_MIN_HEIGHT => 1,
        _ => 0,
    };
    // The ticker takes a row over the footer when there's room for one.
    let ticker = app
        .ticker
        .as_ref()
        .filter(|_| frame.area().height >= TICKER_MIN_HEIGHT)
        .and_then(|t| super::tape::line(t, frame.area().width as usize, now));
    let [header, main, tape, footer] = Layout::vertical([
        Constraint::Length(header_h),
        Constraint::Min(1),
        Constraint::Length(u16::from(ticker.is_some())),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    match header_h {
        0 => {}
        1 => render_compact_header(frame, app, header),
        _ => render_header(frame, app, header, now),
    }
    let grid = match &app.mode {
        Mode::Grid { sel, reply } => Some((*sel, reply.as_ref())),
        Mode::GridKill { sel, .. } => Some((*sel, None)),
        _ => None,
    };
    if let Some((sel, reply)) = grid {
        let area = Rect {
            height: main.height + tape.height + footer.height,
            ..main
        };
        super::grid::render_grid(frame, app, sel, reply, area, now);
        if let Mode::GridKill { handle, .. } = &app.mode {
            kill_popup(frame, handle);
        }
        return;
    }
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(32), Constraint::Percentage(68)]).areas(main);

    // Left: one equal share per account box, plus one for the fincas; a
    // hidden box (H) shrinks to its border and one line.
    let shares: Vec<Constraint> = app
        .groups()
        .iter()
        .map(|g| match app.is_hidden(&g.name) {
            true => Constraint::Length(3),
            false => Constraint::Fill(1),
        })
        .chain((!app.fincas.is_empty()).then_some(Constraint::Fill(1)))
        .collect();
    let groups = app.groups().len();
    let mut boxes = Layout::vertical(shares).split(left).to_vec();
    if let Some(fincas) = boxes.get(groups).copied() {
        render_fincas(frame, app, fincas, now);
        boxes.truncate(groups);
    }
    render_project_boxes(frame, app, &boxes);
    let pins = app.pinned_rows();
    let right = if pins.is_empty() {
        right
    } else {
        let h = pins.len().min(9) as u16 + 2;
        let [top, rest] =
            Layout::vertical([Constraint::Length(h), Constraint::Fill(1)]).areas(right);
        render_pinned(frame, app, &pins, top, now);
        rest
    };
    match app.selected_finca() {
        Some(f) => render_finca_detail(frame, app, f, right, now),
        None if app.todo_slug.is_some() => {
            let [sessions, todo] =
                Layout::vertical([Constraint::Fill(1), Constraint::Fill(1)]).areas(right);
            render_sessions(frame, app, sessions, now);
            render_todos(frame, app, todo);
        }
        None => render_sessions(frame, app, right, now),
    }
    if let Some(line) = ticker {
        frame.render_widget(Paragraph::new(line), tape);
    }
    render_footer(frame, app, footer, now);

    match &app.mode {
        Mode::Prompt { kind, input } => {
            let title = match kind {
                PromptKind::NewSession => " Session name (Enter for default, Esc to cancel) ",
                PromptKind::AddProject => {
                    " Add project — path (Tab completes, Enter adds, Esc cancels) "
                }
                PromptKind::AddFinca => {
                    " Add finca — SSH destination (Tab completes, Enter sets it up, Esc cancels) "
                }
                PromptKind::RenameSession { .. } => " Rename session (Enter saves, Esc cancels) ",
                PromptKind::RenameShell { .. } => " Rename shell (Enter saves, Esc cancels) ",
            };
            let area = popup(frame.area(), 70, 3);
            blank(frame, area);
            frame.render_widget(
                Paragraph::new(format!("{input}█")).block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .title(title)
                        .fg(pal().accent),
                ),
                area,
            );
            render_completions(frame, app, area);
        }
        Mode::Help => render_help(frame),
        Mode::Themes { sel, .. } => render_themes(frame, *sel),
        Mode::Search { query, sel } => render_search(frame, app, query, *sel, now),
        Mode::Confirm { label, .. } => kill_popup(frame, label),
        Mode::ConfirmShell { label, .. } => {
            confirm_popup(frame, &format!("Close shell {label}?  y / n"))
        }
        Mode::ConfirmRemove { name, .. } => {
            let area = popup(frame.area(), 60, 3);
            blank(frame, area);
            frame.render_widget(
                Paragraph::new(format!("Remove project {name}? Its folder stays.  y / n")).block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .title(" Confirm ")
                        .fg(pal().error),
                ),
                area,
            );
        }
        Mode::ConfirmCreate { path } => {
            let area = popup(frame.area(), 70, 3);
            blank(frame, area);
            frame.render_widget(
                Paragraph::new(format!("Create {path} and git init it?  y / n")).block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .title(" New project folder ")
                        .fg(pal().accent),
                ),
                area,
            );
        }
        // Drawn by grid::render_grid instead (plan task 8).
        Mode::Normal | Mode::Grid { .. } | Mode::GridKill { .. } => {}
    }
}

/// Tab-completion matches, in a box just under the prompt.
fn render_completions(frame: &mut Frame, app: &App, prompt: Rect) {
    const MAX_ROWS: usize = 8;
    if app.completions.is_empty() {
        return;
    }
    let screen = frame.area();
    let room = screen.bottom().saturating_sub(prompt.bottom()) as usize;
    let rows = app
        .completions
        .len()
        .min(MAX_ROWS)
        .min(room.saturating_sub(2));
    if rows == 0 {
        return;
    }
    // Scroll so the highlighted match stays visible.
    let selected = app.completion_idx.unwrap_or(0);
    let start = selected.saturating_sub(rows - 1);
    let items: Vec<ListItem> = app.completions[start..start + rows]
        .iter()
        .map(|p| {
            let name = p.trim_end_matches('/').rsplit('/').next().unwrap_or(p);
            ListItem::new(format!("{name}/"))
        })
        .collect();
    let area = Rect {
        x: prompt.x,
        y: prompt.bottom(),
        width: prompt.width,
        height: rows as u16 + 2,
    };
    let title = format!(" {} matches — Tab or j/k to cycle ", app.completions.len());
    let mut state = ListState::default().with_selected(app.completion_idx.map(|i| i - start));
    blank(frame, area);
    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .title(title)
                    .fg(pal().dim),
            )
            .highlight_style(highlight(true)),
        area,
        &mut state,
    );
}

/// A pane titled with `spans`. Styling the spans themselves keeps their
/// colors when the focused border style is applied.
fn pane_block<'a>(spans: Vec<Span<'a>>, focused: bool) -> Block<'a> {
    let mut title = vec![Span::raw(" ")];
    // Uncolored words would take the border's (focus) color.
    title.extend(spans.into_iter().map(|s| match s.style.fg {
        Some(_) => s,
        None => s.fg(pal().muted),
    }));
    title.push(Span::raw(" "));
    let b = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(title));
    if focused {
        b.border_style(Style::new().fg(pal().accent))
    } else {
        b.border_style(Style::new().fg(pal().dim))
    }
}

/// One box per account group (play above work) in `areas`; a single
/// PROJECTS box when no account owns a folder.
fn render_project_boxes(frame: &mut Frame, app: &App, areas: &[Rect]) {
    let groups = app.groups();
    let mut start = 0;
    for (g, area) in groups.iter().zip(areas.iter()) {
        let len = app.projects[start..]
            .iter()
            .take_while(|p| app.account_of(p).name == g.name)
            .count();
        let title = if groups.len() == 1 {
            "PROJECTS".to_string()
        } else {
            g.name.to_uppercase()
        };
        if app.is_hidden(&g.name) {
            let block = pane_block(vec![Span::raw(title).fg(pal().dim)], false);
            let note = Paragraph::new("hidden · H shows it").fg(pal().dim);
            frame.render_widget(note.block(block), *area);
            continue;
        }
        let empty = match app
            .cfg
            .accounts
            .get(&g.name)
            .and_then(|a| a.folder.as_deref())
        {
            Some(folder) => format!("No projects in {folder}."),
            None => "No projects yet — press a to add one.".to_string(),
        };
        render_projects(
            frame,
            app,
            *area,
            &title,
            g.color.as_deref(),
            start..start + len,
            &empty,
        );
        start += len;
    }
}

fn render_projects(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    title: &str,
    color: Option<&str>,
    rows: std::ops::Range<usize>,
    empty: &str,
) {
    let has_selection = app.pane == Pane::Projects && rows.contains(&app.proj_idx);
    let word = match parse_color(color) {
        Some(c) => Span::raw(title.to_string()).fg(c),
        None => Span::raw(title.to_string()),
    };
    let block = pane_block(vec![word], has_selection);
    if rows.is_empty() {
        let p = Paragraph::new(empty.to_string())
            .wrap(Wrap { trim: true })
            .fg(pal().dim)
            .block(block);
        frame.render_widget(p, area);
        return;
    }
    let inner_w = area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = app.projects[rows.clone()]
        .iter()
        .map(|p| {
            if p.error.is_some() {
                return ListItem::new(Line::from(vec![
                    Span::raw("✗ ").fg(pal().error),
                    Span::raw(p.name.clone()).fg(pal().error),
                    Span::raw(" (bad file)").fg(pal().dim),
                ]));
            }
            let dot = Span::styled(
                "● ",
                Style::new().fg(parse_color(p.color.as_deref()).unwrap_or(pal().accent)),
            );
            let todo = app
                .open_todos
                .get(&p.slug)
                .filter(|n| **n > 0)
                .map(|n| format!("  ☐{n}"));
            // Live sessions by state, most urgent first, in the Sessions
            // pane's colors (starting counts as working). Each state has a
            // fixed slot at the right edge so the counts line up.
            let count = |want: &[State]| {
                app.sessions
                    .iter()
                    .filter(|s| s.project == p.slug && want.contains(&s.state))
                    .count()
            };
            let slots = [
                (State::NeedsYou, count(&[State::NeedsYou])),
                (State::YourTurn, count(&[State::YourTurn])),
                (State::Working, count(&[State::Working, State::Starting])),
            ];
            let mut name = p.name.clone();
            let mut right = Vec::new();
            if slots.iter().any(|(_, n)| *n > 0) {
                right = slots
                    .iter()
                    .map(|&(state, n)| match n {
                        0 => Span::raw(" ".repeat(SLOT_W)),
                        n => Span::styled(
                            format!("{}{n:<w$}", state.glyph(), w = SLOT_W - 2),
                            Style::new().fg(state_color(state)),
                        ),
                    })
                    .collect();
                let fixed = 2 + todo.as_ref().map_or(0, |t| t.chars().count()) + 3 * SLOT_W + 1;
                let room = inner_w.saturating_sub(fixed);
                if name.chars().count() > room {
                    name = name.chars().take(room.saturating_sub(1)).collect();
                    name.push('…');
                }
            }
            let mut spans = vec![dot, Span::raw(name)];
            if let Some(t) = todo {
                spans.push(Span::raw(t).fg(pal().dim));
            }
            if !right.is_empty() {
                let used: usize = spans.iter().map(|s| s.width()).sum();
                let pad = inner_w.saturating_sub(used + 3 * SLOT_W);
                spans.push(Span::raw(" ".repeat(pad.max(1))));
                spans.extend(right);
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    // The selection may be in another box, or down in the fincas list.
    let selected = rows
        .contains(&app.proj_idx)
        .then(|| app.proj_idx - rows.start);
    let mut state = ListState::default().with_selected(selected);
    let list = List::new(items)
        .block(block)
        .highlight_style(highlight(app.pane == Pane::Projects));
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_fincas(frame: &mut Frame, app: &App, area: Rect, now: DateTime<Utc>) {
    use crate::finca::Reach;
    let block = pane_block(
        vec![Span::raw("FINCAS").fg(pal().fincas)],
        app.pane == Pane::Projects && app.selected_finca().is_some(),
    );
    let items: Vec<ListItem> = app
        .fincas
        .iter()
        .map(|f| {
            let color = parse_color(f.color.as_deref()).unwrap_or(pal().accent);
            let name = Span::raw(f.name.clone());
            let line = match (&f.error, &f.reach) {
                (Some(_), _) => vec![
                    Span::raw("✗ ").fg(pal().error),
                    name.fg(pal().error),
                    Span::raw(" (bad file)").fg(pal().dim),
                ],
                (None, None) => vec![
                    Span::raw("◌ ").fg(pal().dim),
                    name,
                    Span::raw("  checking…").fg(pal().dim),
                ],
                (None, Some(Reach::Reachable(st))) => {
                    let mut v = vec![Span::styled("● ", Style::new().fg(color)), name];
                    v.push(Span::raw(format!("  {} live", st.live)).fg(pal().dim));
                    // A different Grove there: its version, in warning colour.
                    if crate::finca::version_gap(&f.name, &f.slug, st.grove.as_deref()).is_some() {
                        let there = st.grove.as_deref().unwrap_or("old");
                        v.push(Span::raw(format!("  v{there}")).fg(pal().warn));
                    }
                    if st.needs_you > 0 {
                        v.push(
                            Span::raw(format!("  {}{}", State::NeedsYou.glyph(), st.needs_you))
                                .fg(pal().error)
                                .bold(),
                        );
                    }
                    v
                }
                (None, Some(Reach::Unreachable { .. })) => {
                    let since = f
                        .last_ok
                        .map_or("never".to_string(), |t| format_age(t, now));
                    vec![
                        Span::raw("○ ").fg(pal().dim),
                        name.fg(pal().dim),
                        Span::raw(format!("  unreachable · {since}")).fg(pal().dim),
                    ]
                }
                (None, Some(Reach::NeedsUpdate { .. })) => {
                    vec![
                        Span::raw("◐ ").fg(pal().warn),
                        name,
                        Span::raw("  needs update").fg(pal().warn),
                    ]
                }
            };
            ListItem::new(Line::from(line))
        })
        .collect();
    let selected = app.proj_idx.checked_sub(app.projects.len());
    let mut state = ListState::default().with_selected(selected);
    let list = List::new(items)
        .block(block)
        .highlight_style(highlight(app.pane == Pane::Projects));
    frame.render_stateful_widget(list, area, &mut state);
}

/// Read-only view of a finca's sessions, from its latest (or last good) status.
fn render_finca_detail(frame: &mut Frame, app: &App, f: &FincaRow, area: Rect, now: DateTime<Utc>) {
    use crate::finca::Reach;
    let host = f
        .last_status
        .as_ref()
        .map(|s| format!(" ({})", s.host))
        .unwrap_or_default();
    let title = vec![
        Span::raw("FINCA").fg(pal().fincas),
        Span::raw(format!(" — {}{host}", f.name)),
    ];
    let block = pane_block(title, app.pane == Pane::Sessions);
    let mut lines: Vec<Line> = Vec::new();
    match &f.reach {
        None => lines.push(Line::raw("Checking…").fg(pal().dim)),
        Some(Reach::NeedsUpdate { here }) => lines.push(
            Line::raw(format!(
                "{}. Enter still connects.",
                crate::finca::needs_update_advice(&f.name, &f.slug, *here)
            ))
            .fg(pal().warn),
        ),
        Some(Reach::Unreachable { error }) => {
            let since = f.last_ok.map_or("never reachable".to_string(), |t| {
                format!("last reachable {} ago", format_age(t, now))
            });
            lines.push(Line::raw(format!("Unreachable: {error}")).fg(pal().error));
            lines.push(Line::raw(format!("({since}; showing last known sessions)")).fg(pal().dim));
            lines.push(Line::raw(""));
        }
        Some(Reach::Reachable(st)) => {
            if let Some((_, gap)) = crate::finca::version_gap(&f.name, &f.slug, st.grove.as_deref())
            {
                lines.push(Line::raw(gap).fg(pal().warn));
                lines.push(Line::raw(""));
            }
        }
    }
    let items = app.finca_items();
    let name_w = name_width(items.iter().filter_map(|i| match i {
        FincaItem::Session { session, .. } => Some(session.name.as_str()),
        FincaItem::Shell(_) => None,
    }));
    // The selected row, filled to the pane's width like a list highlight.
    let inner = area.width.saturating_sub(2) as usize;
    let mark = |line: Line<'static>, i: usize| {
        if i != app.sess_idx {
            return line;
        }
        let pad = inner.saturating_sub(line.width());
        let mut line = line.patch_style(highlight(app.pane == Pane::Sessions));
        line.spans.push(Span::raw(" ".repeat(pad)));
        line
    };
    let no_sessions = !items.iter().any(|i| matches!(i, FincaItem::Session { .. }));
    if f.last_status.is_some() && no_sessions {
        lines.push(Line::raw(format!("No live sessions on {}.", f.name)).fg(pal().dim));
    }
    let mut heading: Option<&str> = None;
    for (i, item) in items.iter().enumerate() {
        match item {
            FincaItem::Session {
                project,
                session: s,
            } => {
                if heading != Some(project.as_str()) {
                    heading = Some(project);
                    lines.push(Line::raw(project.clone()).bold());
                }
                let color = state_color(s.state);
                lines.push(mark(
                    Line::from(vec![
                        Span::styled(format!("  {} ", s.state.glyph()), Style::new().fg(color)),
                        Span::raw(format!("{:<w$}", s.name, w = name_w)).bold(),
                        Span::styled(format!("{:<10}", s.state.label()), Style::new().fg(color)),
                        Span::raw(format!(" · {}", format_age(s.updated_at, now))).fg(pal().dim),
                    ]),
                    i,
                ));
                if let Some(m) = &s.message {
                    lines.push(Line::styled(format!("      {m}"), Style::new().fg(color)));
                }
            }
            FincaItem::Shell(w) => {
                if heading != Some("") {
                    heading = Some("");
                    lines.push(Line::raw("Shells").bold());
                }
                let mut spans = vec![
                    Span::raw("  ●  ").fg(pal().fincas),
                    Span::raw(format!("{:<w$}", w.label(), w = name_w)).bold(),
                ];
                if let Some(t) = &w.title {
                    spans.push(Span::raw(t.clone()).fg(pal().dim));
                }
                lines.push(mark(Line::from(spans), i));
            }
        }
    }
    lines.push(Line::raw(""));
    lines.push(
        Line::raw("Enter: open · n: new session there · s: shell · C-q: come back").fg(pal().dim),
    );
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        area,
    );
}

fn render_sessions(frame: &mut Frame, app: &App, area: Rect, now: DateTime<Utc>) {
    let mut title = vec![Span::raw("SESSIONS").fg(pal().sessions)];
    if let Some(p) = app.selected_project() {
        let a = app.account_of(p);
        let account = Span::raw(a.name.to_uppercase());
        title.push(Span::raw(" — "));
        title.push(match parse_color(a.color.as_deref()) {
            Some(c) => account.fg(c),
            None => account,
        });
        title.push(Span::raw(format!(" — {}", p.name)));
    }
    let block = pane_block(title, app.pane == Pane::Sessions);
    let sessions = app.visible_sessions();
    let shells = app.visible_shells();
    if sessions.is_empty() && shells.is_empty() {
        let hint = if app.selected_project().is_some() {
            "No sessions — press Enter to start one, n to name it."
        } else {
            ""
        };
        let mut lines = Vec::new();
        // Nothing running here at all: the grove rests above the hint.
        let scene = flair::quiet_grove();
        let idle = app.sessions.iter().all(|s| s.state == State::Ended);
        let inner = block.inner(area);
        let scene_w = scene.iter().map(Line::width).max().unwrap_or(0);
        if idle && inner.height as usize >= scene.len() + 3 && inner.width as usize >= scene_w {
            lines.extend(scene);
            lines.push(Line::raw(""));
        }
        lines.push(Line::raw(hint));
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(block),
            area,
        );
        return;
    }
    let names = sessions.iter().map(|s| s.name.as_str());
    let name_w = name_width(names.chain(shells.iter().map(|s| s.name.as_str())));
    let shell_rows = shells.iter().map(|s| {
        ListItem::new(Line::from(vec![
            // As wide as a session's glyph and space, so the columns match.
            Span::raw("$  ").fg(pal().shell),
            Span::raw(format!("{:<w$}", s.name, w = name_w)).bold(),
            Span::raw(format!("{:<10}", "shell")).fg(pal().shell),
            Span::raw(format!(" · {}", s.command)).fg(pal().dim),
        ]))
    });
    let items: Vec<ListItem> = sessions
        .iter()
        .map(|s| {
            let color = state_color(s.state);
            let mut spans = vec![
                Span::styled(format!("{} ", s.state.glyph()), Style::new().fg(color)),
                Span::raw(format!("{:<w$}", s.name, w = name_w)).bold(),
                Span::styled(format!("{:<10}", s.state.label()), Style::new().fg(color)),
                Span::raw(format!(" · {:<4}", format_age(s.updated_at, now))).fg(pal().dim),
            ];
            if let Some(st) = app.stats.get(&s.id) {
                spans.push(Span::raw(format!(" · {}", stats_label(st))).fg(pal().dim));
            }
            let mut lines = vec![Line::from(spans)];
            // On its own line so long permission prompts aren't cut off.
            if let Some(m) = &s.message {
                lines.push(Line::from(Span::styled(
                    format!("    {m}"),
                    Style::new().fg(color),
                )));
            }
            ListItem::new(lines)
        })
        .chain(shell_rows)
        .collect();
    let mut state = ListState::default().with_selected(Some(app.sess_idx));
    let list = List::new(items)
        .block(block)
        .highlight_style(highlight(app.pane == Pane::Sessions));
    frame.render_stateful_widget(list, area, &mut state);
}

/// A session's cost and context use, as its status line last saw them:
/// `$1.23 · 23%`.
pub(super) fn stats_label(st: &crate::stats::SessionStats) -> String {
    match st.context_pct {
        Some(p) => format!("${:.2} · {}%", st.last.cost, p.clamp(0.0, 100.0) as u32),
        None => format!("${:.2}", st.last.cost),
    }
}

/// Session-name column: the longest name plus a gap, so states line up and
/// never run into a name.
fn name_width<'a>(names: impl Iterator<Item = &'a str>) -> usize {
    names.map(|n| n.chars().count()).max().unwrap_or(0).max(12) + 2
}

fn render_todos(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.pane == Pane::Todo;
    let block = pane_block(vec![Span::raw("TO-DO").fg(pal().todo)], focused);
    let t = &app.todos;
    if t.items.is_empty() {
        let p = Paragraph::new("No to-dos — press o (or a) to add one (t or Tab to get here).")
            .wrap(Wrap { trim: true })
            .fg(pal().dim)
            .block(block);
        frame.render_widget(p, area);
        return;
    }
    let items: Vec<ListItem> = t
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let mut text = item.text.clone();
            if focused && t.inserting() && i == t.idx {
                text.push('█');
            }
            let line = if item.done {
                Line::from(vec![
                    Span::raw("[x] ").fg(pal().dim),
                    Span::raw(text).fg(pal().dim).crossed_out(),
                ])
            } else {
                Line::from(vec![Span::raw("[ ] ").fg(pal().accent), Span::raw(text)])
            };
            ListItem::new(line)
        })
        .collect();
    let mut state = ListState::default().with_selected(focused.then_some(t.idx));
    let list = List::new(items)
        .block(block)
        .highlight_style(highlight(focused));
    frame.render_stateful_widget(list, area, &mut state);
}

/// The key list in two columns, sections kept whole, so it fits a laptop.
/// The `/` popup: the query, then each match on two rows, its handle and
/// machine, then what it's about; matched letters stand out.
fn render_search(frame: &mut Frame, app: &App, query: &str, sel: usize, now: DateTime<Utc>) {
    let hits = super::search::search(app, query);
    let screen = frame.area();
    let rows = (hits.len().max(1) * 2 + 4) as u16;
    let mut area = popup(screen, 80, rows.min(screen.height.saturating_sub(2)));
    area.width = area.width.max(screen.width.min(60));
    area.x = screen.x + (screen.width - area.width) / 2;
    blank(frame, area);
    let inner_w = area.width.saturating_sub(2) as usize;
    let fits = (area.height.saturating_sub(4) / 2).max(1) as usize;
    let first = sel.saturating_sub(fits - 1);
    let lit = |text: &str, hl: &[usize], base: Style| -> Vec<Span<'static>> {
        text.chars()
            .enumerate()
            .map(|(i, c)| {
                let style = if hl.contains(&i) {
                    base.fg(pal().accent).bold()
                } else {
                    base
                };
                Span::styled(c.to_string(), style)
            })
            .collect()
    };
    let mut lines = vec![
        Line::from(vec![
            Span::raw(" / ").fg(pal().accent).bold(),
            Span::raw(format!("{query}█")),
        ]),
        Line::raw(""),
    ];
    if hits.is_empty() {
        lines.push(Line::raw("   no sessions match").fg(pal().dim));
    }
    for (i, h) in hits.iter().enumerate().skip(first).take(fits) {
        let it = &h.item;
        let ended = it.state == State::Ended;
        let base = if ended {
            Style::new().fg(pal().dim)
        } else {
            Style::new()
        };
        let mut top = vec![
            Span::raw(if i == sel { " ▸ " } else { "   " }).fg(pal().accent),
            Span::raw(format!("{} ", it.state.glyph())),
        ];
        top.extend(lit(&it.handle, &h.handle_hl, base));
        let right = format!("{} · {} ", it.host, format_age(it.updated_at, now));
        let used: usize = top.iter().map(Span::width).sum();
        top.push(Span::raw(
            " ".repeat(inner_w.saturating_sub(used + right.chars().count())),
        ));
        top.push(Span::raw(right).fg(pal().dim));
        let mut top = Line::from(top);
        if i == sel {
            top = top.bg(pal().selected_bg);
        }
        lines.push(top);
        let about = it.about.as_deref().unwrap_or("");
        let room = inner_w.saturating_sub(7);
        let about: String = about.chars().take(room).collect();
        let mut bottom = vec![Span::raw("      ")];
        bottom.extend(lit(&about, &h.about_hl, Style::new().fg(pal().dim)));
        lines.push(Line::from(bottom));
    }
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" Find a session ")
                .title_bottom(" Enter jump / resume · ↑↓ move · Esc close ")
                .border_style(Style::new().fg(pal().accent)),
        ),
        area,
    );
}

fn render_help(frame: &mut Frame) {
    let sections: Vec<Vec<Line>> = super::help::SECTIONS
        .iter()
        .map(|(section, keys)| {
            // Each heading in its pane's color.
            let color = match *section {
                "Projects" => pal().accent,
                "Sessions" => pal().sessions,
                "To-do" => pal().todo,
                "Fincas" => pal().fincas,
                _ => pal().bright,
            };
            let mut lines = vec![Line::raw(*section).bold().fg(color)];
            for (k, what) in *keys {
                let mut spans = vec![
                    Span::raw("  "),
                    Span::raw(format!("{k:<10} ")).bold().fg(pal().key),
                ];
                // A trailing aside in parentheses is dimmed.
                match what.find(" (").filter(|_| what.ends_with(')')) {
                    Some(i) => {
                        spans.push(Span::raw(&what[..i]));
                        spans.push(Span::raw(&what[i..]).fg(pal().dim));
                    }
                    None => spans.push(Span::raw(*what)),
                }
                lines.push(Line::from(spans));
            }
            lines.push(Line::raw(""));
            lines
        })
        .collect();
    // Split where the taller column is shortest.
    let total: usize = sections.iter().map(Vec::len).sum();
    let mut acc = 0;
    let split = (0..=sections.len())
        .min_by_key(|&k| {
            let l = acc;
            acc += sections.get(k).map_or(0, Vec::len);
            l.max(total - l)
        })
        .unwrap_or(0);
    let mut left = Vec::new();
    let mut right = Vec::new();
    for (k, s) in sections.into_iter().enumerate() {
        if k < split {
            left.extend(s);
        } else {
            right.extend(s);
        }
    }
    // No spacer under a column's last section.
    for col in [&mut left, &mut right] {
        if col.last().is_some_and(|l| l.width() == 0) {
            col.pop();
        }
    }
    // Plus the top and bottom border.
    let height = left.len().max(right.len()) as u16 + 2;
    let area = popup(frame.area(), 90, height);
    blank(frame, area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(pal().dim))
        .title(Line::from(vec![
            Span::raw(" Keys ").fg(pal().accent).bold(),
            Span::raw(format!(
                "(any key closes) · Grove {} ",
                crate::update::this_version()
            ))
            .fg(pal().dim),
        ]))
        .title_bottom(state_legend());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [l, r] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(inner);
    frame.render_widget(Paragraph::new(left), l);
    frame.render_widget(Paragraph::new(right), r);
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect, now: DateTime<Utc>) {
    let mut spans = Vec::new();
    let n = app.needs_you_count();
    if n > 0 {
        let remote: Vec<String> = app
            .remote_needs_you()
            .iter()
            .map(|(name, k)| format!("{k} on {name}"))
            .collect();
        let detail = if remote.is_empty() {
            String::new()
        } else {
            format!("({}) ", remote.join(", "))
        };
        spans.push(
            Span::raw(format!(
                " {} {n} needs you {detail}",
                State::NeedsYou.glyph()
            ))
            .fg(pal().on_alert)
            .bg(pal().needs_you)
            .bold(),
        );
        spans.push(Span::raw(" "));
    }
    match &app.footer {
        Some(msg) => spans.push(Span::raw(msg.clone()).fg(pal().warn)),
        None if app.pane == Pane::Todo && app.todos.inserting() => {
            spans.push(
                Span::raw(" -- INSERT -- ")
                    .fg(pal().ink)
                    .bg(pal().warn)
                    .bold(),
            );
            spans.push(Span::raw(" enter/esc save · ctrl-u clear").fg(pal().dim));
        }
        None => {
            let (color, hints) = super::hints::for_focus(app);
            let used: usize = spans.iter().map(Span::width).sum();
            let room = (area.width as usize).saturating_sub(used + 1);
            // Now and then, when nothing else needs saying, the grove speaks,
            // as long as the top few hints still fit beside it.
            let quip = flair::quip(now).filter(|_| n == 0).and_then(|q| {
                let q = Span::raw(q).fg(pal().dim).italic();
                let keys = room.checked_sub(q.width() + 2)?;
                (super::hints::fit(&hints, keys) >= hints.len().min(3)).then_some((q, keys))
            });
            match quip {
                Some((q, keys)) => {
                    let h = super::hints::spans(&hints, color, keys);
                    let w: usize = h.iter().map(Span::width).sum();
                    spans.extend(h);
                    spans.push(Span::raw(
                        " ".repeat(room.saturating_sub(w + q.width()).max(2)),
                    ));
                    spans.push(q);
                }
                None => spans.extend(super::hints::spans(&hints, color, room)),
            }
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn highlight(focused: bool) -> Style {
    if focused {
        Style::new()
            .bg(pal().cursor_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().bg(pal().cursor_dim_bg)
    }
}

/// Rows the header takes (the logo's two plus a blank one under it), and
/// the window height below which it's hidden.
const HEADER_ROWS: u16 = 3;
const HEADER_MIN_HEIGHT: u16 = 24;
/// The window height below which the ticker gives its row back.
const TICKER_MIN_HEIGHT: u16 = 16;

use super::flair;
use crate::config::Header;

/// An olive branch growing to the right: (text, color) runs per row. The
/// stem is a run of upper half blocks, so it sits where the rows meet.
const WORDMARK: [&str; 2] = ["█▀▀ █▀█ █▀█ █ █ █▀▀", "█▄█ █▀▄ █▄█ ▀▄▀ ██▄"];

/// Sessions needing you, waiting on you and working, in their colors.
fn header_totals(app: &App) -> Line<'static> {
    let count = |want: &[State]| {
        app.sessions
            .iter()
            .filter(|s| want.contains(&s.state))
            .count()
    };
    let total = |n: usize, label: &str, state: State| {
        let style = if n > 0 {
            Style::new().fg(state_color(state))
        } else {
            Style::new().fg(pal().dim)
        };
        Span::styled(format!("{}{n} {label}", state.glyph()), style)
    };
    Line::from(vec![
        total(count(&[State::NeedsYou]), "needs you", State::NeedsYou),
        Span::raw("  "),
        total(count(&[State::YourTurn]), "your turn", State::YourTurn),
        Span::raw("  "),
        total(
            count(&[State::Working, State::Starting]),
            "working",
            State::Working,
        ),
        Span::raw(" "),
    ])
}

/// This machine, and how many fincas are up.
fn header_place(app: &App) -> Vec<Span<'static>> {
    let mut place = app.host.clone();
    if !app.fincas.is_empty() {
        let up = app
            .fincas
            .iter()
            .filter(|f| matches!(f.reach, Some(crate::finca::Reach::Reachable(_))))
            .count();
        place = format!("{place} · fincas {up}/{} up", app.fincas.len());
    }
    vec![Span::raw(place).fg(pal().dim)]
}

/// Grove's version, and a newer one that's out (update.rs).
fn version_spans(app: &App) -> Vec<Span<'static>> {
    let mut spans = vec![Span::raw(format!("v{}", crate::update::this_version())).fg(pal().dim)];
    if let Some((v, _)) = &app.update {
        spans.push(Span::raw(format!(" → {v}")).fg(pal().warn).bold());
    }
    spans
}

/// The compact top bar (`header = "compact"`, or a short window): the name
/// and place on the left, the totals on the right, no art.
fn render_compact_header(frame: &mut Frame, app: &App, area: Rect) {
    let totals = header_totals(app);
    let mut left = vec![Span::raw(" grove ").fg(pal().accent).bold()];
    left.extend(version_spans(app));
    let place = header_place(app);
    let place_w: usize = place.iter().map(Span::width).sum();
    let used: usize = left.iter().map(Span::width).sum();
    if used + 2 + place_w + totals.width() < area.width as usize {
        left.push(Span::raw("  "));
        left.extend(place);
    }
    frame.render_widget(Paragraph::new(Line::from(left)), area);
    frame.render_widget(
        Paragraph::new(totals).alignment(ratatui::layout::Alignment::Right),
        area,
    );
}

/// Logo and a window of sky with its olive trees on the left, leaves blowing
/// through while anything works; this machine's session totals, the fincas that are up and the day's
/// harvest on the right.
fn render_header(frame: &mut Frame, app: &App, area: Rect, now: DateTime<Utc>) {
    let totals = header_totals(app);
    let mut place = Line::from(header_place(app));
    place.spans.push(Span::raw(" "));

    let local = now.with_timezone(&chrono::Local);
    let today = local.format("%Y-%m-%d").to_string();
    let mut harvest = flair::harvest_line(
        &crate::stats::harvest(&app.stats, &today),
        &app.cfg,
        (area.width as usize).saturating_sub(4),
    );

    // The sky fills what the totals leave beside the wordmark: it narrows,
    // then goes, in a narrow window. Its grass stops short of the harvest.
    let wordmark_w = Span::raw(WORDMARK[0]).width();
    let left = 1 + wordmark_w + 2;
    let sky_w = (area.width as usize)
        .saturating_sub(left + totals.width().max(place.width()) + 1)
        .min(flair::SKY_MAX);
    let ground = (area.width as usize)
        .saturating_sub(left + harvest.width() + 2)
        .min(sky_w);
    // The weather: rain while anything needs you, leaves while anything
    // works, sun when all is calm.
    let any = |state| app.sessions.iter().any(|s| s.state == state);
    let mood = if any(State::NeedsYou) {
        flair::Mood::Rain
    } else if any(State::Working) {
        flair::Mood::Busy
    } else {
        flair::Mood::Calm
    };
    let sky = (sky_w >= flair::SKY_MIN).then(|| flair::sky(sky_w, ground, local.hour(), now, mood));
    let logo: Vec<Line> = (0..3)
        .map(|i| {
            let mut spans = vec![Span::raw(" ")];
            match WORDMARK.get(i) {
                Some(w) => spans.push(Span::styled(*w, Style::new().fg(pal().accent).bold())),
                // The version under the wordmark, right-aligned to it.
                None => {
                    let v = version_spans(app);
                    let w: usize = v.iter().map(Span::width).sum();
                    spans.push(Span::raw(" ".repeat(wordmark_w.saturating_sub(w))));
                    spans.extend(v);
                }
            }
            if let Some(sky) = &sky {
                spans.push(Span::raw("  "));
                spans.extend(sky[i].spans.iter().cloned());
            }
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(logo), area);

    harvest.spans.push(Span::raw(" "));
    frame.render_widget(
        Paragraph::new(vec![totals, place, harvest]).alignment(ratatui::layout::Alignment::Right),
        area,
    );
}

/// What each session glyph means, in its colour.
fn state_legend() -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for s in [
        State::Starting,
        State::Working,
        State::YourTurn,
        State::NeedsYou,
        State::Ended,
    ] {
        spans.push(Span::raw(format!("{} {}  ", s.glyph(), s.label())).fg(state_color(s)));
    }
    spans.pop();
    spans.push(
        Span::raw(format!(
            "{} {} ",
            State::Ended.glyph(),
            State::Ended.label()
        ))
        .fg(pal().dim),
    );
    Line::from(spans)
}

pub(super) fn state_color(s: State) -> Color {
    match s {
        State::NeedsYou => pal().needs_you,
        State::YourTurn => pal().your_turn,
        State::Working => pal().working,
        State::Starting => pal().starting,
        State::Ended => pal().dim,
    }
}

/// A color from config.toml or a project: `#rrggbb`, or a name (`blue`)
/// in the theme the dashboard is drawing with.
pub(super) fn parse_color(c: Option<&str>) -> Option<Color> {
    let (r, g, b) = super::theme::lookup(c?, pal())?;
    Some(Color::Rgb(r, g, b))
}

/// The theme's background and text color.
fn base_style() -> Style {
    Style::new().bg(pal().bg).fg(pal().fg)
}

/// Empties `area` for a popup, back to the theme's background.
fn blank(frame: &mut Frame, area: Rect) {
    frame.render_widget(Clear, area);
    frame.render_widget(Block::new().style(base_style()), area);
}

/// A centered box `width_pct`% wide and `height` rows tall, clamped to `area`.
fn popup(area: Rect, width_pct: u16, height: u16) -> Rect {
    let w = (area.width * width_pct / 100).max(20).min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

/// "Kill session <label>?  y / n", centred.
fn kill_popup(frame: &mut Frame, label: &str) {
    confirm_popup(frame, &format!("Kill session {label}?  y / n"));
}

fn confirm_popup(frame: &mut Frame, text: &str) {
    let area = popup(frame.area(), 44, 3);
    blank(frame, area);
    frame.render_widget(
        Paragraph::new(text.to_string()).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" Confirm ")
                .fg(pal().error),
        ),
        area,
    );
}

/// The Pinned pane: one row per pin, numbered for 1–9, each saying where
/// its session lives (account or finca).
fn render_pinned(
    frame: &mut Frame,
    app: &App,
    rows: &[super::app::PinRow],
    area: Rect,
    now: DateTime<Utc>,
) {
    let focused = app.pane == Pane::Pinned;
    // Its own colour: green is the focus border's.
    let mut title = vec![Span::raw("PINNED").fg(pal().key)];
    if app.cfg.pin_order != crate::config::PinOrder::Added {
        title.push(Span::raw(format!(" · {}", app.cfg.pin_order.name())).fg(pal().dim));
    }
    let block = pane_block(title, focused);
    let origins: Vec<String> = rows
        .iter()
        .map(|r| match (&r.finca, &r.account) {
            (Some((_, name, _)), _) => name.clone(),
            (None, Some(a)) => a.name.clone(),
            (None, None) => String::new(),
        })
        .collect();
    // Columns: where it runs, the project, the session's own name.
    let width = |f: &dyn Fn(usize) -> usize| (0..rows.len()).map(f).max().unwrap_or(0);
    let origin_w = width(&|i| origins[i].chars().count());
    let project_w = width(&|i| rows[i].project.chars().count());
    let name_w = width(&|i| rows[i].name.chars().count());
    let items: Vec<ListItem> = rows
        .iter()
        .zip(&origins)
        .enumerate()
        .map(|(i, (r, origin))| {
            let num = if i < 9 {
                format!("{}  ", i + 1)
            } else {
                "   ".into()
            };
            let (color, label) = if r.unreachable {
                (pal().dim, "unreachable")
            } else {
                (state_color(r.state), r.state.label())
            };
            let icon = r.state.glyph();
            let origin_color = match (&r.finca, &r.account) {
                _ if r.unreachable => pal().dim,
                (Some(_), _) => pal().fincas,
                (None, Some(a)) => parse_color(a.color.as_deref()).unwrap_or(pal().muted),
                (None, None) => pal().muted,
            };
            let pad = |s: &str, w: usize| format!("{s}{}", " ".repeat(w - s.chars().count() + 2));
            let mut spans = vec![
                Span::raw(num).fg(pal().dim),
                Span::styled(format!("{icon} "), Style::new().fg(color)),
                Span::styled(pad(origin, origin_w), Style::new().fg(origin_color)),
                Span::raw(pad(&r.project, project_w)).fg(pal().muted),
                Span::raw(pad(&r.name, name_w)).fg(pal().bright).bold(),
                Span::styled(format!("{label:<12}"), Style::new().fg(color)),
                Span::raw(format!("{:<5}", format_age(r.updated_at, now))).fg(pal().dim),
            ];
            if let Some(m) = r.message.as_deref().filter(|_| !r.unreachable) {
                spans.push(Span::styled(m.to_string(), Style::new().fg(color)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let mut state = ListState::default().with_selected(focused.then_some(app.pin_idx));
    let list = List::new(items)
        .block(block)
        .highlight_style(highlight(focused));
    frame.render_stateful_widget(list, area, &mut state);
}

/// The `T` picker: each theme's name beside swatches of its pane colors.
fn render_themes(frame: &mut Frame, sel: usize) {
    let themes = super::theme::all();
    let area = popup(frame.area(), 40, themes.len() as u16 + 4);
    let area = Rect {
        width: area.width.max(44).min(frame.area().width),
        ..area
    };
    let area = Rect {
        x: frame.area().x + (frame.area().width - area.width) / 2,
        ..area
    };
    blank(frame, area);
    let mut lines: Vec<Line> = themes
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let here = i == sel;
            let mut spans = vec![
                Span::raw(if here { " ▸ " } else { "   " }).fg(pal().accent),
                Span::raw(format!("{:<22}", t.name)).fg(if here {
                    pal().bright
                } else {
                    pal().muted
                }),
            ];
            // On the theme's own background, so the flavors tell apart.
            spans.push(Span::raw(" ").bg(t.bg));
            for c in [t.accent, t.sessions, t.todo, t.fincas, t.shell, t.key] {
                spans.push(Span::raw("●").fg(c).bg(t.bg));
            }
            spans.push(Span::raw(" ").bg(t.bg));
            let line = Line::from(spans);
            if here {
                line.bg(pal().selected_bg)
            } else {
                line
            }
        })
        .collect();
    // In a short window the list scrolls, keeping the selection in view.
    let rows = usize::from(area.height.saturating_sub(4)).max(1);
    if lines.len() > rows {
        let start = sel.saturating_sub(rows / 2).min(lines.len() - rows);
        lines = lines.drain(start..start + rows).collect();
    }
    lines.push(Line::raw(""));
    lines.push(Line::raw(" j/k preview · Enter keep · Esc cancel").fg(pal().dim));
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(pal().accent))
                .title(Span::raw(" Theme ").fg(pal().accent).bold()),
        ),
        area,
    );
}
