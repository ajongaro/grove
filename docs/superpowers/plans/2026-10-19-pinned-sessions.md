# Pinned Sessions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Pinned box at the top of the dashboard's right column holding live sessions from any project, account or finca, toggled with `p` and reached with `1`–`9`.

**Architecture:** Pins live in `~/.grove/pins.json` (new `src/pins.rs`). `App::reload` reads them, prunes ended ones against local sessions and finca polls, and saves when pruning changed them. Keys mutate `app.pins` and return `Action::SavePins`. Finca sessions are identified by a new optional `id` in `grove status --json`.

**Tech Stack:** Rust, ratatui, serde_json, the existing tmux test servers.

**Spec:** `docs/superpowers/specs/2026-10-19-pinned-sessions-design.md`

## Global Constraints

- Pins span Claude accounts: stored in Grove's home, never in a `~/.claude*` dir.
- Status JSON stays `version: 1`; `id` is optional and additive.
- `p` in the to-do list stays paste; `1`–`9` do nothing in the to-do list.
- At most 9 visible rows in the box; numbers only on rows 1–9.
- `cargo fmt`, clippy `-D warnings` and the full suite before every commit; ship per CLAUDE.md at the end.

## Review Focus

1. A finca that's unreachable must not lose its pins (only a *successful* poll that lacks the id prunes).
2. A pin whose session moved between tiles/rows must act on that session (look up by id, never by row index alone at action time).
3. The help screen must still fit at 110×32 (`every_help_line_shows_in_full_at_laptop_size`).
4. Number keys must not fire inside prompts, confirms, the grid or the to-do list.
5. Pins file corrupt or from the future: treated as no pins, never a crash.

---

### Task 1: Session ids in status, and the pins store

**Files:**
- Modify: `src/status.rs` (SessionStatus gains `id`)
- Create: `src/pins.rs`
- Modify: `src/main.rs` (`mod pins;`)
- Test: in-file tests in both

**Interfaces:**
- Produces: `status::SessionStatus { id: Option<String>, .. }`; `pins::Pin { finca: Option<String>, id: String }`; `pins::load(&Paths) -> Vec<Pin>`; `pins::save(&Paths, &[Pin]) -> anyhow::Result<()>`.

- [ ] **Step 1: failing tests**

```rust
// status.rs tests
#[test]
fn status_sessions_carry_their_id_and_old_status_still_parses() {
    // build() on a store with one live session: its id is in the JSON.
    // A status JSON without "id" parses with id: None.
}
// pins.rs tests
#[test]
fn pins_round_trip_and_bad_files_are_empty() {
    let d = tempfile::tempdir().unwrap();
    let paths = Paths::new(d.path().to_path_buf());
    assert!(load(&paths).is_empty());
    let pins = vec![Pin { finca: None, id: "a".into() }, Pin { finca: Some("pi".into()), id: "b".into() }];
    save(&paths, &pins).unwrap();
    assert_eq!(load(&paths), pins);
    std::fs::write(paths.home.join("pins.json"), "nope").unwrap();
    assert!(load(&paths).is_empty());
    std::fs::write(paths.home.join("pins.json"), r#"{"version":9,"pins":[]}"#).unwrap();
    assert!(load(&paths).is_empty());
}
```

- [ ] **Step 2:** run, expect compile failure.
- [ ] **Step 3:** implement: `#[serde(default, skip_serializing_if = "Option::is_none")] pub id: Option<String>` set from `s.id` in `build`; `pins.rs` with `PINS_VERSION = 1`, `{"version":1,"pins":[...]}`, `write_atomic`.
- [ ] **Step 4:** run, expect pass. **Step 5:** commit `feat(pins): session ids in status and the pins store`.

### Task 2: App state — pins, rows, pruning

**Files:** `src/ui/app.rs`, tests in `src/ui/tests.rs`

**Interfaces:**
- Consumes: Task 1.
- Produces:
  - `Pane::Pinned`; `App.pins: Vec<Pin>`, `App.pin_idx: usize`;
  - `PinRow { pin: Pin, finca: Option<(String /*slug*/, String /*name*/, Option<String> /*via*/)>, account: Option<Account>, handle: String, state: State, updated_at: DateTime<Utc>, message: Option<String>, window: Option<String>, unreachable: bool }`;
  - `App::pinned_rows(&self) -> Vec<PinRow>`;
  - `App::prune_pins(&mut self) -> bool` (true when it removed any);
  - `App::is_pinned(&self, finca: Option<&str>, id: &str) -> bool`;
  - `Action::SavePins { pins: Vec<Pin> }`.
- `reload` calls `pins::load`, then `prune_pins`, and saves if it pruned.

Pruning rules: local pin kept iff a session with that id is live (`state_is_live`); finca pin dropped if the finca row is gone, or its `reach` is `Reachable(st)` and no session in `st` has that id; otherwise kept. A finca pin's row comes from `last_status` (by id); `unreachable` = reach isn't `Reachable`. A kept finca pin missing from `last_status` (no poll yet) shows state `Starting`, handle `?`.

- [ ] Tests: prune drops ended/missing local; drops finca session absent from a good poll; keeps it when the finca is unreachable (marked unreachable); drops pins of a removed finca; rows carry account for local and finca name/via for remote.
- [ ] Implement, pass, commit `feat(pins): pinned rows and pruning`.

### Task 3: Keys

**Files:** `src/ui/input.rs`, `src/ui/run.rs` (perform SavePins), `src/ui/app.rs` (Mode::Confirm gains `finca: Option<String>`), tests in `src/ui/tests.rs`, `src/ui/run_tests.rs`

**Interfaces:**
- Consumes: Task 2.
- Produces: `pinned_key`, number-key handling; `Mode::Confirm { session_id, label, finca: Option<String> }` — y on a finca confirm returns `Action::GridKill { finca: Some((slug, via)), .. }`.

Behaviour:
- `p`: Sessions pane (live local session) / Fincas pane on a `FincaItem::Session` with an id / Pinned pane → toggle; footer `Pinned <handle> (<n>)` or `Unpinned <handle>`; finca session without id → `update Grove on <finca> to pin its sessions`; returns `SavePins`.
- `1`–`9` in Normal mode, any pane but To-do → the Nth row: local → `Jump`, finca → `FincaJump`; unreachable → `<finca> isn't answering`; none → `no pin <n>`.
- Pinned pane: `j`/`k` move (j at bottom → Sessions, pane `Sessions`), `Enter` jump as above, `x` → `Mode::Confirm` with finca, `p` unpin.
- `k` at top of Sessions (sess_idx 0, local project) with pins → Pinned.
- Tab cycle: Projects → Pinned (if any) → Sessions → To-do.
- Pinned pane with no pins (all pruned) falls back to Sessions.

- [ ] Tests for each bullet (keys produce the right Action/mode/footer), plus perform(SavePins) writes the file.
- [ ] Implement, pass, commit `feat(pins): p to pin, 1–9 to jump, the Pinned pane`.

### Task 4: Render, help, README

**Files:** `src/ui/render.rs`, `src/ui/help.rs`, `README.md`, tests in `src/ui/tests.rs`

- Right column: when pins exist, `Layout::vertical([Length(min(rows, 9) + 2), Fill(1)])`; Pinned on top, the existing Sessions/To-do/finca-detail split below.
- Row: `{n}  {icon} {origin} · {handle}  {state:<10} {age}  {message}`; origin = account name in its colour or finca name in FINCAS_COLOR; unreachable rows dark grey with `unreachable` in place of state.
- `pane_block("PINNED", app.pane == Pane::Pinned)`; highlight the `pin_idx` row when focused.
- Help: a Pinned section (`p` pin/unpin, `1–9` jump to a pin); keep it fitting at 110×32.
- README: "### Pinned sessions" and key-table rows.

- [ ] Tests: render with a play, work and finca pin shows origins and numbers; unreachable row says `unreachable`; no box without pins; help fits.
- [ ] Implement, pass, commit `feat(pins): the Pinned box, help and README`.
- [ ] Ship per CLAUDE.md: release, install, deploy-fincas (fincas need the status `id`).
