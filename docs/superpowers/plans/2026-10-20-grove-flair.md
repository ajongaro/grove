# Grove flair (swagger Part B) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Session states drawn as growth stages everywhere; a grove banner with drifting leaves and a per-account harvest line in the header; a quiet-grove scene; occasional footer quips.

**Architecture:** One `State::glyph()` in model.rs replaces every hard-coded ⚑/● state glyph. All animation and quip timing is a pure function of the `now` that `render` already receives, so nothing adds redraws (the loop already redraws every ≤250ms) and tests are deterministic. New art lives in a new `src/ui/flair.rs` so render.rs doesn't grow further.

**Tech Stack:** Rust, ratatui (`Span::width` / `Line::width` for display widths).

**Spec:** `docs/superpowers/specs/2026-10-20-grove-swagger-design.md` (Part B, "Usable by anyone"). Part A shipped in v0.1.11 (`stats::harvest`, `App.stats`).

## Global Constraints

- Glyphs: starting 🌱 · working 🌿 · your turn 🌳 · needs you 💧 · ended 🍂. Colours unchanged (`state_color`).
- Emoji are 2 columns wide. Width maths uses `Span::width()`/`Line::width()` or explicit 2-column glyph constants, never `chars().count()` on strings containing them.
- No new timers, threads, or redraws; animation advances only on existing redraws and is static when no session is working.
- The header still collapses below `HEADER_MIN_HEIGHT` (24 rows); it stays `HEADER_ROWS` = 3.
- Nothing personal: tests use `work`/`personal`; harvest colours come from config.
- `cargo fmt`, clippy `-D warnings`, full `cargo test` green; ship per CLAUDE.md.

## Review Focus

1. **Old ⚑ window names after upgrade:** the reconcile loop starts with an empty `flagged` map, so its first pass renames every live window via `window_title`. Expect no `⚑` left and no double mark. (Task 1 test `window_title_uses_drop_mark` plus an assertion that the mark is `"💧 "`.)
2. **Narrow dashboard (80 cols):** project rows with counts in all three slots still line up and names truncate with `…` instead of pushing counts off. (Task 1 test `project_counts_align_with_wide_glyphs`.)
3. **Harvest wider than the space right of the logo:** line counts drop before accounts do; never overlaps the wordmark. (Task 2 test `harvest_drops_line_counts_when_narrow`.)
4. **No stats at all (fresh install):** header shows the dim `🧺 nothing harvested yet today`, no panic, no `$0.00` clutter. (Task 2 test `harvest_empty_is_dim_placeholder`.)
5. **A message or needs-you alert in the footer:** the quip never shows over it. (Task 3 test `quip_never_covers_a_message`.)

---

### Task 1: Growth-stage glyphs everywhere

**Files:**
- Modify: `src/model.rs`: `impl State { pub fn glyph(self) -> &'static str }`
- Modify: `src/ui/render.rs`: project-row slots (`SLOT_W` 4 → 5, `format!("{glyph}{n:<w$}", w = SLOT_W - 2)`), session rows (line ~570), finca session rows (~491), finca reachable `⚑{n}` (~396), footer alert (~743), header totals (~834), pinned rows icon (~955)
- Modify: `src/ui/grid.rs`: top-line counts (~227) and tile title icon (~321)
- Modify: `src/ui/run.rs:34`: `NEEDS_YOU_MARK = "💧 "`
- Modify: `src/runfiles.rs`: `window-status-format` matches `💧 *`; comment; its test's window name
- Modify: `src/ui/help.rs`: a legend line `🌱 starting  🌿 working  🌳 your turn  💧 needs you  🍂 ended`
- Modify: `README.md`: the ⚑/● mentions (lines ~165, 168, 324)
- Test: `src/model.rs`, `src/ui/tests.rs`, `src/ui/run_tests.rs`, `src/runfiles.rs`

Project-colour dots (`● ` before project names), finca reach dots, shell dots and the olive `●` in the logo are **not** state glyphs and stay.

- [ ] **Step 1: Failing tests:**
  - model.rs `glyphs_are_growth_stages`: the five glyphs as in Global Constraints.
  - run_tests.rs: change the existing `"⚑ p:needy"`-style expectations to `"💧 p:…"`; add `window_title_uses_drop_mark`: a NeedsYou session's title is `"💧 p:a"`, a Working one's `"p:a"`.
  - ui/tests.rs `project_counts_align_with_wide_glyphs`: a project with 1 needs-you, 1 your-turn and 2 working sessions, rendered at 80×24 → its row contains `"💧1  🌳1  🌿2"` and ends at the pane border (the `│` follows the last slot's padding).
  - Update every existing ui/tests.rs assertion on state ⚑/● (e.g. `"⚑1  ●1  ●2"` → `"💧1  🌳1  🌿2"`, `"⚑1 needs you  ●2 your turn  ●1 working"` → `"💧1 needs you  🌳2 your turn  🌿1 working"`, `fg_of(.., "⚑1")` → `"💧1"`). Note: TestBackend renders a 2-wide symbol in one cell followed by an empty-string cell, so `screen()` joins them correctly.
  - runfiles.rs status-bar test: window `"💧 p:needy"` shown, `"p:quiet"` hidden.
- [ ] **Step 2: Run** `cargo test --bin grove` → the new and updated tests FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test` → all PASS; `grep -rn "⚑" src README.md` → no hits.
- [ ] **Step 5: Commit** `feat(ui): sessions as growth stages — 🌱 🌿 🌳 💧 🍂`.

---

### Task 2: Grove banner, drifting leaves, harvest line (`src/ui/flair.rs`)

**Files:**
- Create: `src/ui/flair.rs` (tests at the bottom; `mod flair;` in `src/ui/mod.rs`)
- Modify: `src/ui/render.rs` `render_header`: draws `flair::grove(...)` right of the branch on rows 0–1, leaves on row 2, totals/place on rows 0–1 right (as now), `flair::harvest_line(...)` on row 2 right
- Modify: `src/stats.rs`: drop the two `#[allow(dead_code)]`

**Interfaces:**
- Consumes: `stats::{harvest, Harvest, SessionStats}`, `App.stats`, `App.cfg` (`Config::account`, `Config.accounts`)
- Produces:
  ```rust
  /// A few small trees, (text, color) runs per row, 2 rows; drawn after the branch.
  pub fn grove() -> [Vec<(&'static str, Color)>; 2];
  /// Leaf glyphs on one row of `width` cells at time `now`; all spaces when !working.
  pub fn leaves(width: usize, now: DateTime<Utc>, working: bool) -> String;
  /// The harvest as styled spans fitting `max` columns.
  pub fn harvest_line(h: &[Harvest], cfg: &Config, max: usize) -> Line<'static>;
  pub fn short_count(n: u64) -> String;   // 340 → "340", 2100 → "2.1k", 12000 → "12k"
  ```
- Tree art: builder's call, same LEAF/STEM/OLIVE palette (move those consts to flair.rs, `pub(super)`), at most ~16 columns, 2 rows, half-block style to match the wordmark.
- Leaves: 1–2 leaf chars (e.g. `❦`, `❧`, `,`) in LEAF; x position = `(now.timestamp_millis() / 400) as usize % width`, second leaf offset by `width / 2` and a different phase; leaf char alternates with the step to "tumble".
- Harvest formats (spec):
  - one account (`cfg.accounts.is_empty()`): `🧺 $14.20 · ✏️ +2.1k −340 · 6 sessions today` (`1 session today` singular)
  - several: `🧺 ` then per account `{name} ${cost:.2} +{added} −{removed}` joined by ` · `, each name+numbers in the account colour (fallback default fg), accounts in `harvest` order
  - empty: `🧺 nothing harvested yet today`, dark gray
  - too wide for `max`: first drop the `+a −r` parts, then the ` · N sessions today` part; if still too wide, nothing (header totals are more important)
- "Today" = `chrono::Local::now().format("%Y-%m-%d")` computed in `render_header`.

- [ ] **Step 1: Failing tests** in flair.rs:
  - `short_counts`: 0→"0", 340→"340", 999→"999", 2100→"2.1k", 12_000→"12k", 1_500_000→"1.5M".
  - `leaves_still_when_idle`: `leaves(30, t, false)` is 30 spaces.
  - `leaves_drift`: `leaves(30, t, true)` and `leaves(30, t + 400ms, true)` differ, both have display width 30 and contain a non-space.
  - `harvest_one_account`: cfg default, one Harvest{account:"default", totals:(14.2, 2100, 340), sessions:6}, max 80 → text `"🧺 $14.20 · ✏️ +2.1k −340 · 6 sessions today"`.
  - `harvest_splits_accounts_in_colour`: cfg with `[accounts.work] color="#ff8800"` and default named `personal`; harvests personal (4.4,700,140,1) and work (9.8,1400,200,2) → text `"🧺 personal $4.40 +700 −140 · work $9.80 +1.4k −200"`; the `work` span's fg is `Color::Rgb(255,136,0)`.
  - `harvest_drops_line_counts_when_narrow`: same two accounts, max 36 → text `"🧺 personal $4.40 · work $9.80"`.
  - `harvest_empty_is_dim_placeholder`: `harvest_line(&[], ..)` text is `"🧺 nothing harvested yet today"`, fg DarkGray.
  - ui/tests.rs `header_shows_grove_and_harvest`: app with a stats file recorded today for a session; render 120×30 → row 2 contains `🧺 $`; row 0 still contains `needs you`.
- [ ] **Step 2: Run** `cargo test --bin grove flair:: header_shows` → FAIL (module missing).
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** full `cargo test` → PASS. Visual check: `cargo run -- ` is not usable here (needs tmux attach); instead print `screen(&app,120,30)` from the header test once with `--nocapture` and eyeball it.
- [ ] **Step 5: Commit** `feat(ui): a grove in the header, drifting leaves, the daily harvest`.

---

### Task 3: Quiet grove and footer quips

**Files:**
- Modify: `src/ui/flair.rs`: `pub fn quiet_grove() -> Vec<Line<'static>>` (≤6 rows, ≤34 cols, LEAF/STEM art plus `the grove is quiet` in dark gray), `pub const QUIPS: &[&str]` (15–25), `pub fn quip(now: DateTime<Utc>) -> Option<&'static str>`
- Modify: `src/ui/render.rs`: `render_sessions` empty branch: when **no live local session exists at all** (`app.sessions.iter().all(|s| s.state == State::Ended)`) and the pane is tall enough (inner height ≥ scene rows + 2), draw the scene then the existing hint; otherwise just the hint. `render_footer(frame, app, area, now)`: in the default-hints branch only (no `app.footer`, no needs-you alert, not in Todo pane), append the quip right-aligned in dark gray when hints + 2 + quip fit in `area.width`.

**Quip timing:** `quip(now)` is `Some(QUIPS[(now.timestamp() / 240) as usize % QUIPS.len()])` when `now.timestamp() % 240 < 25`, else `None` → at most one quip per 4 minutes, for 25 seconds.

**Quip tone:** dry, a bit odd, grove-flavoured, ≤ 40 chars each, at most one branching pun. Examples of the register: "the olives are listening", "photosynthesis: pending", "no trees were harmed in this commit", "roots: deep. context: deeper". Write the rest.

- [ ] **Step 1: Failing tests** in flair.rs:
  - `quip_shows_briefly_every_few_minutes`: at a timestamp ≡ 0 mod 240 → Some; ≡ 30 → None; two windows 240s apart give (generally) different quips — assert `QUIPS.len() >= 15` and all ≤ 40 chars.
  - ui/tests.rs `quiet_grove_when_nothing_runs`: one project, no sessions, 120×30 → contains `the grove is quiet` and the existing `press Enter to start one` hint.
  - ui/tests.rs `no_quiet_grove_when_another_project_is_busy`: project alpha selected with no sessions, beta has a Working session → no `the grove is quiet`.
  - ui/tests.rs `quip_never_covers_a_message`: `app.footer = Some("saved")`, render at a quip-window `now` (the test `screen` helper uses `t0()+2min`; add a `screen_at(app,w,h,now)` variant) → footer row contains `saved` and none of `QUIPS`.
  - ui/tests.rs `quip_in_idle_footer`: no footer message, quip-window time, 200 wide → footer row contains `flair::quip(now).unwrap()`.
- [ ] **Step 2: Run** those tests → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** full `cargo test` → PASS.
- [ ] **Step 5: Commit** `feat(ui): the quiet grove and the occasional quip`.

---

### Task 4: README, ship

- [ ] **Step 1:** README: the session-states paragraph uses the growth-stage glyphs and names them; a sentence on the header's harvest (per account, today, this machine's sessions; cost is an estimate); `src/ui/flair.rs` in the source table (`ui/` row).
- [ ] **Step 2:** `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` green.
- [ ] **Step 3:** Commit `docs: growth stages and the harvest`.
- [ ] **Step 4:** Final whole-branch review (fresh reviewer), fix pass.
- [ ] **Step 5:** `scripts/release.sh <next patch>`, `cargo install --path .`, `scripts/deploy-fincas.sh`.
