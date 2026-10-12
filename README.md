# Grove

A tmux dashboard for running and supervising Claude Code sessions across your
projects.

- **Projects:** each has a folder, a color and a one-line summary.
- **Sessions:** each Claude Code session runs in its own tmux window. Grove
  shows which ones are working, which **need you** (a permission prompt) and
  which are waiting for your next message, and sends a desktop alert when one
  needs you.
- **Accounts:** optionally, a project's folder picks its Claude account, for
  example `~/Work/...` on a work account and everything else on a personal one.
- **To-do lists:** a small vim-style checklist per project.
- **Fincas:** other machines running their own Grove (a home server, a
  Raspberry Pi), listed on your dashboard, one key away over SSH.

## Setting it up

These steps are written so you, or Claude Code working on your machine, can
follow them top to bottom. Each step ends with a check.

### 1. Prerequisites

| Needs | Check | Install |
|---|---|---|
| Rust (cargo) | `cargo --version` | <https://rustup.rs> |
| tmux 3.2 or newer | `tmux -V` | macOS: `brew install tmux` · Debian/Ubuntu: `sudo apt install tmux` |
| Claude Code, logged in | `claude --version` | `curl -fsSL https://claude.ai/install.sh \| bash`, then run `claude` once to log in ([docs](https://code.claude.com/docs)) |

macOS and Linux only.

### 2. Install

```sh
git clone <this repo> grove && cd grove
cargo install --path .
```

Check: `grove --version` prints a version. If `grove` isn't found, add
`~/.cargo/bin` to your `PATH`.

### 3. Point Grove at your Claude account

Grove runs every session with `CLAUDE_CONFIG_DIR` set to one Claude config
folder, and refuses to start a session if that folder doesn't exist. It never
silently falls back to another account.

- **One account, Claude Code's default (`~/.claude`)?** Nothing to do.
- **Claude Code already uses a different folder** (you set `CLAUDE_CONFIG_DIR`
  yourself)? Create `~/.grove/config.toml` with that folder:

  ```toml
  claude_config_dir = "~/.claude-personal"
  ```

- **Two accounts, chosen by folder** (for example personal and work)? See
  [More than one Claude account](#more-than-one-claude-account). It's three
  more lines.

Check: `grove doctor` shows `✓ account …` for each account.

### 4. Add projects

```sh
grove add ~/code/some-project     # a folder that exists
grove add                         # the current folder
grove add --new ~/code/new-thing  # create it (with git init) first
```

Check: `grove doctor` lists `✓ project <name>` with the account it will use.

### 5. Start it

```sh
grove
```

This opens (or re-attaches to) the dashboard in tmux. It needs a real
terminal: run it yourself, not from a script. Press `?` for every key.

`grove doctor` checks everything above at once and lists every problem. A ✓ on
every line means you're set. A `!` is a warning (for example a finca that's
offline right now), not a failure.

### Notes for Claude setting this up for someone

- **Ask before configuring:**
  1. Which folders should become projects?
  2. One Claude account, or several? If several: which config folder each one
     uses (each must already be logged in, e.g.
     `CLAUDE_CONFIG_DIR=~/.claude-work claude`), and which folder of projects
     goes with each.
  3. Any other machines to add as fincas? Each needs key-based SSH from this
     machine and Grove installed there.
- **Don't edit `~/.claude*/settings.json`.** Grove passes its hooks and MCP
  server to each session on the command line (`--settings`, `--mcp-config`).
  There's nothing to install into Claude Code.
- **Don't run `grove` with no arguments.** It attaches tmux and needs the
  user's terminal. Use `grove doctor` to verify, then ask the user to run
  `grove`.
- `grove add` is non-interactive given a path. Use `--new` to create a folder
  without the confirmation prompt.
- The files Grove keeps are plain TOML/JSON/Markdown under `~/.grove` (see
  [Files](#files-grove-or-grove_home)). `grove doctor` reports malformed ones.

## Using it

```sh
grove                           # open the dashboard
grove add ~/code/some-project   # add a project
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
| `a` | dashboard | add a project (Tab completes, `j`/`k` pick a match) |
| `e` | dashboard | edit the project file in `$EDITOR` (popup) |
| `r` | session | rename it (tmux window too) |
| `x` | session | kill it (asks first) |
| `c` | dashboard | clear ended sessions for the project |
| `d` | project | remove it from Grove (asks first; the folder is kept) |
| `t` | dashboard | jump to the project's to-do list |
| `R` | dashboard | update Grove after `cargo install` |
| `?` | dashboard | list every key |
| `q` | dashboard | detach; sessions keep running |

`C-g` is taken from Claude Code inside Grove, where it would open the prompt
in your editor.

Session states: **starting** → **working** ⇄ **needs you** (permission prompt)
→ **your turn** (Claude finished) → **ended**. A session that needs you also
gets ⚑ in front of its tmux window name.

While the dashboard runs, it sends a **desktop alert** when a session needs
you, here or on a finca, unless that session's window is already on screen
in a focused terminal. It uses `osascript` on macOS and `notify-send` on
Linux. Set `alerts = false` in `config.toml` to turn alerts off, for example
on a finca nobody sits at.

Each new session starts knowing its project's name, folder and summary.
Claude can update the summary through Grove's MCP tools (`list_projects`,
`get_project`, `update_project`).

### Updating

```sh
git pull && cargo install --path .
```

Then press `R` on the dashboard. It refreshes Grove's generated files and tmux
config and restarts the dashboard on the new build. Running sessions aren't
touched.

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
on every change. It's just for you: Claude sessions don't see it. Projects
with open items show the count next to their name (`☐3`).

## More than one Claude account

A project's **folder** picks its Claude account. Name the extra accounts in
`config.toml`, each with the folder its projects live under. Projects
anywhere else use the default account (`claude_config_dir`):

```toml
claude_config_dir = "~/.claude-personal"   # default account, shown as "personal"
# account_name = "home"                    # optional: name it yourself
account_color = "#7aa2f7"                  # optional: its color on the dashboard

[accounts.work]
dir = "~/.claude-work"                     # log in once: CLAUDE_CONFIG_DIR=~/.claude-work claude
color = "#e8a33d"
folder = "~/Work"                          # projects under here use this account
```

When an account has a `folder`, the dashboard's left column splits into one
box per account (the default on top), and `j`/`k` run straight through them.
The tmux status bar shows the account of the window you're in, and `grove add`
says which account a folder picked. To move a project to another account,
move its folder and add it again.

Without any `folder`, there's a single PROJECTS box and everything uses the
default account.

## Fincas (other machines)

A **finca** is another machine running its own Grove, such as a home
server or a Raspberry Pi. Your local dashboard lists them under **FINCAS**.
Each row shows live sessions and ⚑ needs-you, polled over SSH every 30s, and
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

Setting up a finca:
1. On the finca: install tmux and Claude Code, and log in to Claude.
2. Install Grove there in `~/.local/bin` (or run `scripts/deploy-fincas.sh`
   after step 4, which builds and copies it for you).
3. Make sure this machine can SSH to it without a password
   (`ssh-copy-id me@homebox.local`).
4. On this machine:

```sh
grove finca add homebox me@homebox.local   # checks SSH and finds grove there
grove finca add homebox me@homebox.local --backup me@homebox.<tailnet>.ts.net
grove finca ls
grove finca rm homebox
grove status            # what this machine reports (--json is what fincas send)
scripts/deploy-fincas.sh [name...]   # build + install grove on your fincas
```

Grove shares one SSH connection per finca (`ControlMaster`, sockets in
`~/.grove/run/`, kept 10 minutes after last use), so polls, new shell
windows and reconnects skip the login handshake. Your `~/.ssh/config` isn't
touched.

The finca's Grove runs through its login shell over SSH, so Homebrew and
`~/.local/bin` tools are found. If Claude Code there uses a folder other
than `~/.claude`, set `claude_config_dir` in that machine's
`~/.grove/config.toml`.

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
# account_name, account_color, [accounts.*]: see "More than one Claude account"
```

Grove never edits `~/.claude*/settings.json`. Hooks are passed per session
with `--settings run/hooks.json` and the MCP server with
`--mcp-config run/mcp.json`.

## Extending it

Each file does one job, so new features usually touch one or two:

| File | Job |
|---|---|
| `src/state.rs` | hook event → session state (pure; start here for new signals) |
| `src/hook.rs` | `grove hook`: applies events, prints session-start context |
| `src/store.rs` | all disk reads/writes (atomic) |
| `src/config.rs` | `config.toml`, accounts and which folder picks which |
| `src/launch.rs` | builds the `claude` command + window env |
| `src/tmux.rs` | tmux CLI wrapper (takes a socket) |
| `src/mcp.rs` | MCP tools |
| `src/status.rs` | the `status --json` document fincas report |
| `src/finca.rs` | finca files, SSH probe, poller, connect, shells |
| `src/todo.rs` | to-do lists and their vim keys (pure) |
| `src/ui/` | dashboard: `app.rs` state, `input.rs` keys, `render.rs` drawing, `run.rs` actions + loop, `help.rs` the `?` list |

```sh
cargo test     # ~280 tests, including real tmux servers on throwaway sockets
cargo clippy --all-targets -- -D warnings
```

Design history: [`docs/superpowers/specs/`](docs/superpowers/specs/).
