# TODO priorities and due dates

**Date:** 2026-11-19
**Status:** Approved

## Why

Some TODO lists carry real plans: bookings, payments, deadlines. A flat
checklist doesn't show what matters most or what's due soon.

## Approaches

- **Three bands (picked):** high at the top, unmarked items in the middle,
  low at the bottom. Existing lists keep working untouched, and items with
  no priority (including Claude's) have a natural home.
- **Two sections only:** every item is high or low. Existing items would
  all have to be put in one, and every new item needs a choice.
- **Custom sections** ("Bookings", "Payments"): left for later.

## Design

### File

Still a Markdown checklist in `~/.grove/todos/<project>.md`:

```
## High priority
- [ ] Pay the deposit (due 2026-12-01)

## Normal
- [ ] Update the budget sheet

## Low priority
- [ ] Buy a car seat
```

- Items before any heading are normal, so today's lists parse as they are.
- A heading starting "High" (any case) opens the high band, "Low" the low
  band; any other `##` heading is normal.
- Grove writes the headings itself. A list with only normal items is
  written as today, with no headings.
- A due date is ` (due YYYY-MM-DD)` at the end of the item's text.
- Archived (cleared) items keep their due date as text.

### Dashboard

- Items show grouped: high, normal, low. HIGH and LOW headers appear only
  when that band has items; the middle has no header.
- The due date is right-aligned as `Dec 1` (with the year when it isn't
  this year). Within 3 days: warning colour. Overdue: warning colour and
  `!`. Done items stay dim.

### Keys

- `+` / `-`: raise or lower the item one band. It goes to the nearest edge
  of that band (the end of the band above, the start of the band below)
  and the cursor follows it.
- `@`: asks for a due date. Accepts `2026-12-01`, `12/1` or `12-1` (next
  such date), `dec 1` / `1 dec`, a weekday (`fri`, next one), `today`,
  `tomorrow`, `+N` (days). Empty clears it. Unparseable: says so, no change.
- `n` / `O` add in the cursor's band.

### Claude (MCP and session context)

- `get_todos` shows each item's priority and due date.
- `edit_todos`: `add` and `reword` take optional `priority`
  (high/normal/low) and `due` (YYYY-MM-DD, or "" to clear); a new `set` op
  changes only those. Numbers are as `get_todos` shows them.
- The session context lists open items high first with their due dates,
  and tells Claude to set a priority or due date only when the user gave
  one, never invented.

## Non-goals

Sorting by date, custom sections, due-soon or overdue warnings on the
project row, recurring items.

## Left to the builder

Key names that clash with existing todo keys, the exact date parser, and
how numbers map when bands reorder items.
