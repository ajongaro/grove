# Account-scoped Grove MCP: projects, sessions and to-dos for Claude

## Goal

Claude, in any session, can see and work with the user's Grove projects:
what each one is, what its sessions are doing, its to-do list. It can
also start sessions and hand prompts to waiting ones. The user can then
ask a Claude "what's going on across my work projects?" or "start a
session in api to fix the flaky test", and it can answer or act.

Each Claude sees only its own account's projects. A work-account Claude
sees projects under the work folder. A personal one sees everything else.
Neither can read, edit or message the other's projects, or learn their
names.

Success:
- In a plain `claude` on the work account (started outside Grove), ask
  for a summary of the work projects and get their sessions, states and
  open to-dos.
- Ask the same Claude about a personal project and get "no such
  project".
- From a Grove session, start a session in another project with a first
  prompt, and see it appear on the dashboard.

## What exists

`grove mcp` (src/mcp.rs) is a stdio server with `list_projects`,
`get_project` and `update_project` (summary, color). Only sessions Grove
launches get it, through `--mcp-config ~/.grove/run/mcp.json`. It isn't
scoped: every session sees every project.

## Account scope

**Which account.** `grove mcp --account <name>` names it. `<name>` is a
key under `[accounts]` in config.toml, or the default account's name.
- Grove-launched sessions: `mcp.json` can't vary per session, so the
  account comes from the launch instead. Grove already sets the
  session's account on its window; `launch.rs` adds `GROVE_ACCOUNT=<name>`
  to the window env. `grove mcp` with no `--account` uses
  `$GROVE_ACCOUNT`.
- Other sessions: the user registers `grove mcp --account <name>` once
  per account (see Setup).
- Neither one: the server starts, but every tool returns an error that
  says how to register it. It shows nothing, rather than guessing an
  account.
- An unknown account name is the same error, and names the accounts
  that exist.

**A guard against registering in the wrong account.** If
`CLAUDE_CONFIG_DIR` is set in the server's environment and matches a
*different* account's `dir`, every tool refuses: "registered for work
but running in play's Claude config". Claude Code passes its
environment to stdio servers: in a Grove session, the existing server
reads `$GROVE_PROJECT` from the window env (checked 2026-10-16). Grove
sets `CLAUDE_CONFIG_DIR` in that same env, so it arrives too. When the
variable is absent (a plain `claude` on the default `~/.claude`), the
check is skipped.

**What's in scope.** A project is in scope when
`cfg.account_for(&project.path).name == account`. That's the rule that
already picks the account a project's sessions run on. Sessions and
to-dos belong to projects and follow them. Outside the scope:
- a project name or session id gives the same "no such project" or
  "no such session" error as one that doesn't exist;
- lists leave such projects out;
- error messages that list project names list only names in scope.

Fincas aren't included.

## Tools

The `project` parameter is optional wherever it appears, and defaults to
`$GROVE_PROJECT` (the current Grove session's project), as today.
`session` takes a session's name or id, matched within the account.

| Tool | Parameters | Returns / does |
|---|---|---|
| `list_projects` | — | each project: name, folder, summary, counts (needs you / your turn / working), open to-dos |
| `get_project` | project | the above, plus color, git (branch, uncommitted file count, last 5 commits as `hash subject (age)`), the first 40 lines of `README.md` and of `CLAUDE.md` if present, its sessions, its to-do list |
| `update_project` | project, summary?, color? | as today |
| `list_sessions` | project? | each live session (not ended): name, project, state, last activity ("3m ago"), its message if any |
| `read_session` | session, lines? (default 60, max 500) | the last `lines` of the session's screen (`capture-pane -p -J`), trailing blank lines trimmed |
| `get_todos` | project | numbered list: `1. [ ] text` |
| `edit_todos` | project, ops: list of {op: add / check / uncheck / reword / remove, n?, text?} | applies the ops in order, saves once, returns the new list. Numbers refer to the list as it was before any op. Out of range: error, nothing saved. |
| `start_session` | project, prompt?, name? | starts a session as `n` on the dashboard does, on the project's account. A `prompt` is passed to `claude` as its first message. Returns the session's name. Needs Grove's tmux session running; otherwise: "Grove isn't running: start it with `grove`". |
| `send_prompt` | session, text | only when the session's state is `your turn`. Pastes `text` as a bracketed paste, then Enter. Otherwise it's refused with the state ("it's working", "it needs you: answer its prompt on the dashboard"). A Claude can't message its own session. |

Not included: answering permission prompts, killing or renaming sessions,
fincas.

`edit_todos` works on the stored Markdown checklist through
`todo::Todos` load/serialize, so the dashboard and the tool agree on the
format. The dashboard already reloads the list when the file changes
(unless the user is editing it right then).

`git` output comes from running `git -C <folder>` with a 2-second
timeout. A folder that isn't a repo, or a timeout, gives "git: none".

## Setup

- **Grove sessions:** automatic. `mcp.json` stays as it is, and the
  account comes from `GROVE_ACCOUNT`.
- **Every other session:** `grove mcp setup` prints one command per
  account, ready to paste:

  ```
  # play (~/.claude-play)
  CLAUDE_CONFIG_DIR=~/.claude-play claude mcp add --scope user grove -- /opt/homebrew/opt/grove/bin/grove mcp --account play
  # work (~/.claude-work)
  CLAUDE_CONFIG_DIR=~/.claude-work claude mcp add --scope user grove -- /opt/homebrew/opt/grove/bin/grove mcp --account work
  ```

  It's verified that `--scope user` writes to `$CLAUDE_CONFIG_DIR/.claude.json`,
  so each registration reaches only that account's sessions. The path
  is `runfiles::grove_exe()` (stable across `brew upgrade`). Grove
  prints these commands and never runs them, since Grove never edits
  Claude's configuration.
- **`grove doctor`** adds one line per account: `✓ mcp work` if that
  account's `.claude.json` has a `grove` server under `mcpServers`;
  otherwise `· mcp work: not registered (grove mcp setup)`. This is
  informational, not a failure.
- In a Grove session with a user-scope `grove` server also registered,
  both servers are named `grove` and do the same thing. Whichever one
  Claude Code picks gives the same tools and the same account.

## Structure

- `src/mcp.rs`:
  - `GroveMcp` gains `cfg: Config` and `scope: Result<String, String>`
    (the account, or the error every tool returns);
  - `fn scope_for(cfg, flag, grove_account_env, claude_config_dir_env) -> Result<String, String>`
    is pure;
  - `in_scope(&Project)`, and the new tools.
- `src/mcp_overview.rs` (new): project overview text (git, README and
  CLAUDE.md excerpts, counts), kept apart so mcp.rs stays about tools
  and scope.
- `src/launch.rs`: `GROVE_ACCOUNT` in the window env; an optional first
  prompt for `start`.
- `src/main.rs`: `mcp --account`, `mcp setup`.
- `src/doctor.rs`: the per-account registration line.
- `src/todo.rs`: `apply_ops`, pure, if `Todos` doesn't already make
  this short.
- README: an "MCP tools" section (what Claude can do, setup, scope).

## Testing

- **scope_for:**
  - flag beats env;
  - `GROVE_ACCOUNT` used when there's no flag;
  - neither gives an error naming `grove mcp setup`;
  - an unknown name;
  - a `CLAUDE_CONFIG_DIR` that matches another account → refused;
  - one that matches this account, or is absent, → allowed.
- **Isolation:** a store with `alpha` under the work folder and `beta`
  elsewhere, and two accounts. For each tool, a work-scoped server:
  - succeeds on alpha;
  - on beta, gets the same error text as for a project that doesn't
    exist;
  - never shows "beta" in any output, including the not-found message's
    list of names.
- **get_project:** a temp git repo with commits, a README and a
  CLAUDE.md; also a non-repo folder.
- **edit_todos:**
  - every op;
  - numbers refer to the original list;
  - an out-of-range number saves nothing;
  - the file round-trips with the dashboard's format.
- **Sessions, on a throwaway tmux server** (like the existing launch
  tests):
  - `start_session` with a stand-in claude shows the prompt in its
    argv, and sets `GROVE_ACCOUNT`;
  - `read_session` returns the screen's text;
  - `send_prompt` delivers text plus Enter to a `your turn` session;
  - it refuses `working` and `needs you`, and its own session.
- **mcp setup:** the commands for each account, with the stable exe
  path.
- **Live:**
  - register on both accounts;
  - in a plain `claude` on each, list projects and check the other
    account's are absent;
  - start a session with a prompt;
  - message a waiting one;
  - read a screen.

## Open questions (settled by trial)

- Does `CLAUDE_CONFIG_DIR` reach the server from a plain `claude`
  started with it set in the shell? It does from Grove sessions (see
  above); confirm it from a terminal during the live test.
- Is 40 lines of README and CLAUDE.md the right amount for
  `get_project`? Tune it after using it.
- Does a pasted prompt plus Enter submit reliably in Claude Code's
  input, or does it need a short pause between paste and Enter, as
  pasting sometimes does?
- Tool descriptions will need wording passes once Claude uses them for
  real.

## Out of scope

- Fincas' projects and sessions.
- Answering permission prompts, killing or renaming sessions.
- Notifications from the MCP server (Claude being told when a session
  needs you).
- MCP resources or prompts. Only tools.
