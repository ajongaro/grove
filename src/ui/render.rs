//! Draws the dashboard.

use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Wrap};

use super::app::{App, FincaRow, Mode, Pane, PromptKind, format_age};
use crate::model::State;

/// Focus: the focused pane's border and the selected row.
const ACCENT: Color = Color::Rgb(102, 187, 106);
/// Each pane's title word keeps its color whatever has focus (green and
/// red stay free for focus and needs-you). Account boxes use the account's.
const SESSIONS_COLOR: Color = Color::Rgb(0xbb, 0x9a, 0xf7);
const TODO_COLOR: Color = Color::Rgb(0x7d, 0xcf, 0xff);
const FINCAS_COLOR: Color = Color::Rgb(0xf7, 0x76, 0x8e);

pub fn render(frame: &mut Frame, app: &App, now: DateTime<Utc>) {
    let [main, footer] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(32), Constraint::Percentage(68)]).areas(main);

    let projects = if app.fincas.is_empty() {
        left
    } else {
        let rows = (app.fincas.len() as u16 + 2).min(left.height / 2).max(3);
        let [projects, fincas] =
            Layout::vertical([Constraint::Min(3), Constraint::Length(rows)]).areas(left);
        render_fincas(frame, app, fincas, now);
        projects
    };
    render_project_boxes(frame, app, projects);
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
    render_footer(frame, app, footer);

    match &app.mode {
        Mode::Prompt { kind, input } => {
            let title = match kind {
                PromptKind::NewSession => " Session name (Enter for default, Esc to cancel) ",
                PromptKind::AddProject => {
                    " Add project — path (Tab completes, Enter adds, Esc cancels) "
                }
                PromptKind::RenameSession { .. } => " Rename session (Enter saves, Esc cancels) ",
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
        Mode::Confirm { label, .. } => {
            let area = popup(frame.area(), 44, 3);
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(format!("Kill session {label}?  y / n")).block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .title(" Confirm ")
                        .fg(Color::Red),
                ),
                area,
            );
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
        Mode::Normal => {}
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

/// One box per account group (play above work), stacked in equal shares;
/// a single PROJECTS box when no account owns a folder.
fn render_project_boxes(frame: &mut Frame, app: &App, area: Rect) {
    let groups = app.groups();
    let areas = Layout::vertical(groups.iter().map(|_| Constraint::Fill(1))).split(area);
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
    let needs = app.needs_you_by_project();
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
            let mut spans = vec![dot, Span::raw(p.name.clone())];
            let live = app
                .sessions
                .iter()
                .filter(|s| s.project == p.slug && s.state != State::Ended)
                .count();
            if live > 0 {
                spans.push(Span::raw(format!("  {live}")).dark_gray());
            }
            if let Some(n) = needs.get(&p.slug) {
                spans.push(Span::raw(format!("  ⚑{n}")).red().bold());
            }
            if let Some(n) = app.open_todos.get(&p.slug).filter(|n| **n > 0) {
                spans.push(Span::raw(format!("  ☐{n}")).dark_gray());
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
                        v.push(Span::raw(format!("  ⚑{}", st.needs_you)).red().bold());
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
    if let Some(st) = &f.last_status {
        let mut any = false;
        let name_w = name_width(
            st.projects
                .iter()
                .flat_map(|p| &p.sessions)
                .map(|s| s.name.as_str()),
        );
        for p in st.projects.iter().filter(|p| !p.sessions.is_empty()) {
            any = true;
            lines.push(Line::raw(p.name.clone()).bold());
            for s in &p.sessions {
                let color = state_color(s.state);
                lines.push(Line::from(vec![
                    Span::styled("  ● ", Style::new().fg(color)),
                    Span::raw(format!("{:<w$}", s.name, w = name_w)).bold(),
                    Span::styled(format!("{:<10}", s.state.label()), Style::new().fg(color)),
                    Span::raw(format!(" · {}", format_age(s.updated_at, now))).dark_gray(),
                ]));
                if let Some(m) = &s.message {
                    lines.push(Line::styled(format!("      {m}"), Style::new().fg(color)));
                }
            }
        }
        if !any {
            lines.push(Line::raw(format!("No live sessions on {}.", f.name)).dark_gray());
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::raw("Enter: connect · s: shell · C-q: come back").dark_gray());
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
    if sessions.is_empty() {
        let hint = if app.selected_project().is_some() {
            "No sessions — press Enter to start one, n to name it."
        } else {
            ""
        };
        frame.render_widget(
            Paragraph::new(hint).wrap(Wrap { trim: true }).block(block),
            area,
        );
        return;
    }
    let name_w = name_width(sessions.iter().map(|s| s.name.as_str()));
    let items: Vec<ListItem> = sessions
        .iter()
        .map(|s| {
            let color = state_color(s.state);
            let spans = vec![
                Span::styled("● ", Style::new().fg(color)),
                Span::raw(format!("{:<w$}", s.name, w = name_w)).bold(),
                Span::styled(format!("{:<10}", s.state.label()), Style::new().fg(color)),
                Span::raw(format!(" · {:<4}", format_age(s.updated_at, now))).dark_gray(),
            ];
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
        .collect();
    let mut state = ListState::default().with_selected(Some(app.sess_idx));
    let list = List::new(items)
        .block(block)
        .highlight_style(highlight(app.pane == Pane::Sessions));
    frame.render_stateful_widget(list, area, &mut state);
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
            let mut lines = vec![Line::raw(*section).bold().fg(ACCENT)];
            for (k, what) in *keys {
                lines.push(Line::from(vec![
                    Span::raw(format!("  {k:<10} ")).bold(),
                    Span::raw(*what),
                ]));
            }
            lines.push(Line::raw(""));
            lines
        })
        .collect();
    // Fill the left column until it holds about half the lines.
    let total: usize = sections.iter().map(Vec::len).sum();
    let (mut left, mut right) = (Vec::new(), Vec::new());
    for s in sections {
        if left.len() < total / 2 {
            left.extend(s);
        } else {
            right.extend(s);
        }
    }
    let height = left.len().max(right.len()) as u16 + 1;
    let area = popup(frame.area(), 90, height);
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(" Keys (any key closes) ")
        .fg(ACCENT);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [l, r] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(inner);
    frame.render_widget(Paragraph::new(left), l);
    frame.render_widget(Paragraph::new(right), r);
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
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
            Span::raw(format!(" ⚑ {n} needs you {detail}"))
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
        None => spans.push(
            Span::raw("enter open/jump · n new · r rename · a add · t to-do · x kill · d remove · tab pane · ? help · q detach").dark_gray(),
        ),
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

fn state_color(s: State) -> Color {
    match s {
        State::NeedsYou => Color::Red,
        State::YourTurn => Color::Yellow,
        State::Working => Color::Green,
        State::Starting => Color::Cyan,
        State::Ended => Color::DarkGray,
    }
}

fn parse_hex(c: Option<&str>) -> Option<Color> {
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
