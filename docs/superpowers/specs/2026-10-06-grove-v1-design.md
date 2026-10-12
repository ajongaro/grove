# Grove v1 — Design

**Date:** 2026-10-06
**Status:** Approved in conversation, pending written review

## Purpose

Grove is a personal, stripped-down take on Yeehaw: a tmux-based dashboard for
running and supervising several Claude Code sessions across projects. The
goal is a smaller core its owner controls and can grow (v2 adds remote machines).
Claude builds v1 end to end; it must be structured so features can be added one
module at a time.

### Success criteria

1. `grove` opens (or re-attaches) a tmux session whose window 0 is a dashboard
   of projects and their Claude sessions.
2. Starting a session from the dashboard opens Claude Code in the project's
   directory, in its own tmux window, **always on the `~/.claude-play` account**.
3. The dashboard shows each session's state (working / needs you / your turn /
   ended) within ~1s of it changing, and flags the tmux window of any session
   that needs attention.
4. Inside a Grove session, Claude can read and write the project's summary and
   wiki through Grove's MCP server, and starts each session knowing which wiki
   sections exist.
5. Grove never alters `~/.claude*/settings.json` or registers anything globally,
   and a Grove bug can never block a Claude session.

### Out of scope (v1)

Remote machines ("barns" — planned for v2), password vault, Slack, scheduled
jobs/workflows (Yeehaw's trails/worms), issue-tracker integration, multiple
Claude accounts, sounds and desktop notifications, wiki deletion via MCP, MCP
resources/prompts.

## Architecture

One Rust binary, `grove`, with four modes sharing plain files under `~/.grove`
(overridable with `GROVE_HOME`, which tests use). No daemon.

| Command | Role |
|---|---|
| `grove` | Create/attach tmux session `grove`; window 0 runs the dashboard (`grove dashboard`, internal) |
| `grove hook` | Invoked by Claude Code hooks; updates one session's state file |
| `grove mcp` | stdio MCP server over projects + wiki |
| `grove add [path]` | Register a project from the CLI (same as `a` in the dashboard) |

Stack: Rust (edition 2024), `ratatui` 0.30 + `crossterm`, `rmcp` 3.x (feature
`server`, stdio transport), `tokio`, `serde`, `serde_json`, `toml`, `uuid`,
`notify`, `clap`, `anyhow`. Requires tmux ≥ 3.2 (developed on 3.6a).

## Storage (`~/.grove`)

```
config.toml               settings (below)
projects/<slug>.toml      name, path, color, summary
wiki/<slug>/<section>.md  one Markdown file per wiki section; title = first "# " heading
sessions/<uuid>.json      one per Claude session
run/hooks.json            generated settings file passed via --settings
run/mcp.json              generated MCP config passed via --mcp-config
grove.log                 errors from hook/mcp/dashboard
```

`config.toml` keys and defaults:

```toml
claude_config_dir = "~/.claude-play"   # always injected as CLAUDE_CONFIG_DIR
tmux_session = "grove"
editor = ""                            # empty → $EDITOR → vi
claude_args = []                       # extra args appended to every launch
```

Project file:

```toml
name = "streakwithfriends"
path = "/Users/me/code/some-app"
color = "#2E8B57"     # optional; used for window + dashboard accents
summary = ""          # optional; editable by MCP
```

Slugs are the lowercase name with non-alphanumerics → `-`. Project name is unique.

Session file:

```json
{
  "id": "uuid",
  "project": "slug",
  "name": "auth-fix",
  "host": "local",
  "tmux_window": "@12",
  "state": "needs_you",
  "message": "Claude needs your permission to use Bash",
  "created_at": "RFC3339",
  "updated_at": "RFC3339"
}
```

`host` is always `"local"` in v1; it exists so v2 remote sessions fit the same
file without a migration.

**All writes are atomic** (write `*.tmp` in the same dir, then rename) and all
go through `store.rs`.

## Session lifecycle and state

Grove generates the session UUID and passes it with `--session-id`, so the
session file (state `starting`) exists before the first hook fires. Hooks look
up `sessions/<session_id>.json` from the hook's stdin `session_id`; events for
unknown IDs are ignored (non-Grove or forked sessions).

| State | Set by hook event |
|---|---|
| `starting` | Created by the launcher |
| `working` | `UserPromptSubmit`, `PostToolUse`, `PostToolUseFailure` |
| `needs_you` | `PermissionRequest`; `Notification` with `notification_type` = `permission_prompt` |
| `your_turn` | `Stop`, `StopFailure`; `Notification` with `idle_prompt` |
| `ended` | `SessionEnd`; or dashboard finds the tmux window gone |

`SessionStart` does not change state; it prints the context block (see MCP).
The mapping lives in `state.rs` as a pure function
`fn next(current: State, event: &HookEvent) -> Option<(State, Option<String>)>`
(`None` = no change; the string is the message to store).

`grove hook` reads stdin JSON, applies the transition, writes atomically, and
**always exits 0** within milliseconds. Errors are appended to `grove.log`.

### Generated `run/hooks.json`

Rewritten by `grove` on every start (absolute path to the current binary):

```json
{
  "hooks": {
    "SessionStart":      [{ "hooks": [{ "type": "command", "command": "<grove>", "args": ["hook"] }] }],
    "UserPromptSubmit":  [ … same … ],
    "PostToolUse":       [ … ],
    "PostToolUseFailure":[ … ],
    "PermissionRequest": [ … ],
    "Notification":      [ … ],
    "Stop":              [ … ],
    "StopFailure":       [ … ],
    "SessionEnd":        [ … ]
  }
}
```

Exec form (`command` + `args`) so paths with spaces need no quoting.

## Launching a session

`launch.rs` builds, and `tmux.rs` runs:

```
tmux new-window -t grove -n "<project>:<name>" -c <project path> \
  -e CLAUDE_CONFIG_DIR=<expanded claude_config_dir> \
  -e GROVE_PROJECT=<slug> -e GROVE_SESSION=<uuid> \
  claude --session-id <uuid> --settings <run/hooks.json> \
         --mcp-config <run/mcp.json> -n <name> <claude_args…>
```

- `claude_config_dir` is expanded and must exist; if not, Grove refuses to
  launch and shows the error (never falls back to `~/.claude`).
- `-e` sets env on the window itself, so a tmux server started from a
  `claude-work` shell can't leak the wrong account.
- Default session name: `s<N>` (next free number for the project).
- Window colored from the project's `color` via `window-status-style`.

## tmux integration

- `grove` outside tmux: `tmux -f <grove tmux.conf> new-session -A -s grove`
  with window 0 running `grove dashboard`. Inside the `grove` session: switch to
  window 0. Inside another tmux session: `switch-client` to `grove`.
- Grove's tmux.conf (written to `run/tmux.conf`): mouse on, history 50000,
  `C-y` → window 0, `C-h`/`C-l` → previous/next window, status bar styled
  "GROVE" (deep green). Only applies to the `grove` tmux server/session.
- Attention flag: when a session enters `needs_you`, the dashboard sets that
  window's status style to highlight (and clears it on leaving the state).
- `tmux.rs` is a thin wrapper over the `tmux` CLI (`new-window`,
  `list-windows -F`, `select-window`, `kill-window`, `set-window-option`). It
  takes the server/socket as a parameter so tests can use `tmux -L grove-test`
  and v2 can target remote hosts.

## Dashboard

Two panes: **Projects** (left) and the selected project's **Sessions** (right),
each row `● name  state · age  message`. Footer: total sessions needing you +
key hints. Colors: needs_you red, your_turn yellow, working green, ended grey.

| Key | Action |
|---|---|
| `j`/`k`, `↑`/`↓` | Move |
| `Tab` / `h`/`l` | Switch pane |
| `Enter` | Project: new session · Session: jump to its window |
| `n` | New session (prompt for optional name) |
| `a` | Add project (path prompt, default = cwd; name = folder name) |
| `e` | Edit project TOML in editor (`tmux display-popup -E`, 80%×80%) |
| `w` | Open project's wiki dir in editor (same popup) |
| `x` | Kill session (confirm) → marks `ended` |
| `c` | Clear ended sessions from the list (deletes their files) |
| `q` | Detach (sessions keep running) |

Refresh: `notify` watcher on `sessions/` and `projects/` (debounced ~150ms)
plus a 2s tick that reconciles sessions against `tmux list-windows` (window
gone → `ended`). A malformed project file renders as an error row, never a crash.

## MCP server (`grove mcp`)

stdio server built with `rmcp`'s tool macros, operating only through `store.rs`.
`project` is optional everywhere; it defaults to `$GROVE_PROJECT` (inherited
from the session's window env). Unknown project → tool error with the list of
valid names.

| Tool | Params | Returns |
|---|---|---|
| `list_projects` | — | name, path, summary for each |
| `get_project` | `project?` | fields + wiki section titles |
| `update_project` | `project?`, `summary?`, `color?` (`#RRGGBB`) | updated project |
| `read_wiki` | `project?`, `section?` | one section, or all concatenated |
| `write_wiki_section` | `project?`, `title`, `content` | create/replace; returns path |

`run/mcp.json`:

```json
{ "mcpServers": { "grove": { "command": "<grove>", "args": ["mcp"] } } }
```

### Session-start context

On `SessionStart`, `grove hook` prints to stdout (which Claude Code adds to
context):

```
Grove project: <name> (<path>)
Summary: <summary or "none yet">
Wiki sections: Architecture, Commands, … (read with the grove read_wiki tool)
```

Kept to a few lines — titles only, never wiki bodies.

## Module layout

```
src/main.rs      clap CLI dispatch
src/config.rs    config.toml, paths, GROVE_HOME, claude dir check
src/store.rs     projects / sessions / wiki CRUD; the only disk-writing module
src/state.rs     pure event → state transition
src/hook.rs      stdin → state.rs → store.rs; SessionStart context; exit 0
src/tmux.rs      tmux CLI wrapper (socket-parameterised)
src/launch.rs    claude command + env construction
src/mcp.rs       rmcp tool server over store.rs
src/ui/          app.rs (state), render.rs, input.rs
```

## Error handling

- `grove hook`: never non-zero, never blocks; errors → `grove.log`.
- `grove mcp`: tool errors returned as MCP errors with actionable text.
- Dashboard: bad files shown inline; tmux command failures shown in the footer.
- Startup: missing tmux / tmux < 3.2 / missing `claude` / missing claude config
  dir → clear message and exit 1.

## Testing

- `state.rs`: table-driven tests for every transition, using real hook JSON
  samples captured from Claude Code into `tests/fixtures/hooks/`.
- `store.rs`: temp `GROVE_HOME`; atomic write, slugging, wiki title parsing.
- `hook.rs`: run the binary with fixture stdin; assert file changes, exit 0
  even on garbage input.
- `mcp.rs`: call each tool in-process against a temp store.
- `tmux.rs` + `launch.rs`: integration tests on a throwaway `tmux -L grove-test`
  server, launching a stub command instead of `claude`.
- UI: a few `ratatui` `TestBackend` render snapshots.

## v2 readiness (not built now)

Remote machines: sessions already carry `host`; `tmux.rs` already takes a
target server. v2 adds a host registry and SSH/tmux transport without changing
the file formats.
