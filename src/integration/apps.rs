//! Apps and their data on a real folder tree (macOS layout), and
//! uninstalling through the dialog.

#![cfg(target_os = "macos")]

use crossterm::event::KeyCode;

use super::{browser, find, menu, now, open, press, rows, scan, tick_until, Fixture, KIB, MIB};
use crate::apps;
use crate::reports::ReportKind;
use crate::tree::SizeMode;

const PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.example.sketchpad</string>
<key>CFBundleName</key><string>Sketchpad</string>
</dict></plist>
"#;

/// An installed app with data in Application Support and Caches, and the
/// data of an app that is gone.
fn fixture() -> Fixture {
    let mut f = Fixture::new();
    let plist = f.path("Applications/Sketchpad.app/Contents/Info.plist");
    std::fs::create_dir_all(plist.parent().unwrap()).unwrap();
    std::fs::write(&plist, PLIST).unwrap();
    f.file(
        "Applications/Sketchpad.app/Contents/MacOS/Sketchpad",
        200 * KIB,
        b'e',
        0,
    );
    f.file(
        "Library/Application Support/com.example.sketchpad/state",
        300 * KIB,
        b's',
        0,
    );
    f.file(
        "Library/Caches/com.example.sketchpad/blob",
        100 * KIB,
        b'c',
        0,
    );
    f.file(
        "Library/Application Support/com.example.longgone/data",
        2 * MIB,
        b'g',
        400,
    );
    f
}

#[test]
fn an_app_is_grouped_with_its_data() {
    let f = fixture();
    let t = scan(f.root()).tree;
    let list = apps::run(&t, SizeMode::Apparent);
    let row = list
        .rows
        .iter()
        .find(|r| r.label == "Sketchpad")
        .expect("the app row");
    let app = find(&t, "Applications/Sketchpad.app");
    assert_eq!(row.nodes[0], app, "the bundle comes first");
    let mut data: Vec<_> = row.nodes[1..].iter().map(|&id| t.path_of(id)).collect();
    data.sort();
    assert_eq!(
        data,
        [
            t.path_of(find(
                &t,
                "Library/Application Support/com.example.sketchpad"
            )),
            t.path_of(find(&t, "Library/Caches/com.example.sketchpad")),
        ]
    );
    assert_eq!(row.size(), 600 * KIB + u64::try_from(PLIST.len()).unwrap());
}

#[test]
fn leftovers_of_a_removed_app_are_listed_and_installed_ones_are_not() {
    let f = fixture();
    let t = scan(f.root()).tree;
    let list = apps::orphans(&t, SizeMode::Apparent, now(), 0);
    let labels: Vec<_> = list.rows.iter().map(|r| r.label.clone()).collect();
    assert!(
        labels.iter().any(|l| l.contains("com.example.longgone")),
        "{labels:?}"
    );
    assert!(
        !labels.iter().any(|l| l.contains("com.example.sketchpad")),
        "{labels:?}"
    );
}

#[test]
fn uninstall_moves_the_app_and_its_data_to_the_trash() {
    let f = fixture();
    let mut app = open(f.root());
    // Menu item 3: apps and their data.
    menu(&mut app, 3);
    tick_until(&mut app, "the Apps report", |a| {
        browser(a)
            .results
            .as_ref()
            .is_some_and(|r| r.report == Some(ReportKind::Apps))
    });
    let at = rows(&app).iter().position(|l| l == "Sketchpad").unwrap();
    for _ in 0..at {
        press(&mut app, KeyCode::Down);
    }

    press(&mut app, KeyCode::Char('u'));
    let b = browser(&app);
    let dialog = b.uninstall.as_ref().expect("the uninstall dialog");
    assert_eq!(
        dialog.items[0].0,
        find(&b.tree, "Applications/Sketchpad.app")
    );
    assert_eq!(dialog.items.len(), 3);
    assert!(dialog.items.iter().all(|i| i.1), "everything checked");

    // Uncheck the cache, then uninstall the rest.
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char(' '));
    press(&mut app, KeyCode::Char('e'));
    tick_until(&mut app, "the uninstall", |a| browser(a).deleting.is_none());

    assert!(!f.path("Applications/Sketchpad.app").exists());
    let state = f.path("Library/Application Support/com.example.sketchpad");
    let cache = f.path("Library/Caches/com.example.sketchpad");
    // The dialog lists data in report order; whichever was third stays.
    assert!(
        state.exists() != cache.exists(),
        "exactly one data folder kept"
    );
    assert!(!rows(&app).contains(&"Sketchpad".to_string()));
}
