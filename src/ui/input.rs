//! Key handling: mutates `App` and returns the side effect to perform.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::app::{Action, App, FincaItem, Mode, Pane, PromptKind, Reply, state_is_live};
use super::complete::{complete_path, complete_ssh_host};
use crate::model::State;
use crate::todo::Effect;

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    if key.kind == KeyEventKind::Release {
        return Action::None;
    }
    match app.mode.clone() {
        Mode::Prompt { kind, input } => prompt_key(app, kind, input, key),
        Mode::ConfirmCreate { path } => match key.code {
            KeyCode::Char('y' | 'Y') => {
                app.mode = Mode::Normal;
                Action::CreateProject { path }
            }
            // Anything else goes back to the prompt, text intact.
            _ => {
                app.mode = Mode::Prompt {
                    kind: PromptKind::AddProject,
                    input: path,
                };
                Action::None
            }
        },
        Mode::ConfirmRemove { slug, .. } => {
            app.mode = Mode::Normal;
            match key.code {
                KeyCode::Char('y' | 'Y') => Action::RemoveProject { slug },
                _ => Action::None,
            }
        }
        Mode::Confirm {
            session_id,
            label,
            finca,
        } => {
            app.mode = Mode::Normal;
            match (key.code, finca) {
                (KeyCode::Char('y' | 'Y'), None) => Action::Kill { session_id },
                (KeyCode::Char('y' | 'Y'), Some(slug)) => {
                    let via = app
                        .fincas
                        .iter()
                        .find(|f| f.slug == slug)
                        .and_then(|f| f.via.clone());
                    Action::GridKill {
                        finca: Some((slug, via)),
                        session_id,
                        handle: label,
                    }
                }
                _ => Action::None,
            }
        }
        Mode::ConfirmShell { window, label } => {
            app.mode = Mode::Normal;
            match key.code {
                KeyCode::Char('y' | 'Y') => Action::CloseShell { window, label },
                _ => Action::None,
            }
        }
        Mode::Help => {
            app.mode = Mode::Normal;
            Action::None
        }
        Mode::Search { query, sel } => search_key(app, query, sel, key),
        Mode::Normal => normal_key(app, key),
        Mode::Grid { sel, reply } => grid_key(app, sel, reply, key),
        Mode::GridKill {
            sel,
            session_id,
            finca,
            handle,
        } => {
            app.mode = Mode::Grid { sel, reply: None };
            if !matches!(key.code, KeyCode::Char('y' | 'Y')) {
                return Action::None;
            }
            // Its session, wherever its tile is now.
            let tiles = super::grid::tiles(app, chrono::Utc::now());
            let target = tiles.iter().position(|t| {
                t.session_id.as_deref() == Some(session_id.as_str())
                    && t.finca.as_ref().map(|f| &f.0) == finca.as_ref()
            });
            match target.map(|k| (k, &tiles[k])) {
                None => {
                    app.footer = Some(format!("{handle} has ended"));
                    Action::None
                }
                Some((_, t)) if t.stale => {
                    let name = t.finca.as_ref().map_or("it", |f| f.1.as_str());
                    app.footer = Some(format!("{name} isn't answering"));
                    Action::None
                }
                Some((k, t)) => {
                    app.mode = Mode::Grid {
                        sel: k,
                        reply: None,
                    };
                    Action::GridKill {
                        finca: t
                            .finca
                            .as_ref()
                            .map(|(slug, _)| (slug.clone(), t.via.clone())),
                        session_id,
                        handle,
                    }
                }
            }
        }
    }
}

/// Typing in the `/` search: letters narrow it, arrows (or C-j / C-k)
/// move, Enter goes to the selected session, Esc closes.
fn search_key(app: &mut App, mut query: String, mut sel: usize, key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let n = super::search::search(app, &query).len();
    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
            return Action::None;
        }
        KeyCode::Enter => {
            app.mode = Mode::Normal;
            let hits = super::search::search(app, &query);
            return match hits.get(sel).map(super::search::enter) {
                Some(Ok(action)) => action,
                Some(Err(why)) => {
                    app.footer = Some(why);
                    Action::None
                }
                None => Action::None,
            };
        }
        KeyCode::Up => sel = sel.saturating_sub(1),
        KeyCode::Char('k' | 'p') if ctrl => sel = sel.saturating_sub(1),
        KeyCode::Down => sel = (sel + 1).min(n.saturating_sub(1)),
        KeyCode::Char('j' | 'n') if ctrl => sel = (sel + 1).min(n.saturating_sub(1)),
        KeyCode::Char('u') if ctrl => {
            query.clear();
            sel = 0;
        }
        KeyCode::Backspace => {
            query.pop();
            sel = 0;
        }
        KeyCode::Char(c) if !ctrl => {
            query.push(c);
            sel = 0;
        }
        _ => {}
    }
    app.mode = Mode::Search { query, sel };
    Action::None
}

fn prompt_key(app: &mut App, kind: PromptKind, mut input: String, key: KeyEvent) -> Action {
    if key.code == KeyCode::Tab && matches!(kind, PromptKind::AddProject | PromptKind::AddFinca) {
        tab_complete(app, kind, input);
        return Action::None;
    }
    // j / k (or C-j / C-k) step through an open match list (down / up).
    // With no list open, plain j and k are just letters.
    if let KeyCode::Char(c @ ('j' | 'k')) = key.code {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if !app.completions.is_empty() && (ctrl || key.modifiers.is_empty()) {
            cycle_completion(app, kind, if c == 'j' { 1 } else { -1 });
            return Action::None;
        }
        if ctrl {
            return Action::None;
        }
    }
    // Any other key ends a Tab cycle.
    app.completions.clear();
    app.completion_idx = None;
    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
            Action::None
        }
        KeyCode::Enter => {
            app.mode = Mode::Normal;
            let text = input.trim().to_string();
            match kind {
                PromptKind::NewSession => match app.selected_project() {
                    Some(p) => Action::NewSession {
                        slug: p.slug.clone(),
                        name: (!text.is_empty()).then_some(text),
                    },
                    None => Action::None,
                },
                PromptKind::AddProject if text.is_empty() => Action::None,
                PromptKind::AddProject if crate::config::expand_tilde(&text).exists() => {
                    Action::AddProject { path: text }
                }
                PromptKind::AddProject => {
                    app.mode = Mode::ConfirmCreate { path: text };
                    Action::None
                }
                PromptKind::AddFinca if text.is_empty() => Action::None,
                PromptKind::AddFinca => Action::SetupFinca { dest: text },
                PromptKind::RenameShell { window, finca } => {
                    if text.is_empty() || text == app.shell_label(&window, finca.as_deref()) {
                        Action::None
                    } else {
                        Action::RenameShell {
                            window,
                            finca,
                            name: text,
                        }
                    }
                }
                PromptKind::RenameSession { session_id } => {
                    let current = app
                        .sessions
                        .iter()
                        .find(|s| s.id == session_id)
                        .map(|s| s.name.as_str());
                    if text.is_empty() || current == Some(text.as_str()) {
                        Action::None
                    } else {
                        Action::RenameSession {
                            session_id,
                            name: text,
                        }
                    }
                }
            }
        }
        KeyCode::Backspace => {
            input.pop();
            app.mode = Mode::Prompt { kind, input };
            Action::None
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.mode = Mode::Prompt {
                kind,
                input: String::new(),
            };
            Action::None
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            input.push(c);
            app.mode = Mode::Prompt { kind, input };
            Action::None
        }
        _ => Action::None,
    }
}

/// First Tab completes as far as it can; while several matches are listed,
/// each further Tab puts the next one in the prompt.
fn tab_complete(app: &mut App, kind: PromptKind, input: String) {
    if !app.completions.is_empty() {
        cycle_completion(app, kind, 1);
        return;
    }
    let c = match kind {
        PromptKind::AddFinca => complete_ssh_host(&input, &crate::setup::ssh_hosts(&app.ssh_dir)),
        _ => complete_path(&input),
    };
    app.completions = c.candidates;
    app.completion_idx = None;
    app.mode = Mode::Prompt {
        kind,
        input: c.input,
    };
}

fn normal_key(app: &mut App, key: KeyEvent) -> Action {
    app.footer = None;
    if app.pane == Pane::Todo {
        return todo_key(app, key);
    }
    // Raw mode delivers Ctrl-C as Char('c'); a reflexive Ctrl-C must not
    // clear sessions, so modified keys never map to actions.
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return Action::None;
    }
    if key.code == KeyCode::Char('v') {
        app.mode = Mode::Grid {
            sel: 0,
            reply: None,
        };
        return Action::None;
    }
    if key.code == KeyCode::Char('/') {
        app.mode = Mode::Search {
            query: String::new(),
            sel: 0,
        };
        return Action::None;
    }
    if let Some(action) = pin_key(app, key.code) {
        return action;
    }
    if let Some(action) = finca_key(app, key.code) {
        return action;
    }
    match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('?') => {
            app.mode = Mode::Help;
            Action::None
        }
        KeyCode::Char('R') => Action::Update,
        KeyCode::Char('F') => {
            app.mode = add_finca_prompt();
            Action::None
        }
        KeyCode::Char('j') | KeyCode::Down => {
            // Off the bottom of the sessions and shells: down into the to-do list.
            let last = app.session_rows().saturating_sub(1);
            if app.pane == Pane::Sessions && app.todo_slug.is_some() && app.sess_idx >= last {
                app.pane = Pane::Todo;
            } else {
                move_selection(app, 1);
            }
            Action::None
        }
        KeyCode::Char('k') | KeyCode::Up => {
            move_selection(app, -1);
            Action::None
        }
        KeyCode::Tab => {
            cycle_pane(app, 1);
            Action::None
        }
        KeyCode::BackTab => {
            cycle_pane(app, -1);
            Action::None
        }
        KeyCode::Char('t') if app.todo_slug.is_some() => {
            app.pane = Pane::Todo;
            Action::None
        }
        KeyCode::Char('l') | KeyCode::Right => {
            app.pane = Pane::Sessions;
            Action::None
        }
        KeyCode::Char('h') | KeyCode::Left => {
            app.pane = Pane::Projects;
            Action::None
        }
        KeyCode::Char('a') => {
            app.mode = Mode::Prompt {
                kind: PromptKind::AddProject,
                input: app.cwd.clone(),
            };
            Action::None
        }
        KeyCode::Enter if app.pane == Pane::Sessions && app.selected_shell().is_some() => {
            let window = app.selected_shell().map(|s| s.id.clone());
            Action::Jump {
                window: window.unwrap_or_default(),
            }
        }
        KeyCode::Char('r') if app.pane == Pane::Sessions && app.selected_shell().is_some() => {
            if let Some(s) = app.selected_shell() {
                app.mode = Mode::Prompt {
                    kind: PromptKind::RenameShell {
                        window: s.id.clone(),
                        finca: None,
                    },
                    input: s.name.clone(),
                };
            }
            Action::None
        }
        KeyCode::Char('x') if app.pane == Pane::Sessions && app.selected_shell().is_some() => {
            if let Some(s) = app.selected_shell() {
                app.mode = Mode::ConfirmShell {
                    window: s.id.clone(),
                    label: format!("{}:{}", s.project, s.name),
                };
            }
            Action::None
        }
        KeyCode::Enter if app.pane == Pane::Sessions => match app.selected_session() {
            Some(s) if state_is_live(s.state) => match &s.tmux_window {
                Some(w) => Action::Jump { window: w.clone() },
                None => Action::None,
            },
            Some(s) => Action::Resume {
                session_id: s.id.clone(),
            },
            None => Action::None,
        },
        KeyCode::Char('r') => {
            if let Some(s) = app
                .selected_session()
                .filter(|_| app.pane == Pane::Sessions)
            {
                app.mode = Mode::Prompt {
                    kind: PromptKind::RenameSession {
                        session_id: s.id.clone(),
                    },
                    input: s.name.clone(),
                };
            }
            Action::None
        }
        KeyCode::Char('x') => match app.selected_session() {
            Some(s) if app.pane == Pane::Sessions && state_is_live(s.state) => {
                app.mode = Mode::Confirm {
                    session_id: s.id.clone(),
                    label: s.name.clone(),
                    finca: None,
                };
                Action::None
            }
            _ => Action::None,
        },
        code => {
            let Some(slug) = usable_project(app) else {
                return Action::None;
            };
            match code {
                KeyCode::Enter => Action::NewSession { slug, name: None },
                KeyCode::Char('n') => {
                    app.mode = Mode::Prompt {
                        kind: PromptKind::NewSession,
                        input: String::new(),
                    };
                    Action::None
                }
                KeyCode::Char('e') => Action::EditProject { slug },
                KeyCode::Char('s') => Action::ProjectShell { slug },
                KeyCode::Char('S') => Action::NewShell { slug },
                KeyCode::Char('c') => Action::ClearEnded { slug },
                KeyCode::Char('d') if app.pane == Pane::Projects => {
                    let name = app.selected_project().map(|p| p.name.clone());
                    app.mode = Mode::ConfirmRemove {
                        slug,
                        name: name.unwrap_or_default(),
                    };
                    Action::None
                }
                _ => Action::None,
            }
        }
    }
}

/// The selected project's slug, unless there is none or it failed to parse
/// (in which case the footer says why).
fn usable_project(app: &mut App) -> Option<String> {
    let p = app.selected_project()?;
    if let Some(err) = &p.error {
        app.footer = Some(format!("{} can't be used: {err}", p.path.display()));
        return None;
    }
    Some(p.slug.clone())
}

/// The to-do pane: every key goes to the list (todo.rs), except Tab, h, q
/// outside insert mode.
fn todo_key(app: &mut App, key: KeyEvent) -> Action {
    let Some(slug) = app.todo_slug.clone() else {
        app.pane = Pane::Projects;
        return Action::None;
    };
    if !app.todos.inserting() && key.modifiers.is_empty() {
        match key.code {
            KeyCode::Tab => {
                cycle_pane(app, 1);
                return Action::None;
            }
            KeyCode::BackTab => {
                cycle_pane(app, -1);
                return Action::None;
            }
            KeyCode::Char('h') | KeyCode::Left => {
                app.pane = Pane::Projects;
                return Action::None;
            }
            // Off the top of the list: back up to the sessions.
            KeyCode::Char('k') | KeyCode::Up if app.todos.at_top() => {
                app.pane = Pane::Sessions;
                return Action::None;
            }
            KeyCode::Char('q') => return Action::Quit,
            KeyCode::Char('?') => {
                app.mode = Mode::Help;
                return Action::None;
            }
            _ => {}
        }
    }
    match app.todos.key(key) {
        Effect::Nothing => Action::None,
        Effect::Changed => Action::SaveTodos {
            slug,
            text: app.todos.serialize(),
        },
        Effect::OpenEditor => Action::EditTodos { slug },
    }
}

/// Projects → Pinned (when there are pins) → Sessions → To-do (when a
/// project is selected) and around.
fn cycle_pane(app: &mut App, step: isize) {
    let mut panes = vec![Pane::Projects];
    if !app.pinned_rows().is_empty() {
        panes.push(Pane::Pinned);
    }
    panes.push(Pane::Sessions);
    if app.todo_slug.is_some() {
        panes.push(Pane::Todo);
    }
    let at = panes.iter().position(|p| *p == app.pane).unwrap_or(0) as isize;
    app.pane = panes[(at + step).rem_euclid(panes.len() as isize) as usize];
}

fn move_selection(app: &mut App, delta: isize) {
    let (idx, len) = match app.pane {
        Pane::Projects => {
            let len = app.left_len();
            (&mut app.proj_idx, len)
        }
        Pane::Sessions => {
            let len = match app.selected_finca() {
                Some(_) => app.finca_items().len(),
                None => app.session_rows(),
            };
            (&mut app.sess_idx, len)
        }
        Pane::Pinned => {
            let len = app.pinned_rows().len();
            (&mut app.pin_idx, len)
        }
        // todo_key handles its own movement.
        Pane::Todo => return,
    };
    if len == 0 {
        *idx = 0;
        return;
    }
    *idx = (*idx as isize + delta).clamp(0, len as isize - 1) as usize;
    if app.pane == Pane::Projects {
        app.sess_idx = 0;
    }
}

/// Keys that mean something different when a finca is selected. `None`
/// falls through to the normal handling (navigation, `a`, `q`, Tab).
fn finca_key(app: &mut App, code: KeyCode) -> Option<Action> {
    let f = app.selected_finca()?;
    let (slug, name, error, via) = (
        f.slug.clone(),
        f.name.clone(),
        f.error.clone(),
        f.via.clone(),
    );
    let item = (app.pane == Pane::Sessions)
        .then(|| app.selected_finca_item())
        .flatten();
    match code {
        KeyCode::Enter => Some(match (error, item) {
            (Some(e), _) => {
                app.footer = Some(format!("{name} can't be used: {e}"));
                Action::None
            }
            (None, Some(FincaItem::Shell(w))) => Action::FincaShellWindow { slug, window: w.id },
            (None, Some(FincaItem::Session { session, .. })) if session.window.is_some() => {
                Action::FincaJump {
                    slug,
                    via,
                    window: session.window.unwrap_or_default(),
                }
            }
            // A finca Grove too old to say which window: its dashboard.
            (None, _) => Action::ConnectFinca { slug, via },
        }),
        KeyCode::Char('a') => {
            app.mode = add_finca_prompt();
            Some(Action::None)
        }
        KeyCode::Char('r') => {
            if let Some(FincaItem::Shell(w)) = item {
                app.mode = Mode::Prompt {
                    kind: PromptKind::RenameShell {
                        window: w.id.clone(),
                        finca: Some(slug),
                    },
                    input: w.label(),
                };
            }
            Some(Action::None)
        }
        KeyCode::Char('s') => Some(match error {
            Some(e) => {
                app.footer = Some(format!("{name} can't be used: {e}"));
                Action::None
            }
            None => Action::FincaShell { slug, via },
        }),
        KeyCode::Char('n' | 'e' | 'c' | 'x') => {
            app.footer = Some(format!(
                "Sessions on {name} are managed there: connect with Enter (C-q to come back)"
            ));
            Some(Action::None)
        }
        _ => None,
    }
}

fn add_finca_prompt() -> Mode {
    Mode::Prompt {
        kind: PromptKind::AddFinca,
        input: String::new(),
    }
}

/// Puts the next (`step` = 1) or previous (-1) match in the prompt, wrapping.
/// From no selection, forward starts at the first match and back at the last.
fn cycle_completion(app: &mut App, kind: PromptKind, step: isize) {
    let n = app.completions.len() as isize;
    let next = match app.completion_idx {
        Some(i) => (i as isize + step).rem_euclid(n),
        None if step > 0 => 0,
        None => n - 1,
    } as usize;
    app.completion_idx = Some(next);
    app.mode = Mode::Prompt {
        kind,
        input: app.completions[next].clone(),
    };
}

/// Keys in the session grid, and in its reply line while that's open.
fn grid_key(app: &mut App, sel: usize, reply: Option<Reply>, key: KeyEvent) -> Action {
    use super::grid::{Dir, next_waiting, step, tiles};
    let tiles = tiles(app, chrono::Utc::now());
    // Sessions come and go under the selection.
    let mut sel = sel.min(tiles.len().saturating_sub(1));
    if let Some(mut r) = reply {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                app.footer = None;
                app.mode = Mode::Grid { sel, reply: None };
                return Action::None;
            }
            KeyCode::Enter => {
                // Its session, wherever its tile is now.
                let target = tiles.iter().position(|t| {
                    t.session_id.as_deref() == Some(r.session_id.as_str())
                        && t.finca.as_ref().map(|f| &f.0) == r.finca.as_ref()
                });
                let action = match target.map(|k| &tiles[k]) {
                    None => {
                        app.footer = Some(format!("{} has ended", r.handle));
                        Action::None
                    }
                    Some(t) if t.stale => {
                        let name = t.finca.as_ref().map_or("it", |f| f.1.as_str());
                        app.footer = Some(format!("{name} isn't answering"));
                        Action::None
                    }
                    Some(t) => Action::GridReply {
                        finca: t
                            .finca
                            .as_ref()
                            .map(|(slug, _)| (slug.clone(), t.via.clone())),
                        session_id: r.session_id.clone(),
                        handle: r.handle.clone(),
                        text: r.text.clone(),
                    },
                };
                // Kept until the send succeeds (run.rs clears it).
                app.mode = Mode::Grid {
                    sel: target.unwrap_or(sel),
                    reply: Some(r),
                };
                return action;
            }
            KeyCode::Char('u') if ctrl => r.text.clear(),
            KeyCode::Backspace => {
                r.text.pop();
            }
            KeyCode::Char(c) if !ctrl => r.text.push(c),
            _ => {}
        }
        app.mode = Mode::Grid {
            sel,
            reply: Some(r),
        };
        return Action::None;
    }
    app.footer = None;
    let n = tiles.len();
    let cols = app.grid_cols.get();
    let mut action = Action::None;
    match key.code {
        KeyCode::Esc | KeyCode::Char('v') => {
            app.mode = Mode::Normal;
            return Action::None;
        }
        KeyCode::Char('?') => {
            app.mode = Mode::Help;
            return Action::None;
        }
        KeyCode::Char('q') => return Action::Quit,
        KeyCode::Char('h') | KeyCode::Left => sel = step(sel, n, cols, Dir::Left),
        KeyCode::Char('l') | KeyCode::Right => sel = step(sel, n, cols, Dir::Right),
        KeyCode::Char('k') | KeyCode::Up => sel = step(sel, n, cols, Dir::Up),
        KeyCode::Char('j') | KeyCode::Down => sel = step(sel, n, cols, Dir::Down),
        KeyCode::Tab | KeyCode::BackTab => {
            let states: Vec<_> = tiles.iter().map(|t| t.state).collect();
            sel = next_waiting(&states, sel, key.code == KeyCode::Tab);
        }
        KeyCode::Enter => {
            if let Some(t) = tiles.get(sel) {
                match (&t.finca, &t.window) {
                    (None, Some(w)) => action = Action::Jump { window: w.clone() },
                    (Some((slug, _)), Some(w)) => {
                        action = Action::FincaJump {
                            slug: slug.clone(),
                            via: t.via.clone(),
                            window: w.clone(),
                        }
                    }
                    (_, None) => app.footer = Some(format!("{} has no window yet", t.handle())),
                }
            }
        }
        KeyCode::Char('x') => {
            if let Some(t) = tiles.get(sel) {
                let finca = t.finca.as_ref().map_or("it", |f| f.1.as_str());
                match &t.session_id {
                    _ if t.stale => app.footer = Some(format!("{finca} isn't answering")),
                    None => app.footer = Some(format!("update Grove on {finca} to kill from here")),
                    Some(id) => {
                        app.mode = Mode::GridKill {
                            sel,
                            session_id: id.clone(),
                            finca: t.finca.as_ref().map(|f| f.0.clone()),
                            handle: t.handle(),
                        };
                        return Action::None;
                    }
                }
            }
        }
        KeyCode::Char('i') => {
            if let Some(t) = tiles.get(sel) {
                let h = t.handle();
                let finca = t.finca.as_ref().map_or("it", |f| f.1.as_str());
                if t.stale {
                    app.footer = Some(format!("{finca} isn't answering"));
                } else if t.session_id.is_none() {
                    app.footer = Some(format!("update Grove on {finca} to reply from here"));
                } else {
                    match t.state {
                        State::YourTurn => {
                            app.mode = Mode::Grid {
                                sel,
                                reply: Some(Reply {
                                    session_id: t.session_id.clone().unwrap_or_default(),
                                    finca: t.finca.as_ref().map(|f| f.0.clone()),
                                    handle: h,
                                    text: String::new(),
                                }),
                            };
                            return Action::None;
                        }
                        State::NeedsYou => {
                            app.footer = Some(format!("{h} needs you: Enter to answer it there"))
                        }
                        _ => app.footer = Some(format!("{h} is working; wait for your turn")),
                    }
                }
            }
        }
        _ => {}
    }
    app.mode = Mode::Grid { sel, reply: None };
    action
}

/// Pins: `p` on a session, `1`–`9` from anywhere on the dashboard, and the
/// Pinned pane's own keys. None lets the key through to the usual handling.
fn pin_key(app: &mut App, code: KeyCode) -> Option<Action> {
    let rows = app.pinned_rows();
    if app.pane == Pane::Pinned && rows.is_empty() {
        // The box went (a poll pruned it) without the focus showing
        // anywhere: this key only brings focus back to Sessions.
        app.pane = Pane::Sessions;
        return Some(Action::None);
    }
    if let KeyCode::Char(c @ '1'..='9') = code {
        let n = c as usize - '0' as usize;
        return Some(match rows.get(n - 1) {
            Some(r) => jump_to_pin(app, r),
            None => {
                app.footer = Some(format!("no pin {n}"));
                Action::None
            }
        });
    }
    match app.pane {
        Pane::Pinned => {
            let row = rows.get(app.pin_idx.min(rows.len().saturating_sub(1)));
            Some(match code {
                KeyCode::Char('j') | KeyCode::Down => {
                    if app.pin_idx + 1 >= rows.len() {
                        app.pane = Pane::Sessions;
                        app.sess_idx = 0;
                    } else {
                        app.pin_idx += 1;
                    }
                    Action::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    app.pin_idx = app.pin_idx.saturating_sub(1);
                    Action::None
                }
                KeyCode::Enter => row.map_or(Action::None, |r| jump_to_pin(app, r)),
                KeyCode::Char('p') => row.map_or(Action::None, |r| {
                    let pin = r.pin.clone();
                    toggle_pin(app, pin, &r.handle)
                }),
                KeyCode::Char('x') => {
                    match row {
                        Some(r) if r.unreachable => {
                            let name = r.finca.as_ref().map_or("it", |f| f.1.as_str());
                            app.footer = Some(format!("{name} isn't answering"));
                        }
                        Some(r) => {
                            app.mode = Mode::Confirm {
                                session_id: r.pin.id.clone(),
                                label: r.handle.clone(),
                                finca: r.pin.finca.clone(),
                            };
                        }
                        None => {}
                    }
                    Action::None
                }
                // Keys that work anywhere on the dashboard pass through;
                // session and project keys don't apply here.
                KeyCode::Char('q' | '?' | 'R' | 'F' | 'a' | 't' | 'h' | 'l')
                | KeyCode::Tab
                | KeyCode::BackTab
                | KeyCode::Left
                | KeyCode::Right => return None,
                _ => Action::None,
            })
        }
        Pane::Sessions => match code {
            KeyCode::Char('k') | KeyCode::Up if app.sess_idx == 0 && !rows.is_empty() => {
                app.pane = Pane::Pinned;
                app.pin_idx = rows.len() - 1;
                Some(Action::None)
            }
            KeyCode::Char('p') => {
                if app.selected_finca().is_some() {
                    let f = app.selected_finca()?;
                    let (slug, name) = (f.slug.clone(), f.name.clone());
                    match app.selected_finca_item() {
                        Some(FincaItem::Session { project, session }) => {
                            let handle = format!("{project}:{}", session.name);
                            Some(match session.id {
                                Some(id) => toggle_pin(
                                    app,
                                    crate::pins::Pin {
                                        finca: Some(slug),
                                        id,
                                    },
                                    &handle,
                                ),
                                None => {
                                    app.footer =
                                        Some(format!("update Grove on {name} to pin its sessions"));
                                    Action::None
                                }
                            })
                        }
                        _ => Some(Action::None),
                    }
                } else {
                    let s = app
                        .selected_session()
                        .filter(|s| state_is_live(s.state))
                        .cloned();
                    Some(match s {
                        Some(s) => toggle_pin(
                            app,
                            crate::pins::Pin {
                                finca: None,
                                id: s.id.clone(),
                            },
                            &format!("{}:{}", s.project, s.name),
                        ),
                        None => Action::None,
                    })
                }
            }
            _ => None,
        },
        _ => None,
    }
}

/// Pins `pin`, or unpins it if it's pinned.
fn toggle_pin(app: &mut App, pin: crate::pins::Pin, handle: &str) -> Action {
    match app.pins.iter().position(|p| *p == pin) {
        Some(i) => {
            app.pins.remove(i);
            app.footer = Some(format!("Unpinned {handle}"));
        }
        None => {
            app.pins.push(pin);
            app.footer = Some(format!("Pinned {handle} ({})", app.pins.len()));
        }
    }
    Action::SavePins {
        pins: app.pins.clone(),
    }
}

/// Goes to a pin's window: here, or connecting to its finca.
fn jump_to_pin(app: &mut App, r: &super::app::PinRow) -> Action {
    match (&r.finca, &r.window) {
        (Some((_, name, _)), _) if r.unreachable => {
            app.footer = Some(format!("{name} isn't answering"));
            Action::None
        }
        (None, Some(w)) => Action::Jump { window: w.clone() },
        (Some((slug, _, via)), Some(w)) => Action::FincaJump {
            slug: slug.clone(),
            via: via.clone(),
            window: w.clone(),
        },
        (Some((slug, _, via)), None) => Action::ConnectFinca {
            slug: slug.clone(),
            via: via.clone(),
        },
        (None, None) => {
            app.footer = Some(format!("{} has no window yet", r.handle));
            Action::None
        }
    }
}
