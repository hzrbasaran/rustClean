//! The iPhone / iPad backups report on a real folder: backups found by the
//! real scanner, their details read from `Info.plist`, and one moved to the
//! (test) trash through the interface.

use crossterm::event::KeyCode;

use super::{browser, index_of, menu, now, open, press, rows, tick_until, Fixture, DAY, KIB};
use crate::app::App;
use crate::delete::test_trash;
use crate::reports::{MenuItem, ReportKind};

const BACKUP: &str = "Library/Application Support/MobileSync/Backup";
const SAMPLE: &str = include_str!("../../tests/fixtures/backup_info.plist");

/// The sample `Info.plist` with its backup made `days` ago.
fn info(days: u64) -> String {
    let when = chrono::DateTime::from_timestamp(i64::try_from(now() - days * DAY).unwrap(), 0)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    SAMPLE.replace("2026-03-14T09:30:00Z", &when)
}

/// Two backups of the sample iPhone: `PHONE` (larger, 100 days ago) and an
/// older one (400 days ago), kept under a dated folder name.
fn fixture() -> Fixture {
    let mut f = Fixture::new();
    f.file(&format!("{BACKUP}/PHONE/Manifest.db"), 300 * KIB, b'm', 0);
    f.file(&format!("{BACKUP}/PHONE/ab/abcdef"), 200 * KIB, b'a', 0);
    f.file(
        &format!("{BACKUP}/PHONE-20250101-000000/Manifest.db"),
        100 * KIB,
        b'o',
        0,
    );
    std::fs::write(f.path(&format!("{BACKUP}/PHONE/Info.plist")), info(100)).unwrap();
    std::fs::write(
        f.path(&format!("{BACKUP}/PHONE-20250101-000000/Info.plist")),
        info(400),
    )
    .unwrap();
    f.file("Documents/notes.txt", KIB, b'n', 0);
    f
}

fn report(f: &Fixture) -> App {
    let mut app = open(f.root());
    menu(
        &mut app,
        index_of(MenuItem::Report(ReportKind::DeviceBackups)),
    );
    tick_until(&mut app, "the report", |a| {
        browser(a)
            .results
            .as_ref()
            .is_some_and(|r| r.report() == Some(ReportKind::DeviceBackups))
    });
    app
}

#[test]
fn backups_are_listed_with_their_device_and_can_be_trashed() {
    let f = fixture();
    let mut app = report(&f);
    assert_eq!(
        rows(&app),
        ["Demo iPhone · iPhone 13 Pro", "Demo iPhone · iPhone 13 Pro"]
    );
    let r = browser(&app).results.as_ref().unwrap();
    assert!(
        r.rows[0].detail.ends_with(" · en yenisi · iOS 17.5.1"),
        "{}",
        r.rows[0].detail
    );
    assert!(
        !r.rows[1].detail.contains("en yenisi"),
        "{}",
        r.rows[1].detail
    );
    assert!(r.note.contains("geri yüklenemez"), "{}", r.note);

    // The older backup goes to the trash, after the confirmation.
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char('x'));
    assert_eq!(browser(&app).confirm.as_ref().map(Vec::len), Some(1));
    press(&mut app, KeyCode::Char('e'));
    tick_until(&mut app, "the deletion", |a| browser(a).deleting.is_none());
    assert!(browser(&app).failures.is_none());
    assert!(!f.path(&format!("{BACKUP}/PHONE-20250101-000000")).exists());
    assert!(f.path(&format!("{BACKUP}/PHONE")).exists());
    let trashed = std::fs::read_dir(test_trash::dir())
        .unwrap()
        .flatten()
        .any(|e| {
            e.file_name()
                .to_string_lossy()
                .ends_with("-PHONE-20250101-000000")
        });
    assert!(trashed);
    assert_eq!(rows(&app), ["Demo iPhone · iPhone 13 Pro"]);
}

#[test]
fn the_age_filter_goes_by_the_last_backup() {
    let f = fixture();
    let mut app = report(&f);
    // Both folders were written just now; the backups are months old.
    for days in [30, 90, 180] {
        press(&mut app, KeyCode::Char('f'));
        tick_until(&mut app, "the filtered report", |a| {
            browser(a)
                .results
                .as_ref()
                .is_some_and(|r| r.min_age_days() == days)
        });
    }
    let r = browser(&app).results.as_ref().unwrap();
    assert!(r.title.contains("son yedek ≥ 180 gün önce"), "{}", r.title);
    let r = browser(&app).results.as_ref().unwrap();
    let t = &browser(&app).tree;
    let left: Vec<&str> = r.rows.iter().map(|row| t.name(row.nodes[0])).collect();
    assert_eq!(left, ["PHONE-20250101-000000"]);
}
