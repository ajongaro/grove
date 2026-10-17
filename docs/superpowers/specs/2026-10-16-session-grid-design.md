# Session grid: every live session at a glance

## Goal

With several Claude sessions running here and on fincas, finding the one
that's waiting means switching windows or reading state dots. The goal is
one view that shows every live session's latest output at once, jumps to
the ones that need you, and lets you answer a waiting session without
leaving the view.

Success:
- Press `v` on the dashboard and see a tile per live session, on this Mac
  and on every reachable finca, each showing the bottom of its screen.
- `Tab` lands on the next session that needs you or is waiting.
- On a "your turn" session, `i`, type, `Enter` sends it, and the tile
  shows Claude starting to work.

Inspired by yeehaw's session grid (github.com/Colmbus72/yeehaw,
`src/views/session_grid.rs`). It keeps Grove's own keys and look.

## The view

`v` in Normal mode on the dashboard replaces the panes with the grid.
`v` or `Esc` goes back to the panes, keeping the dashboard selection as
it was. The header (wordmark, branch, totals) stays when the terminal is
tall enough, as on the dashboard.

**Top line**:
- Left: `SESSIONS · <n> live · ⚑<a> ●<b> ●<c>`, with the counts in the
  state colours.
- When everything doesn't fit, `· showing <i>–<j>` is added after the
  count.
- Right: `v back · ? help`.

**Tiles**: one per live (not ended) session.
- Border title: `<project>:<name> <icon> <state>  <age>`. The icon and
  colour match the Sessions pane:
  - red ⚑ needs you;
  - yellow ● your turn;
  - green ● working (starting counts as working).
- Finca tiles are titled `<finca> · <project>:<name> …`, with the finca
  name in FINCAS_COLOR.
- The selected tile has a thick border in ACCENT.
- Body: the bottom lines of the session's screen. Text is shown as is,
  trimmed to the tile's width, with trailing blank lines dropped so the
  latest text sits at the bottom.

**Order**:
- This Mac's sessions first, then each finca's, with fincas in the
  Fincas pane order.
- Within a machine: by project (Projects pane order), then oldest
  session first, as in the Sessions pane.

**Size and scrolling**:
- Tiles are at least 40 columns by 8 rows (including the border).
- Columns = available width / 40 (at least 1), and the width is shared
  evenly between them.
- Rows = available height / 8; rows grow evenly to fill the height.
- When there are more tiles than fit, the grid scrolls by whole rows so
  the selected tile is visible.

**Footer**: `Enter jump · i reply · Tab next waiting · hjkl move`. A
message (an error, `sent to alpha:fix`) replaces it for a few seconds,
like the dashboard footer.

**Empty**: `No live sessions. Esc back, then Enter on a project to start
one.`

## Keys (grid mode)

| Key | Does |
|---|---|
| `h` `j` `k` `l`, arrows | move to the neighbouring tile; at an edge, stay. `j` from the last visible row scrolls. |
| `Tab` / `Shift-Tab` | next / previous tile whose state is needs you or your turn, wrapping; nothing waiting: stay |
| `Enter` | jump to the session's window (local: as Enter on a session today; finca: connect and focus, as on a finca session today) |
| `i` | reply (below); only on a "your turn" session |
| `v` `Esc` | back to the dashboard |
| `?` | help (gains a Grid section) |
| `q` | detach, as on the dashboard |

`i` on a session that's in any other state shows a message in the footer
and opens nothing:
- needs you: `alpha:s1 needs you: Enter to answer it there`;
- working or starting: `alpha:s1 is working; wait for your turn`.

## Replying

- `i` opens a one-line input under the grid: `reply ▸ <text>` above
  `Enter send · Esc cancel`. Editing works as in Grove's other prompts:
  typing, Backspace, `C-u` to clear the line.
- `Enter` sends. On success the footer shows `sent to <handle>`. On
  failure the error shows and the typed text is kept, so it can be fixed
  and sent again.
- `Esc` cancels.

**Shared rules**: the grid and the MCP `send_prompt` tool use one shared
function, `prompt::type_into(tmux, store, cfg, session, text)`. Its
checks, in order:
1. the session exists and is live;
2. its state is your turn;
3. control characters other than `\n` and `\t` are removed;
4. the text isn't empty;
5. no line starts with `!` or `/` (counting `\n`, U+2028 and U+2029 as
   line breaks);
6. the tmux window still belongs to the session (`@grove_session`, or
   `@grove_account` for older windows);
7. the text is pasted (its own buffer, through stdin), then a short
   pause, then Enter.

`mcp.rs` keeps its own checks that come before this: the account scope,
and refusing its own session.

**Finca sessions**:
- The dashboard runs `grove type-prompt <session-id>` on the finca, with
  the text on stdin, over the finca's shared SSH connection (the
  destination that last answered).
- `type-prompt` is a hidden command. It runs `prompt::type_into` there
  and exits 1 with the error message if a check fails.
- A finca whose Grove lacks the command: `<finca>'s Grove is too old to
  reply from here: update it (grove finca update)`.

## Live screens

**This Mac**:
- While the grid is open, every 500 ms the dashboard runs `tmux
  capture-pane -p -J` (no `-e`, so no escape codes) on each visible
  tile's window, for its height in lines.
- A window that fails to capture shows `(window gone)` in the tile.

**Fincas**:
- A new hidden command: `grove screens --json [--lines N]` (default 40,
  max 200). It prints the live sessions with their screens:

  ```json
  {"version":1,"host":"pi","sessions":[
    {"id":"…","project":"blog","name":"s2","state":"working",
     "updated_at":"…","window":"@3","screen":"…"}]}
  ```

  `screen` is empty for a session with no window, or whose window can't
  be captured.
- `screens::Fetcher` is a background thread, started when the grid opens
  and stopped when it closes. Every 2 s it asks each reachable finca for
  its screens in parallel. It runs `<grove_bin> screens --json --lines
  <tile height>` through the shared connection (the same options as the
  status poll), with a 5 s timeout. Results arrive on a channel, as the
  status poll's do.
- A finca's tiles come from its latest `screens` result. Before the
  first result arrives, they come from the latest status poll, with no
  screen.
- **Stale**: a finca whose last good result is more than 6 s old has its
  tiles drawn dimmed (dark grey border, grey title and badge) with
  `stale` in the title, so a frozen screen doesn't keep showing "your
  turn" in colour. Reply is refused on a stale tile: `<finca> isn't
  answering`.
- **Older Grove** (the command is unknown there): tiles come from the
  status poll (state and message, no screen). One footer note per finca
  when the grid opens: `update Grove on <finca> for live screens`.
- An unreachable finca has no tiles. The top line notes `· <finca>
  unreachable`.

## Structure

- `src/prompt.rs` (new): `type_into` and its checks, moved out of
  `mcp.rs`. `mcp.rs` calls it.
- `src/screens.rs` (new):
  - `Screens` and `ScreenSession` (serde types for the JSON);
  - `screens(store, tmux, lines) -> Screens` (local);
  - the `Fetcher` thread;
  - `type_prompt_remote(finca, dest, session_id, text)`.
- `src/ui/grid.rs` (new):
  - `Layout { cols, rows, tile_w, tile_h }`;
  - `layout(n, area)` and `visible(scroll, …)`, pure;
  - `Tile` (origin, handle, state, age, screen, stale);
  - `tiles(app) -> Vec<Tile>`;
  - `move_sel(dir)` and `next_waiting(from, forward)`, pure;
  - `render_grid`.
- `src/ui/app.rs`:
  - `Mode::Grid { sel, scroll, reply: Option<String> }`;
  - the app holds local screens and the finca screen results;
  - `apply_screens(result)`.
- `src/ui/input.rs`: `v` in Normal mode; the grid keys.
- `src/ui/run.rs`:
  - capture local screens while in grid mode;
  - start and stop the Fetcher;
  - new actions `GridReply` and `GridJump` (reusing the existing jump
    actions).
- `src/ui/render.rs`: draw the grid in grid mode.
- `src/main.rs`: hidden `screens` and `type-prompt` subcommands.
- `src/ui/help.rs`: a Grid section; dashboard hints gain `v grid`.
- README: a short "Session grid" section and the key table.

## Testing

- **Layout** (pure):
  - columns and rows at 80×24, 110×32, 200×60;
  - one tile fills the area;
  - tiles never get smaller than the minimum;
  - scrolling keeps the selection visible;
  - `showing i–j`.
- **Movement** (pure):
  - `hjkl` at the edges and on a short last row (`l` from a tile with no
    right neighbour stays put; `j` onto a short last row goes to its last
    tile);
  - `Tab` and `Shift-Tab` wrap and skip working tiles;
  - no waiting tiles means `Tab` stays put.
- **Tiles**:
  - order across this Mac and fincas;
  - ended sessions are left out;
  - finca tiles before their first screens result;
  - stale after 6 s;
  - an older finca's tiles have no screen.
- **Keys** (ui tests):
  - `v` opens the grid and `Esc` closes it;
  - `i` on your turn opens the reply line;
  - `i` on needs you or working shows the message;
  - `Enter` in the reply line produces `GridReply` with the text;
  - `Esc` keeps the text unsent.
- **Render** (screen() previews): the grid at laptop size with mixed
  states, a finca tile, a stale tile, scrolling, and the reply line.
- **`prompt::type_into`**: the existing send_prompt tests move to it.
  mcp tests check the account scope and the own-session refusal still
  apply.
- **`screens`**, on a test tmux server:
  - JSON with a captured screen;
  - an ended session left out;
  - a missing window gives an empty screen.
- **`type-prompt`**, on a test tmux server:
  - delivers to a your-turn session;
  - exits 1 with the message for the others.
- **Live**: open the grid with sessions here, on the Pi and on therver;
  `Tab` to a waiting one; reply with `i` to a local and to a finca
  session; `Enter` into a finca session and `C-g` back.

## Out of scope

- Answering permission prompts from the grid.
- Colour in tile screens (would need parsing escape codes safely).
- Killing, renaming or starting sessions from the grid (Esc to the
  dashboard for those).
- Number keys to jump to a tile.
- Filtering tiles by machine or project.
