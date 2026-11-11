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

/// The focused section's hints, and the color its title is drawn in.
pub fn for_focus(app: &App) -> (Color, Vec<Hint>) {
    let p = pal();
    match app.pane {
        Pane::Projects if app.selected_finca().is_some() => (
            p.fincas,
            vec![
                ("enter", "connect"),
                ("n", "new session there"),
                ("s", "shell there"),
                ("l", "its sessions"),
                ("a", "add finca"),
                ("K", "projects"),
            ],
        ),
        Pane::Projects => {
            let mut h = vec![
                ("enter", "start"),
                ("n", "named"),
                ("s/S", "shell"),
                ("t", "to-do"),
                ("e", "edit"),
                ("c", "clear ended"),
                ("a", "add"),
                ("d", "remove"),
            ];
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
        Pane::Pinned => (
            p.key,
            vec![
                ("enter", "jump"),
                ("1–9", "jump to pin"),
                ("p", "unpin"),
                ("x", "kill"),
                ("s", "shell"),
                ("J", "sessions"),
            ],
        ),
        Pane::Sessions if app.selected_finca().is_some() => (
            p.fincas,
            match app.selected_finca_item() {
                Some(FincaItem::Shell(_)) => {
                    vec![("enter", "open"), ("r", "rename"), ("h", "fincas")]
                }
                Some(FincaItem::Session { .. }) => {
                    vec![
                        ("enter", "jump"),
                        ("p", "pin"),
                        ("s", "shell"),
                        ("h", "fincas"),
                    ]
                }
                None => vec![
                    ("enter", "connect"),
                    ("n", "new session there"),
                    ("s", "shell"),
                    ("h", "fincas"),
                ],
            },
        ),
        Pane::Sessions if app.selected_shell().is_some() => (
            p.shell,
            vec![
                ("enter", "open"),
                ("r", "rename"),
                ("x", "close"),
                ("S", "new shell"),
            ],
        ),
        Pane::Sessions => {
            let mut h = match app.selected_session() {
                Some(s) if s.state == State::Ended => vec![
                    ("enter", "resume"),
                    ("r", "rename"),
                    ("p", "pin"),
                    ("c", "clear ended"),
                ],
                Some(_) => vec![
                    ("enter", "jump"),
                    ("p", "pin"),
                    ("r", "rename"),
                    ("x", "kill"),
                ],
                None => vec![("enter", "start"), ("n", "named")],
            };
            h.extend([("v", "grid"), ("/", "find")]);
            if app.todo_slug.is_some() {
                h.push(("J", "to-do"));
            }
            (p.sessions, h)
        }
        Pane::Todo => (
            p.todo,
            vec![
                ("o", "add"),
                ("x", "done"),
                ("i", "edit"),
                ("dd", "delete"),
                ("p", "paste"),
                ("u", "undo"),
                ("E", "vim"),
                ("K", "sessions"),
            ],
        ),
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
