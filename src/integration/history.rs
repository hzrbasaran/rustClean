//! Saved scans and "what changed" between two scans of a real folder.

use crossterm::event::KeyCode;

use super::{browser, menu, now, open, press, rows, scan, tick_until, Fixture, MIB};
use crate::history;

#[test]
fn changes_since_an_earlier_scan() {
    let mut f = Fixture::standard();
    // An earlier scan, saved an hour ago.
    let before = scan(f.root()).tree;
    let dir = history::dir_for(before.root_path()).unwrap();
    history::save(&dir, &history::capture(&before, now() - 3600)).unwrap();

    f.file("Media/new.mov", 2 * MIB, b'N', 0);
    f.file("New/blob", 3 * MIB, b'W', 0);
    f.remove("Backup");

    let mut app = open(f.root());
    // The app saves this scan too, in the background.
    tick_until(&mut app, "the scan to be saved", |_| {
        history::list(&dir).len() == 2
    });

    // Changes, which lists the earlier scans.
    menu(&mut app, super::index_of(crate::reports::MenuItem::Changes));
    let picker = browser(&app)
        .snapshot_picker
        .as_ref()
        .expect("the scan picker");
    assert_eq!(picker.0.len(), 1, "the current scan is not offered");
    press(&mut app, KeyCode::Enter);

    assert_eq!(rows(&app), ["New/", "Media/"]);
    let r = browser(&app).results.as_ref().unwrap();
    assert!(r.rows[0].detail.starts_with("yeni"), "{}", r.rows[0].detail);
    assert!(r.note.contains("Backup"), "{}", r.note);
}

#[test]
fn without_an_earlier_scan_changes_explains_itself() {
    let f = Fixture::standard();
    let mut app = open(f.root());
    let dir = history::dir_for(browser(&app).tree.root_path()).unwrap();
    tick_until(&mut app, "the scan to be saved", |_| {
        history::list(&dir).len() == 1
    });

    menu(&mut app, super::index_of(crate::reports::MenuItem::Changes));
    let b = browser(&app);
    assert!(b.snapshot_picker.is_none());
    assert!(b.status.as_ref().unwrap().error);
}

#[test]
fn saved_scans_round_trip() {
    let f = Fixture::standard();
    let t = scan(f.root()).tree;
    let dir = history::dir_for(t.root_path()).unwrap();
    let snap = history::capture(&t, now());
    let file = history::save(&dir, &snap).unwrap();
    let back = history::load(&file).unwrap();
    assert_eq!(back.header.total, snap.header.total);
    assert_eq!(back.entries.len(), snap.entries.len());
    assert!(back.entries.iter().any(|e| e.path == "Media"));
}
