//! Draws the dashboard.

use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Wrap};

use super::app::{App, Mode, Pane, PromptKind, format_age};
use crate::model::State;

const ACCENT: Color = Color::Rgb(102, 187, 106);

pub fn render(frame: &mut Frame, app: &App, now: DateTime<Utc>) {
    let [main, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());
    let [left, right] = Layout::horizontal([Constraint::Percentage(32), Constraint::Percentage(68)]).areas(main);

    render_projects(frame, app, left);
    render_sessions(frame, app, right, now);
    render_footer(frame, app, footer);

    match &app.mode {
        Mode::Prompt { kind, input } => {
            let title = match kind {
                PromptKind::NewSession => " Session name (Enter for default, Esc to cancel) ",
                PromptKind::AddProject => " Add project — path (Enter to add, Esc to cancel) ",
            };
            let area = popup(frame.area(), 70, 3);
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(format!("{input}█")).block(Block::bordered().border_type(BorderType::Rounded).title(title).fg(ACCENT)),
                area,
            );
        }
        Mode::Confirm { label, .. } => {
            let area = popup(frame.area(), 44, 3);
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(format!("Kill session {label}?  y / n"))
                    .block(Block::bordered().border_type(BorderType::Rounded).title(" Confirm ").fg(Color::Red)),
                area,
            );
        }
        Mode::Normal => {}
    }
}

fn pane_block(title: &str, focused: bool) -> Block<'_> {
    let b = Block::bordered().border_type(BorderType::Rounded).title(format!(" {title} "));
    if focused { b.border_style(Style::new().fg(ACCENT)) } else { b.border_style(Style::new().fg(Color::DarkGray)) }
}

fn render_projects(frame: &mut Frame, app: &App, area: Rect) {
    let block = pane_block("Projects", app.pane == Pane::Projects);
    if app.projects.is_empty() {
        let p = Paragraph::new("No projects yet — press a to add one.").wrap(Wrap { trim: true }).block(block);
        frame.render_widget(p, area);
        return;
    }
    let needs = app.needs_you_by_project();
    let items: Vec<ListItem> = app
        .projects
        .iter()
        .map(|p| {
            if p.error.is_some() {
                return ListItem::new(Line::from(vec![Span::raw("✗ ").red(), Span::raw(p.name.clone()).red(), Span::raw(" (bad file)").dark_gray()]));
            }
            let dot = Span::styled("● ", Style::new().fg(parse_hex(p.color.as_deref()).unwrap_or(ACCENT)));
            let mut spans = vec![dot, Span::raw(p.name.clone())];
            let live = app.sessions.iter().filter(|s| s.project == p.slug && s.state != State::Ended).count();
            if live > 0 {
                spans.push(Span::raw(format!("  {live}")).dark_gray());
            }
            if let Some(n) = needs.get(&p.slug) {
                spans.push(Span::raw(format!("  ⚑{n}")).red().bold());
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let mut state = ListState::default().with_selected(Some(app.proj_idx));
    let list = List::new(items).block(block).highlight_style(highlight(app.pane == Pane::Projects));
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_sessions(frame: &mut Frame, app: &App, area: Rect, now: DateTime<Utc>) {
    let title = match app.selected_project() {
        Some(p) => format!("Sessions — {}", p.name),
        None => "Sessions".to_string(),
    };
    let block = pane_block(&title, app.pane == Pane::Sessions);
    let sessions = app.visible_sessions();
    if sessions.is_empty() {
        let hint = if app.selected_project().is_some() { "No sessions — press Enter to start one, n to name it." } else { "" };
        frame.render_widget(Paragraph::new(hint).wrap(Wrap { trim: true }).block(block), area);
        return;
    }
    let items: Vec<ListItem> = sessions
        .iter()
        .map(|s| {
            let color = state_color(s.state);
            let mut spans = vec![
                Span::styled("● ", Style::new().fg(color)),
                Span::raw(format!("{:<14}", s.name)).bold(),
                Span::styled(format!("{:<10}", s.state.label()), Style::new().fg(color)),
                Span::raw(format!(" · {:<4}", format_age(s.updated_at, now))).dark_gray(),
            ];
            let mut lines = vec![Line::from(spans)];
            // On its own line so long permission prompts aren't cut off.
            if let Some(m) = &s.message {
                lines.push(Line::from(Span::styled(format!("    {m}"), Style::new().fg(color))));
            }
            ListItem::new(lines)
        })
        .collect();
    let mut state = ListState::default().with_selected(Some(app.sess_idx));
    let list = List::new(items).block(block).highlight_style(highlight(app.pane == Pane::Sessions));
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let mut spans = Vec::new();
    let n = app.needs_you_count();
    if n > 0 {
        spans.push(Span::raw(format!(" ⚑ {n} needs you ")).white().on_red().bold());
        spans.push(Span::raw(" "));
    }
    match &app.footer {
        Some(msg) => spans.push(Span::raw(msg.clone()).yellow()),
        None => spans.push(
            Span::raw("enter open/jump · n new · a add · e edit · w wiki · x kill · c clear · tab pane · q detach").dark_gray(),
        ),
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn highlight(focused: bool) -> Style {
    if focused {
        Style::new().bg(Color::Rgb(46, 74, 52)).add_modifier(Modifier::BOLD)
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
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}
