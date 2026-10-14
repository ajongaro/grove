use super::*;
use crate::finca::testing::fake_ssh;
use crate::finca::tests::with_env;

fn at(h: u32, m: u32, s: u32) -> chrono::DateTime<chrono::Local> {
    use chrono::TimeZone;
    chrono::Local
        .with_ymd_and_hms(2026, 10, 14, h, m, s)
        .unwrap()
}

#[test]
fn names_are_cleaned_and_stamped() {
    assert_eq!(
        remote_name("shot.png", at(9, 30, 12)),
        "20261014-093012-shot.png"
    );
    assert_eq!(
        remote_name("Screen Shot 'a' é.png", at(9, 30, 12)),
        "20261014-093012-Screen-Shot--a---.png"
    );
    assert_eq!(remote_name("", at(1, 2, 3)), "20261014-010203-file");
}

#[test]
fn session_names_map_to_finca_slugs() {
    assert_eq!(
        crate::finca::finca_for_session("finca-homebox"),
        Some("homebox")
    );
    assert_eq!(
        crate::finca::finca_for_session("finca-homebox-sh"),
        Some("homebox")
    );
    assert_eq!(crate::finca::finca_for_session("grove"), None);
}

fn finca(ssh: &str, backup: Option<&str>) -> Finca {
    Finca {
        name: "homebox".into(),
        ssh: ssh.into(),
        grove_bin: "/g".into(),
        color: None,
        backup: backup.map(str::to_string),
    }
}

#[test]
fn recorded_destination_must_belong_to_the_finca() {
    let d = tempfile::tempdir().unwrap();
    let ssh = fake_ssh(d.path(), None);
    let f = finca("me@homebox.local", Some("me@homebox.ts.net"));
    with_env(&ssh, d.path(), || {
        assert_eq!(
            pick_dest(&f, Some("me@homebox.ts.net")).unwrap(),
            "me@homebox.ts.net"
        );
        // Not one of its destinations (the file changed): probe instead.
        assert_eq!(
            pick_dest(&f, Some("me@elsewhere")).unwrap(),
            "me@homebox.local"
        );
        assert_eq!(pick_dest(&f, None).unwrap(), "me@homebox.local");
    });
    std::fs::create_dir_all(d.path().join("down")).unwrap();
    let down = fake_ssh(&d.path().join("down"), Some(255));
    let err = with_env(&down, d.path(), || pick_dest(&f, None)).unwrap_err();
    assert!(format!("{err:#}").contains("homebox"), "{err:#}");
}

#[test]
fn send_lands_in_the_inbox_and_clears_old_files() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("remote home");
    let inbox = home.join(".grove/inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    let old = inbox.join("20200101-000000-old.png");
    std::fs::write(&old, "x").unwrap();
    std::process::Command::new("touch")
        .args(["-t", "202001010000"])
        .arg(&old)
        .status()
        .unwrap();
    let src = d.path().join("shot.png");
    std::fs::write(&src, b"\x89PNG data").unwrap();
    let ssh = fake_ssh(d.path(), None);
    let path = with_env(&ssh, &home, || {
        send("me@box", &src, "20261014-093012-shot.png")
    })
    .unwrap();
    assert_eq!(
        path,
        inbox.join("20261014-093012-shot.png").to_string_lossy()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"\x89PNG data");
    assert!(!old.exists(), "older than 7 days");
    assert!(!inbox.join("20261014-093012-shot.png.part").exists());
}
