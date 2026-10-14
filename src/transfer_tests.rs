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

/// A fake osascript: `FAKE_CLIP_FILE` answers the file query, and
/// `FAKE_CLIP_PNG` (a file) the image query. Like the real one, a failed
/// image read leaves an empty file behind.
fn fake_osascript(dir: &Path) -> std::path::PathBuf {
    crate::finca::tests::script(
        dir,
        "osascript",
        "for a; do last=$a; done\n\
         case \"$*\" in\n\
         *furl*) [ -n \"$FAKE_CLIP_FILE\" ] || exit 1; echo \"$FAKE_CLIP_FILE\";;\n\
         *PNGf*) : > \"$last\"; [ -n \"$FAKE_CLIP_PNG\" ] || exit 1; cat \"$FAKE_CLIP_PNG\" > \"$last\";;\n\
         *) exit 1;;\n\
         esac",
    )
}

fn with_clip<T>(dir: &Path, file: Option<&Path>, png: Option<&Path>, f: impl FnOnce() -> T) -> T {
    let _g = crate::finca::tests::ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let set = |k: &str, v: Option<&Path>| match v {
        Some(v) => unsafe { std::env::set_var(k, v) },
        None => unsafe { std::env::remove_var(k) },
    };
    set("GROVE_OSASCRIPT", Some(&fake_osascript(dir)));
    set("FAKE_CLIP_FILE", file);
    set("FAKE_CLIP_PNG", png);
    let r = f();
    for k in ["GROVE_OSASCRIPT", "FAKE_CLIP_FILE", "FAKE_CLIP_PNG"] {
        unsafe { std::env::remove_var(k) };
    }
    r
}

#[test]
fn file_reference_wins_over_an_image() {
    let d = tempfile::tempdir().unwrap();
    let doc = d.path().join("report.pdf");
    std::fs::write(&doc, "pdf").unwrap();
    let png = d.path().join("icon.png");
    std::fs::write(&png, "icon").unwrap();
    let clip = with_clip(d.path(), Some(&doc), Some(&png), || {
        read_clipboard(d.path())
    });
    assert_eq!(clip, Clip::File(doc));
}

#[test]
fn image_is_written_to_a_temp_png() {
    let d = tempfile::tempdir().unwrap();
    let png = d.path().join("shot.png");
    std::fs::write(&png, "pixels").unwrap();
    let clip = with_clip(d.path(), None, Some(&png), || read_clipboard(d.path()));
    let Clip::Image(p) = clip else {
        panic!("{clip:?}")
    };
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "pixels");
}

#[test]
fn text_clipboard_is_nothing_and_leaves_no_temp_file() {
    let d = tempfile::tempdir().unwrap();
    let run = d.path().join("run");
    std::fs::create_dir_all(&run).unwrap();
    let clip = with_clip(d.path(), None, None, || read_clipboard(&run));
    assert_eq!(clip, Clip::Nothing);
    assert_eq!(std::fs::read_dir(&run).unwrap().count(), 0);
}
