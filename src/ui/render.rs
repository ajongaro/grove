//! Draws the dashboard.

use chrono::{DateTime, Timelike, Utc};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Wrap};

use super::app::{App, FincaItem, FincaRow, Mode, Pane, PromptKind, format_age};
use crate::model::State;

/// Focus: the focused pane's border and the selected row.
/// Keys in the help popup.
pub(super) const KEY_COLOR: Color = Color::Rgb(0xe0, 0xaf, 0x68);
/// Width of each session-state count on a project row: a two-column glyph,
/// two digits, a space.
const SLOT_W: usize = 5;
pub(super) const ACCENT: Color = Color::Rgb(102, 187, 106);
/// Each pane's title word keeps its color whatever has focus (green and
/// red stay free for focus and needs-you). Account boxes use the account's.
pub(super) const SESSIONS_COLOR: Color = Color::Rgb(0xbb, 0x9a, 0xf7);
pub(super) const TODO_COLOR: Color = Color::Rgb(0x7d, 0xcf, 0xff);
pub(super) const FINCAS_COLOR: Color = Color::Rgb(0xf7, 0x76, 0x8e);
/// A shell's row in Sessions: apart from the session-state colours.
const SHELL_COLOR: Color = Color::Rgb(0x7a, 0xa2, 0xf7);

pub fn render(frame: &mut Frame, app: &App, now: DateTime<Utc>) {
    // The header gives its rows back to the lists in a short window.
    let header_h = if frame.area().height >= HEADER_MIN_HEIGHT {
        HEADER_ROWS
    } else {
        0
    };
    let [header, main, footer] = Layout::vertical([
        Constraint::Length(header_h),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    if header_h > 0 {
        render_header(frame, app, header, now);
    }
    let grid = match &app.mode {
        Mode::Grid { sel, reply } => Some((*sel, reply.as_ref())),
        Mode::GridKill { sel, .. } => Some((*sel, None)),
        _ => None,
    };
    if let Some((sel, reply)) = grid {
        let area = Rect {
            height: main.height + footer.height,
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

    // Left: one equal share per account box, plus one for the fincas.
    let groups = app.groups().len();
    let shares = groups + usize::from(!app.fincas.is_empty());
    let mut boxes = Layout::vertical((0..shares).map(|_| Constraint::Fill(1)))
        .split(left)
        .to_vec();
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
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(format!("{input}█")).block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .title(title)
                        .fg(ACCENT),
                ),
                area,
            );
            render_completions(frame, app, area);
        }
        Mode::Help => render_help(frame),
        Mode::Confirm { label, .. } => kill_popup(frame, label),
        Mode::ConfirmShell { label, .. } => {
            confirm_popup(frame, &format!("Close shell {label}?  y / n"))
        }
        Mode::ConfirmRemove { name, .. } => {
            let area = popup(frame.area(), 60, 3);
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(format!("Remove project {name}? Its folder stays.  y / n")).block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .title(" Confirm ")
                        .fg(Color::Red),
                ),
                area,
            );
        }
        Mode::ConfirmCreate { path } => {
            let area = popup(frame.area(), 70, 3);
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(format!("Create {path} and git init it?  y / n")).block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .title(" New project folder ")
                        .fg(ACCENT),
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
    frame.render_widget(Clear, area);
    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .title(title)
                    .fg(Color::DarkGray),
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
        None => s.fg(Color::Gray),
    }));
    title.push(Span::raw(" "));
    let b = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(title));
    if focused {
        b.border_style(Style::new().fg(ACCENT))
    } else {
        b.border_style(Style::new().fg(Color::DarkGray))
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
    let word = match parse_hex(color) {
        Some(c) => Span::raw(title.to_string()).fg(c),
        None => Span::raw(title.to_string()),
    };
    let block = pane_block(vec![word], has_selection);
    if rows.is_empty() {
        let p = Paragraph::new(empty.to_string())
            .wrap(Wrap { trim: true })
            .dark_gray()
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
                    Span::raw("✗ ").red(),
                    Span::raw(p.name.clone()).red(),
                    Span::raw(" (bad file)").dark_gray(),
                ]));
            }
            let dot = Span::styled(
                "● ",
                Style::new().fg(parse_hex(p.color.as_deref()).unwrap_or(ACCENT)),
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
                spans.push(Span::raw(t).dark_gray());
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
        vec![Span::raw("FINCAS").fg(FINCAS_COLOR)],
        app.pane == Pane::Projects && app.selected_finca().is_some(),
    );
    let items: Vec<ListItem> = app
        .fincas
        .iter()
        .map(|f| {
            let color = parse_hex(f.color.as_deref()).unwrap_or(ACCENT);
            let name = Span::raw(f.name.clone());
            let line = match (&f.error, &f.reach) {
                (Some(_), _) => vec![
                    Span::raw("✗ ").red(),
                    name.red(),
                    Span::raw(" (bad file)").dark_gray(),
                ],
                (None, None) => vec![
                    Span::raw("◌ ").dark_gray(),
                    name,
                    Span::raw("  checking…").dark_gray(),
                ],
                (None, Some(Reach::Reachable(st))) => {
                    let mut v = vec![Span::styled("● ", Style::new().fg(color)), name];
                    v.push(Span::raw(format!("  {} live", st.live)).dark_gray());
                    if st.needs_you > 0 {
                        v.push(
                            Span::raw(format!("  {}{}", State::NeedsYou.glyph(), st.needs_you))
                                .red()
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
                        Span::raw("○ ").dark_gray(),
                        name.dark_gray(),
                        Span::raw(format!("  unreachable · {since}")).dark_gray(),
                    ]
                }
                (None, Some(Reach::NeedsUpdate)) => {
                    vec![
                        Span::raw("◐ ").yellow(),
                        name,
                        Span::raw("  needs update").yellow(),
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
        Span::raw("FINCA").fg(FINCAS_COLOR),
        Span::raw(format!(" — {}{host}", f.name)),
    ];
    let block = pane_block(title, app.pane == Pane::Sessions);
    let mut lines: Vec<Line> = Vec::new();
    match &f.reach {
        None => lines.push(Line::raw("Checking…").dark_gray()),
        Some(Reach::NeedsUpdate) => lines.push(
            Line::raw(format!("{}'s grove is too old to report status — run scripts/deploy-fincas.sh. Enter still connects.", f.name))
                .yellow(),
        ),
        Some(Reach::Unreachable { error }) => {
            let since = f.last_ok.map_or("never reachable".to_string(), |t| format!("last reachable {} ago", format_age(t, now)));
            lines.push(Line::raw(format!("Unreachable: {error}")).red());
            lines.push(Line::raw(format!("({since}; showing last known sessions)")).dark_gray());
            lines.push(Line::raw(""));
        }
        Some(Reach::Reachable(_)) => {}
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
        lines.push(Line::raw(format!("No live sessions on {}.", f.name)).dark_gray());
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
                        Span::raw(format!(" · {}", format_age(s.updated_at, now))).dark_gray(),
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
                    Span::raw("  ●  ").fg(FINCAS_COLOR),
                    Span::raw(format!("{:<w$}", w.label(), w = name_w)).bold(),
                ];
                if let Some(t) = &w.title {
                    spans.push(Span::raw(t.clone()).dark_gray());
                }
                lines.push(mark(Line::from(spans), i));
            }
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::raw("Enter: open · s: shell · C-q: come back").dark_gray());
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        area,
    );
}

fn render_sessions(frame: &mut Frame, app: &App, area: Rect, now: DateTime<Utc>) {
    let mut title = vec![Span::raw("SESSIONS").fg(SESSIONS_COLOR)];
    if let Some(p) = app.selected_project() {
        let a = app.account_of(p);
        let account = Span::raw(a.name.to_uppercase());
        title.push(Span::raw(" — "));
        title.push(match parse_hex(a.color.as_deref()) {
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
            Span::raw("$  ").fg(SHELL_COLOR),
            Span::raw(format!("{:<w$}", s.name, w = name_w)).bold(),
            Span::raw(format!("{:<10}", "shell")).fg(SHELL_COLOR),
            Span::raw(format!(" · {}", s.command)).dark_gray(),
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
                Span::raw(format!(" · {:<4}", format_age(s.updated_at, now))).dark_gray(),
            ];
            if let Some(st) = app.stats.get(&s.id) {
                spans.push(Span::raw(format!(" · {}", stats_label(st))).dark_gray());
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
    let block = pane_block(vec![Span::raw("TO-DO").fg(TODO_COLOR)], focused);
    let t = &app.todos;
    if t.items.is_empty() {
        let p = Paragraph::new("No to-dos — press o to add one (t or Tab to get here).")
            .wrap(Wrap { trim: true })
            .dark_gray()
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
                    Span::raw("[x] ").dark_gray(),
                    Span::raw(text).dark_gray().crossed_out(),
                ])
            } else {
                Line::from(vec![Span::raw("[ ] ").fg(ACCENT), Span::raw(text)])
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
fn render_help(frame: &mut Frame) {
    let sections: Vec<Vec<Line>> = super::help::SECTIONS
        .iter()
        .map(|(section, keys)| {
            // Each heading in its pane's color.
            let color = match *section {
                "Projects" => ACCENT,
                "Sessions" => SESSIONS_COLOR,
                "To-do" => TODO_COLOR,
                "Fincas" => FINCAS_COLOR,
                _ => Color::White,
            };
            let mut lines = vec![Line::raw(*section).bold().fg(color)];
            for (k, what) in *keys {
                let mut spans = vec![
                    Span::raw("  "),
                    Span::raw(format!("{k:<10} ")).bold().fg(KEY_COLOR),
                ];
                // A trailing aside in parentheses is dimmed.
                match what.find(" (").filter(|_| what.ends_with(')')) {
                    Some(i) => {
                        spans.push(Span::raw(&what[..i]));
                        spans.push(Span::raw(&what[i..]).dark_gray());
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
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::DarkGray))
        .title(Line::from(vec![
            Span::raw(" Keys ").fg(ACCENT).bold(),
            Span::raw("(any key closes) ").dark_gray(),
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
            .white()
            .on_red()
            .bold(),
        );
        spans.push(Span::raw(" "));
    }
    match &app.footer {
        Some(msg) => spans.push(Span::raw(msg.clone()).yellow()),
        None if app.pane == Pane::Todo && app.todos.inserting() => {
            spans.push(Span::raw(" -- INSERT -- ").black().on_yellow().bold());
            spans.push(Span::raw(" enter/esc save · ctrl-u clear").dark_gray());
        }
        None if app.pane == Pane::Todo => spans.push(
            Span::raw("o/O add · i/A edit · cc change · x done · dd delete · p paste · u undo · E vim · h back · ? help").dark_gray(),
        ),
        None => {
            const KEYS: &str = "enter open/jump · n new · s/S shell · r rename · a add · t to-do · v grid · x kill · d remove · tab pane · ? help · q detach";
            // Now and then, when nothing else needs saying, the grove speaks,
            // taking the key list's room if it must ("? help" stays).
            let used: usize = spans.iter().map(Span::width).sum();
            let quip = flair::quip(now).filter(|_| n == 0).and_then(|q| {
                let q = Span::raw(q).dark_gray().italic();
                [KEYS, "? help"].into_iter().find_map(|keys| {
                    let pad = (area.width as usize)
                        .checked_sub(used + Span::raw(keys).width() + q.width() + 1)?;
                    (pad >= 2).then(|| (keys, pad, q.clone()))
                })
            });
            match quip {
                Some((keys, pad, q)) => {
                    spans.push(Span::raw(keys).dark_gray());
                    spans.push(Span::raw(" ".repeat(pad)));
                    spans.push(q);
                }
                None => spans.push(Span::raw(KEYS).dark_gray()),
            }
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn highlight(focused: bool) -> Style {
    if focused {
        Style::new()
            .bg(Color::Rgb(46, 74, 52))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().bg(Color::Rgb(40, 40, 40))
    }
}

/// Rows the header takes (the logo's two plus a blank one under it), and
/// the window height below which it's hidden.
const HEADER_ROWS: u16 = 3;
const HEADER_MIN_HEIGHT: u16 = 24;

use super::flair;

/// An olive branch growing to the right: (text, color) runs per row. The
/// stem is a run of upper half blocks, so it sits where the rows meet.
const WORDMARK: [&str; 2] = ["█▀▀ █▀█ █▀█ █ █ █▀▀", "█▄█ █▀▄ █▄█ ▀▄▀ ██▄"];

/// Logo and a window of sky with its olive trees on the left, leaves blowing
/// through while anything works; this machine's session totals, the fincas that are up and the day's
/// harvest on the right.
fn render_header(frame: &mut Frame, app: &App, area: Rect, now: DateTime<Utc>) {
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
            Style::new().dark_gray()
        };
        Span::styled(format!("{}{n} {label}", state.glyph()), style)
    };
    let totals = Line::from(vec![
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
    ]);
    let mut place = app.host.clone();
    if !app.fincas.is_empty() {
        let up = app
            .fincas
            .iter()
            .filter(|f| matches!(f.reach, Some(crate::finca::Reach::Reachable(_))))
            .count();
        place = format!("{place} · fincas {up}/{} up", app.fincas.len());
    }
    let place = Line::from(vec![Span::raw(place).dark_gray(), Span::raw(" ")]);

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
    let working = app.sessions.iter().any(|s| s.state == State::Working);
    let sky =
        (sky_w >= flair::SKY_MIN).then(|| flair::sky(sky_w, ground, local.hour(), now, working));
    let logo: Vec<Line> = (0..3)
        .map(|i| {
            let mut spans = vec![Span::raw(" ")];
            spans.push(match WORDMARK.get(i) {
                Some(w) => Span::styled(*w, Style::new().fg(ACCENT).bold()),
                None => Span::raw(" ".repeat(wordmark_w)),
            });
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
        .dark_gray(),
    );
    Line::from(spans)
}

pub(super) fn state_color(s: State) -> Color {
    match s {
        State::NeedsYou => Color::Red,
        State::YourTurn => Color::Yellow,
        State::Working => Color::Green,
        State::Starting => Color::Cyan,
        State::Ended => Color::DarkGray,
    }
}

pub(super) fn parse_hex(c: Option<&str>) -> Option<Color> {
    let h = c?.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some(Color::Rgb(v(0)?, v(2)?, v(4)?))
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
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(text.to_string()).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" Confirm ")
                .fg(Color::Red),
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
    let block = pane_block(vec![Span::raw("PINNED").fg(KEY_COLOR)], focused);
    let origins: Vec<String> = rows
        .iter()
        .map(|r| match (&r.finca, &r.account) {
            (Some((_, name, _)), _) => name.clone(),
            (None, Some(a)) => a.name.clone(),
            (None, None) => String::new(),
        })
        .collect();
    let handle_w = rows
        .iter()
        .zip(&origins)
        .map(|(r, o)| o.chars().count() + 3 + r.handle.chars().count())
        .max()
        .unwrap_or(0)
        + 2;
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
                (Color::DarkGray, "unreachable")
            } else {
                (state_color(r.state), r.state.label())
            };
            let icon = r.state.glyph();
            let origin_color = match (&r.finca, &r.account) {
                _ if r.unreachable => Color::DarkGray,
                (Some(_), _) => FINCAS_COLOR,
                (None, Some(a)) => parse_hex(a.color.as_deref()).unwrap_or(Color::Gray),
                (None, None) => Color::Gray,
            };
            let pad = handle_w - (origin.chars().count() + 3 + r.handle.chars().count());
            let mut spans = vec![
                Span::raw(num).dark_gray(),
                Span::styled(format!("{icon} "), Style::new().fg(color)),
                Span::styled(origin.clone(), Style::new().fg(origin_color)),
                Span::raw(" · ").dark_gray(),
                Span::raw(format!("{}{}", r.handle, " ".repeat(pad))).bold(),
                Span::styled(format!("{label:<12}"), Style::new().fg(color)),
                Span::raw(format!("{:<5}", format_age(r.updated_at, now))).dark_gray(),
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
