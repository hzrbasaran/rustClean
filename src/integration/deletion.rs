//! Moving entries to the trash through the interface: select, confirm, the
//! background deletion, and what the browser shows afterwards. Test builds
//! move entries into `delete::test_trash` instead of the real trash.

use crossterm::event::KeyCode;

use super::{browser, menu, open, press, rows, tick_until, Fixture};
use crate::app::App;
use crate::delete::test_trash;
use crate::reports::ReportKind;

/// The app on the Downloads report: setup.dmg, then photos.zip.
fn downloads(f: &Fixture) -> App {
    let mut app = open(f.root());
    menu(&mut app, 8);
    tick_until(&mut app, "the Downloads report", |a| {
        browser(a)
            .results
            .as_ref()
            .is_some_and(|r| r.report() == Some(ReportKind::Downloads))
    });
    app
}

fn wait_for_deletion(app: &mut App) {
    tick_until(app, "the deletion", |a| browser(a).deleting.is_none());
}

/// Whether the test trash holds an entry moved from a file called `name`.
fn in_trash(name: &str) -> bool {
    std::fs::read_dir(test_trash::dir())
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            e.file_name()
                .to_string_lossy()
                .ends_with(&format!("-{name}"))
        })
}

#[test]
fn x_asks_first_then_moves_the_entry_to_the_trash() {
    let f = Fixture::standard();
    let mut app = downloads(&f);
    let dmg = f.path("Downloads/setup.dmg");

    press(&mut app, KeyCode::Char('x'));
    assert_eq!(browser(&app).confirm.as_ref().map(Vec::len), Some(1));
    assert!(dmg.exists(), "nothing happens before the answer");

    // Any other key cancels.
    press(&mut app, KeyCode::Char('h'));
    assert!(browser(&app).confirm.is_none());
    assert!(dmg.exists());

    press(&mut app, KeyCode::Char('x'));
    press(&mut app, KeyCode::Char('e'));
    wait_for_deletion(&mut app);

    assert!(!dmg.exists());
    assert!(in_trash("setup.dmg"));
    let b = browser(&app);
    assert!(
        !b.status.as_ref().unwrap().error,
        "{:?}",
        b.status.as_ref().unwrap().text
    );
    assert_eq!(rows(&app), ["Downloads/photos.zip"]);
    assert_eq!(b.trashed.apparent, 300 * super::KIB);
    assert!(b.failures.is_none());
}

#[test]
fn the_basket_is_moved_as_one_batch() {
    let f = Fixture::standard();
    let mut app = downloads(&f);
    // Space adds the row and moves to the next one.
    press(&mut app, KeyCode::Char(' '));
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(browser(&app).basket.items().len(), 2);

    press(&mut app, KeyCode::Char('x'));
    assert_eq!(browser(&app).confirm.as_ref().map(Vec::len), Some(2));
    press(&mut app, KeyCode::Char('e'));
    wait_for_deletion(&mut app);

    assert!(!f.path("Downloads/setup.dmg").exists());
    assert!(!f.path("Downloads/photos.zip").exists());
    assert!(f.path("Downloads/notes.txt").exists(), "not selected");
    let b = browser(&app);
    assert!(b.basket.is_empty());
    assert!(rows(&app).is_empty());
}

#[test]
fn entries_gone_from_disk_are_refused_before_asking() {
    let f = Fixture::standard();
    let mut app = downloads(&f);
    std::fs::remove_file(f.path("Downloads/setup.dmg")).unwrap();

    press(&mut app, KeyCode::Char('x'));
    let b = browser(&app);
    assert!(b.confirm.is_none(), "nothing to confirm");
    assert!(b.status.as_ref().unwrap().error);
}

#[test]
fn a_failed_move_is_reported_and_the_entry_kept() {
    let f = Fixture::standard();
    let mut app = downloads(&f);
    press(&mut app, KeyCode::Char('x'));
    // Gone between the question and the answer: moving it fails.
    std::fs::remove_file(f.path("Downloads/setup.dmg")).unwrap();
    press(&mut app, KeyCode::Char('e'));
    wait_for_deletion(&mut app);

    let b = browser(&app);
    assert!(b.status.as_ref().unwrap().error);
    assert_eq!(b.failures.as_ref().map(|d| d.items.len()), Some(1));
    assert_eq!(b.trashed.apparent, 0);
    // The row stays until a rescan shows it is gone.
    assert_eq!(rows(&app), ["Downloads/setup.dmg", "Downloads/photos.zip"]);
}
