# Grove swagger: statusline, stats and grove-themed flair

## Why

Grove works, but it looks like a plain session list. The user wants it to feel
like *Grove*: quirky ASCII art and some fun. They also want their personal
Claude Code status line script (`~/.claude/statusline.sh`, the
`🤖 Opus 5.5 | PLAY | 📁 grove | 🌿 main | 🧠 ▓░░ 23% | 💰 … ` bar) built
into Grove, so everyone who uses Grove gets it. The stats that script already
sees (cost, context, lines changed) give the art something real to show.

Two parts, built in this order: **A** (the statusline and its stats), then
**B** (the flair, which uses A's stats).

## Goals

- Every Claude session Grove launches shows Grove's status line, with no setup.
- The dashboard shows each session's stats and a daily "harvest" total.
- The dashboard looks alive: a grove banner, states drawn as growth stages,
  something nice when nothing is running.
- Session states use one consistent set of grove glyphs everywhere, replacing ⚑.

## Non-goals

- Themes, art packs or config knobs for the art.
- Stats and harvest totals for finca sessions (this machine only for now).
- Changing the user's own Claude settings outside Grove.

## Anti-goals (a failure even if it works)

- Anything that makes the dashboard slower, flicker, or redraw noticeably more.
- A status line that's slower than the shell script, or that ever prints an
  error or blank line instead of a status.
- Art that pushes real information off a normal-sized terminal. The header
  already hands its rows back below `HEADER_MIN_HEIGHT` (render.rs); the new
  art has to do the same.
- Personal details in the code. Grove is a shareable tool, so nothing like
  `.claude-play` → "PLAY" goes in. That label comes from Grove's account config.

## Part A: `grove statusline`

### What it shows

A port of `~/.claude/statusline.sh` to Rust as a `grove statusline`
subcommand. It reads Claude Code's status JSON on stdin and prints one
coloured line. Keep the script's segments, order, emoji and colours:

1. 🤖 model display name (cyan)
2. the **Grove account name**, in the account's colour (replaces WORK/PLAY)
3. agent name, if present
4. 📁 the **Grove window name** (`project:session`) when the session is
   Grove's, else the project folder's basename
5. 🌿 git branch, plus staged `+N` and modified `~N` counts, cached about 5s
   per directory like the script does
6. 🧠 a 10-cell context bar and %: green below 70, yellow from 70, red from 90
7. 💰 cost, `$0.00`
8. ⏱️ duration
9. ✏️ `+added -removed`
10. 📊 5-hour usage %, only when `rate_limits.five_hour` is present

Field names come from the Claude Code statusline docs
(https://code.claude.com/docs/en/statusline): `model.display_name`,
`workspace.project_dir`, `cost.total_cost_usd`, `cost.total_duration_ms`,
`cost.total_lines_added`/`removed`, `context_window.used_percentage`
(input-only, may be null early on), `rate_limits.five_hour.used_percentage`
(may be absent), `session_id`, `agent.name`. Read the docs again before
writing the parser. Every field is optional: a missing field drops its
segment and never fails the line.

No `jq`, no shelling out except `git`. It has to start fast: Claude Code
debounces at 300ms and cancels a run that's still going.

### How sessions get it

Grove already launches every session with `--settings run/hooks.json`
(launch.rs, runfiles.rs). Add a `statusLine` entry to that file,
`{ "type": "command", "command": "<grove exe> statusline", "padding": 2 }`,
using `grove_exe()` like the hooks do. Command-line settings rank above user
settings, so inside Grove this replaces the user's own status line, and
nothing changes outside Grove. *(My call: the user's personal script will be
replaced by this one inside Grove, which is the point.)*

Opt-out: a config setting (for example `statusline = false` in
`config.toml`) leaves `statusLine` out of hooks.json, so the user's own
status line shows instead. *(My call; the name is up to the builder.)*

### Stats back to Grove

Each run also records the numbers it parsed for that session: cost, duration,
lines added/removed, context %, 5-hour %, and a timestamp. It must not
rewrite the session's main record, which hooks also write, because the two
would race. Keep a small per-session stats file under Grove's home, written
atomically with `write_atomic` (store.rs), keyed by the **Grove session**:
`$GROVE_SESSION` first, falling back to the payload's `session_id`, exactly
as hook.rs does. `/clear` and `/resume` start a new Claude conversation id
in the same window, and the row has to stay attached. Keep the stats files
out of the directory the dashboard watches for hook writes. The file layout
is the builder's call.

The row shows what Claude's own status line shows: the current
conversation's cost and context %. *(My call.)*

**Daily harvest.** `cost.total_cost_usd` and the line counts are running
totals per conversation, so "today" can't be a plain sum. On each run,
compare with the last snapshot for that Grove session and add the increase
to a tally for today's local date. If a total goes *down*, a new
conversation started (`/clear` or `/resume`): count its whole new value as
the increase, rather than ignoring it or subtracting. *(My call.)* The harvest is cost, lines added and removed, and the
number of sessions that had any activity that day. **Tallies are per
account per day**: the user wants each account's spending kept apart, never
summed across accounts. The account is the session's Grove account
(`cfg.account_for`, as launch.rs uses). Old daily tallies can be pruned
after a while; the builder decides how long.

If writing stats fails, still print the status line. Log it the way hooks
log errors (`log::error`).

### In the dashboard

- Session rows (Sessions pane and the grid) get the session's cost and
  context %, compact and dim, hidden when they don't fit or aren't known yet.
- The header shows the harvest line (Part B).
- The dashboard reads stats files on its existing poll; no new timers.

## Part B: the flair

### Growth-stage glyphs

One glyph per state, used everywhere a state glyph shows today: project-row
counts, the header totals, the footer alert, the grid, finca rows, tmux
window names, and the tmux status bar match in runfiles.rs:

| state | glyph | colour (unchanged) |
|-------|-------|--------------------|
| starting | 🌱 | cyan |
| working | 🌿 | green |
| your turn | 🌳 | yellow |
| needs you | 💧 | red |
| ended | 🍂 | dark gray |

The user picked 💧 for "needs you": a thirsty plant that wants watering.

- Emoji are two columns wide, and ⚑ and ● are one. Fix every place that
  assumes one column, such as `SLOT_W` in render.rs, grid layout and
  truncation, using ratatui's width handling, not `len()`. Snapshot tests
  in ui/tests.rs that assert on ⚑ or ● change along with them.
- **Migration:** existing tmux windows are named `⚑ project:session`
  (`NEEDS_YOU_MARK`, ui/run.rs). When Grove renames a window, strip the old
  `⚑ ` and the new `💧 ` prefix, so windows from before the upgrade don't
  keep a stale flag or end up with both.
- The tmux status bar (`window-status-format` in runfiles.rs) matches the
  new mark, and its test changes with it. The README's ⚑ mentions change too.

### Header banner

The current header (render.rs `render_header`) has a block-letter `GROVE`
wordmark with an olive branch off the E, and session totals and fincas on
the right. Keep the wordmark. Grow it into a small grove: a few ASCII trees
in the same LEAF, STEM and OLIVE palette, within the existing `HEADER_ROWS`
budget or at most one more row. Below `HEADER_MIN_HEIGHT` it still collapses.

- **Drifting leaves:** while any session is working, a leaf or two drifts
  across the banner. It may only advance on redraws that already happen (the
  250ms event poll, ui/run.rs). It must not add redraws or CPU while
  everything is idle, and it stops when nothing is working.
- **Harvest line** on the right, under the totals, one segment per
  account that had activity today, each in that account's colour:
  `🧺 work $9.80 +1.4k −200 · play $4.40 +700 −140`. With a single
  account, drop the name: `🧺 $14.20 · ✏️ +2.1k −340 · 6 sessions today`.
  Dim when nothing has happened today. When it doesn't fit, drop the line
  counts before dropping accounts.

The tree shapes and leaf motion are up to the builder; keep them small and
charming, not busy.

### Quiet grove

When this machine has no live sessions, the Sessions pane shows a small
ASCII grove scene with a line like "the grove is quiet", instead of an
empty box.

### Footer quips

The footer sometimes shows a short grove-flavoured quip when it has nothing
else to say (no message, no needs-you alert, not inserting). At most one
every few minutes, and it never covers real messages. Write about 15–25 of
them. Tone: dry and a bit odd, no puns about trees "branching out" more than
once.

## Usable by anyone

The user wants Grove to work for anyone, not just them. For this work:

- **Zero setup:** a fresh install with one default account gets the status
  line, stats, glyphs and art with no config. Nothing assumes two accounts,
  WORK/PLAY, `~/Play`, macOS, or a Max subscription.
- **Degrade quietly:** no git repo means no 🌿 segment. No `rate_limits`
  (API-key or Pro users, or early in a session) means no 📊. An account with
  no colour uses the existing default. A terminal that draws emoji badly
  still gets aligned rows, because widths come from ratatui, not guesses.
- **Never take over someone's setup without a way back:** Grove's status
  line replaces the user's own only inside Grove, and the opt-out restores
  theirs. Grove doesn't touch `~/.claude/settings.json` or any script of
  theirs.
- **Cost is labelled honestly:** on subscription accounts,
  `cost.total_cost_usd` is an estimate, not a bill. The README says so;
  the UI doesn't need to.
- **Documented:** the README gets a short section on the status line (what
  it shows, the opt-out) and the harvest. It's written for a stranger, with
  generic examples (`work`/`personal`, not the user's accounts). The help
  popup lists the glyph legend.
- **Nothing personal in code, tests or docs:** account names, colours and
  hosts come from config. Tests use generic names.

## Constraints

- Rust, ratatui, existing code style: comment density, naming, small
  functions. Tests next to code (`#[cfg(test)]`), real tmux where tmux is
  involved, as runfiles.rs does.
- `cargo fmt`, clippy with `-D warnings`, and the full suite green.
- Every change ships via `scripts/release.sh`, `cargo install --path .`,
  then `scripts/deploy-fincas.sh` (CLAUDE.md). Parts A and B can be separate
  releases.
- Fincas run Linux, including a Pi Zero: no macOS-only calls in
  `grove statusline`. Git cache paths go under Grove's home, not `/tmp`.

## Left to the builder

- Stats file layout and pruning window.
- The opt-out setting's name.
- Tree art, leaf animation, scene art, quip wording.
- Exact placement of per-session stats in the rows.
