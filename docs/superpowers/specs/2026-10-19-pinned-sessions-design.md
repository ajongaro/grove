# Pinned sessions

## Goal

The Sessions pane shows one project's sessions. When several sessions
across projects, both Claude accounts and fincas are in active use,
getting back to each means picking its project first. Pins keep the
sessions you're working in at the top of the right column, wherever they
live, one key away.

Success:
- `p` on any live session, local or on a finca, pins it; it shows in a
  Pinned box at the top of the right column with its state.
- `1`–`9` jump straight to a pin's window from the dashboard.
- A pinned session that ends drops out of the box on its own.

## The box

- A **Pinned** box at the top of the right column, above Sessions (or
  above a finca's detail when a finca is selected). Hidden when nothing
  is pinned.
- Its height fits its pins: at most 9 rows, then it scrolls (the
  selection stays visible). Sessions and To-do share what's left.
- One row per pin:

  ```
  1  ● play · grove:s2      your turn  3m   Want me to commit?
  2  ⚑ pi · blog:s1         needs you  1m   Bash: rm -rf build
  ```

  - its number (1–9; rows past 9 have none);
  - the state icon and colour, as in Sessions (⚑ needs you red, ● your
    turn yellow, ● working green);
  - where it lives: the account name in its box colour, or the finca
    name in FINCAS_COLOR;
  - `project:name`, state, age, and Claude's latest message, trimmed to
    the width.
- Order: the order pinned, newest at the bottom, so numbers stay put.
- Title: `PINNED`, with the border in ACCENT when the pane is focused,
  like the other panes.

## Keys

| Where | Key | Does |
|---|---|---|
| Sessions, Fincas (on a session), Pinned | `p` | pin / unpin the session |
| Dashboard (not in the to-do list) | `1`–`9` | jump to that pin's window |
| Pinned | `j` `k` | move |
| Pinned | `Enter` | jump to its window |
| Pinned | `x` | kill it (y/n) |

- `p` in the to-do list stays paste.
- Jumping: a local pin selects its window (as Enter on a session); a
  finca pin connects and focuses (as Enter on a finca session).
- `x` on a local pin works as in Sessions; on a finca pin it runs
  `grove kill-session` there, as the grid does.
- A number with no pin, or a pin whose finca isn't answering: a footer
  message, nothing else.
- Moving between panes:
  - Tab cycles Projects → Pinned → Sessions → To-do (Pinned only when
    shown);
  - `k` at the top of Sessions goes up into Pinned; `j` at the bottom of
    Pinned goes down into Sessions;
  - `l` still goes to Sessions, `h` to Projects.
- Messages:
  - pinned: `Pinned grove:s2 (3)`; unpinned: `Unpinned grove:s2`;
  - `p` on something that isn't a live session (a finca shell, an ended
    session): nothing;
  - a finca session without an id: `update Grove on <finca> to pin its
    sessions`.

## State

- `~/.grove/pins.json`: `{"version":1,"pins":[{"finca":null,"id":"…"},
  {"finca":"pi","id":"…"}]}`, in pin order. Grove's own folder, not a
  Claude account's, so pins span accounts. Written atomically on each
  change; a missing or unreadable file means no pins.
- **Pruning** (on each reload and poll result):
  - a local pin goes when its session is ended or gone;
  - a finca pin goes when a successful poll of that finca doesn't list
    its id;
  - a finca whose last poll failed keeps its pins: they're drawn dimmed
    with `unreachable`, and Enter, `x` and number keys on them say
    `<finca> isn't answering`.
  - a finca removed from Grove: its pins go.
- **Finca session ids**: `grove status --json` gains an optional `id` on
  each session (additive; the status version stays 1). A finca on an
  older Grove reports none, so its sessions can't be pinned until it's
  updated.

## Structure

- `src/pins.rs` (new): `Pin { finca: Option<String>, id: String }`,
  `load(paths)`, `save(paths, &[Pin])`, and pure `prune(pins, local,
  fincas) -> Vec<Pin>`.
- `src/status.rs`: `SessionStatus.id: Option<String>`.
- `src/ui/app.rs`:
  - `Pane::Pinned`; `pins: Vec<Pin>`, `pin_idx`;
  - `pinned_rows(now)`: the rows to draw and act on (pin, origin,
    handle, state, age, message, window, unreachable);
  - `toggle_pin(...)`.
- `src/ui/input.rs`: `p`, `1`–`9`, Pinned pane keys, Tab and j/k
  crossing.
- `src/ui/render.rs`: the Pinned box and the right column split.
- `src/ui/help.rs`: a Pinned section; Sessions and Fincas gain `p`.
- README: a short "Pinned sessions" section and the key table.

## Testing

- `pins`: save/load round trip; missing or bad file is empty; prune
  drops ended and missing local sessions and finca sessions absent from
  a good poll, keeps an unreachable finca's, drops a removed finca's.
- `status`: JSON includes session ids; an old status without them still
  parses.
- Keys:
  - `p` pins from Sessions and from a finca session, unpins from Pinned
    and from Sessions; `p` on a finca shell does nothing; a finca session
    without an id gives the update message;
  - `1`–`9` produce Jump / FincaJump, and a message when empty or
    unreachable; inactive in the to-do list;
  - Tab order with and without pins; `k`/`j` crossing;
  - `x` in Pinned confirms, then kills locally or remotely.
- Render: the box with a play, a work and a finca pin; an unreachable
  pin dimmed; hidden with no pins; help still fits at laptop size.
- Pins survive a dashboard restart (load on start).

## Out of scope

- Reordering pins (unpin and re-pin moves one to the bottom).
- Pins in the session grid.
- Pinning projects.
- Number keys inside the grid.
