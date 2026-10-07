//! Key handling: mutates `App` and returns the side effect to perform.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::app::{Action, App, Mode, Pane, PromptKind, state_is_live};
use super::complete::complete_path;

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    if key.kind == KeyEventKind::Release {
        return Action::None;
    }
    match app.mode.clone() {
        Mode::Prompt { kind, input } => prompt_key(app, kind, input, key),
        Mode::Confirm { session_id, .. } => {
            app.mode = Mode::Normal;
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => Action::Kill { session_id },
                _ => Action::None,
            }
        }
        Mode::Normal => normal_key(app, key),
    }
}

fn prompt_key(app: &mut App, kind: PromptKind, mut input: String, key: KeyEvent) -> Action {
    if key.code == KeyCode::Tab && kind == PromptKind::AddProject {
        tab_complete(app, kind, input);
        return Action::None;
    }
    // C-j / C-k step through an open match list (down / up).
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && let KeyCode::Char(c @ ('j' | 'k')) = key.code
    {
        if !app.completions.is_empty() {
            cycle_completion(app, kind, if c == 'j' { 1 } else { -1 });
        }
        return Action::None;
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
                PromptKind::AddProject => Action::AddProject { path: text },
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
        KeyCode::Char('j') | KeyCode::Down => {
            move_selection(app, 1);
            Action::None
        }
        KeyCode::Char('k') | KeyCode::Up => {
            move_selection(app, -1);
            Action::None
        }
        KeyCode::Tab | KeyCode::BackTab => {
            app.pane = if app.pane == Pane::Projects {
                Pane::Sessions
            } else {
                Pane::Projects
            };
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
                KeyCode::Char('w') => Action::OpenWiki { slug },
                KeyCode::Char('c') => Action::ClearEnded { slug },
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
    let (slug, name, error) = (f.slug.clone(), f.name.clone(), f.error.clone());
    match code {
        KeyCode::Enter => Some(match error {
            Some(e) => {
                app.footer = Some(format!("{name} can't be used: {e}"));
                Action::None
            }
            None => Action::ConnectFinca { slug },
        }),
        KeyCode::Char('n' | 'e' | 'w' | 'c' | 'x') => {
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
