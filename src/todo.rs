//! A project's TODO list and its vim-style editing. Pure: the dashboard
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
    /// First key of `gg` or `cc`.
    pending: Option<char>,
    /// The last key was a `d` that deleted: a second `d` (vim's `dd`) is
    /// swallowed rather than deleting the next item too.
    after_d: bool,
    /// Done items `c`/`C` cleared, waiting to be archived by the caller.
    cleared: Vec<Item>,
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
        serialize_items(&self.items)
    }

    /// On the first item (or an empty list) with no half-typed command, so
    /// `k` can leave the list.
    pub fn at_top(&self) -> bool {
        self.idx == 0 && self.pending.is_none() && !self.inserting()
    }

    /// Not typing and not halfway through a command like `d`.
    pub fn idle(&self) -> bool {
        self.pending.is_none() && !self.inserting()
    }

    /// The cursor to the first item, as when arriving from above.
    pub fn go_top(&mut self) {
        self.idx = 0;
    }

    /// The items `c`/`C` cleared since the last call, to archive.
    pub fn take_cleared(&mut self) -> Vec<Item> {
        std::mem::take(&mut self.cleared)
    }

    pub fn inserting(&self) -> bool {
        self.insert.is_some()
    }

    pub fn key(&mut self, key: KeyEvent) -> Effect {
        if self.inserting() {
            return self.insert_key(key);
        }
        let pending = self.pending.take();
        let after_d = std::mem::take(&mut self.after_d);
        // Raw mode delivers Ctrl-C as 'c': modified keys never edit.
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return Effect::Nothing;
        }
        match (pending, key.code) {
            // d deletes at once; vim's `dd` is the same delete.
            (_, KeyCode::Char('d')) if after_d => Effect::Nothing,
            (_, KeyCode::Char('d')) => {
                self.after_d = true;
                self.change(|t| {
                    if !t.items.is_empty() {
                        t.register = Some(t.items.remove(t.idx));
                        t.clamp();
                    }
                })
            }
            (Some('g'), KeyCode::Char('g')) => {
                self.idx = 0;
                Effect::Nothing
            }
            (Some('c'), KeyCode::Char('c')) => self.begin_insert(None, true),
            // c on a done item clears it, like a session list; on an open
            // one it starts cc.
            (_, KeyCode::Char('c')) if self.items.get(self.idx).is_some_and(|i| i.done) => self
                .change(|t| {
                    let item = t.items.remove(t.idx);
                    t.cleared.push(item);
                    t.clamp();
                }),
            (_, KeyCode::Char('C')) if self.items.iter().any(|i| i.done) => self.change(|t| {
                let at = t.idx;
                let before = t.items[..at.min(t.items.len())]
                    .iter()
                    .filter(|i| !i.done)
                    .count();
                let (done, open): (Vec<Item>, Vec<Item>) = std::mem::take(&mut t.items)
                    .into_iter()
                    .partition(|i| i.done);
                t.items = open;
                t.cleared.extend(done);
                // The cursor stays on the open item it was on, or the next.
                t.idx = before;
                t.clamp();
            }),
            (_, KeyCode::Char(c @ ('g' | 'c'))) => {
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
            // The keys the other panes use: n new, r rename (edit), Enter
            // the main thing (done), e the editor. The vim keys still work.
            (_, KeyCode::Char('n' | 'o')) => self.begin_insert(Some(1), false),
            (_, KeyCode::Char('O')) => self.begin_insert(Some(0), false),
            (_, KeyCode::Char('r' | 'i' | 'a' | 'A')) => self.begin_insert(None, false),
            (_, KeyCode::Char('x' | ' ') | KeyCode::Enter) => self.change(|t| {
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
            (_, KeyCode::Char('e' | 'E')) => Effect::OpenEditor,
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
        // An empty list has nothing to edit: a/i/cc start its first item.
        let add = add.or(self.items.is_empty().then_some(0));
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

/// One edit to a list from outside the dashboard (the MCP tools).
/// Numbers are 1-based positions in the list before any edit.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Add(String),
    Check(usize),
    Uncheck(usize),
    Reword(usize, String),
    Remove(usize),
}

/// Applies `ops` in order; any bad op fails the whole batch.
pub fn apply_ops(items: &[Item], ops: &[Op]) -> Result<Vec<Item>, String> {
    let mut slots: Vec<Option<Item>> = items.iter().cloned().map(Some).collect();
    let mut added = Vec::new();
    let len = items.len();
    let at = |n: usize| -> Result<usize, String> {
        if n == 0 || n > len {
            Err(format!(
                "there's no item {n}; the list has {len} item{}",
                if len == 1 { "" } else { "s" }
            ))
        } else {
            Ok(n - 1)
        }
    };
    let text_ok = |t: &str| -> Result<String, String> {
        let t = t.trim();
        if t.is_empty() {
            Err("a TODO needs some text".into())
        } else if t.contains(['\n', '\r']) {
            Err("a TODO is one line; add each one separately".into())
        } else {
            Ok(t.to_string())
        }
    };
    for op in ops {
        match op {
            Op::Add(t) => added.push(Item {
                text: text_ok(t)?,
                done: false,
            }),
            Op::Remove(n) => {
                let i = at(*n)?;
                slots[i]
                    .take()
                    .ok_or_else(|| format!("item {n} was already removed"))?;
            }
            Op::Check(n) | Op::Uncheck(n) | Op::Reword(n, _) => {
                let i = at(*n)?;
                let item = slots[i]
                    .as_mut()
                    .ok_or_else(|| format!("item {n} was removed"))?;
                match op {
                    Op::Check(_) => item.done = true,
                    Op::Uncheck(_) => item.done = false,
                    Op::Reword(_, t) => item.text = text_ok(t)?,
                    _ => unreachable!(),
                }
            }
        }
    }
    Ok(slots.into_iter().flatten().chain(added).collect())
}

/// The stored Markdown checklist for `items`.
pub fn serialize_items(items: &[Item]) -> String {
    items
        .iter()
        .map(|i| format!("- [{}] {}\n", if i.done { 'x' } else { ' ' }, i.text))
        .collect()
}

/// Unchecked items in a TODO file's text.
pub fn open_count(text: &str) -> usize {
    parse(text).iter().filter(|i| !i.done).count()
}

fn blank() -> Item {
    Item {
        text: String::new(),
        done: false,
    }
}

/// Checklist lines (`-` or `*`, `[ ]`/`[x]`/`[X]`) become items; any other
/// non-blank line becomes an open item with that text.
pub fn parse(text: &str) -> Vec<Item> {
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
