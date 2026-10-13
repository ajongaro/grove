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
