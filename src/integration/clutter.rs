//! The clutter report on a real folder: empty folders, broken symbolic
//! links and temporary files, found by the real scanner and moved to the
//! (test) trash through the interface.

use super::{now, scan, Fixture, KIB};
use crate::reports::{self, ReportKind};
use crate::tree::{SizeMode, ROOT};

fn fixture() -> Fixture {
    let mut f = Fixture::new();
    f.file("Notes/keep.txt", 3 * KIB, b'k', 10);
    f.file("Work/report.tmp", 2 * KIB, b't', 3);
    f.file("Work/fresh.tmp", 2 * KIB, b'f', 0);
    f.file("Work/.DS_Store", 6 * KIB, b'd', 0);
    f.file("Project/main.rs", KIB, b'r', 10);
    for dir in [
        "Empty/Nested/Deeper",
        "Notes/Sub",
        "Project/.git/refs/heads",
        "Tools/Thing.app/Contents",
    ] {
        std::fs::create_dir_all(f.path(dir)).unwrap();
    }
    f
}

#[test]
fn finds_empty_folders_and_old_temporary_files() {
    let f = fixture();
    let t = scan(f.root()).tree;
    let list = reports::run(&t, ROOT, SizeMode::Apparent, now(), ReportKind::Clutter, 0);
    let names = |label: &str| -> Vec<String> {
        let mut v: Vec<String> = list
            .rows
            .iter()
            .find(|r| r.label == label)
            .map(|r| {
                r.nodes
                    .iter()
                    .map(|&id| crate::lists::relative_label(&t, ROOT, id).replace('\\', "/"))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    };
    assert_eq!(names("Boş klasörler"), ["Empty/", "Notes/Sub/"]);
    assert_eq!(
        names("Geçici dosyalar"),
        ["Work/.DS_Store", "Work/report.tmp"]
    );
    assert!(names("Kırık bağlantılar").is_empty());
}

#[cfg(unix)]
#[test]
fn broken_links_are_found_and_moved_to_the_trash() {
    use super::{browser, menu, open, press, rows, tick_until};
    use crossterm::event::KeyCode;

    let f = fixture();
    let link = |target: &str, at: &str| {
        std::fs::create_dir_all(f.path(at).parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(target, f.path(at)).unwrap();
    };
    link("/nonexistent/rustclean-test", "Links/broken");
    link(f.path("Notes/keep.txt").to_str().unwrap(), "Links/fine");
    link("/nonexistent/rustclean-test", ".hidden/broken");

    let mut app = open(f.root());
    // Menu item 10: clutter.
    menu(&mut app, 10);
    tick_until(&mut app, "the report", |a| {
        browser(a)
            .results
            .as_ref()
            .is_some_and(|r| r.report() == Some(ReportKind::Clutter))
    });
    let groups = rows(&app);
    let at = groups
        .iter()
        .position(|g| g == "Kırık bağlantılar")
        .expect("a link group");
    for _ in 0..at {
        press(&mut app, KeyCode::Down);
    }
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        rows(&app),
        ["Links/broken"],
        "the working and the hidden link stay out"
    );

    // The link itself goes to the trash; the missing target is not needed.
    press(&mut app, KeyCode::Char('x'));
    press(&mut app, KeyCode::Char('e'));
    tick_until(&mut app, "the deletion", |a| browser(a).deleting.is_none());
    assert!(std::fs::symlink_metadata(f.path("Links/broken")).is_err());
    assert!(std::fs::symlink_metadata(f.path("Links/fine")).is_ok());
    assert!(browser(&app).failures.is_none());
}
