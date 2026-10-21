# Grove statusline (swagger Part A) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every Claude session Grove launches shows Grove's status line. Each run records the session's stats, and the dashboard shows them per session. Per-account daily harvest data is stored for Part B to display.

**Architecture:** A hidden `grove statusline` subcommand reads Claude Code's status JSON, prints one coloured line, and records stats. Each Grove session gets one stats file holding its last snapshot plus what it accrued per local day and per account. Only that session's status line writes the file, so nothing races. The harvest is computed by summing those files. `run/hooks.json` gains a `statusLine` entry unless the user opts out.

**Tech Stack:** Rust, serde_json, chrono (`Local` for day keys), ratatui.

**Spec:** `docs/superpowers/specs/2026-10-20-grove-swagger-design.md` (Part A and "Usable by anyone").

## Global Constraints

- `grove statusline` never fails visibly: panics are swallowed, a stats write error is logged with `log::error(paths, "statusline", …)`, and the line still prints.
- No `jq`. No subprocess except `git`. No macOS-only calls (fincas run Linux, including a Pi Zero).
- Every status JSON field is optional. A missing field drops its segment.
- Nothing personal in code, tests or docs. Tests use generic accounts `work` / `personal`.
- Grove never touches `~/.claude/settings.json`.
- `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` green before each commit.
- Ship per CLAUDE.md: `scripts/release.sh <next patch>`, `cargo install --path .`, `scripts/deploy-fincas.sh`.

## Review Focus

1. **`/clear` mid-session:** the cost goes from $3.00 to $0.20. Today's tally must gain $0.20, not lose $2.80 or gain nothing. (Task 1 test `reset_counts_new_value`.)
2. **Session not launched by Grove** (`claude` run by hand, no `GROVE_SESSION`): print the line using the folder basename and no account segment, and write no stats file. (Task 2 test `non_grove_session_prints_and_records_nothing`.)
3. **Malformed or empty stdin:** print something, never an error. (Task 2 test `garbage_input_still_prints_a_line`.)
4. **Midnight crossing:** a snapshot from yesterday followed by one today credits only the increase to today. (Task 1 test `increase_lands_on_todays_date`.)
5. **Ended session deleted:** its stats file goes too, so harvests don't count ghosts forever. Stats files with no live session still count for their days. (Task 4 test `delete_session_removes_stats`.)

---

### Task 1: Stats model and recording (`src/stats.rs`)

**Files:**
- Create: `src/stats.rs` (tests in a `#[cfg(test)] mod tests` at the bottom)
- Modify: `src/config.rs` (`Paths` gets `pub stats: PathBuf` = `home.join("stats")`, created by `ensure()`)
- Modify: `src/main.rs` (`mod stats;`)

**Interfaces:**
- Produces:
  ```rust
  /// The subset of Claude Code's status JSON Grove reads; every field optional.
  #[derive(Deserialize, Default)] pub struct StatusInput {
      pub session_id: Option<String>, pub model: Option<Model>, pub workspace: Option<Workspace>,
      pub cost: Option<Cost>, pub context_window: Option<ContextWindow>,
      pub rate_limits: Option<RateLimits>, pub agent: Option<Agent>,
  }
  // Model{display_name, id}, Workspace{project_dir, current_dir}, Cost{total_cost_usd: Option<f64>,
  // total_duration_ms: Option<u64>, total_lines_added: Option<u64>, total_lines_removed: Option<u64>},
  // ContextWindow{used_percentage: Option<f64>}, RateLimits{five_hour: Option<Window>},
  // Window{used_percentage: Option<f64>}, Agent{name: Option<String>}

  #[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
  pub struct Totals { pub cost: f64, pub added: u64, pub removed: u64 }

  #[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
  pub struct SessionStats {
      pub account: String,
      pub last: Totals,                      // the conversation's running totals as last seen
      pub context_pct: Option<f64>,
      pub five_hour_pct: Option<f64>,
      pub updated_at: DateTime<Utc>,
      pub days: BTreeMap<String, Totals>,    // "YYYY-MM-DD" local → accrued that day
  }

  pub fn accrue(prev: Option<&SessionStats>, account: &str, input: &StatusInput,
                today: &str, now: DateTime<Utc>) -> SessionStats;
  pub fn record(paths: &Paths, key: &str, account: &str, input: &StatusInput) -> Result<SessionStats>;
  pub fn load_all(paths: &Paths) -> HashMap<String, SessionStats>;   // key → stats; bad files skipped
  pub fn remove(paths: &Paths, key: &str);                           // NotFound ignored
  #[derive(Debug, PartialEq)] pub struct Harvest { pub account: String, pub totals: Totals, pub sessions: usize }
  pub fn harvest(all: &HashMap<String, SessionStats>, day: &str) -> Vec<Harvest>; // sorted by account name
  ```
  The file is `paths.stats.join(format!("{key}.json"))`, written with `store::write_atomic`.

**`accrue` algorithm** (per field: cost, added, removed):
```
delta = if cur >= prev_last { cur - prev_last } else { cur }   // a drop means a new conversation
```
A field missing from the input counts as unchanged: delta 0, `last` keeps the old value. Add the deltas to `days[today]`, creating the entry only when any delta > 0. Drop `days` entries more than 30 days older than `today`. `context_pct` and `five_hour_pct` take the input's value when present, else keep the old one.

- [ ] **Step 1: Write the failing tests** in `src/stats.rs`, using a helper `input(cost, added, removed)` that builds `StatusInput` from `serde_json::json!`:
  - `first_snapshot_accrues_everything`: `accrue(None, "work", input(1.5,10,2), "2026-10-20", now)` → `days["2026-10-20"] == Totals{cost:1.5,added:10,removed:2}`, `last` same, `account == "work"`.
  - `increase_lands_on_todays_date`: prev with `last` = (1.0, 5, 0) and `days{"2026-10-19": (1.0,5,0)}`, input (1.4, 9, 1), today "2026-10-20" → `days["2026-10-19"]` unchanged, `days["2026-10-20"]` ≈ (0.4, 4, 1) (compare cost with `(a-b).abs() < 1e-9`).
  - `reset_counts_new_value`: prev last (3.0, 100, 0), input (0.2, 3, 0) → today's entry = (0.2, 3, 0), `last` = (0.2, 3, 0).
  - `no_change_adds_no_day`: prev last (1.0,5,0), same input → `days` has no "2026-10-20" key.
  - `missing_fields_change_nothing`: prev last (1.0,5,0), input `{}` → `last` unchanged, no new day.
  - `old_days_pruned`: prev with a "2026-09-01" entry, today "2026-10-20" → the entry is gone.
  - `harvest_splits_by_account`: three stats for `work`, `work`, `personal` with today entries (1,10,0), (2,0,5), (0.5,1,1), plus one `work` with only yesterday → `[Harvest{account:"personal", totals:(0.5,1,1), sessions:1}, Harvest{account:"work", totals:(3,10,5), sessions:2}]`.
  - `record_round_trips`: tempdir `Paths`. `record(&p, "abc", "work", &input(1.0,1,0))`, then `load_all(&p)["abc"].last.cost == 1.0`. Writing a garbage `stats/bad.json` doesn't break `load_all`.
  - `parses_real_status_json`: deserialize the docs' example payload (session_id, model.display_name "Opus", cost.*, context_window.used_percentage 8, rate_limits.five_hour.used_percentage 23.5, extra unknown fields) → the fields come through. Also `{}` parses.

- [ ] **Step 2: Run** `cargo test --bin grove stats::` and expect compile failure: the module doesn't exist yet.
- [ ] **Step 3: Implement** the interfaces above. `record` reads the existing file (missing or unparseable → `None`), calls `accrue` with `chrono::Local::now().format("%Y-%m-%d")` and `Utc::now()`, then writes it.
- [ ] **Step 4: Run** `cargo test --bin grove stats::` and `cargo test --bin grove config::`. Expect PASS.
- [ ] **Step 5: Commit** `feat(stats): per-session stats with per-day, per-account tallies`.

---

### Task 2: `grove statusline` (`src/statusline.rs`)

**Files:**
- Create: `src/statusline.rs` (tests at the bottom)
- Modify: `src/main.rs`: a hidden `Statusline` subcommand next to `Hook`, run like `run_hook` (panic hook swallowed, read stdin, print, exit 0)

**Interfaces:**
- Consumes: `stats::{StatusInput, record}`, `Store::get_session`, `config::load_config`, `Config::account(Some(name))`
- Produces:
  ```rust
  /// What the line shows beyond Claude's JSON. None fields drop their segment.
  pub struct Context { pub account: Option<(String, Option<String>)>,  // name, "#rrggbb"
                       pub place: String,                               // "project:session" or folder basename
                       pub git: Option<GitInfo> }
  pub struct GitInfo { pub branch: String, pub staged: usize, pub modified: usize }
  pub fn render(input: &StatusInput, ctx: &Context) -> String;
  pub fn run(paths: &Paths, stdin: &str) -> String;   // parse, look up, record, render; never panics on bad input
  ```

**Segments and colours.** Copy the user's script: segments are joined with `" \x1b[90m|\x1b[0m "`, in this order, each omitted when its data is missing:
1. `🤖 \x1b[36m{model.display_name or model.id}\x1b[0m`
2. account: `\x1b[1;38;2;R;G;Bm{NAME uppercased}\x1b[0m` using the account's hex colour. With no colour, plain bold green.
3. agent: `\x1b[90m[\x1b[33m{name}\x1b[90m]\x1b[0m`, joined to the previous segment by a space, not a `|`
4. `📁 \x1b[1m{place}\x1b[0m`
5. `🌿 {branch}`, plus ` \x1b[32m+{staged}\x1b[0m` when staged > 0 and ` \x1b[33m~{modified}\x1b[0m` when modified > 0
6. `🧠 {bar} {color}{pct}%\x1b[0m`: 10 cells, `▓` filled in the colour and `░` empty in dim. `pct = used_percentage` rounded down and clamped to 0..=100. Green below 70, yellow 70–89, red 90+. Missing `used_percentage` shows 0%.
7. `💰 $X.XX`
8. `⏱️ {m}m {s}s`, or `{s}s` under a minute
9. `✏️ \x1b[32m+{added}\x1b[0m \x1b[31m-{removed}\x1b[0m`
10. `📊 {five_hour rounded}%`, only when present

**`run` lookup:**
- `key` = `$GROVE_SESSION` if `store.get_session` finds it, else the payload's `session_id` if that finds a session, else no Grove session.
- For a Grove session: place = `"{session.project}:{session.name}"`; the account name is `$GROVE_ACCOUNT` (or the default account's name); the colour comes from `load_config(paths)` → `cfg.account(Some(name))`. A config error means no colour. Then call `stats::record(paths, &session.id, &account_name, &input)`; on error, `log::error` and carry on.
- Otherwise: place = basename of `workspace.project_dir`, else of `current_dir`, else `"?"`. No account, nothing recorded.
- Git: `git_info(dir, &paths.run.join("git-cache"))` caches `branch:staged:modified` for 5s per dir, keyed by a hash of the path (`std::hash::DefaultHasher`) and using file mtime. The commands match the script: `git -C dir rev-parse --is-inside-work-tree`, `symbolic-ref --short HEAD`, falling back to `rev-parse --short HEAD`, then `diff --cached --numstat` and `diff --numstat` line counts.

- [ ] **Step 1: Write the failing tests.** Strip ANSI in assertions with a test helper, `fn plain(s:&str)->String` (a regex-free loop that drops `\x1b[`…`m`).
  - `renders_every_segment`: full input (Opus, cost 1.234, 125_000 ms, +10 −2, ctx 23.0, five_hour 12.4), ctx with account `("work", Some("#ff8800"))`, place `"api:s1"`, git `main` +1 ~2 → `plain(..) == "🤖 Opus | WORK | 📁 api:s1 | 🌿 main +1 ~2 | 🧠 ▓▓░░░░░░░░ 23% | 💰 $1.23 | ⏱️ 2m 5s | ✏️ +10 -2 | 📊 12%"`. The raw string contains `38;2;255;136;0`.
  - `missing_fields_drop_segments`: input `{}`, ctx with no account, place `"x"`, no git → `plain == "📁 x | 🧠 ░░░░░░░░░░ 0% | 💰 $0.00 | ⏱️ 0s | ✏️ +0 -0"`.
  - `context_colours`: 69 → contains `\x1b[32m`, 70 → `\x1b[33m`, 95 → `\x1b[31m`. 150 renders as 100%.
  - `garbage_input_still_prints_a_line`: `run(&paths, "not json")` is non-empty and contains `📁`.
  - `non_grove_session_prints_and_records_nothing`: with `GROVE_SESSION` unset (use a tempdir `Paths` with no sessions; read the env var through a parameter, see Step 3), payload with `workspace.project_dir "/tmp/foo"` → contains `📁 foo`, and `paths.stats` is empty or missing.
  - `grove_session_records_and_names_window`: save a `Session{id:"g1", project:"api", name:"s2", ..}` in a tempdir store, run with grove_session `Some("g1")` and account `Some("work")` → contains `api:s2` and `WORK`, and `load_all(&paths)` has key `"g1"`.

- [ ] **Step 2: Run** `cargo test --bin grove statusline::` and expect compile failure.
- [ ] **Step 3: Implement.** To keep tests off the process environment, `run` delegates to `fn run_with(paths, stdin, grove_session: Option<String>, grove_account: Option<String>) -> String`, which the tests call. `run` reads the two env vars. Wire up `Cmd::Statusline` (hidden) in main.rs.
- [ ] **Step 4: Run** `cargo test --bin grove statusline::` and expect PASS. Then smoke test: `echo '{"model":{"display_name":"Opus"},"cost":{"total_cost_usd":0.5}}' | cargo run -q -- statusline` prints a coloured line.
- [ ] **Step 5: Commit** `feat(statusline): grove statusline, ported from a shell script`.

---

### Task 3: Sessions get the status line, with an opt-out

**Files:**
- Modify: `src/config.rs`: `Config.statusline: bool`, default `true`, with a doc comment like the neighbouring fields; the `Default` impl gets it too
- Modify: `src/runfiles.rs`: `hooks_json(grove_bin: &Path, statusline: bool)`, `write_all(paths, grove_bin, session, statusline: bool)`
- Modify: `src/main.rs:395`: pass `cfg.statusline`

**Interfaces:**
- Produces: when `statusline` is true, hooks.json gets a top-level `"statusLine": {"type":"command","command":"<grove exe> statusline","padding":2}`. The exe path is quoted when it contains spaces, since `statusLine.command` is a shell string, unlike hooks' exec form: wrap it in single quotes with `'` escaped.

- [ ] **Step 1: Failing tests** in runfiles.rs:
  - `hooks_json_sets_the_status_line`: `hooks_json(Path::new("/Apps/My Tools/grove"), true)["statusLine"]` → `type == "command"`, `command == "'/Apps/My Tools/grove' statusline"`, `padding == 2`.
  - `status_line_opt_out`: `hooks_json(.., false)` has no `statusLine` key.
  - Update `hooks_json_registers_every_event_in_exec_form` and `write_all_writes_valid_files_under_spaced_home` to the new signatures.
  - config.rs: `statusline_defaults_on`: `Config::default().statusline`, and `toml::from_str::<Config>("statusline = false")` gives false.
- [ ] **Step 2: Run** `cargo test --bin grove runfiles:: config::` and expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the same tests and expect PASS.
- [ ] **Step 5: Commit** `feat(statusline): Grove sessions use it; statusline = false opts out`.

---

### Task 4: Stats in the dashboard, cleaned up with their session

**Files:**
- Modify: `src/store.rs`: `delete_session` also calls `stats::remove(&self.paths, id)`
- Modify: `src/ui/app.rs`: `pub stats: HashMap<String, SessionStats>`, refreshed in `reload` with `stats::load_all`
- Modify: `src/ui/render.rs` `render_sessions`: after the age span, a dim ` · $1.23 · 23%` when `app.stats` has the session (cost from `last.cost` with 2 decimals, context % rounded down; omit the % if None). Never on shell rows.
- Modify: `src/ui/grid.rs`: if the grid's per-session title line has room, append the same compact `$1.23 · 23%`; if it doesn't fit, skip it. Builder's call.

**Interfaces:**
- Consumes: `stats::{load_all, remove, SessionStats}`

- [ ] **Step 1: Failing tests:**
  - store_tests.rs `delete_session_removes_stats`: record stats for `"s1"`, `delete_session("s1")` → `load_all` lacks `"s1"`.
  - ui/tests.rs `session_row_shows_cost_and_context`: an app with one session `s1` and `stats["s1"]` with last.cost 1.234 and context 23.0. Render at 120×30 (use the file's existing render helper) → output contains `$1.23 · 23%`. Without stats, no `$`.
- [ ] **Step 2: Run** `cargo test --bin grove delete_session_removes_stats session_row_shows` and expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the full `cargo test` and expect all PASS. Fix any snapshot tests whose rows legitimately changed.
- [ ] **Step 5: Commit** `feat(dashboard): per-session cost and context from the status line`.

---

### Task 5: README, then ship

**Files:**
- Modify: `README.md`: a short "Status line" section after the session-states paragraph covering:
  - what the line shows
  - that inside Grove it replaces your own status line, without touching your Claude settings
  - `statusline = false` in `~/.grove/config.toml` brings yours back
  - cost is Claude Code's estimate, and on subscription accounts it isn't a bill
  - the dashboard shows each session's cost and context

  Use generic examples (`work`). Also add `stats/` to the GROVE_HOME layout listing and `src/stats.rs` and `src/statusline.rs` to the source table.

- [ ] **Step 1:** Write the README changes.
- [ ] **Step 2:** `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test`, all green.
- [ ] **Step 3:** Commit `docs: status line and stats`.
- [ ] **Step 4:** `scripts/release.sh <next patch>`, `cargo install --path .`, `scripts/deploy-fincas.sh`. Record the version and each finca's version.
- [ ] **Step 5:** Live check: press `R` in the dashboard, start a session, and confirm the Grove line shows and the row gains `$… · …%`.
