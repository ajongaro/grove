# Grove Fincas Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remote machines ("fincas"), each running its own Grove, are visible from the local dashboard (polled status) and reachable with Enter and `C-q` back.

**Architecture:** New `status.rs` (the JSON document every Grove can print), `finca.rs` (finca files, ssh probe, poller, connect). The store gains a `fincas/` dir. The UI gets a second list in the left pane, a read-only finca detail view and a poller channel. Setup of homebox and pi is operational (Task 6).

**Tech Stack:** as v1 (Rust 2024, ratatui 0.30, rmcp 3.5, tmux ≥ 3.2), plus `std::process` + threads for polling (no async).

**Spec:** `docs/superpowers/specs/2026-10-07-grove-fincas-design.md`

> **Execution note:** The user approved the spec and said "go ahead". Executed natively in-session like v1, with one fresh whole-branch review at the end.

## Global Constraints

- Every ssh call uses `GROVE_SSH` (default `ssh`) plus `-o BatchMode=yes -o ConnectTimeout=5`, except the interactive connect (`-t`, no BatchMode).
- Polling never blocks the UI thread; each poll is killed after 10s.
- Finca files go through `store.rs` (atomic writes); slug = `slugify(name)` and must match the file stem.
- The `grove-finca` key table holds only `C-q`; finca sessions set `key-table grove-finca` and `prefix None`.
- `grove status --json` never contacts anything; it reads the local store only.
- Builds use `DEVELOPER_DIR=/Library/Developer/CommandLineTools SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk`.

## Review Focus

1. **SSH destinations and paths with spaces or shell metacharacters** in the connect wrapper (`sh -c`): quoting must hold. Test in Task 4.
2. **A finca hanging mid-poll** (TCP open, no reply): must be killed at 10s and shown unreachable, not freeze the poller. Test in Task 3.
3. **A finca running an older Grove** (no `status`) or a newer one (unknown fields / higher version): must show "needs update" or parse leniently. Test in Tasks 1 and 3.
4. **Selection across two lists when one is empty or changes size**: no panics, sensible selection. Test in Task 5.
5. **Connecting twice / after the remote exited**: no duplicate sessions, no stale `finca-*` sessions. Test in Task 4.

---

### Task 1: `Status` document + `grove status --json`
**Files:** create `src/status.rs`; modify `src/main.rs`.
**Produces:**
- `struct Status { version: u32, host: String, generated_at, needs_you: usize, live: usize, projects: Vec<ProjectStatus> }`
- `struct ProjectStatus { name, slug, sessions: Vec<SessionStatus> }`
- `struct SessionStatus { name, state: State, message: Option<String>, updated_at }`
- `const STATUS_VERSION: u32 = 1`
- `fn build(store: &Store, host: &str) -> Status`
- `fn parse(json: &str) -> Result<Status, ParseError>` where `ParseError::{Invalid, WrongVersion(u32)}`

**Tests:** round trip; ended sessions omitted; projects without sessions present; unknown fields tolerated; wrong version detected; binary test for `grove status --json` on a temp home.

### Task 2: Finca files + `grove finca add|rm|ls`
**Files:** create `src/finca.rs` (model + CRUD + probe); modify `config.rs` (`Paths.fincas`), `store.rs` (finca CRUD), `main.rs`.
**Produces:**
- `struct Finca { name, ssh, grove_bin, color: Option<String> }` with `slug()`
- `Store::{list_fincas, get_finca, add_finca, remove_finca}`
- `fn ssh_program() -> String`
- `fn probe(dest, bin: Option<&str>) -> Result<String /*abs grove path*/>`

**Tests:** CRUD; mismatch error row; probe with a fake `GROVE_SSH` script (success / ssh fail / no grove); CLI `finca add` writes only on success; `finca rm`; `finca ls`.

### Task 3: Poller
**Files:** `src/finca.rs`.
**Produces:**
- `enum Reach { Reachable(Status), Unreachable { error: String }, NeedsUpdate }`
- `fn poll_one(f: &Finca, timeout: Duration) -> Reach`
- `struct Poller` spawning a thread that sends `(slug, Reach, DateTime)` on a channel every 30s, and immediately on `kick()`.

**Tests:** fake ssh prints status JSON → Reachable; exits 255 → Unreachable; prints usage error → NeedsUpdate; sleeps 30 → Unreachable within ~timeout; poller delivers results for all fincas.

### Task 4: Connect + `C-q`
**Files:** `src/finca.rs`, `src/tmux.rs` (`set_option`, `switch_client`, `kill_session`), `src/runfiles.rs` (tmux.conf binding takes the session name).
**Produces:**
- `fn connect_argv(f: &Finca) -> Vec<String>` (the sh wrapper)
- `fn connect(tmux, f, grove_session) -> Result<()>`
- `fn disconnect(tmux, slug)`
- `runfiles::tmux_conf(session: &str)`

**Tests:** quoting with spaces / `'` in dest and bin; connect creates the session with key-table and prefix None; a second connect reuses it; `list-keys -T grove-finca` shows `C-q` → `switch-client -t =grove`; `finca rm` kills the session.

### Task 5: Dashboard
**Files:** `src/ui/app.rs`, `input.rs`, `render.rs`, `run.rs`.
**Produces:**
- `App.fincas: Vec<FincaRow>`, where `FincaRow { slug, name, color, reach: Option<Reach>, last_ok, status: Option<Status>, error }`
- unified left-pane selection (`Selection::Project(i) | Selection::Finca(i)`)
- `Action::ConnectFinca { slug }`
- footer sum

**Tests:** render reachable / unreachable / needs-update rows; detail view groups by project; footer `(2 on homebox)`; j/k crosses the lists; empty projects + fincas; session keys disabled with a footer hint; Enter → ConnectFinca.

### Task 6: Machine setup, deploy script, live E2E
**Files:** create `scripts/deploy-fincas.sh`; update `README.md`.
**Steps:**
- Install tmux on both machines.
- Read the Claude Code install docs, install on the Pi, and have the user log in.
- Install rustup on the Pi, deploy, and write each finca's `~/.grove/config.toml`.
- `grove finca add` both machines.
- Live checks: status shows, connect / `C-q` works, needs-you on homebox shows ⚑ locally.
