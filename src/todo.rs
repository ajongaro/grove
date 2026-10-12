//! A project's to-do list and its vim-style editing. Pure: the dashboard
//! saves when a key returns `Effect::Changed`.
//!
//! Stored as a Markdown checklist (`- [ ] text` / `- [x] text`).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub text: String,
    pub done: bool,
}

/// What the caller should do after a key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Effect {
    Nothing,
    /// The list changed: save it.
    Changed,
    /// Open the list's file in the editor.
    OpenEditor,
}

/// The items and cursor as they were before a change.
type Snapshot = (Vec<Item>, usize);

#[derive(Debug, Clone)]
struct Insert {
    /// The item was added by o/O (so empty means "never mind").
    new: bool,
    before: Snapshot,
}

#[derive(Debug, Clone, Default)]
pub struct Todos {
    pub items: Vec<Item>,
    pub idx: usize,
    /// The last item `dd` deleted, for `p`/`P`.
    register: Option<Item>,
    /// Before the last change, for `u` (one level).
    undo: Option<Snapshot>,
    /// First key of `dd`, `gg` or `cc`.
    pending: Option<char>,
    insert: Option<Insert>,
}

impl Todos {
    /// Takes the file's contents. A list that differs from ours (an outside
    /// edit) replaces it and drops undo; the cursor and register stay.
    pub fn load(&mut self, text: &str) {
        if self.inserting() {
            return;
        }
        let items = parse(text);
        if items != self.items {
            self.items = items;
            self.undo = None;
        }
        self.clamp();
    }

    /// For another project's list: everything but the register goes.
    pub fn reset(&mut self) {
        *self = Self {
            register: self.register.take(),
            ..Self::default()
        };
    }

    pub fn serialize(&self) -> String {
        self.items
            .iter()
            .map(|i| format!("- [{}] {}\n", if i.done { 'x' } else { ' ' }, i.text))
            .collect()
    }

    pub fn inserting(&self) -> bool {
        self.insert.is_some()
    }

    pub fn key(&mut self, key: KeyEvent) -> Effect {
        if self.inserting() {
            return self.insert_key(key);
        }
        let pending = self.pending.take();
        // Raw mode delivers Ctrl-C as 'c': modified keys never edit.
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return Effect::Nothing;
        }
        match (pending, key.code) {
            (Some('d'), KeyCode::Char('d')) => self.change(|t| {
                if !t.items.is_empty() {
                    t.register = Some(t.items.remove(t.idx));
                    t.clamp();
                }
            }),
            (Some('g'), KeyCode::Char('g')) => {
                self.idx = 0;
                Effect::Nothing
            }
            (Some('c'), KeyCode::Char('c')) => self.begin_insert(None, true),
            (_, KeyCode::Char(c @ ('d' | 'g' | 'c'))) => {
                self.pending = Some(c);
                Effect::Nothing
            }
            (_, KeyCode::Char('j') | KeyCode::Down) => {
                self.idx = (self.idx + 1).min(self.items.len().saturating_sub(1));
                Effect::Nothing
            }
            (_, KeyCode::Char('k') | KeyCode::Up) => {
                self.idx = self.idx.saturating_sub(1);
                Effect::Nothing
            }
            (_, KeyCode::Char('G')) => {
                self.idx = self.items.len().saturating_sub(1);
                Effect::Nothing
            }
            (_, KeyCode::Char('o')) => self.begin_insert(Some(1), false),
            (_, KeyCode::Char('O')) => self.begin_insert(Some(0), false),
            (_, KeyCode::Char('i' | 'a' | 'A')) => self.begin_insert(None, false),
            (_, KeyCode::Char('x' | ' ')) => self.change(|t| {
                if let Some(i) = t.items.get_mut(t.idx) {
                    i.done = !i.done;
                }
            }),
            (_, KeyCode::Char('p')) => self.paste(1),
            (_, KeyCode::Char('P')) => self.paste(0),
            (_, KeyCode::Char('u')) => match self.undo.take() {
                Some((items, idx)) => {
                    self.items = items;
                    self.idx = idx;
                    self.clamp();
                    Effect::Changed
                }
                None => Effect::Nothing,
            },
            (_, KeyCode::Char('E')) => Effect::OpenEditor,
            _ => Effect::Nothing,
        }
    }

    fn insert_key(&mut self, key: KeyEvent) -> Effect {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(item) = self.items.get_mut(self.idx) else {
            self.insert = None;
            return Effect::Nothing;
        };
        match key.code {
            KeyCode::Enter | KeyCode::Esc => return self.finish_insert(),
            KeyCode::Backspace => {
                item.text.pop();
            }
            KeyCode::Char('u') if ctrl => item.text.clear(),
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                item.text.push(c)
            }
            _ => {}
        }
        Effect::Nothing
    }

    /// Edits the item under the cursor, or with `add` a blank one below (1)
    /// or above (0) it; `clear` empties the text first.
    fn begin_insert(&mut self, add: Option<usize>, clear: bool) -> Effect {
        let before = (self.items.clone(), self.idx);
        let new = add.is_some();
        if let Some(offset) = add {
            let at = if self.items.is_empty() {
                0
            } else {
                self.idx + offset
            };
            self.items.insert(at, blank());
            self.idx = at;
        } else if let Some(i) = self.items.get_mut(self.idx) {
            if clear {
                i.text.clear();
            }
        } else {
            return Effect::Nothing;
        }
        self.insert = Some(Insert { new, before });
        Effect::Nothing
    }

    /// Ends insert mode. An item left empty is removed (a new one quietly).
    fn finish_insert(&mut self) -> Effect {
        let Some(ins) = self.insert.take() else {
            return Effect::Nothing;
        };
        let item = &mut self.items[self.idx];
        item.text = item.text.trim().to_string();
        if item.text.is_empty() {
            self.items.remove(self.idx);
            if ins.new {
                self.idx = ins.before.1;
            }
            self.clamp();
        }
        if self.items == ins.before.0 {
            return Effect::Nothing;
        }
        self.undo = Some(ins.before);
        Effect::Changed
    }

    /// `offset` 1 pastes below the cursor, 0 above.
    fn paste(&mut self, offset: usize) -> Effect {
        let Some(item) = self.register.clone() else {
            return Effect::Nothing;
        };
        self.change(|t| {
            let at = if t.items.is_empty() {
                0
            } else {
                t.idx + offset
            };
            t.items.insert(at, item);
            t.idx = at;
        })
    }

    /// Applies `f`, recording undo if it changed anything.
    fn change(&mut self, f: impl FnOnce(&mut Self)) -> Effect {
        let before = (self.items.clone(), self.idx);
        f(self);
        if self.items == before.0 {
            return Effect::Nothing;
        }
        self.undo = Some(before);
        Effect::Changed
    }

    fn clamp(&mut self) {
        self.idx = self.idx.min(self.items.len().saturating_sub(1));
    }
}

fn blank() -> Item {
    Item {
        text: String::new(),
        done: false,
    }
}

/// Checklist lines (`-` or `*`, `[ ]`/`[x]`/`[X]`) become items; any other
/// non-blank line becomes an open item with that text.
fn parse(text: &str) -> Vec<Item> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let rest = l.strip_prefix("- ").or_else(|| l.strip_prefix("* "));
            let checked = rest.and_then(|r| {
                r.strip_prefix("[ ] ")
                    .map(|t| (t, false))
                    .or_else(|| r.strip_prefix("[x] ").map(|t| (t, true)))
                    .or_else(|| r.strip_prefix("[X] ").map(|t| (t, true)))
            });
            match checked {
                Some((t, done)) => Item {
                    text: t.trim().to_string(),
                    done,
                },
                None => Item {
                    text: l.to_string(),
                    done: false,
                },
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "todo_tests.rs"]
mod tests;
