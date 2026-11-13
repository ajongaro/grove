//! The footer's key hints: the most used keys for whatever has focus, most
//! common first, so the line stays useful however much of it fits.
//! Keep in step with input.rs (the `?` help in help.rs has the full list).

use ratatui::style::{Color, Stylize};
use ratatui::text::Span;

use super::app::{App, FincaItem, Pane};
use super::theme::pal;
use crate::model::State;

pub type Hint = (&'static str, &'static str);

/// Always last, and kept when nothing else fits.
pub const HELP: Hint = ("?", "help");

/// Keys that work from every list, after each section's own.
const ANYWHERE: [Hint; 2] = [("v", "grid"), ("/", "find")];

/// The focused section's hints, and the color its title is drawn in. Most
/// needed first: a narrow footer drops them from the end.
pub fn for_focus(app: &App) -> (Color, Vec<Hint>) {
    let p = pal();
    let waiting = app.sessions.iter().any(|s| s.state == State::YourTurn);
    match app.pane {
        Pane::Projects if app.selected_finca().is_some() => {
            let mut h = vec![
                ("enter", "connect"),
                ("n", "new session there"),
                ("s", "shell there"),
                ("l", "its sessions"),
            ];
            h.extend(ANYWHERE);
            h.extend([("a", "add finca"), ("K", "projects")]);
            (p.fincas, h)
        }
        Pane::Projects => {
            let mut h = vec![
                ("enter", "start"),
                ("n", "named"),
                ("s/S", "shell"),
                ("t", "to-do"),
            ];
            h.extend(ANYWHERE);
            if waiting {
                h.push(("U", "restart waiting"));
            }
            h.extend([
                ("e", "edit"),
                ("a", "add"),
                ("C", "clear ended"),
                ("d", "remove"),
            ]);
            if !app.fincas.is_empty() || app.groups().len() > 1 {
                h.push(("J/K", "section"));
            }
            if app.groups().len() > 1 {
                // With a box hidden, the way back comes right after Enter.
                match app.hidden.is_empty() {
                    true => h.push(("H", "hide box")),
                    false => h.insert(1, ("H", "show all")),
                }
            }
            h.extend([("T", "theme"), ("B", "top bar")]);
            (p.accent, h)
        }
        Pane::Pinned => {
            let mut h = vec![
                ("enter", "jump"),
                ("1–9", "jump to pin"),
                ("V", "desk"),
                ("x", "kill"),
                ("p", "unpin"),
                ("o", "order"),
                ("s", "shell"),
            ];
            h.extend(ANYWHERE);
            h.push(("J", "sessions"));
            (p.key, h)
        }
        Pane::Sessions if app.selected_finca().is_some() => {
            let mut h = match app.selected_finca_item() {
                Some(FincaItem::Shell(_)) => vec![
                    ("enter", "open"),
                    ("r", "rename"),
                    ("s", "shell"),
                    ("n", "new session"),
                ],
                Some(FincaItem::Session { .. }) => vec![
                    ("enter", "jump"),
                    ("n", "new"),
                    ("u", "restart"),
                    ("p", "pin"),
                    ("s", "shell"),
                ],
                None => vec![
                    ("enter", "connect"),
                    ("n", "new session there"),
                    ("s", "shell"),
                ],
            };
            h.extend(ANYWHERE);
            h.push(("h", "fincas"));
            (p.fincas, h)
        }
        Pane::Sessions if app.selected_shell().is_some() => {
            let mut h = vec![
                ("enter", "open"),
                ("r", "rename"),
                ("x", "close"),
                ("S", "new shell"),
                ("n", "new session"),
            ];
            h.extend(ANYWHERE);
            if app.todo_slug.is_some() {
                h.push(("J", "to-do"));
            }
            (p.shell, h)
        }
        Pane::Sessions => {
            let mut h = match app.selected_session() {
                Some(s) if s.state == State::Ended => vec![
                    ("enter", "resume"),
                    ("n", "new"),
                    ("c", "clear it"),
                    ("C", "clear all ended"),
                    ("r", "rename"),
                    ("p", "pin"),
                ],
                Some(_) => vec![
                    ("enter", "jump"),
                    ("n", "new"),
                    ("r", "rename"),
                    ("p", "pin"),
                    ("u", "restart"),
                    ("x", "kill"),
                ],
                None => vec![("enter", "start"), ("n", "named"), ("s/S", "shell")],
            };
            h.extend(ANYWHERE);
            if waiting {
                h.push(("U", "restart waiting"));
            }
            if app.todo_slug.is_some() {
                h.push(("J", "to-do"));
            }
            (p.sessions, h)
        }
        Pane::Todo => {
            let on_done = app.todos.items.get(app.todos.idx).is_some_and(|i| i.done);
            let any_done = app.todos.items.iter().any(|i| i.done);
            let mut h = vec![("n", "new"), ("enter/x", "done")];
            if on_done {
                h.push(("c", "clear"));
            }
            h.extend([("r", "edit"), ("d", "delete")]);
            if any_done {
                h.push(("C", "clear done"));
            }
            h.extend([
                ("u", "undo"),
                ("p", "paste"),
                ("e", "editor"),
                ("K", "sessions"),
            ]);
            (p.todo, h)
        }
    }
}

/// `hints` (then `HELP`) as spans, keys in `color`, dropping hints from
/// the end until the line fits in `width` columns.
pub fn spans(hints: &[Hint], color: Color, width: usize) -> Vec<Span<'static>> {
    let shown = fit(hints, width);
    let mut out = Vec::new();
    for (i, (k, what)) in hints[..shown].iter().chain([&HELP]).enumerate() {
        if i > 0 {
            out.push(Span::raw(" · ").fg(pal().dim));
        }
        out.push(Span::raw(*k).fg(color).bold());
        out.push(Span::raw(format!(" {what}")).fg(pal().dim));
    }
    out
}

/// How many of `hints` fit in `width` beside `HELP`.
pub fn fit(hints: &[Hint], width: usize) -> usize {
    let w = |(k, what): &Hint| k.chars().count() + 1 + what.chars().count();
    let mut used = w(&HELP);
    let mut n = 0;
    for h in hints {
        let next = used + 3 + w(h);
        if next > width {
            break;
        }
        used = next;
        n += 1;
    }
    n
}
