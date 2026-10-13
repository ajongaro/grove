# Finca Setup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `grove finca setup [dest]` (and `F` on the dashboard) takes a reachable machine to a live finca. It sets up SSH keys, installs tmux, Claude Code and Grove, logs in to Claude and finds a Tailscale backup, asking before each change. `grove finca update` replaces the deploy script's logic.

**Architecture:**
- A new `src/setup.rs` holds pure parsers and decisions, plus a wizard written against an `Io` trait. Scripted answers drive the wizard in tests; the terminal drives it in real use.
- Remote work goes through `finca.rs` helpers built on the existing `ssh_program()` and shared-connection options, so the existing fake ssh (`finca::testing`) runs the "remote" commands locally against a fake home.
- Interactive steps (password, sudo, Claude login) are argv lists handed to `Io::interactive`. That runs them on the TTY for real, and records them in tests.

**Tech Stack:** Rust 2024, clap, ratatui, tmux CLI, OpenSSH, POSIX sh, tar.

**Spec:** `docs/superpowers/specs/2026-10-13-finca-setup-design.md`

## Global Constraints

- Grove never edits `~/.claude*/settings.json`.
- Every step that changes something asks `[Y/n]` first. Declining a needed step ends the wizard with what to do by hand, and saves nothing.
- Install commands, verbatim:
  - tmux: `brew install tmux` (macOS, needs brew), `sudo apt-get install -y tmux` or `sudo dnf install -y tmux` (Linux);
  - Claude Code: `curl -fsSL https://claude.ai/install.sh | bash`;
  - Rust: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y`.
- Claude login check: `claude auth status`, where exit 0 means logged in. Login: `claude auth login`.
- Grove goes to `<remote $HOME>/.local/bin/grove` unless one was found in the existing search paths. The binary is replaced by writing `<bin>.new`, then `mv -f`.
- The source build excludes `target`, `.git` and `.superpowers`, and goes to `~/src/grove` on the finca.
- No personal hosts or names in code, tests or docs. Use generic examples (`homebox`, `me@homebox.local`).
- Every commit ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Run `cargo test` and `cargo clippy --all-targets -- -D warnings` before each commit.

## Review Focus

1. **Claude installed but not on the login PATH.** The installer puts it in `~/.local/bin`, which may not be on that PATH. The re-check must say exactly that and stop, never loop or claim success. Test: Task 4, `claude_installed_off_path_stops_with_a_hint`.
2. **Re-running against an existing finca** must not ask for a name, must not create a second file, and must keep its backup. Test: Task 4, `rerun_on_an_existing_finca_is_a_health_check`.
3. **Destination with a user or alias and a host-key prompt.** On "Host key verification failed", accept the key once. Treat "Permission denied" separately. Test: Task 4, `unknown_host_key_is_accepted_once`.
4. **Source folder gone** (Grove installed from a deleted checkout) on a different-CPU finca. Fail with "install Grove there by hand"; never stream an empty tarball. Test: Task 3, `source_build_without_source_folder_fails_clearly`.
5. **Dashboard prompt with `user@` typed before Tab.** Completion keeps `user@` and completes only the host part. Test: Task 5, `ssh_completion_keeps_the_user_part`.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/setup.rs` (new) | `Facts` and its parser, install commands, Tailscale match, ssh host lists, the `Io` trait, `TerminalIo`, the wizard `Setup` |
| `src/setup_tests.rs` (new) | tests for the above |
| `src/finca.rs` | `run_remote`, `remote_stdin`, `interactive_ssh`, `facts_script`, `install_grove`; `login_shell_cmd` made `pub` |
| `src/finca_tests.rs` | install tests; `ENV_LOCK`/`with_env` made `pub(crate)` for setup tests |
| `src/store.rs` | `save_finca` (overwrite) |
| `src/main.rs` | `finca setup`, `finca update` |
| `src/ui/{app,input,run,render,help,complete}.rs` | `F`/`a` prompt, ssh completion, `SetupFinca` popup |
| `scripts/deploy-fincas.sh` | thin wrapper |
| `README.md` | setup docs |

---

### Task 1: Pure building blocks: facts, install commands, Tailscale, ssh hosts

**Files:**
- Create: `src/setup.rs`, `src/setup_tests.rs`
- Modify: `src/main.rs` (add `mod setup;`)

**Interfaces:**
- Produces:
  - `pub struct Facts { pub os: String, pub arch: String, pub host: String, pub home: String, pub tools: BTreeSet<String>, pub grove: Option<(String, String)> }`, where `os`/`arch` are in `std::env::consts` form (`macos`/`linux`, `aarch64`/`x86_64`) and `grove` is `(path, version line)`;
  - `pub fn parse_facts(out: &str) -> Option<Facts>`;
  - `impl Facts { pub fn has(&self, t: &str) -> bool; pub fn same_platform_as_here(&self) -> bool }`;
  - `pub fn install_command(tool: &str, f: &Facts) -> Option<&'static str>`;
  - `pub fn tailscale_backup(json: &str, host: &str) -> Option<String>`;
  - `pub fn ssh_hosts(ssh_dir: &Path) -> Vec<String>`.

- [ ] **Step 1: Write failing tests** in `src/setup_tests.rs`:

```rust
use super::*;

const MAC: &str = "os=Darwin\narch=arm64\nhost=homebox\nhome=/Users/me\nhas=tmux\nhas=brew\ngrove=/Users/me/.local/bin/grove\ngrove_version=grove 0.1.0\n";

#[test]
fn facts_parse_and_normalize_platform_names() {
    let f = parse_facts(MAC).unwrap();
    assert_eq!((f.os.as_str(), f.arch.as_str()), ("macos", "aarch64"));
    assert_eq!(f.host, "homebox");
    assert_eq!(f.home, "/Users/me");
    assert!(f.has("tmux") && f.has("brew") && !f.has("claude"));
    assert_eq!(
        f.grove,
        Some(("/Users/me/.local/bin/grove".into(), "grove 0.1.0".into()))
    );
    let pi = parse_facts("os=Linux\narch=aarch64\nhost=pi\nhome=/home/me\n").unwrap();
    assert_eq!((pi.os.as_str(), pi.arch.as_str(), pi.grove.clone()), ("linux", "aarch64", None));
}

#[test]
fn facts_need_os_arch_host_and_home() {
    assert!(parse_facts("os=Linux\narch=x86_64\n").is_none());
    // A login profile's chatter before the facts is ignored.
    assert!(parse_facts(&format!("Welcome!\n{MAC}")).is_some());
}

#[test]
fn install_commands_per_os_and_package_manager() {
    let mut f = parse_facts(MAC).unwrap();
    assert_eq!(install_command("tmux", &f), Some("brew install tmux"));
    f.tools.remove("brew");
    assert_eq!(install_command("tmux", &f), None, "no brew: say what to install");
    let mut l = parse_facts("os=Linux\narch=aarch64\nhost=pi\nhome=/home/me\nhas=apt-get\n").unwrap();
    assert_eq!(install_command("tmux", &l), Some("sudo apt-get install -y tmux"));
    l.tools = ["dnf".to_string()].into();
    assert_eq!(install_command("tmux", &l), Some("sudo dnf install -y tmux"));
    assert_eq!(
        install_command("claude", &l),
        Some("curl -fsSL https://claude.ai/install.sh | bash")
    );
    assert_eq!(
        install_command("cargo", &l),
        Some("curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y")
    );
}

#[test]
fn tailscale_backup_matches_host_name_ignoring_case() {
    let json = r#"{"Peer":{"k1":{"HostName":"HomeBox","DNSName":"homebox.tail1234.ts.net."},"k2":{"HostName":"pi","DNSName":"pi.tail1234.ts.net."}}}"#;
    assert_eq!(tailscale_backup(json, "homebox").as_deref(), Some("homebox.tail1234.ts.net"));
    assert_eq!(tailscale_backup(json, "laptop"), None);
    assert_eq!(tailscale_backup(r#"{"Peer":null}"#, "pi"), None, "logged out");
    assert_eq!(tailscale_backup("not json", "pi"), None);
}

#[test]
fn ssh_hosts_from_config_and_known_hosts() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(
        d.path().join("config"),
        "Host homebox pi\n  User me\nHost *.example *\nhost  lower\nMatch all\n",
    )
    .unwrap();
    std::fs::write(
        d.path().join("known_hosts"),
        "homebox.local,192.168.1.5 ssh-ed25519 AAAA\n|1|hashed= ssh-ed25519 AAAA\n[nas]:2222 ssh-rsa AAAA\npi ssh-ed25519 AAAA\n",
    )
    .unwrap();
    assert_eq!(
        ssh_hosts(d.path()),
        ["192.168.1.5", "homebox", "homebox.local", "lower", "pi"]
    );
    assert!(ssh_hosts(&d.path().join("missing")).is_empty());
}
```

- [ ] **Step 2: Run** `cargo test --bin grove setup::` and expect a compile failure (module missing).

- [ ] **Step 3: Implement** `src/setup.rs` (top part; the wizard is added in Task 4):

```rust
//! `grove finca setup`: takes a reachable machine to a working finca, asking
//! before each change. Pure parts here are tested on strings; the wizard
//! runs against `Io`, so tests script the answers.

use std::collections::BTreeSet;
use std::path::Path;

/// What one SSH call learns about a machine (see `finca::facts_script`).
#[derive(Debug, Clone, PartialEq)]
pub struct Facts {
    /// `std::env::consts::OS` form: macos, linux.
    pub os: String,
    /// `std::env::consts::ARCH` form: aarch64, x86_64.
    pub arch: String,
    pub host: String,
    pub home: String,
    /// Tools found: tmux, claude, cargo, brew, apt-get, dnf.
    pub tools: BTreeSet<String>,
    /// A working grove there: (path, `--version` line).
    pub grove: Option<(String, String)>,
}

impl Facts {
    pub fn has(&self, tool: &str) -> bool {
        self.tools.contains(tool)
    }

    pub fn same_platform_as_here(&self) -> bool {
        self.os == std::env::consts::OS && self.arch == std::env::consts::ARCH
    }
}

/// Parses `key=value` lines; other lines (login chatter) are ignored.
pub fn parse_facts(out: &str) -> Option<Facts> {
    let (mut os, mut arch, mut host, mut home) = (None, None, None, None);
    let (mut grove, mut version) = (None, None);
    let mut tools = BTreeSet::new();
    for line in out.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim().to_string();
        match k {
            "os" => os = Some(match v.as_str() {
                "Darwin" => "macos".to_string(),
                "Linux" => "linux".to_string(),
                other => other.to_lowercase(),
            }),
            "arch" => arch = Some(if v == "arm64" { "aarch64".to_string() } else { v }),
            "host" => host = Some(v),
            "home" => home = Some(v),
            "has" => {
                tools.insert(v);
            }
            "grove" => grove = Some(v),
            "grove_version" => version = Some(v),
            _ => {}
        }
    }
    Some(Facts {
        os: os?,
        arch: arch?,
        host: host?,
        home: home?,
        tools,
        grove: grove.zip(version),
    })
}

/// The command that installs `tool` there, if Grove knows one.
pub fn install_command(tool: &str, f: &Facts) -> Option<&'static str> {
    match tool {
        "claude" => Some("curl -fsSL https://claude.ai/install.sh | bash"),
        "cargo" => Some("curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"),
        "tmux" if f.os == "macos" => f.has("brew").then_some("brew install tmux"),
        "tmux" if f.has("apt-get") => Some("sudo apt-get install -y tmux"),
        "tmux" if f.has("dnf") => Some("sudo dnf install -y tmux"),
        _ => None,
    }
}

/// The Tailscale name of the peer whose host name is `host`, from
/// `tailscale status --json`.
pub fn tailscale_backup(json: &str, host: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    v.get("Peer")?.as_object()?.values().find_map(|p| {
        let name = p.get("HostName")?.as_str()?;
        let dns = p.get("DNSName")?.as_str()?.trim_end_matches('.');
        (name.eq_ignore_ascii_case(host) && !dns.is_empty()).then(|| dns.to_string())
    })
}

/// Hosts to offer for a destination: `Host` aliases in `config` (no
/// patterns) and plain names in `known_hosts`; sorted, no duplicates.
pub fn ssh_hosts(ssh_dir: &Path) -> Vec<String> {
    let mut hosts = BTreeSet::new();
    if let Ok(cfg) = std::fs::read_to_string(ssh_dir.join("config")) {
        for line in cfg.lines() {
            let mut words = line.split_whitespace();
            if words.next().is_some_and(|w| w.eq_ignore_ascii_case("host")) {
                hosts.extend(
                    words
                        .filter(|w| !w.contains(['*', '?', '!']))
                        .map(str::to_string),
                );
            }
        }
    }
    if let Ok(known) = std::fs::read_to_string(ssh_dir.join("known_hosts")) {
        for first in known.lines().filter_map(|l| l.split_whitespace().next()) {
            hosts.extend(
                first
                    .split(',')
                    .filter(|h| !h.starts_with('|') && !h.starts_with('['))
                    .map(str::to_string),
            );
        }
    }
    hosts.into_iter().collect()
}

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
```

Add `mod setup;` to `src/main.rs`, between `mod runfiles;` and `mod state;`. Clippy will flag dead code until Task 4 uses these items: put `#![allow(dead_code)]` at the top of `setup.rs` with the comment `// Used by the wizard (next tasks).`, and remove it in Task 4.

- [ ] **Step 4: Run** `cargo test --bin grove setup::` and expect all 5 to pass. Then run `cargo test` and clippy.

- [ ] **Step 5: Commit** `feat(setup): parse finca facts, install commands, tailscale and ssh hosts`.

---

### Task 2: Remote helpers in finca.rs

**Files:**
- Modify: `src/finca.rs`, `src/finca_tests.rs`

**Interfaces:**
- Produces:
  - `pub fn login_shell_cmd(cmd: &str) -> String` (made pub);
  - `pub fn run_remote(dest: &str, cmd: &str) -> Result<std::process::Output>`: batch, shared connection, stdin null;
  - `pub fn remote_stdin(dest: &str, cmd: &str, input: std::process::Stdio) -> Result<std::process::Output>`;
  - `pub fn interactive_ssh(dest: &str, cmd: Option<&str>, extra: &[&str]) -> Vec<String>`: an argv for `ssh -t`;
  - `pub fn facts_script() -> String`.
- In the fake ssh (`testing::fake_ssh`), `$FAKE_PATH`, when set, replaces PATH for the "remote" command.
- `finca_tests.rs`: `pub(crate) static ENV_LOCK`, `pub(crate) fn with_env`; `finca.rs`: `pub(crate) mod tests`.

- [ ] **Step 1: Write failing tests** (append to `src/finca_tests.rs`):

```rust
#[test]
fn facts_script_reports_the_machine() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("remote home");
    std::fs::create_dir_all(home.join(".local/bin")).unwrap();
    fake_grove(&home.join(".local/bin"), "{}");
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    script(&bin, "tmux", "exit 0");
    let ssh = fake_ssh(d.path(), None);
    let out = with_env(&ssh, &home, || {
        unsafe { std::env::set_var("FAKE_PATH", format!("{}:/usr/bin:/bin", bin.display())) };
        let r = run_remote("me@box", &facts_script());
        unsafe { std::env::remove_var("FAKE_PATH") };
        r
    })
    .unwrap();
    let f = crate::setup::parse_facts(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(f.home, home.to_string_lossy());
    assert!(f.has("tmux") && !f.has("claude"));
    assert_eq!(
        f.grove,
        Some((
            home.join(".local/bin/grove").to_string_lossy().into(),
            "grove 0.1.0".into()
        ))
    );
}

#[test]
fn remote_stdin_feeds_the_command() {
    let d = tempfile::tempdir().unwrap();
    let ssh = fake_ssh(d.path(), None);
    let src = d.path().join("in.txt");
    std::fs::write(&src, "hello").unwrap();
    let out = with_env(&ssh, d.path(), || {
        remote_stdin("me@box", "cat", std::fs::File::open(&src).unwrap().into())
    })
    .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hello");
}

#[test]
fn interactive_ssh_has_a_tty_and_the_command() {
    let argv = interactive_ssh("me@box", Some("claude auth login"), &[]);
    assert!(argv.contains(&"-t".to_string()), "{argv:?}");
    assert_eq!(argv.last().unwrap(), &login_shell_cmd("claude auth login"));
    let accept = interactive_ssh("me@box", Some("true"), &["-o", "StrictHostKeyChecking=accept-new"]);
    assert!(accept.windows(2).any(|w| w == ["-o", "StrictHostKeyChecking=accept-new"]));
}
```

- [ ] **Step 2: Run** `cargo test --bin grove finca::` and expect a compile failure.

- [ ] **Step 3: Implement in `src/finca.rs`:**

Make `login_shell_cmd` `pub`. Change `mod tests;` at the bottom to `pub(crate) mod tests;`. In `finca_tests.rs`, make `ENV_LOCK` `pub(crate) static` and `with_env` `pub(crate) fn`. In `testing::fake_ssh`, change the success body's last line to:

```
HOME=\"$FAKE_HOME\" PATH=\"${FAKE_PATH:-$PATH}\" exec sh -c \"$*\"
```

Add:

```rust
/// Runs `cmd` at `dest` without a terminal (BatchMode, shared connection).
pub fn run_remote(dest: &str, cmd: &str) -> Result<std::process::Output> {
    remote_stdin(dest, cmd, std::process::Stdio::null())
}

/// Like `run_remote`, with `input` as the command's stdin.
pub fn remote_stdin(
    dest: &str,
    cmd: &str,
    input: std::process::Stdio,
) -> Result<std::process::Output> {
    std::process::Command::new(ssh_program())
        .args(SSH_BATCH_OPTS)
        .args(share_opts())
        .arg(dest)
        .arg(cmd)
        .stdin(input)
        .output()
        .with_context(|| format!("running {}", ssh_program()))
}

/// argv for an interactive `ssh -t` to `dest`: `cmd` under the login shell,
/// or a login shell when None. `extra` options go before the destination.
pub fn interactive_ssh(dest: &str, cmd: Option<&str>, extra: &[&str]) -> Vec<String> {
    let mut argv = vec![ssh_program(), "-t".to_string()];
    argv.extend(share_opts());
    argv.extend(extra.iter().map(|s| s.to_string()));
    argv.push(dest.to_string());
    argv.extend(cmd.map(login_shell_cmd));
    argv
}

/// POSIX sh printing `key=value` facts for `setup::parse_facts`.
pub fn facts_script() -> String {
    format!(
        concat!(
            "echo \"os=$(uname -s)\"; echo \"arch=$(uname -m)\"; ",
            "echo \"host=$(hostname -s 2>/dev/null || hostname)\"; echo \"home=$HOME\"; ",
            "for t in tmux claude cargo brew apt-get dnf; do ",
            "command -v \"$t\" >/dev/null 2>&1 && echo \"has=$t\"; done; ",
            "[ -x \"$HOME/.cargo/bin/cargo\" ] && echo has=cargo; ",
            "for b in {}; do if [ -x \"$b\" ] && v=$(\"$b\" --version 2>/dev/null); then ",
            "echo \"grove=$b\"; echo \"grove_version=$v\"; break; fi; done; true"
        ),
        CANDIDATES.join(" ")
    )
}
```

(A duplicate `has=cargo` line is harmless, since `tools` is a set.)

- [ ] **Step 4: Run** `cargo test --bin grove finca::`, then run `cargo test` and clippy. All should pass.

- [ ] **Step 5: Commit** `feat(fincas): remote run, stdin, interactive ssh and facts helpers`.

---

### Task 3: Installing Grove on a finca, and `grove finca update`

**Files:**
- Modify: `src/finca.rs`, `src/finca_tests.rs`, `src/main.rs`, `scripts/deploy-fincas.sh`

**Interfaces:**
- Consumes: `Facts` (Task 1); `remote_stdin`, `run_remote`, `login_shell_cmd` (Task 2).
- Produces:
  - `pub fn install_grove(dest: &str, facts: &crate::setup::Facts, bin: &str, exe: &Path, source: Option<&Path>, wrap: fn(&str) -> String) -> Result<String>`, which returns the new `--version` line;
  - `pub fn source_dir() -> Option<PathBuf>`;
  - CLI `grove finca update [names...]`.

`wrap` is `login_shell_cmd` in real use. Tests pass `plain` (`|c| c.to_string()`), so the "remote" build doesn't depend on the test machine's login profile.

- [ ] **Step 1: Write failing tests:**

```rust
fn plain(c: &str) -> String {
    c.to_string()
}

fn facts_for(home: &std::path::Path, same_platform: bool) -> crate::setup::Facts {
    crate::setup::Facts {
        os: if same_platform { std::env::consts::OS.into() } else { "plan9".into() },
        arch: std::env::consts::ARCH.into(),
        host: "homebox".into(),
        home: home.to_string_lossy().into(),
        tools: ["cargo".to_string()].into(),
        grove: None,
    }
}

#[test]
fn same_platform_install_copies_this_binary_via_a_swap() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("remote home");
    std::fs::create_dir_all(&home).unwrap();
    let exe = fake_grove(d.path(), "{}");
    let bin = home.join(".local/bin/grove");
    let ssh = fake_ssh(d.path(), None);
    let v = with_env(&ssh, &home, || {
        install_grove("me@box", &facts_for(&home, true), &bin.to_string_lossy(), &exe, None, plain)
    })
    .unwrap();
    assert_eq!(v, "grove 0.1.0");
    assert_eq!(std::fs::read(&bin).unwrap(), std::fs::read(&exe).unwrap());
    assert!(!home.join(".local/bin/grove.new").exists(), "swapped into place");
}

#[test]
fn other_platform_ships_source_without_target_or_git() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("remote home");
    let src = d.path().join("src");
    for p in ["target/release", ".git", "src"] {
        std::fs::create_dir_all(src.join(p)).unwrap();
    }
    std::fs::write(src.join("Cargo.toml"), "[package]").unwrap();
    std::fs::write(src.join("src/main.rs"), "fn main(){}").unwrap();
    std::fs::write(src.join("target/release/big"), "x").unwrap();
    std::fs::create_dir_all(home.join("src/grove")).unwrap();
    std::fs::write(home.join("src/grove/stale.rs"), "old").unwrap();
    // A fake cargo "build" that installs a grove into target/release.
    let bin_dir = home.join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let grove = fake_grove(d.path(), "{}");
    script(&bin_dir, "cargo", &format!("mkdir -p target/release && cp {} target/release/grove", sh_quote(&grove.to_string_lossy())));
    let bin = home.join(".local/bin/grove");
    let ssh = fake_ssh(d.path(), None);
    let v = with_env(&ssh, &home, || {
        unsafe { std::env::set_var("FAKE_PATH", format!("{}:/usr/bin:/bin", bin_dir.display())) };
        let r = install_grove("me@box", &facts_for(&home, false), &bin.to_string_lossy(), &grove, Some(&src), plain);
        unsafe { std::env::remove_var("FAKE_PATH") };
        r
    })
    .unwrap();
    assert_eq!(v, "grove 0.1.0");
    let shipped = home.join("src/grove");
    assert!(shipped.join("src/main.rs").exists());
    assert!(!shipped.join(".git").exists());
    assert!(!shipped.join("stale.rs").exists(), "old files go");
    assert!(!shipped.join("target/release/big").exists(), "local target isn't shipped");
}

#[test]
fn source_build_without_source_folder_fails_clearly() {
    let d = tempfile::tempdir().unwrap();
    let gone = d.path().join("deleted checkout");
    let err = install_grove("me@box", &facts_for(d.path(), false), "/x/grove", d.path(), Some(&gone), plain)
        .unwrap_err();
    assert!(format!("{err:#}").contains("install Grove there by hand"), "{err:#}");
    let err = install_grove("me@box", &facts_for(d.path(), false), "/x/grove", d.path(), None, plain)
        .unwrap_err();
    assert!(format!("{err:#}").contains("install Grove there by hand"), "{err:#}");
}

#[test]
fn source_build_needs_cargo_there() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("Cargo.toml"), "").unwrap();
    let mut f = facts_for(d.path(), false);
    f.tools.clear();
    let err = install_grove("me@box", &f, "/x/grove", d.path(), Some(d.path()), plain).unwrap_err();
    assert!(format!("{err:#}").contains("cargo"), "{err:#}");
}
```

- [ ] **Step 2: Run** `cargo test --bin grove finca::` and expect a compile failure.

- [ ] **Step 3: Implement in `src/finca.rs`:**

```rust
/// This binary's source folder, recorded at build time, if still there.
pub fn source_dir() -> Option<std::path::PathBuf> {
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.join("Cargo.toml").exists().then_some(p)
}

/// Puts grove at `bin` on the finca and returns its `--version` line.
/// Same OS and CPU: copies `exe`. Otherwise builds `source` there (needs
/// cargo there). `wrap` runs the build under the login shell.
pub fn install_grove(
    dest: &str,
    facts: &crate::setup::Facts,
    bin: &str,
    exe: &std::path::Path,
    source: Option<&std::path::Path>,
    wrap: fn(&str) -> String,
) -> Result<String> {
    let b = sh_quote(bin);
    let new = sh_quote(&format!("{bin}.new"));
    // Rename over the old binary so a running grove keeps its (old) inode.
    let swap = format!("mv -f {new} {b} && {b} --version");
    let mkdir = format!("mkdir -p \"$(dirname {b})\"");
    let out = if facts.same_platform_as_here() {
        let file = std::fs::File::open(exe).with_context(|| format!("reading {}", exe.display()))?;
        remote_stdin(
            dest,
            &format!("set -e; {mkdir}; cat > {new}; chmod 755 {new}; {swap}"),
            file.into(),
        )?
    } else {
        let Some(src) = source.filter(|s| s.join("Cargo.toml").exists()) else {
            bail!(
                "{} is {} {}, unlike this machine, so Grove must be built there, but this grove's source folder is gone; install Grove there by hand",
                facts.host, facts.os, facts.arch
            );
        };
        if !facts.has("cargo") {
            bail!("{} has no cargo to build Grove with", facts.host);
        }
        ship_source(dest, src)?;
        run_remote(
            dest,
            &wrap(&format!(
                "set -e; [ -f ~/.cargo/env ] && . ~/.cargo/env; cd ~/src/grove; cargo build --release --quiet; {mkdir}; install -m 755 target/release/grove {new}; {swap}"
            )),
        )?
    };
    if !out.status.success() {
        bail!(
            "installing grove on {dest} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .last()
        .unwrap_or_default()
        .trim()
        .to_string())
}

/// Replaces ~/src/grove on the finca with `src` (keeping its build cache),
/// streamed as a tar over ssh.
fn ship_source(dest: &str, src: &std::path::Path) -> Result<()> {
    use std::process::{Command, Stdio};
    let mut tar = Command::new("tar")
        .arg("-C")
        .arg(src)
        .args(["--exclude=./target", "--exclude=./.git", "--exclude=./.superpowers", "-cf", "-", "."])
        .stdout(Stdio::piped())
        .spawn()
        .context("running tar")?;
    let stdout = tar.stdout.take().context("tar output")?;
    let out = remote_stdin(
        dest,
        "set -e; mkdir -p ~/src/grove; cd ~/src/grove; find . -mindepth 1 -maxdepth 1 ! -name target -exec rm -rf {} +; tar -xf -",
        stdout.into(),
    )?;
    let tar_ok = tar.wait()?.success();
    if !out.status.success() || !tar_ok {
        bail!("copying Grove's source to {dest} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}
```

In `src/main.rs`, add `FincaCmd::Update { names: Vec<String> }`, documented "Install this build of grove on fincas (all, or the named ones)". In `finca_cmd`:

```rust
FincaCmd::Update { names } => {
    let fincas: Vec<finca::Finca> = if names.is_empty() {
        store.list_fincas().into_iter().filter_map(Result::ok).collect()
    } else {
        names.iter().map(|n| store.get_finca(n)).collect::<Result<_>>()?
    };
    let exe = std::env::current_exe()?;
    let mut failed = 0;
    for f in &fincas {
        match update_one(f, &exe) {
            Ok(line) => println!("== {}: {line}", f.name),
            Err(e) => {
                failed += 1;
                eprintln!("== {}: {e:#}", f.name);
            }
        }
    }
    if failed > 0 {
        bail!("{failed} finca(s) not updated");
    }
}
```

with:

```rust
/// Installs this build on `f` through whichever destination answers.
fn update_one(f: &finca::Finca, exe: &std::path::Path) -> Result<String> {
    let dest = f
        .destinations()
        .find(|d| finca::run_remote(d, "true").is_ok_and(|o| o.status.success()))
        .ok_or_else(|| anyhow::anyhow!("unreachable"))?;
    let out = finca::run_remote(dest, &finca::login_shell_cmd(&finca::facts_script()))?;
    let facts = setup::parse_facts(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| anyhow::anyhow!("couldn't read the machine's details"))?;
    let how = if facts.same_platform_as_here() { "copied" } else { "built there" };
    let v = finca::install_grove(
        dest,
        &facts,
        &f.grove_bin,
        exe,
        finca::source_dir().as_deref(),
        finca::login_shell_cmd,
    )?;
    Ok(format!("{v} ({how}, via {dest})"))
}
```

Replace the body of `scripts/deploy-fincas.sh` after its comment header with:

```bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo install --quiet --path .
exec "${CARGO_HOME:-$HOME/.cargo}/bin/grove" finca update "$@"
```

Update the header comment so it says it installs this checkout locally and then runs `grove finca update`.

- [ ] **Step 4: Run** `cargo test --bin grove finca::`, then `cargo test` and clippy. All should pass.

- [ ] **Step 5: Commit** `feat(fincas): grove finca update installs this build (copy or build there)`.

---

### Task 4: The wizard and `grove finca setup`

**Files:**
- Modify: `src/setup.rs`, `src/setup_tests.rs`, `src/store.rs`, `src/main.rs`

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces:

```rust
pub trait Io {
    fn say(&mut self, line: &str);
    fn confirm(&mut self, question: &str) -> bool;         // [Y/n], default yes
    fn ask(&mut self, question: &str, default: &str) -> String;
    fn interactive(&mut self, argv: &[String]) -> bool;     // on the TTY; true if it exited 0
}
pub struct TerminalIo;
pub struct Setup<'a> {
    pub store: &'a crate::store::Store,
    pub ssh_dir: std::path::PathBuf,          // ~/.ssh here
    pub exe: std::path::PathBuf,
    pub source: Option<std::path::PathBuf>,
    pub tailscale_json: Option<String>,
    pub wrap: fn(&str) -> String,             // finca::login_shell_cmd in real use
}
impl Setup<'_> { pub fn run(&self, io: &mut dyn Io, dest: &str) -> anyhow::Result<crate::finca::Finca> }
```

- `Store::save_finca(&Finca) -> Result<()>` overwrites `fincas/<slug>.toml`.

Flow of `run` (each step prints one line with `io.say`):
1. **ssh_ready(dest):** `run_remote(dest, "true")`. On failure:
   - stderr contains "Host key verification failed": `confirm("Accept {dest}'s host key?")`, then `interactive(interactive_ssh(dest, Some("true"), &["-o","StrictHostKeyChecking=accept-new"]))`, then re-check.
   - still failing: if no `id_*.pub` in `ssh_dir`, `confirm("Create an SSH key (ssh-keygen)?")` then `interactive(["ssh-keygen","-t","ed25519"])`.
   - then `confirm("Copy your key to {dest} (you'll type its password once)?")`, then `interactive(["ssh-copy-id", dest])`, then re-check.
   - still failing: `bail!("can't reach {dest} without a password: {stderr}")`.
2. **facts:** `run_remote(dest, &(self.wrap)(&facts_script()))`, then `parse_facts`.
3. **existing:** a finca in the store with `ssh == dest` or `backup == Some(dest)`. If there is one, say "{name} is already a finca: checking it". Otherwise ask for the name, defaulting to `facts.host`; refuse (and re-ask) an empty slug or one that's taken.
4. **tools:** for `tmux`, then `claude`, when missing:
   - with `install_command`: `confirm("Install {tool} on {host}? ({cmd})")`, then `interactive(interactive_ssh(dest, Some(cmd), &[]))`, then re-fetch facts. If still missing: `bail!("{tool} still isn't on {host}'s login PATH; if it was installed to ~/.local/bin, add that to PATH in the shell profile there")`.
   - no command, or declined: `bail!("{host} needs {tool}: install it there, then run this again")`.
5. **grove:** keep `facts.grove`, unless its version differs from `format!("grove {}", env!("CARGO_PKG_VERSION"))` or it's missing. Otherwise the target is the existing path, or `{home}/.local/bin/grove`. If the build needs cargo (not the same platform) and there's none, `confirm` the rustup install (as in step 4), then re-fetch facts. Then `confirm("Install Grove at {bin}?")` and `install_grove(...)`.
6. **claude login:** `run_remote(dest, &wrap("claude auth status >/dev/null 2>&1"))`. If it isn't successful: `confirm("Log in to Claude on {host}?")`, then `interactive(interactive_ssh(dest, Some("claude auth login"), &[]))`, then re-check. Failure or decline: `bail!("Claude isn't logged in on {host}: run `claude auth login` there")`.
7. **backup:** keep an existing finca's backup. Otherwise, if `tailscale_backup(json, &facts.host)` gives a name different from `dest`, `confirm("Use {ts} (Tailscale) as the backup route?")`. Keep it only if `run_remote(ts, "true")` succeeds; else say it didn't answer.
8. **save:** `store.save_finca(&finca)`, say "Saved finca {name}", return it.

- [ ] **Step 1: Write failing tests** in `src/setup_tests.rs`:

```rust
use crate::finca::testing::{fake_grove, fake_ssh};
use crate::finca::tests::with_env;

/// Scripted answers; records everything said and run.
struct Script {
    answers: std::collections::VecDeque<&'static str>,
    said: Vec<String>,
    ran: Vec<Vec<String>>,
    on_run: Box<dyn FnMut(&[String]) -> bool>,
}

impl Script {
    fn new(answers: &[&'static str]) -> Self {
        Self {
            answers: answers.iter().copied().collect(),
            said: Vec::new(),
            ran: Vec::new(),
            on_run: Box::new(|_| true),
        }
    }
}

impl Io for Script {
    fn say(&mut self, line: &str) {
        self.said.push(line.to_string());
    }
    fn confirm(&mut self, q: &str) -> bool {
        self.said.push(format!("? {q}"));
        self.answers.pop_front().expect(q) != "n"
    }
    fn ask(&mut self, q: &str, default: &str) -> String {
        self.said.push(format!("? {q}"));
        match self.answers.pop_front().expect(q) {
            "" => default.to_string(),
            a => a.to_string(),
        }
    }
    fn interactive(&mut self, argv: &[String]) -> bool {
        self.ran.push(argv.to_vec());
        (self.on_run)(argv)
    }
}

struct World {
    _d: tempfile::TempDir,
    home: std::path::PathBuf,   // the "remote" home
    bin: std::path::PathBuf,    // its PATH dir
    store: crate::store::Store,
    ssh: std::path::PathBuf,
    exe: std::path::PathBuf,
    ssh_dir: std::path::PathBuf,
}

/// A remote with the given tools (fake executables on its PATH) and a
/// logged-in claude unless `claude` is absent from `tools`.
fn world(tools: &[&str]) -> World {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("remote home");
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for t in tools {
        // A fake claude exiting 0 also means `claude auth status`: logged in.
        crate::finca::tests::script(&bin, t, "exit 0");
    }
    let store = crate::store::Store::new(crate::config::Paths::new(d.path().join("grove home")));
    store.paths.ensure().unwrap();
    let ssh = fake_ssh(d.path(), None);
    let exe = fake_grove(d.path(), "{}");
    let ssh_dir = d.path().join("local ssh");
    std::fs::create_dir_all(&ssh_dir).unwrap();
    std::fs::write(ssh_dir.join("id_ed25519.pub"), "ssh-ed25519 AAAA me").unwrap();
    World { home, bin, store, ssh, exe, ssh_dir, _d: d }
}

fn plain(c: &str) -> String {
    c.to_string()
}

fn run(w: &World, io: &mut Script, dest: &str) -> anyhow::Result<crate::finca::Finca> {
    let setup = Setup {
        store: &w.store,
        ssh_dir: w.ssh_dir.clone(),
        exe: w.exe.clone(),
        source: None,
        tailscale_json: None,
        wrap: plain,
    };
    let path = format!("{}:/usr/bin:/bin", w.bin.display());
    with_env(&w.ssh, &w.home, || {
        unsafe { std::env::set_var("FAKE_PATH", &path) };
        let r = setup.run(io, dest);
        unsafe { std::env::remove_var("FAKE_PATH") };
        r
    })
}

#[test]
fn ready_machine_needs_only_a_name_and_the_grove_install() {
    let w = world(&["tmux", "claude"]);
    let mut io = Script::new(&["", "y"]); // default name; install grove
    let f = run(&w, &mut io, "me@homebox").unwrap();
    assert_eq!(f.ssh, "me@homebox");
    assert!(f.grove_bin.ends_with("/.local/bin/grove"), "{}", f.grove_bin);
    assert!(w.home.join(".local/bin/grove").exists());
    assert!(io.ran.is_empty(), "nothing interactive: {:?}", io.ran);
    assert_eq!(w.store.get_finca(&f.name).unwrap(), f);
}

#[test]
fn missing_tmux_is_installed_after_asking() {
    let w = world(&["claude", "apt-get"]);
    // A fresh check reports `uname -s` as this OS; tmux needs a package
    // manager we know for it: apt-get on Linux, brew on macOS.
    let brew = std::env::consts::OS == "macos";
    if brew {
        crate::finca::tests::script(&w.bin, "brew", "exit 0");
    }
    let bin = w.bin.clone();
    let mut io = Script::new(&["", "y", "y"]); // name, install tmux, install grove
    io.on_run = Box::new(move |_| {
        crate::finca::tests::script(&bin, "tmux", "exit 0");
        true
    });
    run(&w, &mut io, "me@homebox").unwrap();
    let cmd = io.ran[0].last().unwrap();
    assert!(cmd.contains("install") && cmd.contains("tmux"), "{cmd}");
}

#[test]
fn declining_a_needed_step_saves_nothing() {
    let w = world(&["tmux", "claude"]);
    let mut io = Script::new(&["", "n"]); // name; decline grove
    let err = run(&w, &mut io, "me@homebox").unwrap_err();
    assert!(format!("{err:#}").contains("Grove"), "{err:#}");
    assert!(w.store.list_fincas().is_empty());
}

#[test]
fn name_clash_asks_again() {
    let w = world(&["tmux", "claude"]);
    w.store
        .add_finca(&crate::finca::Finca {
            name: "taken".into(),
            ssh: "me@elsewhere".into(),
            grove_bin: "/g".into(),
            color: None,
            backup: None,
        })
        .unwrap();
    let mut io = Script::new(&["taken", "fresh", "y"]);
    let f = run(&w, &mut io, "me@homebox").unwrap();
    assert_eq!(f.name, "fresh");
    assert!(io.said.iter().any(|l| l.contains("already")), "{:?}", io.said);
}

#[test]
fn rerun_on_an_existing_finca_is_a_health_check() {
    let w = world(&["tmux", "claude"]);
    let mut io = Script::new(&["", "y"]);
    let first = run(&w, &mut io, "me@homebox").unwrap();
    let mut with_backup = first.clone();
    with_backup.backup = Some("me@homebox.ts.net".into());
    w.store.save_finca(&with_backup).unwrap();
    // grove is now there at the same version: nothing to ask at all.
    let mut io = Script::new(&[]);
    let again = run(&w, &mut io, "me@homebox").unwrap();
    assert_eq!(again, with_backup, "backup kept, same file");
    assert_eq!(w.store.list_fincas().len(), 1);
}

#[test]
fn claude_installed_off_path_stops_with_a_hint() {
    let w = world(&["tmux", "apt-get"]);
    let mut io = Script::new(&["", "y"]); // name, install claude
    io.on_run = Box::new(|_| true); // "installs" but claude never shows up
    let err = run(&w, &mut io, "me@homebox").unwrap_err();
    assert!(format!("{err:#}").contains("PATH"), "{err:#}");
}

#[test]
fn logged_out_claude_offers_login() {
    let w = world(&["tmux"]);
    let flag = w.home.join("logged-in");
    crate::finca::tests::script(
        &w.bin,
        "claude",
        &format!("[ -f {} ]", crate::finca::sh_quote(&flag.to_string_lossy())),
    );
    let mut io = Script::new(&["", "y", "y"]); // name, grove, log in
    io.on_run = Box::new(move |_| {
        std::fs::write(&flag, "").unwrap();
        true
    });
    run(&w, &mut io, "me@homebox").unwrap();
    assert!(io.ran[0].last().unwrap().contains("claude auth login"));
}

#[test]
fn unknown_host_key_is_accepted_once() {
    let w = world(&["tmux", "claude"]);
    let accepted = w.home.join("accepted");
    // ssh fails with a host-key error until the key is accepted.
    let ssh = crate::finca::tests::script(
        w._d.path(),
        "picky ssh",
        &format!(
            "[ -f {a} ] || {{ echo 'Host key verification failed.' >&2; exit 255; }}\nexec {real} \"$@\"",
            a = crate::finca::sh_quote(&accepted.to_string_lossy()),
            real = crate::finca::sh_quote(&w.ssh.to_string_lossy()),
        ),
    );
    let w = World { ssh, ..w };
    let mut io = Script::new(&["y", "", "y"]); // accept key, name, grove
    io.on_run = Box::new(move |argv| {
        assert!(argv.iter().any(|a| a == "StrictHostKeyChecking=accept-new"), "{argv:?}");
        std::fs::write(&accepted, "").unwrap();
        true
    });
    run(&w, &mut io, "me@homebox").unwrap();
    assert_eq!(io.ran.len(), 1);
}

#[test]
fn password_only_host_gets_the_key_copied() {
    let w = world(&["tmux", "claude"]);
    let copied = w.home.join("authorized");
    let ssh = crate::finca::tests::script(
        w._d.path(),
        "locked ssh",
        &format!(
            "[ -f {a} ] || {{ echo 'Permission denied (publickey,password).' >&2; exit 255; }}\nexec {real} \"$@\"",
            a = crate::finca::sh_quote(&copied.to_string_lossy()),
            real = crate::finca::sh_quote(&w.ssh.to_string_lossy()),
        ),
    );
    let w = World { ssh, ..w };
    let mut io = Script::new(&["y", "", "y"]); // copy key, name, grove
    io.on_run = Box::new(move |argv| {
        assert_eq!(argv[0], "ssh-copy-id");
        std::fs::write(&copied, "").unwrap();
        true
    });
    run(&w, &mut io, "me@homebox").unwrap();
}

#[test]
fn tailscale_peer_becomes_the_backup_when_it_answers() {
    let w = world(&["tmux", "claude"]);
    let host = String::from_utf8(std::process::Command::new("sh").args(["-c", "hostname -s 2>/dev/null || hostname"]).output().unwrap().stdout).unwrap();
    let json = format!(r#"{{"Peer":{{"a":{{"HostName":"{}","DNSName":"box.tail.ts.net."}}}}}}"#, host.trim());
    let setup = Setup {
        store: &w.store,
        ssh_dir: w.ssh_dir.clone(),
        exe: w.exe.clone(),
        source: None,
        tailscale_json: Some(json),
        wrap: plain,
    };
    let mut io = Script::new(&["", "y", "y"]); // name, grove, use backup
    let path = format!("{}:/usr/bin:/bin", w.bin.display());
    let f = with_env(&w.ssh, &w.home, || {
        unsafe { std::env::set_var("FAKE_PATH", &path) };
        let r = setup.run(&mut io, "me@homebox");
        unsafe { std::env::remove_var("FAKE_PATH") };
        r
    })
    .unwrap();
    assert_eq!(f.backup.as_deref(), Some("box.tail.ts.net"));
}
```

Also make `fn script` in `finca_tests.rs` `pub(crate)`, and have it return the path (it already does).

- [ ] **Step 2: Run** `cargo test --bin grove setup::` and expect a compile failure (`Io`, `Setup` and `save_finca` are missing).

- [ ] **Step 3: Implement.** In `src/store.rs`, next to `add_finca`:

```rust
/// Writes the finca's file, replacing one with the same slug.
pub fn save_finca(&self, f: &crate::finca::Finca) -> Result<()> {
    let slug = f.slug();
    if slug.is_empty() {
        bail!("finca name {:?} has no letters or digits", f.name);
    }
    write_atomic(&self.paths.fincas.join(format!("{slug}.toml")), toml::to_string_pretty(f)?.as_bytes())
}
```

In `src/setup.rs`, remove `#![allow(dead_code)]` and add:

```rust
use anyhow::{Result, bail};
use crate::finca::{self, Finca, interactive_ssh, run_remote};

pub trait Io {
    fn say(&mut self, line: &str);
    /// A yes/no question; Enter means yes.
    fn confirm(&mut self, question: &str) -> bool;
    /// A question with a default for Enter.
    fn ask(&mut self, question: &str, default: &str) -> String;
    /// Runs `argv` on the terminal (it may prompt); true if it exited 0.
    fn interactive(&mut self, argv: &[String]) -> bool;
}

/// Prompts on stdin/stdout.
pub struct TerminalIo;

impl TerminalIo {
    fn line(prompt: &str) -> String {
        use std::io::Write;
        print!("{prompt}");
        let _ = std::io::stdout().flush();
        let mut s = String::new();
        let _ = std::io::stdin().read_line(&mut s);
        s.trim().to_string()
    }
}

impl Io for TerminalIo {
    fn say(&mut self, line: &str) {
        println!("{line}");
    }
    fn confirm(&mut self, question: &str) -> bool {
        !Self::line(&format!("{question} [Y/n] ")).to_lowercase().starts_with('n')
    }
    fn ask(&mut self, question: &str, default: &str) -> String {
        match Self::line(&format!("{question} [{default}] ")) {
            s if s.is_empty() => default.to_string(),
            s => s,
        }
    }
    fn interactive(&mut self, argv: &[String]) -> bool {
        std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .status()
            .is_ok_and(|s| s.success())
    }
}

pub struct Setup<'a> {
    pub store: &'a crate::store::Store,
    /// ~/.ssh on this machine.
    pub ssh_dir: std::path::PathBuf,
    /// This grove, copied to fincas of the same OS and CPU.
    pub exe: std::path::PathBuf,
    /// Grove's source, built on fincas of another OS or CPU.
    pub source: Option<std::path::PathBuf>,
    /// `tailscale status --json` here, if tailscale is installed.
    pub tailscale_json: Option<String>,
    /// Wraps remote commands: the login shell in real use.
    pub wrap: fn(&str) -> String,
}

fn ok(dest: &str, cmd: &str) -> bool {
    run_remote(dest, cmd).is_ok_and(|o| o.status.success())
}

impl Setup<'_> {
    pub fn run(&self, io: &mut dyn Io, dest: &str) -> Result<Finca> {
        self.ssh_ready(io, dest)?;
        let mut facts = self.facts(dest)?;
        let host = facts.host.clone();
        io.say(&format!("✓ {dest} answers: {host}, {} {}", facts.os, facts.arch));

        let existing = self
            .store
            .list_fincas()
            .into_iter()
            .filter_map(Result::ok)
            .find(|f| f.ssh == dest || f.backup.as_deref() == Some(dest));
        let name = match &existing {
            Some(f) => {
                io.say(&format!("{} is already a finca: checking it", f.name));
                f.name.clone()
            }
            None => self.pick_name(io, &host),
        };

        for tool in ["tmux", "claude"] {
            if facts.has(tool) {
                io.say(&format!("✓ {tool}"));
                continue;
            }
            let Some(cmd) = install_command(tool, &facts) else {
                bail!("{host} needs {tool}: install it there, then run this again");
            };
            if !io.confirm(&format!("Install {tool} on {host}? ({cmd})")) {
                bail!("{host} needs {tool}: install it there, then run this again");
            }
            io.interactive(&interactive_ssh(dest, Some(cmd), &[]));
            facts = self.facts(dest)?;
            if !facts.has(tool) {
                bail!("{tool} still isn't on {host}'s login PATH; if it was installed to ~/.local/bin, add that to PATH in the shell profile there");
            }
            io.say(&format!("✓ {tool} installed"));
        }

        let grove_bin = self.grove(io, dest, &mut facts)?;
        self.claude_login(io, dest, &host)?;
        let backup = match &existing {
            Some(f) => f.backup.clone(),
            None => self.backup(io, dest, &host),
        };

        let finca = Finca {
            name: name.clone(),
            ssh: existing.as_ref().map_or(dest.to_string(), |f| f.ssh.clone()),
            grove_bin,
            color: existing.as_ref().and_then(|f| f.color.clone()),
            backup,
        };
        self.store.save_finca(&finca)?;
        io.say(&format!("✓ Saved finca {name}"));
        Ok(finca)
    }

    fn ssh_ready(&self, io: &mut dyn Io, dest: &str) -> Result<()> {
        let check = || run_remote(dest, "true");
        let out = check()?;
        if out.status.success() {
            io.say(&format!("✓ SSH to {dest}"));
            return Ok(());
        }
        let mut err = String::from_utf8_lossy(&out.stderr).to_string();
        if err.contains("Host key verification failed")
            && io.confirm(&format!("Accept {dest}'s host key?"))
        {
            io.interactive(&interactive_ssh(dest, Some("true"), &["-o", "StrictHostKeyChecking=accept-new"]));
            let out = check()?;
            if out.status.success() {
                io.say(&format!("✓ SSH to {dest}"));
                return Ok(());
            }
            err = String::from_utf8_lossy(&out.stderr).to_string();
        }
        if !err.contains("Permission denied") {
            bail!("can't reach {dest}: {}", err.trim());
        }
        let has_key = std::fs::read_dir(&self.ssh_dir).is_ok_and(|d| {
            d.filter_map(|e| e.ok()).any(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.starts_with("id_") && n.ends_with(".pub")
            })
        });
        if !has_key {
            if !io.confirm("You have no SSH key. Create one (ssh-keygen)?") {
                bail!("can't reach {dest} without a password: create a key and run `ssh-copy-id {dest}`");
            }
            io.interactive(&["ssh-keygen".into(), "-t".into(), "ed25519".into()]);
        }
        if !io.confirm(&format!("Copy your SSH key to {dest}? (you'll type its password once)")) {
            bail!("can't reach {dest} without a password: run `ssh-copy-id {dest}`");
        }
        io.interactive(&["ssh-copy-id".into(), dest.into()]);
        let out = check()?;
        if !out.status.success() {
            bail!("still can't reach {dest} without a password: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        io.say(&format!("✓ SSH to {dest}"));
        Ok(())
    }

    fn facts(&self, dest: &str) -> Result<Facts> {
        let out = run_remote(dest, &(self.wrap)(&finca::facts_script()))?;
        parse_facts(&String::from_utf8_lossy(&out.stdout))
            .ok_or_else(|| anyhow::anyhow!("couldn't read {dest}'s details: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }

    fn pick_name(&self, io: &mut dyn Io, host: &str) -> String {
        loop {
            let name = io.ask("Name for this finca?", host);
            let slug = crate::model::slugify(&name);
            if slug.is_empty() {
                io.say("A name needs a letter or digit.");
            } else if self.store.get_finca(&slug).is_ok() {
                io.say(&format!("There's already a finca called {name}."));
            } else {
                return name;
            }
        }
    }

    fn grove(&self, io: &mut dyn Io, dest: &str, facts: &mut Facts) -> Result<String> {
        let want = format!("grove {}", env!("CARGO_PKG_VERSION"));
        if let Some((path, version)) = &facts.grove
            && *version == want
        {
            io.say(&format!("✓ {version} at {path}"));
            return Ok(path.clone());
        }
        let bin = facts
            .grove
            .as_ref()
            .map_or(format!("{}/.local/bin/grove", facts.home), |(p, _)| p.clone());
        if !facts.same_platform_as_here() && !facts.has("cargo") {
            let cmd = install_command("cargo", facts).unwrap_or_default();
            if !io.confirm(&format!("Building Grove there needs Rust. Install it? ({cmd})")) {
                bail!("{} needs Rust (cargo) to build Grove", facts.host);
            }
            io.interactive(&interactive_ssh(dest, Some(cmd), &[]));
            *facts = self.facts(dest)?;
        }
        if !io.confirm(&format!("Install Grove at {bin}?")) {
            bail!("{} needs Grove: install it there, then run this again", facts.host);
        }
        if !facts.same_platform_as_here() {
            io.say("Building Grove there (this takes a few minutes)…");
        }
        let v = finca::install_grove(dest, facts, &bin, &self.exe, self.source.as_deref(), self.wrap)?;
        io.say(&format!("✓ {v} at {bin}"));
        Ok(bin)
    }

    fn claude_login(&self, io: &mut dyn Io, dest: &str, host: &str) -> Result<()> {
        let status = (self.wrap)("claude auth status >/dev/null 2>&1");
        if ok(dest, &status) {
            io.say("✓ Claude is logged in");
            return Ok(());
        }
        if io.confirm(&format!("Log in to Claude on {host}?")) {
            io.interactive(&interactive_ssh(dest, Some("claude auth login"), &[]));
            if ok(dest, &status) {
                io.say("✓ Claude is logged in");
                return Ok(());
            }
        }
        bail!("Claude isn't logged in on {host}: run `claude auth login` there")
    }

    fn backup(&self, io: &mut dyn Io, dest: &str, host: &str) -> Option<String> {
        let ts = tailscale_backup(self.tailscale_json.as_deref()?, host)?;
        if ts == dest || !io.confirm(&format!("Use {ts} (Tailscale) as the backup route?")) {
            return None;
        }
        if ok(&ts, "true") {
            io.say(&format!("✓ backup {ts}"));
            Some(ts)
        } else {
            io.say(&format!("{ts} didn't answer over SSH; no backup set"));
            None
        }
    }
}
```

`interactive_ssh` wraps its command in `login_shell_cmd` itself, which is right for the real terminal. In tests the commands are only recorded, never run.

In `src/main.rs`, add `FincaCmd::Setup { dest: Option<String>, #[arg(long, hide = true)] pause: bool }`, documented "Set up a machine as a finca, step by step (asks before each change)". Handler:

```rust
FincaCmd::Setup { dest, pause } => {
    let mut io = setup::TerminalIo;
    let dest = match dest {
        Some(d) => d,
        None => setup::Io::ask(&mut io, "SSH destination (user@host)?", ""),
    };
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let tailscale_json = std::process::Command::new("tailscale")
        .args(["status", "--json"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let s = setup::Setup {
        store: &store,
        ssh_dir: home.join(".ssh"),
        exe: std::env::current_exe()?,
        source: finca::source_dir(),
        tailscale_json,
        wrap: finca::login_shell_cmd,
    };
    let r = s.run(&mut io, dest.trim());
    if let Err(e) = &r {
        eprintln!("grove: {e:#}");
    }
    if pause {
        setup::Io::ask(&mut io, "Press Enter to close", "");
    }
    r?;
}
```

An empty destination should `bail!("no destination given")` before `Setup` runs.

- [ ] **Step 4: Run** `cargo test --bin grove setup::`, then `cargo test` and clippy. Fix any test whose assumptions about the fake were wrong by correcting the fake, not the behavior.

- [ ] **Step 5: Commit** `feat(setup): grove finca setup wizard`.

---

### Task 5: Dashboard: `F` to add a finca

**Files:**
- Modify: `src/ui/app.rs`, `src/ui/input.rs`, `src/ui/run.rs`, `src/ui/render.rs`, `src/ui/help.rs`, `src/ui/complete.rs`, `src/ui/tests.rs`, `README.md`

**Interfaces:**
- Consumes: `setup::ssh_hosts`.
- Produces:
  - `PromptKind::AddFinca`;
  - `Action::SetupFinca { dest: String }`;
  - `pub fn complete_ssh_host(input: &str, hosts: &[String]) -> Completion`;
  - `App.ssh_dir: PathBuf` (defaults to `$HOME/.ssh`; tests point it at a temp dir).

- [ ] **Step 1: Write failing tests** (`src/ui/tests.rs`):

```rust
#[test]
fn big_f_prompts_for_a_destination_and_enter_sets_it_up() {
    let (_d, _s, mut app) = loaded(&["alpha"], &[]);
    handle_key(&mut app, ch('F'));
    assert!(matches!(app.mode, Mode::Prompt { kind: PromptKind::AddFinca, .. }));
    for c in "me@newbox".chars() {
        handle_key(&mut app, ch(c));
    }
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        Action::SetupFinca { dest: "me@newbox".into() }
    );
}

#[test]
fn a_on_a_finca_adds_a_finca() {
    let (_d, _s, mut app) = with_fincas(&[], &[], &["homebox"]);
    handle_key(&mut app, ch('a'));
    assert!(matches!(app.mode, Mode::Prompt { kind: PromptKind::AddFinca, .. }));
}

#[test]
fn ssh_completion_keeps_the_user_part() {
    let hosts = vec!["homebox".to_string(), "homebox.local".to_string(), "pi".to_string()];
    let c = complete_ssh_host("me@p", &hosts);
    assert_eq!(c.input, "me@pi");
    let c = complete_ssh_host("me@home", &hosts);
    assert_eq!(c.input, "me@homebox");
    assert_eq!(c.candidates, ["me@homebox", "me@homebox.local"]);
    assert_eq!(complete_ssh_host("zz", &hosts).input, "zz");
}

#[test]
fn tab_in_the_finca_prompt_completes_from_ssh_config() {
    let (d, _s, mut app) = loaded(&["alpha"], &[]);
    let ssh = d.path().join("ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::write(ssh.join("config"), "Host homebox\n").unwrap();
    app.ssh_dir = ssh;
    handle_key(&mut app, ch('F'));
    handle_key(&mut app, ch('h'));
    handle_key(&mut app, key(KeyCode::Tab));
    assert!(matches!(&app.mode, Mode::Prompt { input, .. } if input == "homebox"));
}
```

(Import `complete_ssh_host` in tests via `use super::complete::complete_ssh_host;`.)

- [ ] **Step 2: Run** `cargo test --bin grove ui::` and expect a compile failure.

- [ ] **Step 3: Implement.**

`complete.rs`:

```rust
/// Completes the host part of an SSH destination (after any `user@`)
/// against `hosts`, keeping the user part.
pub fn complete_ssh_host(input: &str, hosts: &[String]) -> Completion {
    let (user, prefix) = match input.rsplit_once('@') {
        Some((u, h)) => (format!("{u}@"), h),
        None => (String::new(), input),
    };
    let want = prefix.to_lowercase();
    let names: Vec<String> = hosts
        .iter()
        .filter(|h| h.to_lowercase().starts_with(&want))
        .cloned()
        .collect();
    match names.len() {
        0 => Completion { input: input.to_string(), candidates: Vec::new() },
        1 => Completion { input: format!("{user}{}", names[0]), candidates: Vec::new() },
        _ => {
            let common = common_prefix(&names);
            let completed = if common.chars().count() >= prefix.chars().count() { common } else { prefix.to_string() };
            Completion {
                input: format!("{user}{completed}"),
                candidates: names.iter().map(|n| format!("{user}{n}")).collect(),
            }
        }
    }
}
```

`app.rs`:
- add `AddFinca` to `PromptKind` and `SetupFinca { dest: String }` to `Action`;
- add `pub ssh_dir: std::path::PathBuf` to `App`, initialized to `std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".ssh")`.

`input.rs`:
- In `prompt_key`, make the Tab branch handle both kinds: `if key.code == KeyCode::Tab && matches!(kind, PromptKind::AddProject | PromptKind::AddFinca)`.
- In `tab_complete`, choose the completer by kind: `PromptKind::AddFinca => complete_ssh_host(&input, &crate::setup::ssh_hosts(&app.ssh_dir))`, else `complete_path(&input)`. `cycle_completion` already works on `app.completions`.
- On Enter: `PromptKind::AddFinca if text.is_empty() => Action::None`, and `PromptKind::AddFinca => Action::SetupFinca { dest: text }`.
- In `normal_key`, before `finca_key`: `KeyCode::Char('F')` opens `Mode::Prompt { kind: PromptKind::AddFinca, input: String::new() }`.
- In `finca_key`, `KeyCode::Char('a')` opens the same prompt.

`render.rs`: the prompt title for `AddFinca` is `" Add finca — SSH destination (Tab completes, Enter sets it up, Esc cancels) "`.

`run.rs` `perform`:

```rust
Action::SetupFinca { dest } => {
    let exe = std::env::current_exe().context("locating the grove binary")?;
    let argv = vec![
        exe.to_string_lossy().into_owned(),
        "finca".into(),
        "setup".into(),
        dest,
        "--pause".into(),
    ];
    ctx.tmux.display_popup(&ctx.store.paths.home, &argv)?;
    Outcome::Nothing
}
```

(The finca watcher re-polls when the file appears.)

`help.rs`: add `("F", "add a finca (guided setup)")` to the Fincas section.

`README.md`, in the "Fincas" section:
- replace the manual "Setting up a finca" steps with: "Press `F` (or run `grove finca setup me@homebox.local`). The setup checks SSH, tmux, Claude Code, Grove and the Claude login, and asks before fixing each one."
- keep `grove finca add` as the non-interactive option;
- list `grove finca update` in the command block, and say `scripts/deploy-fincas.sh` installs this checkout and runs it.
- add `F` to the key table.

- [ ] **Step 4: Run** `cargo test` and clippy. All should pass.

- [ ] **Step 5: Commit** `feat(ui): F adds a finca through the guided setup`.

---

### Task 6: Live verification

- [ ] **Step 1:** `cargo install --path .`
- [ ] **Step 2:** Run `grove finca update`. Expect both fincas to print a `grove 0.1.0` line: one "copied", and the other (a different CPU) "built there".
- [ ] **Step 3:** Health check: in a terminal, run `grove finca setup <an existing finca's ssh destination>`. Expect every step to show ✓, no questions asked, and "already a finca". Then check with `git diff ~/.grove/fincas` or by comparing the file before and after: the content should be unchanged.
- [ ] **Step 4:** On the dashboard, press `R`, then `F`. Type part of a host, press Tab to see it complete, then Esc.
- [ ] **Step 5:** Commit any fixes. Report to the user, including that the full new-machine flow (key copy, installs, login) is covered by tests with a fake remote, and was not run against a fresh real machine.
