# Finca setup: adding a machine with as little by hand as possible

## Goal

Adding a finca today means preparing the machine yourself (SSH key, tmux,
Claude Code, a Claude login, installing Grove with `scripts/deploy-fincas.sh`)
and then running `grove finca add`. The goal: one command, or one key on the
dashboard, that takes a reachable machine to a live finca on the dashboard.
It does every step it can and asks before each one that changes something.
The steps that need you (a password, a Claude login, a sudo prompt) happen
inside the same flow.

Success: from the dashboard, press `F`, type `me@newbox.local`, answer a few
y/n questions and type one password. The finca then appears under FINCAS
with its sessions.

## Entry points

- **`grove finca setup [dest]`**: an interactive, line-based wizard in the
  terminal. It asks for `dest` if none is given. It needs a TTY.
- **Dashboard `F`** (anywhere), plus **`a` while a finca is selected**:
  - opens a prompt, "Add finca: SSH destination". Tab completes `Host`
    aliases from `~/.ssh/config` (no wildcards) and host names from
    `~/.ssh/known_hosts` (unhashed entries only).
  - Enter runs `grove finca setup <dest>` in a tmux popup (like the editor).
    When the popup closes, the dashboard reloads and polls the fincas.
- **`grove finca add`** stays as it is: non-interactive, for scripts.

## The wizard

Each step checks first, says what it found and skips itself when there's
nothing to do. A step that changes something asks `[Y/n]` first. Declining a
step that later steps need ends the wizard with what to do by hand, and
nothing is saved.

1. **SSH.** Try `ssh -o BatchMode=yes dest true` (with Grove's shared
   connection options).
   - On a host-key failure, run one interactive `ssh -o
     StrictHostKeyChecking=accept-new dest true`. That accepts the key and
     prompts for a password if one is needed; it doesn't fix key auth.
   - Without key auth: if there is no `~/.ssh/id_ed25519.pub` (or any
     `id_*.pub`), offer `ssh-keygen -t ed25519`, run interactively so you
     choose the passphrase. Then offer `ssh-copy-id dest`, where you type
     the password once. Re-check with BatchMode.
2. **Machine facts**, from one SSH call under the login shell:
   - `uname -s`, `uname -m` (aarch64 is treated as arm64), `hostname -s`;
   - `command -v` for `tmux`, `claude`, `cargo`, `brew`, `apt-get`, `dnf`;
   - the first of the existing grove search paths that holds a working
     grove, with its `--version`.

   The default finca name is the host name. The wizard asks for a name,
   with Enter taking the default, and refuses one already in use.
3. **tmux and Claude Code.** For each one missing, it shows the install
   command and offers to run it, over `ssh -t` so sudo can prompt:

   | Tool | macOS | Linux |
   |---|---|---|
   | tmux | `brew install tmux` (needs brew) | `sudo apt-get install -y tmux` or `sudo dnf install -y tmux` |
   | Claude Code | `curl -fsSL https://claude.ai/install.sh \| bash` | same |

   With no package manager it knows (or no brew on macOS), it prints what to
   install and stops. After installing, it re-checks with a fresh login
   shell.
4. **Grove.** Grove goes to `~/.local/bin/grove` unless one was found in
   step 2.
   - **Already there and the same version:** skipped.
   - **Same OS and CPU as here:** copies this binary (`current_exe`) over
     the shared connection: write `grove.new`, then `mv -f`, as the deploy
     script does.
   - **Different:** builds from source there, from this binary's own
     source folder, recorded at build time as `CARGO_MANIFEST_DIR`. The
     wizard checks that folder still has `Cargo.toml`. It rsyncs the source
     to `~/src/grove` (no `target`, `.git` or `.superpowers`), runs `cargo
     build --release` and installs the result. If the machine has no
     `cargo`, it offers the rustup installer (`curl --proto '=https'
     --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y`) first. If the
     source folder is gone, it says to install Grove there by hand and
     stops.
5. **Claude login.** Run `claude auth status` there under the login shell
   (exit 0 means logged in). If logged out, offer to run `claude auth
   login` over `ssh -t`, then re-check. This uses Claude's default config
   folder there. A finca whose Grove sets `claude_config_dir` is checked as
   is, and the wizard says so.
6. **Backup route.** If `tailscale` is on this machine,
   `tailscale status --json` lists the peers. If one has the same
   `HostName` as the finca (ignoring case), offer its `DNSName`, without
   the trailing dot, as the backup. Skip quietly if there's no tailscale,
   no match, or the user is logged out. Then confirm that the backup
   answers over SSH.
7. **Save.** Write `fincas/<slug>.toml` with the name, ssh, backup and
   grove_bin, the same way `grove finca add` does. Print a summary of what
   was done. The dashboard's finca watcher picks up the file and polls
   right away.

Running the wizard against an existing finca's destination is allowed: it
works as a health check and repair. Saving offers to update that finca's
file and won't create a duplicate.

## `grove finca update [names]`

Step 4 is extracted as a function, `install_grove(finca, dest)`, and exposed
as `grove finca update`, for every finca or just the named ones. It tries
each finca's ssh, then its backup, and prints each one's resulting version.
`scripts/deploy-fincas.sh` becomes `exec grove finca update "$@"` after a
`cargo install --path .`, so it still deploys the working tree.

## Structure

- `src/setup.rs` (new):
  - `Facts`, parsed from step 2's output (pure, tested on strings);
  - `install_command(tool, &Facts) -> Option<String>` (pure);
  - `tailscale_backup(json, host) -> Option<String>` (pure);
  - the wizard itself, written against a small `Io` trait (ask y/n, ask
    text, say), so tests drive it with scripted answers. Remote commands go
    through the existing `ssh_program()`, so tests use the fake ssh from
    `finca::testing`.
- `src/finca.rs`: `install_grove`, sharing the scp and swap logic. A
  `run_interactive(dest, cmd)` for `ssh -t` steps.
- `src/ui/`: the `F` key, the `PromptKind::AddFinca` prompt with SSH host
  completion (`complete.rs` gains `complete_ssh_host`) and
  `Action::SetupFinca { dest }`, which runs the popup.
- `src/main.rs`: `finca setup` and `finca update` subcommands.

## Testing

- Pure parsers and decisions: facts output, install commands per OS and
  package manager, tailscale JSON matching, ssh config and known_hosts
  host lists.
- Wizard flows with the fake ssh and scripted answers:
  - all present (nothing asked, finca saved);
  - grove missing on the same OS (copied);
  - a declined required step (nothing saved);
  - a name clash.
- `install_grove` against a fake remote home: `.new` then the swap.
- Dashboard: `F` opens the prompt; Enter gives `SetupFinca`; Tab completes
  from a temporary ssh config.
- Live: run the wizard against `therver`, where everything is present, as a
  health check. Nothing should be asked and its file should stay unchanged.

## Out of scope

- Discovering machines on the network (Bonjour/mDNS).
- Windows fincas.
- Installing Homebrew itself.
- Choosing a Claude config folder on the finca.
