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
| tmux 3.2 or newer | `tmux -V` | macOS: `brew install tmux` · Debian/Ubuntu: `sudo apt install tmux` |
| Claude Code, logged in | `claude --version` | `curl -fsSL https://claude.ai/install.sh \| bash`, then run `claude` once to log in ([docs](https://code.claude.com/docs)) |

macOS and Linux only.

### 2. Install

Any one of these:

```sh
# Homebrew (macOS or Linux); also installs tmux
brew install ajongaro/grove/grove

# Or the install script: the right binary for your OS and CPU, into ~/.local/bin
curl -fsSL https://github.com/ajongaro/grove/releases/latest/download/install.sh | sh

# Or from source (needs Rust: https://rustup.rs)
git clone https://github.com/ajongaro/grove.git && cd grove
cargo install --path .
```

Check: `grove --version` prints a version. If `grove` isn't found, add the
folder it went into to your `PATH`: `~/.local/bin` for the install script,
`~/.cargo/bin` for cargo.

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
  3. Any other machines to add as fincas? `grove finca setup <dest>` is
     interactive (it asks before each change and may need a password), so
     ask the user to run it, or press `F` on the dashboard.
- **Don't edit `~/.claude*/settings.json`.** Grove passes its hooks and MCP
  server to each session on the command line (`--settings`, `--mcp-config`).
  There's nothing to install into Claude Code. Optionally, `grove mcp setup`
  prints a command per account that gives plain `claude` sessions Grove's
  tools too; show it to the user rather than running it.
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
grove mcp setup                 # how to give every Claude session Grove's tools
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
| `s` | dashboard | a quick shell in the project's folder (popup; `exit` closes it) |
| `S` | dashboard | a shell in the project's folder that stays open (see [Shells](#shells)) |
| `r` | session | rename it, or a shell (tmux window too) |
| `x` | session | kill it, or close a shell (asks first) |
| `p` | session | pin / unpin it (any account, or a finca session) |
| `1`–`9` | dashboard | jump to that pinned session |
| `c` | dashboard | clear ended sessions for the project |
| `d` | project | remove it from Grove (asks first; the folder is kept) |
| `t` | dashboard | jump to the project's to-do list |
| `v` | dashboard | session grid: every live session's screen |
| `R` | dashboard | restart on a new Grove, after upgrading |
| `F` | dashboard | add a finca (guided setup; `a` on a finca too) |
| `C-v` | finca window | send the clipboard's file or image there, paste its path |
| `prefix r` | anywhere | reload Grove's tmux config |
| `?` | dashboard | list every key |
| `q` | dashboard | detach; sessions keep running |

`C-g` is taken from Claude Code inside Grove, where it would open the prompt
in your editor.

Session states: **starting** → **working** ⇄ **needs you** (permission prompt)
→ **your turn** (Claude finished) → **ended**. A session that needs you also
gets ⚑ in front of its tmux window name. The tmux status bar lists only the
window you're in and the windows that need you (in red); the dashboard has the
rest. Each project row counts its live
sessions by state: red ⚑ needs you, yellow ● your turn, green ● working.

While the dashboard runs, it sends a **desktop alert** when a session needs
you, here or on a finca, unless that session's window is already on screen
in a focused terminal. It uses `osascript` on macOS and `notify-send` on
Linux. Set `alerts = false` in `config.toml` to turn alerts off, for example
on a finca nobody sits at.

Grove sessions get Grove's own **status line** under Claude's prompt:

```
🤖 Opus | WORK | 📁 api:s2 | 🌿 main +1 ~2 | 🧠 ▓▓░░░░░░░░ 23% | 💰 $1.23 | ⏱️ 2m 5s | ✏️ +10 -2 | 📊 12%
```

That's the model, the session's account in its colour (with more than one), `project:session`,
the git branch with staged and modified file counts, context use, cost,
time, and lines changed. The last one is your 5-hour usage, shown only on
Pro and Max plans. Each session's row in the dashboard shows its cost and
context use too (`$1.23 · 23%`). The cost is Claude Code's own estimate;
on a subscription it isn't what you're billed.

Inside Grove it replaces any status line you've set up, without touching
your Claude settings. To keep yours, set `statusline = false` in
`config.toml`.

Each new session starts knowing its project's name, folder and summary.
Claude can update the summary, and see much more, through Grove's MCP
tools (see [Claude's view of your projects](#claudes-view-of-your-projects-mcp)).

### Pinned sessions

`p` on a session pins it: a Pinned box at the top of the right column lists
the sessions you're working in, from every project, both Claude accounts and
your fincas, each with where it lives and its state. `1`–`9` jump straight
to a pin from anywhere on the dashboard (except the to-do list). Tab or `k`
from the top of Sessions moves into the box, where `Enter` jumps, `p`
unpins and `x` kills. A pin goes away when its session ends; a finca
that isn't answering keeps its pins, dimmed, until it does. Pins live in
`~/.grove/pins.json`.

### Shells

`s` opens a quick shell in a popup over the dashboard: handy for one
command, gone when you `exit`, and with no scrollback. `S` opens one that
stays: a full window in the project's folder, so the mouse wheel (or
prefix `[`) scrolls back through it. `C-g` comes back to the dashboard, and
the shell is listed in the project's Sessions pane below its sessions
(`$ sh1`, with what's running in it), where `Enter` goes back to it, `r`
renames it and `x` closes it. A shell lasts until you `exit` it or Grove's tmux server
stops (a reboot); restarting the dashboard doesn't touch it. Shells aren't
in the grid or the pins.

### Session grid

`v` on the dashboard shows every live session, here and on your fincas, as
a tile with its latest output (Claude's input box and status bar are left
out; `✎ draft` in the title means unsent text is typed there), refreshed as
it changes. `h j k l`
move, `Tab` jumps to the next one that needs you or is waiting, `Enter`
goes to its window (`C-g` comes back), `i` replies to a session that's
waiting for its next message without leaving the grid, and `x` kills one
(after a y/n), here or on a finca. Permission prompts
are answered in the session itself. A finca's tiles dim and say `stale`
when it stops answering; one running an older Grove shows states without
screens until you `grove finca update`.

### Updating

Upgrade the way you installed:

```sh
brew upgrade grove                     # Homebrew
curl -fsSL https://github.com/ajongaro/grove/releases/latest/download/install.sh | sh
git pull && cargo install --path .     # source
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

## Claude's view of your projects (MCP)

Every Claude that Grove starts can see your projects through Grove's MCP
tools, limited to that session's account. A work Claude sees only the
projects under the work folder; a personal one sees the rest, and neither
can see, edit or message the other's.

| Tool | What Claude can do |
|---|---|
| `list_projects`, `get_project` | see each project: summary, sessions, to-dos, git branch and recent commits, README and CLAUDE.md |
| `list_sessions`, `read_session` | see which sessions are working, need you or wait for you, and read one's screen |
| `get_todos`, `edit_todos` | read and edit a project's to-do list |
| `update_project` | set a project's summary or color |
| `start_session` | start a session in a project, with a first prompt |
| `send_prompt` | send a prompt to a session that's waiting for you ("your turn") |

Claude asks before using a tool unless you've allowed it. Nothing answers
permission prompts for you: a session that needs you still needs you.

To give the tools to every Claude session, including a plain `claude` in a
terminal, run `grove mcp setup` and run the command it prints for each
account. Grove doesn't change Claude's settings itself. `grove doctor` shows
which accounts have it.

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
Within each box, projects with the most live sessions come first.
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
- Selecting a finca lists its sessions and the shells you've opened there.
  `l` (or Tab) moves into the list; **Enter** on a session connects
  straight into that session's window, Enter on a shell goes back to
  it, and `r` on a shell names it. To start, kill or edit sessions, connect.
- **`C-v`** in a finca window (its Grove or a shell there) sends a file
  you copied in Finder (⌘C) or an image on the clipboard (a screenshot
  taken with ⌘⇧⌃4) to the finca's `~/.grove/inbox`, and pastes its path.
  Claude there reads it as an image. Other clipboard contents pass
  through, so paste text with ⌘V as usual. Inbox files are removed after
  7 days. Reading the clipboard is macOS-only for now.
- A finca can have a **backup** SSH destination, such as its Tailscale
  name. When the main one doesn't answer (you're away from home), polling
  tries the backup, and Enter and `s` connect to whichever one answered last.

Adding a finca: press **`F`** on the dashboard (or run `grove finca setup
me@homebox.local`). The setup checks SSH, tmux, Claude Code, Grove and the
Claude login, and asks before fixing each one:
- no key-based SSH yet: it can create a key and copy it there (you type the
  password once);
- missing tmux or Claude Code: it shows the install command and runs it
  over SSH (sudo can prompt). Claude Code runs on 64-bit machines only, so
  a 32-bit board (an original Pi Zero W) becomes a finca for its shells,
  with no Claude install or login;
- Grove: a machine with this one's OS and CPU gets a copy of this build.
  Any other gets the release of the same version built for it (downloaded
  here, checked, and copied over SSH, so it needs nothing extra), if you
  installed Grove with Homebrew or the install script. If you built Grove
  from a source checkout, it builds that checkout there instead (offering
  to install Rust first), so the finca runs the same code; a 32-bit ARM
  board, too small to build it, gets the release;
- logged out of Claude there: it runs `claude auth login` for you;
- if Tailscale here knows the machine, it offers its Tailscale name as the
  backup.

Running it again on an existing finca checks it and repairs what's
missing.

```sh
grove finca setup me@homebox.local         # guided, asks before each change
grove finca add homebox me@homebox.local   # non-interactive: checks SSH, finds grove
grove finca add homebox me@homebox.local --backup me@homebox.<tailnet>.ts.net
grove finca ls
grove finca rm homebox
grove finca update [name...]   # put this version of grove on your fincas
grove send homebox ~/Desktop/shot.png  # prints the path there (no file: the clipboard)
grove status            # what this machine reports (--json is what fincas send)
```

`scripts/deploy-fincas.sh [name...]` installs this checkout here
(`cargo install --path .`), then runs `grove finca update`.

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
stats/<uuid>.json         one per session: what its status line saw, per day
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
statusline = true                      # Grove's status line in its sessions
# account_name, account_color, [accounts.*]: see "More than one Claude account"
```

Grove never edits `~/.claude*/settings.json`. Hooks and the status line are
passed per session with `--settings run/hooks.json` and the MCP server with
`--mcp-config run/mcp.json`.

## Extending it

Each file does one job, so new features usually touch one or two:

| File | Job |
|---|---|
| `src/state.rs` | hook event → session state (pure; start here for new signals) |
| `src/hook.rs` | `grove hook`: applies events, prints session-start context |
| `src/statusline.rs` | `grove statusline`: the status line in Grove's sessions |
| `src/stats.rs` | what each status line saw: per-session and per-day, per-account totals |
| `src/store.rs` | all disk reads/writes (atomic) |
| `src/config.rs` | `config.toml`, accounts and which folder picks which |
| `src/launch.rs` | builds the `claude` command + window env |
| `src/tmux.rs` | tmux CLI wrapper (takes a socket) |
| `src/mcp.rs` | MCP tools, each limited to one Claude account's projects |
| `src/mcp_overview.rs` | what the MCP tools say about a project: sessions, git, docs |
| `src/status.rs` | the `status --json` document fincas report |
| `src/finca.rs` | finca files, SSH probe, poller, connect, shells, installing grove there |
| `src/transfer.rs` | `grove send` and `C-v` in finca windows: clipboard, inbox |
| `src/setup.rs` | `grove finca setup`: the guided wizard and its machine checks |
| `src/todo.rs` | to-do lists and their vim keys (pure) |
| `src/ui/` | dashboard: `app.rs` state, `input.rs` keys, `render.rs` drawing, `grid.rs` the session grid, `run.rs` actions + loop, `help.rs` the `?` list |
| `src/screens.rs` | `grove screens` and `grove type-prompt`: live screens and replies, here and fetched from fincas |
| `src/prompt.rs` | the checks for typing a reply into a session (grid, fincas, MCP) |
| `src/shells.rs` | shells that stay open (`S`): tmux windows tagged with their project |
| `src/pins.rs` | pinned sessions: `~/.grove/pins.json` |

```sh
cargo test     # ~500 tests, including real tmux servers on throwaway sockets
cargo clippy --all-targets -- -D warnings
```

### Releasing

```sh
scripts/release.sh 0.2.0
```

It sets the version, runs the tests, tags `v0.2.0` and pushes. The tag
starts `.github/workflows/release.yml`, which builds macOS and Linux
binaries (arm64 and x86_64, plus 32-bit ARM Linux for Pi Zeros; Linux
ones are static, musl) and publishes them
with `SHA256SUMS` and `install.sh`. The script then writes the Homebrew
formula (`scripts/formula.sh`) to the
[ajongaro/homebrew-grove](https://github.com/ajongaro/homebrew-grove) tap.

Design history: [`docs/superpowers/specs/`](docs/superpowers/specs/).
