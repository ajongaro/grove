# Grove

A tmux dashboard for running and supervising Claude Code sessions across your
projects.

- **Projects:** each has a folder, a color and a summary.
- **Sessions:** each Claude Code session runs in its own tmux window, on its
  project's Claude account, picked by its folder: `~/Work` → work, else
  play (see [Play and Work](#more-than-one-claude-account-play-and-work)).
- **Signals:** the dashboard shows which sessions are working, which need you
  (a permission prompt), and which are waiting for your next message.
- **Summary:** each new session starts knowing its project's one-line
  summary, which Claude can update through Grove's MCP server.

Design: [`docs/superpowers/specs/2026-10-06-grove-v1-design.md`](docs/superpowers/specs/2026-10-06-grove-v1-design.md)

## Install

```sh
cargo install --path .
```

Needs tmux ≥ 3.2 and `claude` on your PATH.

## Use

```sh
grove add ~/code/some-project   # or `grove add` inside the folder
grove                           # open the dashboard (attaches tmux)
grove rm some-project           # forget it (the folder is kept)
grove doctor                    # check tmux, claude, accounts, projects, fincas
```

Grove runs on its **own tmux server** (`tmux -L grove`), so its key bindings
never touch your normal tmux, and your `~/.tmux.conf` doesn't apply there.
Grove keeps stock tmux keys and adds only its own. Put your own setup
(prefix, splits, theme) in `~/.grove/tmux.local.conf`, which is loaded last.

| Key | Where | Does |
|---|---|---|
| `C-g` | anywhere | back to the dashboard |
| `j` `k` `↑` `↓` | dashboard | move |
| `Tab` `h` `l` | dashboard | switch pane |
| `Enter` | project | start a session (and jump to it) |
| `Enter` | session | jump to its window |
| `n` | dashboard | start a named session |
| `a` | dashboard | add a project (defaults to the current folder) |
| `e` | dashboard | edit the project file in `$EDITOR` (popup) |
| `x` | session | kill it (asks first) |
| `c` | dashboard | clear ended sessions for the project |
| `d` | project | remove it from Grove (asks first; the folder is kept) |
| `t` | dashboard | jump to the project's to-do list |
| `q` | dashboard | detach; sessions keep running |

Session states: **starting** → **working** ⇄ **needs you** (permission prompt)
→ **your turn** (Claude finished) → **ended**. A session that needs you also
gets ⚑ in front of its tmux window name.

While the dashboard runs, it sends a **desktop alert** when a session needs
you, here or on a finca, unless that session's window is already on screen
in a focused terminal. It uses `osascript` on macOS and `notify-send` on
Linux. Set `alerts = false` in `config.toml` to turn alerts off, for example
on a finca nobody sits at.

## To-do lists

Each project has a to-do list under its sessions (`t` or `Tab` to get
there), edited with vim keys:

| Key | Does |
|---|---|
| `j` `k` `gg` `G` | move |
| `o` / `O` | new item below / above (insert mode) |
| `i` `a` `A` / `cc` | edit the item / replace its text |
| `Enter` `Esc` | leave insert mode (saves; an empty item is dropped) |
| `x` `space` | toggle done |
| `dd` / `p` `P` | delete / paste it below or above (works across projects) |
| `u` | undo the last change |
| `E` | open the list in your editor |
| `h` | back to the projects |

It's saved as `~/.grove/todos/<project>.md`, a plain Markdown checklist,
on every change. It's just for you: Claude sessions don't see it.

## Fincas (other machines)

A **finca** is another machine running its own Grove, such as a home
server or a Raspberry Pi. Your local dashboard lists them under **Fincas**. Each
row shows live sessions and ⚑ needs-you, polled over SSH every 30s, and
the footer counts remote sessions that need you.

- **Enter** on a finca opens that machine's own Grove dashboard. Every key
  goes to it.
- **`s`** on a finca opens a plain login shell there. Your tmux prefix
  works in it, and new windows and splits (`prefix c`, `prefix v`) open
  shells on the finca too.
- **`C-q`** comes back to your local dashboard from either. The connection
  stays open, so Enter or `s` returns instantly.
- Selecting a finca shows its sessions read-only. To start, kill or edit
  sessions, connect.
- A finca can have a **backup** SSH destination, such as its Tailscale
  name. When the main one doesn't answer (you're away from home), polling
  tries the backup, and Enter and `s` connect to whichever one answered last.

```sh
grove finca add homebox me@homebox.local   # needs key-based SSH + grove installed there
grove finca add homebox me@homebox.local --backup me@homebox.<tailnet>.ts.net
grove finca ls
grove finca rm homebox
grove status            # what this machine reports (--json for fincas)
scripts/deploy-fincas.sh [name...]   # build + install grove on your fincas
```

Setting up a new finca:
1. Install tmux.
2. Install Claude Code and log in.
3. Install Grove in `~/.local/bin`.
4. Set `claude_config_dir` in its `~/.grove/config.toml` (e.g.
   `~/.claude`).

Its Grove runs through your login shell over SSH, so Homebrew and
`~/.local/bin` tools are found.

## Files (`~/.grove`, or `$GROVE_HOME`)

```
config.toml               settings (below)
tmux.local.conf           your own tmux setup for Grove's server (optional)
projects/<slug>.toml      name, path, color, summary
todos/<slug>.md           the project's to-do list (Markdown checklist)
sessions/<uuid>.json      one per session: state, window, message
fincas/<slug>.toml        name, ssh destination, backup, grove_bin, color
run/                      regenerated on every `grove` start: hooks.json, mcp.json, tmux.conf
grove.log                 errors from hooks / MCP
```

```toml
# ~/.grove/config.toml — every key is optional
claude_config_dir = "~/.claude"        # CLAUDE_CONFIG_DIR for every session
tmux_session = "grove"
editor = ""                            # empty → $EDITOR → vi
claude_args = []                       # e.g. ["--model", "opus"]
alerts = true                          # desktop alerts when a session needs you
```

### More than one Claude account (Play and Work)

A project's **folder** picks its Claude account. On this laptop, projects
under `~/Work` run on the work account and everything else (`~/Play`, or
anywhere) runs on the default play account. Add a folder; that's it.

```toml
claude_config_dir = "~/.claude-play"       # default account, shown as "play"
# account_name = "home"                    # optional: name it yourself
# account_color = "#7aa2f7"

[accounts.work]
dir = "~/.claude-work"
color = "#e8a33d"
folder = "~/Work"                          # projects under here use this account
```

When an account has a `folder`, the dashboard's left column splits into
stacked boxes: **play** on top, **work** below (then fincas). `j`/`k` run
straight through both. The tmux status bar shows the account of the window
you're in, and `grove add` says which account the folder picked. To move a
project to the other account, move its folder and add it again.

Fincas have no `folder` set, so everything there runs on play and the
dashboard keeps a single Projects box.

Grove never edits `~/.claude*/settings.json`. Hooks are passed per session
with `--settings run/hooks.json` and the MCP server with
`--mcp-config run/mcp.json`.

## MCP tools (inside a Grove session)

`list_projects`, `get_project`, `update_project` (summary, `#RRGGBB` color).
`project` defaults to the session's own project.

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
| `src/status.rs` | the `status --json` document fincas report |
| `src/finca.rs` | finca files, SSH probe, poller, connect |
| `src/ui/` | dashboard: `app.rs` state, `input.rs` keys, `render.rs` drawing, `run.rs` actions + loop |


```sh
cargo test     # ~200 tests, including real tmux servers on throwaway sockets
cargo clippy --all-targets -- -D warnings
```
