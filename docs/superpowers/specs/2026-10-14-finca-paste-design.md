# Finca paste: screenshots and files into finca sessions

## Goal

In a Claude session on a finca, pasting a screenshot doesn't work: the
image is on this machine's clipboard, and only keystrokes cross SSH. Today
that means saving the file, copying it over, and typing its path by hand.

Goal: in a window connected to a finca, `C-v` sends this machine's
clipboard image (or a file copied in Finder) to the finca and pastes its
path there. Claude Code on the finca attaches an image from a pasted path,
as it does when a file is dragged in.

Success: take a screenshot with ⌘⇧⌃4, press `C-v` in a finca's Claude
session, and see the image attached there within a second or two.

## Behavior

`C-v` is bound in the `grove-finca` and `grove-shell` key tables, the
tables of finca connections and finca shells (runfiles.rs). Local
sessions (the root table) are untouched, so `C-v` in a local Claude
session still does what Claude does.

The binding runs `grove paste --pane #{pane_id} --session #{session_name}`
in the background (`run-shell -b`), so the client never freezes. It then
does the first of these that applies:

1. **Image on the clipboard:** write it out as a PNG and send it.
2. **A file reference on the clipboard** (Finder ⌘C): send that file as
   it is. If several are copied, the first one is sent.
3. **Anything else** (text, empty, or clipboard reading unsupported here):
   send `C-v` on to the pane (`send-keys -t <pane> C-v`), so it reaches the
   finca as if the binding weren't there.

After a successful send, the remote path plus a trailing space is pasted
into the pane as a paste (`set-buffer` + `paste-buffer -p -d`, bracketed
when the program asked for it), like a dragged-in file. The status line
shows `sent <name> to <finca>`. A failure shows `couldn't send to
<finca>: <reason>` and pastes nothing.

## Sending

`grove send <finca> [file]`: sends `file`, or the clipboard (as in steps 1
and 2) when no file is given. It prints the path on the finca. It exits 1
with a message when there's nothing to send or the send fails.

- **Destination:** finca sessions record the address they connected
  through as the session option `@grove_dest`, set in
  `ensure_finca_session` and `ensure_shell_session`. `grove paste` reads
  it from `--session`. `grove send`, and a session without the option,
  try the finca's `ssh` then its `backup`, taking the first that answers.
- **Transfer:** stream the file over the existing shared connection
  (`remote_stdin`, with `ControlMaster` options). It runs one remote
  command that:
  - creates `~/.grove/inbox`;
  - writes `<name>.part`, then renames it to `<name>`;
  - deletes inbox files older than 7 days (`find -mtime +7`);
  - prints the absolute path.
- **Name:** `YYYYMMDD-HHMMSS-<original name>`. The original name keeps
  only letters, digits, `.`, `-` and `_` (others become `-`), and is
  `clipboard.png` for a clipboard image.

## Reading the clipboard (macOS)

Built-in `osascript`, so nothing needs installing:
- **Image:** `the clipboard as «class PNGf»`, written to a temporary file
  under `~/.grove/run/`. That file is deleted after sending.
- **File reference:** `POSIX path of (the clipboard as «class furl»)`.
- Each one fails cleanly (non-zero exit) when the clipboard holds
  something else. The image check runs first.

On other systems, clipboard reading reports "unsupported", so `C-v`
passes through and `grove send` without a file says it can't read the
clipboard here. Passing a file always works.

## Structure

- `src/transfer.rs` (new):
  - `remote_name(original, now) -> String` (pure);
  - `Clip` (`Image(PathBuf)`, `File(PathBuf)`, `Nothing`) and
    `read_clipboard(run_dir) -> Clip`, which shells out to osascript;
    tests replace osascript with a fake via `GROVE_OSASCRIPT`;
  - `send(f, dest, file, name) -> Result<String>`, which returns the
    remote path;
  - `paste(tmux, store, session, pane)`: the `C-v` flow.
- `src/finca.rs`: `@grove_dest` on both session kinds;
  `finca_for_session(name)`, which maps `finca-<slug>` and
  `finca-<slug>-sh` to the slug.
- `src/runfiles.rs`: the two `C-v` bindings.
- `src/main.rs`: `send` and a hidden `paste` subcommand.
- `src/tmux.rs`: `send_keys`, `paste_text` and `display_message`
  helpers, if they don't already exist.
- README: the Fincas section and the key table; help.rs `Anywhere` gets
  `C-v`.

## Testing

- **Pure:** name cleaning and the timestamp; mapping a session name to a
  slug.
- **send:** against the fake ssh and a fake remote home. Check:
  - the file lands under `inbox/` with the right bytes, and no `.part` is
    left;
  - an 8-day-old inbox file is removed;
  - the printed path is absolute.
- **read_clipboard:** with a fake osascript, for an image, a file and
  text.
- **paste:** on a test tmux server (like the focus test in `tests/cli.rs`)
  with a fake ssh and a fake osascript:
  - an image ends with the path in the pane;
  - text sends `C-v` through;
  - a failed send pastes nothing.
- **Generated config:** it contains both `C-v` bindings.
- **Live:** on pi and therver, paste a screenshot into a Claude session
  and confirm it attaches as an image. Also check a Finder-copied file,
  and text passing through.

## Out of scope

- Reading the clipboard on Linux or Windows (wl-paste/xclip could come
  later).
- Sending from a finca back to this machine.
- Several files in one paste.
- Progress display for large files.
