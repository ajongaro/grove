//! Key handling: mutates `App` and returns the side effect to perform.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::app::{Action, App, Mode, Pane, PromptKind, state_is_live};
use super::complete::complete_path;
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
        Mode::Confirm { session_id, .. } => {
            app.mode = Mode::Normal;
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => Action::Kill { session_id },
                _ => Action::None,
            }
        }
        Mode::Help => {
            app.mode = Mode::Normal;
            Action::None
        }
        Mode::Normal => normal_key(app, key),
    }
}

fn prompt_key(app: &mut App, kind: PromptKind, mut input: String, key: KeyEvent) -> Action {
    if key.code == KeyCode::Tab && kind == PromptKind::AddProject {
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
    let c = complete_path(&input);
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
        KeyCode::Char('j') | KeyCode::Down => {
            // Off the bottom of the sessions: down into the to-do list.
            let last = app.visible_sessions().len().saturating_sub(1);
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
        KeyCode::Enter if app.pane == Pane::Sessions => match app.selected_session() {
            Some(s) if state_is_live(s.state) => match &s.tmux_window {
                Some(w) => Action::Jump { window: w.clone() },
                None => Action::None,
            },
            _ => Action::None,
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

/// Projects → Sessions → To-do (when a project is selected) and around.
fn cycle_pane(app: &mut App, step: isize) {
    let mut panes = vec![Pane::Projects, Pane::Sessions];
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
            let len = app.visible_sessions().len();
            (&mut app.sess_idx, len)
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
    match code {
        KeyCode::Enter => Some(match error {
            Some(e) => {
                app.footer = Some(format!("{name} can't be used: {e}"));
                Action::None
            }
            None => Action::ConnectFinca { slug, via },
        }),
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
