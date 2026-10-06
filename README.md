# Grove

A tmux dashboard for running and supervising Claude Code sessions across your
projects. A stripped-down, personal take on Yeehaw.

- **Projects:** each has a folder, a color and a summary.
- **Sessions:** each Claude Code session runs in its own tmux window, always
  on the `~/.claude-play` account.
- **Signals:** the dashboard shows which sessions are working, which need you
  (a permission prompt), and which are waiting for your next message.
- **Wiki:** per-project Markdown notes that Claude can read and write through
  Grove's MCP server. New sessions start knowing which sections exist.

Design: [`docs/superpowers/specs/2026-10-06-grove-v1-design.md`](docs/superpowers/specs/2026-10-06-grove-v1-design.md)

## Install

```sh
cargo install --path .
```

Needs tmux ≥ 3.2 and `claude` on your PATH.

> **This Mac:** Xcode's `xcodebuild` is currently broken and the Command Line
> Tools' 27.0 SDK won't link, so build with:
> `DEVELOPER_DIR=/Library/Developer/CommandLineTools SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk cargo install --path .`

## Use

```sh
grove add ~/Play/some-project   # or `grove add` inside the folder
grove                           # open the dashboard (attaches tmux)
```

Grove runs on its **own tmux server** (`tmux -L grove`), so its key bindings
never touch your normal tmux.

| Key | Where | Does |
|---|---|---|
| `C-y` | anywhere | back to the dashboard |
| `C-h` / `C-l` | anywhere | previous / next window |
| `j` `k` `↑` `↓` | dashboard | move |
| `Tab` `h` `l` | dashboard | switch pane |
| `Enter` | project | start a session (and jump to it) |
| `Enter` | session | jump to its window |
| `n` | dashboard | start a named session |
| `a` | dashboard | add a project (defaults to the current folder) |
| `e` / `w` | dashboard | edit project file / wiki folder in `$EDITOR` (popup) |
| `x` | session | kill it (asks first) |
| `c` | dashboard | clear ended sessions for the project |
| `q` | dashboard | detach; sessions keep running |

Session states: **starting** → **working** ⇄ **needs you** (permission prompt)
→ **your turn** (Claude finished) → **ended**. A session that needs you also
turns its tmux window tab red.

## Files (`~/.grove`, or `$GROVE_HOME`)

```
config.toml               settings (below)
projects/<slug>.toml      name, path, color, summary
wiki/<slug>/<section>.md  wiki sections (title = first "# " heading)
sessions/<uuid>.json      one per session: state, window, message
run/                      regenerated on every `grove` start: hooks.json, mcp.json, tmux.conf
grove.log                 errors from hooks / MCP
```

```toml
# ~/.grove/config.toml — every key is optional
claude_config_dir = "~/.claude-play"   # CLAUDE_CONFIG_DIR for every session
tmux_session = "grove"
editor = ""                            # empty → $EDITOR → vi
claude_args = []                       # e.g. ["--model", "opus"]
```

Grove never edits `~/.claude*/settings.json`. Hooks are passed per session
with `--settings run/hooks.json` and the MCP server with
`--mcp-config run/mcp.json`.

## MCP tools (inside a Grove session)

`list_projects`, `get_project`, `update_project` (summary, `#RRGGBB` color),
`read_wiki`, `write_wiki_section`. `project` defaults to the session's own
project.

## Extending it

Each file does one job, so new features usually touch one or two:

| File | Job |
|---|---|
| `src/state.rs` | hook event → session state (pure; start here for new signals) |
| `src/hook.rs` | `grove hook`: applies events, prints session-start context |
| `src/store.rs` | all disk reads/writes (atomic) |
| `src/launch.rs` | builds the `claude` command + window env |
| `src/tmux.rs` | tmux CLI wrapper (takes a socket, ready for remote hosts) |
| `src/mcp.rs` | MCP tools |
| `src/ui/` | dashboard: `app.rs` state, `input.rs` keys, `render.rs` drawing, `run.rs` actions + loop |

Sessions already carry a `host` field (`"local"`) for v2 remote machines.

```sh
cargo test     # ~114 tests, including real tmux servers on throwaway sockets
cargo clippy --all-targets -- -D warnings
```
