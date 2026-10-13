use super::*;
use crate::finca::testing::{fake_grove, fake_ssh};
use crate::finca::tests::with_env;

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
    assert_eq!(
        (pi.os.as_str(), pi.arch.as_str(), pi.grove.clone()),
        ("linux", "aarch64", None)
    );
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
    assert_eq!(
        install_command("tmux", &f),
        None,
        "no brew: say what to install"
    );
    let mut l =
        parse_facts("os=Linux\narch=aarch64\nhost=pi\nhome=/home/me\nhas=apt-get\n").unwrap();
    assert_eq!(
        install_command("tmux", &l),
        Some("sudo apt-get install -y tmux")
    );
    l.tools = ["dnf".to_string()].into();
    assert_eq!(
        install_command("tmux", &l),
        Some("sudo dnf install -y tmux")
    );
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
    assert_eq!(
        tailscale_backup(json, "homebox").as_deref(),
        Some("homebox.tail1234.ts.net")
    );
    assert_eq!(tailscale_backup(json, "laptop"), None);
    assert_eq!(
        tailscale_backup(r#"{"Peer":null}"#, "pi"),
        None,
        "logged out"
    );
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

type OnRun = Box<dyn FnMut(&[String]) -> bool>;

/// Scripted answers; records everything said and run.
struct Script {
    answers: std::collections::VecDeque<&'static str>,
    said: Vec<String>,
    ran: Vec<Vec<String>>,
    on_run: OnRun,
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
    home: std::path::PathBuf, // the "remote" home
    bin: std::path::PathBuf,  // its PATH dir
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
    World {
        home,
        bin,
        store,
        ssh,
        exe,
        ssh_dir,
        _d: d,
    }
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
    assert!(
        f.grove_bin.ends_with("/.local/bin/grove"),
        "{}",
        f.grove_bin
    );
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
    assert!(
        io.said.iter().any(|l| l.contains("already")),
        "{:?}",
        io.said
    );
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
        assert!(
            argv.iter().any(|a| a == "StrictHostKeyChecking=accept-new"),
            "{argv:?}"
        );
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
    let host = String::from_utf8(
        std::process::Command::new("sh")
            .args(["-c", "hostname -s 2>/dev/null || hostname"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let json = format!(
        r#"{{"Peer":{{"a":{{"HostName":"{}","DNSName":"box.tail.ts.net."}}}}}}"#,
        host.trim()
    );
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
