//! Fincas: other machines running their own Grove, reached over SSH.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::model::slugify;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finca {
    pub name: String,
    /// Anything `ssh` accepts as a destination, including config aliases.
    pub ssh: String,
    /// Absolute path of grove on that machine (non-interactive ssh has no
    /// login-profile PATH).
    pub grove_bin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Tried when `ssh` doesn't answer, e.g. a Tailscale name for when
    /// you're away from the home network.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
}

impl Finca {
    pub fn slug(&self) -> String {
        slugify(&self.name)
    }

    /// SSH destinations in the order to try them: `ssh`, then `backup`.
    pub fn destinations(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.ssh.as_str()).chain(self.backup.as_deref())
    }
}

/// `$GROVE_SSH`, else `ssh`. Tests point this at a fake.
pub fn ssh_program() -> String {
    std::env::var("GROVE_SSH")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "ssh".into())
}

/// Options for every non-interactive ssh call.
pub const SSH_BATCH_OPTS: [&str; 4] = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=5"];

/// Connection sharing for every ssh Grove runs: the first connection to a
/// host stays open as a master for 10 minutes and later ones (polls, finca
/// shells, new windows) ride inside it, skipping the handshake.
pub fn share_opts() -> Vec<String> {
    share_opts_in(&crate::config::Paths::from_env().run)
}

/// `share_opts` with sockets in `run`. Empty when the socket path would be
/// too long for macOS (104 bytes, and ssh adds a 40-char %C hash plus a
/// temporary 17-char suffix): then each ssh just connects on its own.
pub fn share_opts_in(run: &std::path::Path) -> Vec<String> {
    let path = run.join("ssh-%C").to_string_lossy().into_owned();
    if path.len() - 2 + 40 + 17 >= 104 {
        return Vec::new();
    }
    [
        "ControlMaster=auto".to_string(),
        format!("ControlPath={path}"),
        "ControlPersist=10m".into(),
        // A master keeps the options of the ssh that started it, so it
        // always gets keep-alives: a dead link ends it instead of leaving
        // later connections hanging on it.
        "ServerAliveInterval=15".into(),
        "ServerAliveCountMax=3".into(),
    ]
    .into_iter()
    .flat_map(|o| ["-o".to_string(), o])
    .collect()
}

/// Single-quotes `s` for a POSIX shell.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Where grove usually lives; `~` is expanded by the remote shell.
const CANDIDATES: [&str; 4] = [
    "~/.local/bin/grove",
    "~/.cargo/bin/grove",
    "/opt/homebrew/bin/grove",
    "/usr/local/bin/grove",
];

/// Exit code the probe script uses for "no grove found".
const NO_GROVE: i32 = 3;

/// Checks `dest` is reachable without a password and finds grove there.
/// Returns grove's absolute path on the remote machine.
pub fn probe(dest: &str, bin: Option<&str>) -> Result<String> {
    // POSIX sh that also runs under zsh/bash as a login shell's -c string.
    let explicit = bin.map(sh_quote).unwrap_or_default();
    let script = format!(
        "for b in {explicit} {}; do if [ -x \"$b\" ] && \"$b\" --version >/dev/null 2>&1; then echo \"$b\"; exit 0; fi; done; exit {NO_GROVE}",
        CANDIDATES.join(" ")
    );
    let out = std::process::Command::new(ssh_program())
        .args(SSH_BATCH_OPTS)
        .args(share_opts())
        .arg(dest)
        .arg(script)
        .stdin(std::process::Stdio::null())
        .output()
        .with_context(|| format!("running {}", ssh_program()))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match out.status.code() {
        Some(0) if stdout.starts_with('/') => {
            Ok(stdout.lines().last().unwrap_or(&stdout).to_string())
        }
        Some(NO_GROVE) => bail!(
            "no grove found on {dest} (looked in {}{}); install it there first",
            if bin.is_some() { "--bin, " } else { "" },
            CANDIDATES.join(", ")
        ),
        _ => bail!(
            "can't reach {dest} over ssh without a password: {}\nSet up key-based SSH (`ssh-copy-id {dest}`) and connect once interactively to accept the host key.",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }
}

/// Outcome of polling one finca.
#[derive(Debug, Clone, PartialEq)]
pub enum Reach {
    Reachable(crate::status::Status),
    Unreachable {
        error: String,
    },
    /// Grove is there but too old (or too new) to report status.
    NeedsUpdate,
}

/// One poll result, sent from the poller thread.
#[derive(Debug, Clone)]
pub struct PollResult {
    pub slug: String,
    pub reach: Reach,
    /// The destination that answered; None when none did.
    pub via: Option<String>,
    pub at: chrono::DateTime<chrono::Utc>,
}

/// Polls each destination in turn until one answers.
#[cfg(test)]
pub fn poll_one(f: &Finca, timeout: std::time::Duration) -> Reach {
    poll_via(f, timeout).0
}

/// Like `poll_one`, also returning the destination that answered. Each
/// destination gets the full `timeout` (an unresolvable `.local` name can
/// hang past ssh's ConnectTimeout).
pub fn poll_via(f: &Finca, timeout: std::time::Duration) -> (Reach, Option<String>) {
    let mut errors = Vec::new();
    for dest in f.destinations() {
        match poll_dest(f, dest, timeout) {
            Reach::Unreachable { error } => errors.push(if f.backup.is_some() {
                format!("{dest}: {error}")
            } else {
                error
            }),
            reach => return (reach, Some(dest.to_string())),
        }
    }
    let error = errors.join("; ");
    (Reach::Unreachable { error }, None)
}

/// What a remote command printed, and its exit code (None: timed out).
#[derive(Debug)]
pub struct RemoteOut {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `remote_cmd` at `dest` over the shared connection, with `input` on
/// its stdin, killing it after `timeout`.
pub fn run_timed(
    dest: &str,
    remote_cmd: &str,
    input: Option<Vec<u8>>,
    timeout: std::time::Duration,
) -> RemoteOut {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    let failed = |e: String| RemoteOut {
        code: Some(255),
        stdout: String::new(),
        stderr: e,
    };
    let mut child = match Command::new(ssh_program())
        .args(SSH_BATCH_OPTS)
        .args(share_opts())
        .arg(dest)
        .arg(remote_cmd)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return failed(format!("can't run {}: {e}", ssh_program())),
    };
    if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
        // Dropping stdin when done closes it, so the remote sees EOF.
        std::thread::spawn(move || {
            let _ = stdin.write_all(&bytes);
        });
    }
    // Drain both pipes on threads so a chatty child can't block on a full pipe.
    let drain = |mut r: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = r.read_to_string(&mut s);
            s
        })
    };
    let out = drain(Box::new(child.stdout.take().expect("piped")));
    let err = drain(Box::new(child.stderr.take().expect("piped")));
    let deadline = std::time::Instant::now() + timeout;
    let code = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s.code(),
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                // Not joining the drains: a killed ssh's descendants may
                // still hold the pipes.
                return RemoteOut {
                    code: None,
                    stdout: String::new(),
                    stderr: format!("timed out after {}s", timeout.as_secs_f32()),
                };
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(e) => return failed(e.to_string()),
        }
    };
    RemoteOut {
        code,
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    }
}

/// Runs `grove status --json` at `dest`, killing it after `timeout`.
fn poll_dest(f: &Finca, dest: &str, timeout: std::time::Duration) -> Reach {
    use std::io::Read;
    use std::process::{Command, Stdio};

    // Under the login shell, like connect: grove needs the profile's PATH to
    // find tmux (Homebrew on macOS), or every session looks dead.
    let remote_cmd = login_shell_cmd(&format!("{} status --json", sh_quote(&f.grove_bin)));
    let mut child = match Command::new(ssh_program())
        .args(SSH_BATCH_OPTS)
        .args(share_opts())
        .arg(dest)
        .arg(remote_cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return Reach::Unreachable {
                error: format!("can't run {}: {e}", ssh_program()),
            };
        }
    };
    // Drain both pipes on threads so a chatty child can't block on a full pipe.
    let drain = |mut r: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = r.read_to_string(&mut s);
            s
        })
    };
    let out = drain(Box::new(child.stdout.take().expect("piped")));
    let err = drain(Box::new(child.stderr.take().expect("piped")));

    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(e) => {
                return Reach::Unreachable {
                    error: e.to_string(),
                };
            }
        }
    };
    let Some(status) = status else {
        return Reach::Unreachable {
            error: format!("timed out after {}s", timeout.as_secs_f32()),
        };
    };
    // The killed ssh's descendants may still hold the pipes; only join after exit.
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    match status.code() {
        // The JSON is one line; anything a login profile prints comes before it.
        Some(0) => match crate::status::parse(stdout.trim().lines().last().unwrap_or("")) {
            Ok(st) => Reach::Reachable(st),
            Err(crate::status::ParseError::WrongVersion(_)) => Reach::NeedsUpdate,
            Err(crate::status::ParseError::Invalid(e)) => Reach::Unreachable {
                error: format!("bad status output: {e}"),
            },
        },
        // ssh itself failed: network, auth, host key.
        Some(255) => Reach::Unreachable {
            error: stderr.trim().to_string(),
        },
        // grove ran but didn't know `status` (clap usage error) or failed.
        _ => Reach::NeedsUpdate,
    }
}

/// Background thread polling every finca each `interval` (and on `kick`).
pub struct Poller {
    pub results: std::sync::mpsc::Receiver<PollResult>,
    kick: std::sync::mpsc::Sender<()>,
}

impl Poller {
    pub fn start(
        store: crate::store::Store,
        interval: std::time::Duration,
        timeout: std::time::Duration,
    ) -> Self {
        let (tx, results) = std::sync::mpsc::channel();
        let (kick, kicked) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            loop {
                let fincas: Vec<Finca> = store
                    .list_fincas()
                    .into_iter()
                    .filter_map(Result::ok)
                    .collect();
                // Poll in parallel so one slow finca doesn't delay the rest.
                let handles: Vec<_> = fincas
                    .into_iter()
                    .map(|f| {
                        let tx = tx.clone();
                        std::thread::spawn(move || {
                            let (reach, via) = poll_via(&f, timeout);
                            tx.send(PollResult {
                                slug: f.slug(),
                                reach,
                                via,
                                at: chrono::Utc::now(),
                            })
                            .is_ok()
                        })
                    })
                    .collect();
                let alive = handles.into_iter().all(|h| h.join().unwrap_or(false));
                if !alive {
                    return; // dashboard dropped the receiver
                }
                match kicked.recv_timeout(interval) {
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        });
        Self { results, kick }
    }

    /// Poll again now (e.g. after a finca was added).
    pub fn kick(&self) {
        let _ = self.kick.send(());
    }
}

/// The tmux session name for a finca connection.
pub fn session_name(slug: &str) -> String {
    format!("finca-{slug}")
}

/// The command string ssh hands the remote shell: `bin` run via a login shell.
pub fn remote_login_cmd(bin: &str) -> String {
    login_shell_cmd(&sh_quote(bin))
}

/// `cmd` (already quoted for a shell) run by the remote user's login shell.
pub fn login_shell_cmd(cmd: &str) -> String {
    format!("exec \"$SHELL\" -lc {}", sh_quote(cmd))
}

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

/// Records `dest`'s host key if it's new (`accept-new`), without a
/// terminal and without a shared master: a password login riding on one
/// would make later key-only checks pass.
pub fn accept_host_key(dest: &str) -> Result<std::process::Output> {
    std::process::Command::new(ssh_program())
        .args(SSH_BATCH_OPTS)
        .args([
            "-o",
            "ControlPath=none",
            "-o",
            "StrictHostKeyChecking=accept-new",
        ])
        .arg(dest)
        .arg("true")
        .stdin(std::process::Stdio::null())
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

/// This binary's source folder, recorded at build time, if still there.
pub fn source_dir() -> Option<std::path::PathBuf> {
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.join("Cargo.toml").exists().then_some(p)
}

/// Whether this grove is a release build (the release workflow sets
/// GROVE_RELEASE_BUILD when it compiles one).
pub const RELEASE_BUILD: bool = option_env!("GROVE_RELEASE_BUILD").is_some();

/// How Grove gets onto a finca.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// Same OS and CPU as here: copy this grove.
    Copy,
    /// Download the release of this version built for the finca.
    Download,
    /// Build this grove's source there (needs cargo there).
    Build,
}

/// A finca like this machine gets a copy. Otherwise a source build builds
/// its checkout there, so the finca runs the same code; a release build,
/// one whose checkout is gone, or a 32-bit ARM board (too small to compile
/// Grove) downloads the matching release.
pub fn install_plan(
    facts: &crate::setup::Facts,
    release_build: bool,
    source: Option<&std::path::Path>,
) -> Plan {
    if facts.same_platform_as_here() {
        Plan::Copy
    } else if !release_build
        && !is_arm32(&facts.arch)
        && source.is_some_and(|s| s.join("Cargo.toml").exists())
    {
        Plan::Build
    } else {
        Plan::Download
    }
}

/// `install_plan` for `grove finca update`, which asks nothing: a finca
/// with no cargo (set up from a release) gets the release rather than a
/// build it can't do.
pub fn update_plan(
    facts: &crate::setup::Facts,
    release_build: bool,
    source: Option<&std::path::Path>,
) -> Plan {
    match install_plan(facts, release_build, source) {
        Plan::Build if !facts.has("cargo") => Plan::Download,
        plan => plan,
    }
}

/// 32-bit ARM, as `uname -m` says it (Pi Zero, older Pis).
fn is_arm32(arch: &str) -> bool {
    matches!(arch, "armv6l" | "armv7l")
}

/// The release asset's target for a finca's OS and CPU (`Facts` form).
pub fn release_target(os: &str, arch: &str) -> Result<String> {
    // One armv6 build, which armv7 runs too.
    if os == "linux" && is_arm32(arch) {
        return Ok("arm-unknown-linux-musleabihf".into());
    }
    let os = match os {
        "macos" => "apple-darwin",
        "linux" => "unknown-linux-musl",
        other => bail!("Grove has no release for {other}; install it there from source"),
    };
    match arch {
        "aarch64" | "x86_64" => Ok(format!("{arch}-{os}")),
        other => bail!("Grove has no release for {other} CPUs; install it there from source"),
    }
}

/// Where this version's release assets are: `$GROVE_DOWNLOAD_URL`, else
/// its GitHub release.
pub fn release_url(version: &str) -> String {
    std::env::var("GROVE_DOWNLOAD_URL")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| {
            format!("https://github.com/ajongaro/grove/releases/download/v{version}")
        })
}

/// Downloads grove `version` for `target` from `base`, checks it against
/// the release's SHA256SUMS, and keeps it under `run_dir`. Returns its path;
/// a second call uses the kept copy.
pub fn fetch_release(
    run_dir: &std::path::Path,
    base: &str,
    version: &str,
    target: &str,
) -> Result<std::path::PathBuf> {
    use std::process::Command;
    let dir = run_dir
        .join("releases")
        .join(format!("v{version}"))
        .join(target);
    let kept = dir.join("grove");
    if kept.is_file() {
        return Ok(kept);
    }
    let tmp = dir.with_extension(format!("part-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp)?;
    let result = (|| -> Result<()> {
        let asset = format!("grove-{target}.tar.gz");
        let get = |name: &str| -> Result<()> {
            let out = Command::new("curl")
                .args(["-fsSL", "-o"])
                .arg(tmp.join(name))
                .arg(format!("{base}/{name}"))
                .output()
                .context("running curl")?;
            if !out.status.success() {
                bail!(
                    "couldn't download Grove {version} for {target} ({base}/{name}): {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
            Ok(())
        };
        get(&asset)?;
        get("SHA256SUMS")?;
        let sums = std::fs::read_to_string(tmp.join("SHA256SUMS"))?;
        let want = sums
            .lines()
            .filter_map(|l| l.split_once(char::is_whitespace))
            .find(|(_, name)| name.trim().trim_start_matches('*') == asset)
            .map(|(sum, _)| sum.to_string())
            .with_context(|| format!("Grove {version}'s SHA256SUMS has no {asset}"))?;
        let got = sha256(&tmp.join(&asset))?;
        if got != want {
            bail!("checksum mismatch for Grove {version} ({asset}); not installing it");
        }
        let st = Command::new("tar")
            .arg("-xzf")
            .arg(tmp.join(&asset))
            .arg("-C")
            .arg(&tmp)
            .arg("grove")
            .status()
            .context("running tar")?;
        if !st.success() {
            bail!("couldn't unpack {asset}");
        }
        std::fs::create_dir_all(&dir)?;
        std::fs::rename(tmp.join("grove"), &kept)?;
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result.map(|()| kept)
}

/// A file's SHA-256, by `shasum` (macOS) or `sha256sum` (Linux).
fn sha256(path: &std::path::Path) -> Result<String> {
    for (prog, args) in [("shasum", &["-a", "256"][..]), ("sha256sum", &[][..])] {
        if let Ok(out) = std::process::Command::new(prog)
            .args(args)
            .arg(path)
            .output()
            && out.status.success()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(sum) = text.split_whitespace().next() {
                return Ok(sum.to_string());
            }
        }
    }
    bail!("needs shasum or sha256sum to check the download")
}

/// What `install_grove` puts on the finca.
pub enum How<'a> {
    /// This binary, which runs there.
    Copy(&'a std::path::Path),
    /// Grove's source folder, built there.
    Build(&'a std::path::Path),
}

/// Puts grove at `bin` on the finca and returns its `--version` line.
/// `wrap` runs a build under the login shell.
pub fn install_grove(
    dest: &str,
    facts: &crate::setup::Facts,
    bin: &str,
    how: How,
    wrap: fn(&str) -> String,
) -> Result<String> {
    let b = sh_quote(bin);
    let new = sh_quote(&format!("{bin}.new"));
    // Rename over the old binary so a running grove keeps its (old) inode.
    // ...and only once the new one runs there.
    let swap = format!(
        "if {new} --version >/dev/null 2>&1; then mv -f {new} {b} && {b} --version; else rm -f {new}; echo 'the new grove does not run there' >&2; exit 1; fi"
    );
    let mkdir = format!("mkdir -p \"$(dirname {b})\"");
    let out = match how {
        How::Copy(file) => {
            let f =
                std::fs::File::open(file).with_context(|| format!("reading {}", file.display()))?;
            remote_stdin(
                dest,
                &format!("set -e; {mkdir}; cat > {new}; chmod 755 {new}; {swap}"),
                f.into(),
            )?
        }
        How::Build(src) => {
            if !src.join("Cargo.toml").exists() {
                bail!(
                    "Grove's source isn't at {} any more, so it can't be built on {}",
                    src.display(),
                    facts.host
                );
            }
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
        }
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

/// Replaces ~/src/grove on the finca with `src` (keeping its build cache
/// and any clone's .git),
/// streamed as a tar over ssh.
fn ship_source(dest: &str, src: &std::path::Path) -> Result<()> {
    use std::process::{Command, Stdio};
    let mut tar = Command::new("tar")
        // macOS tar would add ._ files for extended attributes.
        .env("COPYFILE_DISABLE", "1")
        .arg("-C")
        .arg(src)
        .args([
            "--exclude=./target",
            "--exclude=./.git",
            "--exclude=./.superpowers",
            "-cf",
            "-",
            ".",
        ])
        .stdout(Stdio::piped())
        .spawn()
        .context("running tar")?;
    let stdout = tar.stdout.take().context("tar output")?;
    let out = remote_stdin(
        dest,
        "set -e; mkdir -p ~/src/grove; cd ~/src/grove; find . -mindepth 1 -maxdepth 1 ! -name target ! -name .git -exec rm -rf {} +; tar -xf -",
        stdout.into(),
    )?;
    let tar_ok = tar.wait()?.success();
    if !out.status.success() || !tar_ok {
        bail!(
            "copying Grove's source to {dest} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// `sh -c` command that runs interactive ssh to `dest` (one of the finca's
/// destinations) into its grove and, if ssh fails, pauses so the error can
/// be read.
pub fn connect_argv(f: &Finca, dest: &str) -> Vec<String> {
    // Run grove under the remote user's login shell so its profile PATH
    // applies (e.g. Homebrew's tmux and claude on macOS). Quoting layers:
    // this local sh → ssh's remote shell → the login shell's -c.
    let remote = remote_login_cmd(&f.grove_bin);
    vec!["sh".into(), "-c".into(), ssh_script(f, dest, Some(&remote))]
}

/// Like `connect_argv`, running `cmd` (quoted for a shell) there instead
/// of the dashboard.
pub fn remote_argv(f: &Finca, dest: &str, cmd: &str) -> Vec<String> {
    let remote = login_shell_cmd(cmd);
    vec!["sh".into(), "-c".into(), ssh_script(f, dest, Some(&remote))]
}

/// Like `connect_argv`, but with no remote command: ssh starts the remote
/// user's login shell.
pub fn shell_argv(f: &Finca, dest: &str) -> Vec<String> {
    vec!["sh".into(), "-c".into(), ssh_script(f, dest, None)]
}

/// Interactive ssh to `dest`, running `remote` there if given; on failure it
/// pauses so the error can be read.
fn ssh_script(f: &Finca, dest: &str, remote: Option<&str>) -> String {
    ssh_script_with(f, dest, remote, CONNECT_GRACE_SECS)
}

/// How long ssh must have run for a failure to count as a lost connection
/// rather than one that never got going.
const CONNECT_GRACE_SECS: u64 = 10;

fn ssh_script_with(f: &Finca, dest: &str, remote: Option<&str>, grace: u64) -> String {
    // Keep-alives end ssh within ~45s of a dead link (laptop slept, network
    // changed) instead of leaving a frozen session; share_opts has them.
    let mut opts = share_opts();
    if opts.is_empty() {
        opts = [
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=3",
        ]
        .map(String::from)
        .to_vec();
    }
    let opts: Vec<String> = opts.iter().map(|o| sh_quote(o)).collect();
    let mut ssh = format!(
        "{} -t {} {}",
        sh_quote(&ssh_program()),
        opts.join(" "),
        sh_quote(dest),
    );
    if let Some(r) = remote {
        ssh = format!("{ssh} {}", sh_quote(r));
    }
    let other = if remote.is_some() {
        "Grove on $name exited (code $code)"
    } else {
        "The shell on $name exited (code $code)"
    };
    // ssh's own messages (not the remote's output, which -t sends through
    // the tty) go to $err, so the reason can be shown after the screen is
    // reset. The reset leaves whatever full-screen view the remote left
    // behind and turns off the mouse, focus, paste and theme reports it
    // asked for, which would otherwise arrive as typing at the prompt.
    format!(
        r#"name={name}
start=$(date +%s)
err=$(mktemp "${{TMPDIR:-/tmp}}/grove-ssh.XXXXXX" 2>/dev/null) || err=/dev/null
{ssh} 2>"$err"
code=$?
if [ "$code" -ne 0 ]; then
  if [ "$code" -eq 255 ]; then
    if [ $(( $(date +%s) - start )) -ge {grace} ]; then
      msg="Lost the connection to $name"
    else
      msg="Couldn't connect to $name"
    fi
  else
    msg="{other}"
  fi
  why=$(grep -v '^[[:space:]]*$' "$err" | tail -n 1)
  printf '\033[?1049l\033[?1000l\033[?1002l\033[?1003l\033[?1006l\033[?1004l\033[?2004l\033[?2031l\033[?25h\033[0m\033[2J\033[H'
  printf '\n  %s\n' "$msg"
  [ -n "$why" ] && printf '  %s\n' "$why"
  printf '\n  Press Enter to close.'
  stty -echo 2>/dev/null
  read _
  stty echo 2>/dev/null
fi
[ "$err" = /dev/null ] || rm -f "$err""#,
        name = sh_quote(&f.name),
    )
}

pub fn ensure_finca_session(tmux: &crate::tmux::Tmux, f: &Finca, dest: &str) -> Result<bool> {
    let name = session_name(&f.slug());
    if tmux.has_session(&name) {
        return Ok(false);
    }
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/".into());
    tmux.new_session_detached(&name, &f.name, &home, &connect_argv(f, dest))?;
    // Where this connection goes, for C-v (transfer::paste).
    tmux.set_option(&name, "@grove_dest", dest)?;
    tmux.set_option(&name, "key-table", "grove-finca")?;
    tmux.set_option(&name, "prefix", "None")?;
    // When ssh ends (remote q, dropped link) move the client to another
    // session (normally the dashboard) instead of detaching it.
    tmux.set_option(&name, "detach-on-destroy", "off")?;
    if let Some(c) = &f.color {
        let _ = tmux.set_option(&name, "status-style", &format!("bg={c},fg=#ffffff"));
    }
    Ok(true)
}

/// The finca slug of a finca connection (`finca-<slug>`) or shell
/// (`finca-<slug>-sh`) session.
pub fn finca_for_session(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("finca-")?;
    Some(rest.strip_suffix("-sh").unwrap_or(rest)).filter(|s| !s.is_empty())
}

/// The tmux session name for a finca's shell.
pub fn shell_session_name(slug: &str) -> String {
    format!("finca-{slug}-sh")
}

/// A login shell on the finca in its own session. Unlike the grove
/// connection it keeps the normal prefix (splits, windows), and new windows
/// ssh to the finca too; only C-q is taken (the grove-shell table).
pub fn ensure_shell_session(tmux: &crate::tmux::Tmux, f: &Finca, dest: &str) -> Result<bool> {
    let name = shell_session_name(&f.slug());
    if tmux.has_session(&name) {
        return Ok(false);
    }
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/".into());
    let argv = shell_argv(f, dest);
    tmux.new_session_detached(&name, &f.name, &home, &argv)?;
    // Where this connection goes, for C-v (transfer::paste).
    tmux.set_option(&name, "@grove_dest", dest)?;
    tmux.set_option(&name, "key-table", "grove-shell")?;
    tmux.set_option(
        &name,
        "default-command",
        &format!("sh -c {}", sh_quote(&argv[2])),
    )?;
    tmux.set_option(&name, "detach-on-destroy", "off")?;
    if let Some(c) = &f.color {
        let _ = tmux.set_option(&name, "status-style", &format!("bg={c},fg=#ffffff"));
    }
    Ok(true)
}

/// A window of a finca's shell session (`s`), listed on the dashboard.
#[derive(Debug, Clone, PartialEq)]
pub struct ShellWindow {
    pub id: String,
    pub index: u32,
    /// What the remote shell titled it (often user@host: dir), if anything.
    pub title: Option<String>,
    /// The name you gave it (`r`), if any.
    pub name: Option<String>,
}

/// The window option holding a finca shell's name. Window names there
/// can't tell: new windows are named after their command.
pub const SHELL_NAME: &str = "@grove_name";

impl ShellWindow {
    /// Its name, else "shell <index>".
    pub fn label(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| format!("shell {}", self.index))
    }
}

/// Names one of a finca's shell windows (the dashboard and tmux show it).
pub fn rename_shell(tmux: &crate::tmux::Tmux, window: &str, name: &str) -> Result<()> {
    tmux.set_window_option(window, SHELL_NAME, name)?;
    tmux.rename_window(window, name)
}

/// The finca's open shell windows; none if no shell session is open.
pub fn shell_windows(tmux: &crate::tmux::Tmux, slug: &str) -> Vec<ShellWindow> {
    tmux.list_windows_as(
        &shell_session_name(slug),
        &format!(
            "#{{window_id}}\t#{{window_index}}\t#{{pane_title}}\t#{{host}}\t#{{{SHELL_NAME}}}"
        ),
    )
    .map(|lines| parse_shell_windows(&lines.join("\n")))
    .unwrap_or_default()
}

/// Parses `id \t index \t pane title \t host \t name` lines. A title equal
/// to the host name is tmux's default, not something the remote shell set.
fn parse_shell_windows(out: &str) -> Vec<ShellWindow> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let (id, index, title, host) = (f.next()?, f.next()?, f.next()?, f.next()?);
            let name = f.next().filter(|n| !n.is_empty()).map(str::to_string);
            Some(ShellWindow {
                id: id.to_string(),
                index: index.parse().ok()?,
                title: (!title.is_empty() && title != host).then(|| title.to_string()),
                name,
            })
        })
        .collect()
}

/// Makes `window` current in the finca's Grove session, so the connection
/// shows it (now, or when it next attaches).
pub fn focus(f: &Finca, dest: &str, window: &str) -> Result<()> {
    let cmd = login_shell_cmd(&format!(
        "{} focus {}",
        sh_quote(&f.grove_bin),
        sh_quote(window)
    ));
    let out = std::process::Command::new(ssh_program())
        .args(SSH_BATCH_OPTS)
        .args(share_opts())
        .arg(dest)
        .arg(cmd)
        .stdin(std::process::Stdio::null())
        .output()
        .with_context(|| format!("running {}", ssh_program()))?;
    if !out.status.success() {
        bail!(
            "couldn't open that session on {}: {}",
            f.name,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Switches the current client to a shell on the finca, like `connect`.
pub fn open_shell(tmux: &crate::tmux::Tmux, f: &Finca, via: Option<&str>) -> Result<()> {
    ensure_shell_session(tmux, f, via.unwrap_or(&f.ssh))?;
    tmux.switch_client(&format!("={}", shell_session_name(&f.slug())))
}

/// Connects the current client to the finca (creating the session if
/// needed) at `via`, the destination that last answered a poll, else `ssh`.
pub fn connect(tmux: &crate::tmux::Tmux, f: &Finca, via: Option<&str>) -> Result<()> {
    ensure_finca_session(tmux, f, via.unwrap_or(&f.ssh))?;
    tmux.switch_client(&format!("={}", session_name(&f.slug())))
}

/// Closes the finca's connection and shell sessions, if open.
pub fn disconnect(tmux: &crate::tmux::Tmux, slug: &str) {
    let _ = tmux.kill_session(&session_name(slug));
    let _ = tmux.kill_session(&shell_session_name(slug));
}

#[cfg(test)]
pub mod testing {
    //! A stand-in for `ssh`: skips options and the destination, then runs the
    //! remote command locally with `$FAKE_HOME` as HOME.
    use std::path::{Path, PathBuf};

    pub fn fake_ssh(dir: &Path, exit_code: Option<i32>) -> PathBuf {
        let p = dir.join("fake ssh");
        let body = match exit_code {
            Some(code) => format!("#!/bin/sh\necho 'ssh: connect to host x: Connection refused' >&2\nexit {code}\n"),
            None => "#!/bin/sh\nwhile [ $# -gt 0 ]; do case \"$1\" in -o) shift 2;; -t|-T) shift;; *) break;; esac; done\nshift\nHOME=\"$FAKE_HOME\" PATH=\"${FAKE_PATH:-$PATH}\" exec sh -c \"$*\"\n".into(),
        };
        std::fs::write(&p, body).unwrap();
        std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        p
    }

    /// A fake grove that answers --version (as this version) and
    /// `status --json` with `json`.
    pub fn fake_grove(dir: &Path, json: &str) -> PathBuf {
        let p = dir.join("grove");
        let body = format!(
            "#!/bin/sh\ncase \"$1\" in --version) echo 'grove {}';; status) cat <<'JSON'\n{json}\nJSON\n;; *) exit 2;; esac\n",
            env!("CARGO_PKG_VERSION")
        );
        std::fs::write(&p, body).unwrap();
        std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        p
    }
}

#[cfg(test)]
#[path = "finca_tests.rs"]
pub(crate) mod tests;
