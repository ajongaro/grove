# TODO Priorities and Due Dates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** TODO items get a band (high / normal / low) and an optional due date, on the dashboard, in the file and through Claude's MCP tools.

**Architecture:** `todo::Item` gains `priority` and `due`; `todo.rs` keeps items stable-sorted by band, so the list index is still the cursor and the MCP number. Date parsing and display live in a new pure `src/due.rs`. The dashboard draws band headers and a right-aligned due column; `@` opens the existing prompt popup.

**Tech Stack:** Rust, ratatui, chrono (already a dependency), rmcp.

**Spec:** `docs/superpowers/specs/2026-11-19-todo-priorities-design.md`

## Global Constraints

- File stays a Markdown checklist; a list with only normal items serializes byte-for-byte as today (no headings).
- Headings Grove writes: `## High priority`, `## Normal`, `## Low priority`, with a blank line between bands.
- Due suffix: ` (due YYYY-MM-DD)` at the end of the item text.
- "Soon" = due within 3 days (today..=today+3). Overdue = before today.
- Every change ships: fmt, clippy clean, then `scripts/release.sh`, `cargo install --path .`, `scripts/deploy-fincas.sh`.

## Review Focus

- A file edited by hand with bands out of order (low heading first): items must come back grouped high, normal, low, and nothing lost.
- An item whose text itself contains "(due …)" that isn't a valid date (`(due soon)`): stays as text, no due set.
- `p` pasting an item deleted from the high band into the low band: it takes the low band (the list stays grouped).
- `+` on a high item / `-` on a low item: nothing changes, no undo entry.
- MCP `add` with `priority: "urgent"` or `due: "next week"`: the whole batch is refused with a clear message.

---

### Task 1: `src/due.rs`, parsing and showing due dates

**Files:**
- Create: `src/due.rs` (register `pub mod due;` in `src/lib.rs`)

**Interfaces:**
- Produces:
  - `pub fn parse(input: &str, today: NaiveDate) -> Result<Option<NaiveDate>, String>`: `Ok(None)` for empty/blank (clears).
  - `pub fn show(d: NaiveDate, today: NaiveDate) -> String`: `Dec 1`, or `Dec 1 2027` when not today's year.
  - `pub enum Urgency { Later, Soon, Overdue }` and `pub fn urgency(d: NaiveDate, today: NaiveDate) -> Urgency`.

- [ ] **Step 1: Tests (in `#[cfg(test)] mod tests` of due.rs), with today = 2026-11-19 (a Thursday)**

```rust
assert_eq!(parse("2026-12-01", t), Ok(Some(d(2026,12,1))));
assert_eq!(parse("12/1", t), Ok(Some(d(2026,12,1))));
assert_eq!(parse("11-1", t), Ok(Some(d(2027,11,1))));   // passed this year: next year
assert_eq!(parse("Dec 1", t), Ok(Some(d(2026,12,1))));
assert_eq!(parse("1 december", t), Ok(Some(d(2026,12,1))));
assert_eq!(parse("fri", t), Ok(Some(d(2026,11,20))));
assert_eq!(parse("thursday", t), Ok(Some(d(2026,11,26)))); // a weekday is the next one, not today
assert_eq!(parse("today", t), Ok(Some(t)));
assert_eq!(parse("tomorrow", t), Ok(Some(d(2026,11,20))));
assert_eq!(parse("+3", t), Ok(Some(d(2026,11,22))));
assert_eq!(parse("  ", t), Ok(None));
assert!(parse("next week", t).unwrap_err().contains("2026-12-01"));  // the error shows an example
assert!(parse("2/30", t).is_err());
assert_eq!(show(d(2026,12,1), t), "Dec 1");
assert_eq!(show(d(2027,1,5), t), "Jan 5 2027");
assert_eq!(urgency(d(2026,11,18), t), Urgency::Overdue);
assert_eq!(urgency(t, t), Urgency::Soon);
assert_eq!(urgency(d(2026,11,22), t), Urgency::Soon);
assert_eq!(urgency(d(2026,11,23), t), Urgency::Later);
```

- [ ] **Step 2:** `cargo test due::` fails (module missing).
- [ ] **Step 3:** Implement. Case-insensitive; month names and weekdays by 3-letter prefix; `+N` days, N up to 3650.
- [ ] **Step 4:** `cargo test due::` passes.
- [ ] **Step 5:** Commit `feat: due-date parsing for TODOs`.

### Task 2: Bands and due dates in the TODO model and file

**Files:**
- Modify: `src/todo.rs` (Item, parse, serialize_items, Todos keys), `src/todo_tests.rs`

**Interfaces:**
- Consumes: nothing from Task 1 except `NaiveDate` parsing of the stored ISO date (use `NaiveDate::parse_from_str(_, "%Y-%m-%d")`).
- Produces:
  - `#[derive(Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)] pub enum Priority { High, #[default] Normal, Low }` (declared in that order so `Ord` sorts high first), with `pub fn name(self) -> &'static str` ("high"/"normal"/"low") and `pub fn from_name(&str) -> Option<Priority>`.
  - `Item { text, done, pub priority: Priority, pub due: Option<NaiveDate> }` and `Item::new(text: impl Into<String>) -> Item` (normal, no due, open).
  - `Effect::AskDue` (new variant): `@` on an item.
  - `Todos::set_due(&mut self, due: Option<NaiveDate>) -> Effect`: on the cursor's item, with undo.
  - `pub fn sort_bands(items: &mut [Item])`: stable sort by priority.

- [ ] **Step 1: Tests in todo_tests.rs**
  - `bands_are_read_from_headings_and_written_back`: text with `## High priority`, `## Normal`, `## Low priority` sections round-trips exactly; items carry the right priority.
  - `a_list_with_no_bands_is_written_as_before`: `"- [ ] a\n- [x] b\n"` serializes unchanged.
  - `headings_in_any_order_or_case_group_the_items`: `"- [ ] n0\n## low stuff\n- [ ] l\n# HIGH\n- [ ] h\n## Notes\n- [ ] n1\n"` gives `h` (high), `n0`, `n1` (normal), `l` (low), in that order.
  - `a_due_suffix_becomes_the_due_date`: `"- [ ] pay (due 2026-12-01)"` gives text "pay", due 2026-12-01, and writes back the same; `"- [ ] ask (due soon)"` keeps text "ask (due soon)", due None.
  - `plus_and_minus_move_an_item_between_bands`: list high [h], normal [a, b], low [l]; cursor on `b`, `+` makes `b` the last high item (order h, b, a, l) with cursor on b; `-` twice from `h`'s band edge makes it first low. `+` on a high item returns `Effect::Nothing`.
  - `new_items_and_pastes_take_the_cursors_band`: `n` on a low item adds a low item; `d` on a high item, then `p` on a low item gives a low item.
  - `at_asks_for_a_due_date_and_set_due_saves_it`: `@` returns `Effect::AskDue`; `set_due(Some(d))` returns Changed and `u` undoes it; on an empty list `@` is Nothing.
  - Update `other_lines_become_open_items_and_blank_lines_are_dropped`: a `# Notes` line is now a heading (dropped, normal band), not an item.
- [ ] **Step 2:** `cargo test todo::` fails.
- [ ] **Step 3:** Implement. `parse` tracks the current band from lines starting with `#` (strip `#`s and spaces, lowercase, `starts_with("high")` / `starts_with("low")`, else normal), strips a trailing ` (due YYYY-MM-DD)` only when it parses, then `sort_bands`. `serialize_items` writes headings only when any item is not normal, and skips empty bands. `+`/`-`: remove the item and reinsert at the end of the band above / start of the band below. `begin_insert` and `paste` set the inserted item's priority to the cursor item's (normal on an empty list).
- [ ] **Step 4:** `cargo test` passes (the whole suite: other modules build `Item` via `Item::new`).
- [ ] **Step 5:** Commit `feat: TODO bands and due dates in the list file`.

### Task 3: Claude's view (MCP tools and session context)

**Files:**
- Modify: `src/todo.rs` (`Op`, `apply_ops`), `src/mcp.rs` (`numbered`, `TodoOpArg`, tool description), `src/hook.rs` (`todo_context`), `src/mcp_tests.rs`, `src/todo_tests.rs`, `tests/mcp.rs` if it pins the schema.

**Interfaces:**
- Consumes: `Priority`, `Item` (Task 2), `due::parse` is NOT used here: MCP takes strict `YYYY-MM-DD`.
- Produces:
  - `Op::Add { text, priority: Option<Priority>, due: Option<Option<NaiveDate>> }`, `Op::Reword { n, text, priority, due }`, `Op::Set { n, priority, due }` (`due: Some(None)` clears). `apply_ops` appends adds to the end of their band and re-sorts bands at the end.
  - `TodoOpArg` gains `priority: Option<String>` ("high, normal or low") and `due: Option<String>` ("YYYY-MM-DD; empty clears it").
  - `numbered` lines: `3. [ ] [high] Pay the deposit (due 2026-12-01)`; normal items have no tag.

- [ ] **Step 1: Tests**
  - todo_tests `ops_set_priority_and_due_and_keep_bands`: add with high lands after existing high items; `Set` moves an item's band; `Set` with `due: Some(None)` clears.
  - mcp_tests `edit_todos_takes_priority_and_due`: returns the list with `[high]` and `(due 2026-12-01)`; `priority: "urgent"` → Err mentioning "high, normal or low", nothing saved; `due: "next week"` → Err mentioning "YYYY-MM-DD"; `op: "set"` without priority or due → Err.
  - hook test `todo_context_lists_high_items_first_with_due_dates`: context shows `- [high] x (due 2026-12-01)` before normal items and includes the sentence "Only set a priority or due date when the user gave one."
- [ ] **Step 2:** Run them; they fail.
- [ ] **Step 3:** Implement. Tool description: "add, check, uncheck, reword, set or remove items by number; add/reword/set take an optional priority (high, normal, low) and due date (YYYY-MM-DD)".
- [ ] **Step 4:** `cargo test` passes.
- [ ] **Step 5:** Commit `feat: Claude reads and sets TODO priorities and due dates`.

### Task 4: The dashboard

**Files:**
- Modify: `src/ui/render.rs` (`render_todos`, prompt title, help keys), `src/ui/app.rs` (`PromptKind::TodoDue { slug }`), `src/ui/input.rs` (`todo_key` for `Effect::AskDue`, `prompt_key` for `TodoDue`), `src/ui/hints.rs`, `src/ui/tests.rs`, `README.md` (TODO lists section and key table).

**Interfaces:**
- Consumes: `due::{parse, show, urgency, Urgency}` (Task 1); `Effect::AskDue`, `Todos::set_due`, `Item.priority/due` (Task 2).
- Produces: `render_todos(frame, app, area, now)` (now passed in for today, `now.with_timezone(&chrono::Local).date_naive()`).

- [ ] **Step 1: Tests in ui/tests.rs**
  - `todo_bands_show_headers_and_due_dates`: a list with one high (due today+10 → `Dec 1`-style text), two normal, one low item; the screen has a `HIGH` line above the high item and a `LOW` line above the low one, no header over the normal items, and the high item's row ends with its date.
  - `an_overdue_todo_is_marked`: an open item due yesterday shows `!` after its date; a done one doesn't.
  - `at_on_a_todo_asks_for_its_due_date`: `@` opens `Mode::Prompt { kind: TodoDue { slug }, input }` (input prefilled with the current due as YYYY-MM-DD, empty if none); typing `2026-12-01` and Enter returns `Action::SaveTodos` whose text contains `(due 2026-12-01)`; `bogus` and Enter leaves the list unchanged and sets the footer to the parse error.
  - The selected row stays on the item (not a header) when moving with `j`/`k` across a band header.
- [ ] **Step 2:** Run them; they fail.
- [ ] **Step 3:** Implement. Header rows are `HIGH` / `LOW` in `pal().todo`, bold; the List's selected index is `idx` plus the headers above it. Due text right-aligned to the pane's inner width; `Soon` and `Overdue` in `pal().warn`, overdue gets ` !`; done items' dates dim. Hints: add `("+/-", "priority")`, `("@", "due")`. Help and README key table: `+` `-` raise / lower priority (moves between the HIGH, normal and LOW bands), `@` due date (`2026-12-01`, `12/1`, `dec 1`, `fri`, `today`, `tomorrow`, `+3`; empty clears). README also documents the headings and the `(due …)` suffix in the file paragraph.
- [ ] **Step 4:** `cargo test`, `cargo fmt`, `cargo clippy --all-targets` all clean.
- [ ] **Step 5:** Commit `feat: TODO bands and due dates on the dashboard`, then ship (release, install, deploy) and report.
