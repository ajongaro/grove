# Grove Fincas (remote machines) — Design

**Date:** 2026-10-07
**Status:** Approved in conversation, pending written review
**Builds on:** `2026-10-06-grove-v1-design.md` (branch `build/fincas`, off `build/v1`)

## Purpose

A **finca** is another machine running its own complete Grove: Yeehaw's
"barns". From the laptop's Grove, the user can see every finca's sessions and
which ones need attention, and can jump into a finca's dashboard and back
with one key. Nothing is synced or launched remotely: each finca owns its
projects and sessions and keeps working when the laptop is asleep or away.

First fincas: **homebox** (macOS arm64, `me@homebox.local`) and
**pi** (Debian 13 aarch64, `me@pi.local`).

### Success criteria

1. `grove finca add <name> <ssh-dest>` registers a machine only after
   verifying non-interactive SSH works and Grove is installed there.
2. The local dashboard lists fincas with live/needs-you counts refreshed every
   30s without ever blocking the UI. An unreachable finca shows grey, with how
   long ago it was last reachable.
3. Enter on a finca lands in that finca's own Grove dashboard. All keys go to
   the remote, and `C-q` returns to the local dashboard. The connection stays
   open for instant re-entry.
4. Both fincas run Grove and Claude Code on the user's personal account, with
   tmux installed.

### Out of scope

Starting or killing remote sessions from the local dashboard (do it inside
the finca), syncing projects or wikis between machines, desktop notifications,
non-SSH transports, fincas of fincas.

## Data

`~/.grove/fincas/<slug>.toml` (written atomically by `store.rs` like
everything else):

```toml
name = "homebox"
ssh = "me@homebox.local"        # anything `ssh` accepts, incl. ~/.ssh/config aliases
grove_bin = "/Users/homebox/.local/bin/grove"   # absolute; non-interactive ssh has no profile PATH
color = "#5c6bc0"                    # optional
```

Slug and uniqueness rules are the same as projects (`slugify(name)`, name must
match file stem).

## Commands

| Command | Behavior |
|---|---|
| `grove finca add <name> <ssh-dest> [--bin PATH]` | Probes `ssh -o BatchMode=yes -o ConnectTimeout=5 <dest> '<probe>'`. The probe returns the first of `--bin`, `~/.local/bin/grove`, `~/.cargo/bin/grove`, `/opt/homebrew/bin/grove`, `/usr/local/bin/grove` that runs `--version`. Saves the file with that absolute path. Errors without saving if SSH fails (mentions setting up keys) or no Grove is found. |
| `grove finca rm <name>` | Deletes the file; closes the `finca-<slug>` tmux session if open. |
| `grove finca ls` | Prints each finca and its last-known reachability. |
| `grove status --json` | Every Grove. Prints a `Status` document (below) and exits 0. Reads the local store only, never contacts anything. |

`GROVE_SSH` (default `ssh`) overrides the ssh program, for tests.

### `Status` document (`src/status.rs`)

```json
{
  "version": 1,
  "host": "homebox",
  "generated_at": "RFC3339",
  "needs_you": 2,
  "live": 3,
  "projects": [
    { "name": "btt", "slug": "btt", "sessions": [
      { "name": "s1", "state": "needs_you", "message": "Permission needed: Bash",
        "updated_at": "RFC3339" } ] }
  ]
}
```

`live` counts non-ended sessions. Ended sessions are omitted. Projects with no
live sessions are included with empty `sessions`. Unknown fields are ignored
when parsing (forward compatible); `version` lets a future change be detected.

## Connecting (`C-q` to return)

Enter on a finca, from the dashboard (`ui/run.rs` → `finca::connect`):

1. If a local tmux session `finca-<slug>` exists on Grove's server, switch the
   client to it.
2. Otherwise create it (detached) with one window named `<name>` running
   `[GROVE_SSH, "-t", <dest>, <grove_bin>]` wrapped so a non-zero exit pauses
   on a message: `sh -c '<ssh …> || { echo; echo "ssh exited ($?) — press Enter"; read _; }'`
   with all values shell-quoted. Then:
   - `set-option -t =finca-<slug> key-table grove-finca`
   - `set-option -t =finca-<slug> prefix None`
   - window status style from the finca color
   - switch the client to it.
3. Grove's `run/tmux.conf` adds `bind-key -T grove-finca C-q switch-client -t =grove`
   (with the configured session name substituted). That table holds only
   this binding, so every other key, including `C-y`/`C-h`/`C-l`, reaches the
   remote Grove.

When ssh ends (the remote detach `q`, or a dropped connection), the session's
only window closes and tmux returns the client to another session, normally
`grove`.

## Status polling (`src/finca.rs`)

- One background thread started by the dashboard. Every 30s (and immediately
  at start and when a finca file changes) it polls every finca in parallel:
  `[GROVE_SSH, -o BatchMode=yes, -o ConnectTimeout=5, <dest>, <grove_bin>, status, --json]`.
  It kills the process after 10s total.
- Results go over a channel into the UI as `FincaStatus { slug, result:
  Reachable(Status) | Unreachable { error, last_ok: Option<DateTime> } |
  NeedsUpdate }`. `NeedsUpdate` means the command ran but `status` is unknown
  or the JSON has the wrong `version`.
- The UI keeps the last `Status` per finca so the detail view still shows the
  last-known sessions (marked stale) while unreachable.

## Dashboard

- The left pane has two lists. Projects come first; then a **Fincas** header
  and one row per finca:
  - `● homebox  3 live  ⚑2` (finca color dot; ⚑ red when > 0)
  - `○ pi  unreachable · 4m` (grey; age since last success, or "never")
  - `◐ name  needs update` (yellow)
  `j`/`k` move through both lists as one.
- With a finca selected, the right pane title is `Finca — <name> (<host>)`. It
  lists its sessions grouped by project (project header line, then each session
  row like local ones, messages on their own line). It's read-only, with a
  "last updated Ns ago" footer when stale.
- Keys with a finca selected: Enter = connect. `x`, `n`, `e`, `w`, `c` do
  nothing, and the footer says to connect to the finca to do that. `a` still
  adds a local project.
- The footer's needs-you badge sums local and remote: `⚑ 3 needs you (2 on homebox)`.

## Machine setup (operational, done once by Claude)

| | homebox | pi |
|---|---|---|
| tmux | `brew install tmux` | `sudo apt install -y tmux` |
| Grove | laptop's macOS arm64 release binary → `~/.local/bin/grove` | rustup (minimal) + rsync source → `cargo build --release` on the Pi → `~/.local/bin/grove` |
| Claude Code | already installed, personal account in `~/.claude` | official installer (docs read first); **user logs in once** |
| `~/.grove/config.toml` | `claude_config_dir = "~/.claude"` | `claude_config_dir = "~/.claude"` |

Both fincas use `~/.claude` because that is the personal account on those
machines. The laptop keeps `~/.claude-play`.

`scripts/deploy-fincas.sh`:
- Builds the macOS release (with the CLT/SDK env workaround) and copies it to homebox.
- rsyncs the source to the Pi (excluding `target/`) and builds there.
- Then runs each finca's `grove --version`.

Grove's stale-dashboard check restarts each finca's dashboard the next time
it's opened.

## Errors

| Situation | Behavior |
|---|---|
| SSH needs a password / host key unknown | `add` fails with "set up key-based SSH (ssh-copy-id …) and connect once interactively"; poll shows unreachable |
| Finca's Grove predates `status` | `needs update` row; connect still works |
| Poll hangs | killed at 10s → unreachable |
| Finca file malformed / name mismatch | error row like projects |
| Remote dashboard exits / ssh drops while connected | finca session closes; non-zero exit pauses with the error first |

## Testing

- `status.rs`: build from a temp store → JSON → parse round trip. Ended
  sessions are omitted, unknown fields are tolerated, and a wrong `version`
  is detected.
- `finca.rs`: CRUD through the store. `add` runs against a fake `GROVE_SSH`
  script with three cases: success records the path; SSH failure and missing
  Grove each error without writing. Poll parsing, timeout → unreachable, and
  unknown subcommand → needs update.
- `connect`: on a throwaway tmux server with a fake ssh (`sleep`). The
  session is created with key-table `grove-finca` and prefix None, a second
  connect switches instead of duplicating, and `list-keys -T grove-finca`
  has `C-q`.
- UI: render snapshots for the Fincas section (reachable / unreachable /
  needs update), the finca detail view, the combined footer count, and
  navigation across both lists.
- Live: add both fincas, see real status, connect, and return with `C-q`. Start
  a session on homebox, trigger needs-you there, and see ⚑ appear locally
  within 30s.
