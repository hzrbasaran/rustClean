//! The configuration file's settings, applied to a real scan.

use super::{browser, find, menu, open, rows, tick_until, Fixture};
use crate::app::{App, Screen, SortMode};
use crate::config::{self, Config};
use crate::reports::{MenuItem, ReportKind};
use crate::tree::SizeMode;

/// The menu row of a report, wherever the menu puts it.
fn menu_index(kind: ReportKind) -> usize {
    MenuItem::ALL
        .iter()
        .position(|&m| m == MenuItem::Report(kind))
        .unwrap()
}

#[test]
fn excluded_folders_view_and_thresholds() {
    let f = Fixture::standard();
    let media = std::path::absolute(f.path("Media")).unwrap();
    let c = Config {
        exclude: vec![media],
        old_big_min_mib: 1,
        old_big_min_days: 20,
        size: SizeMode::Apparent,
        sort: SortMode::Name,
        ..Config::default()
    };
    config::with(c, || {
        let mut app = open(f.root());
        let b = browser(&app);
        assert_eq!((b.sort, b.size_mode), (SortMode::Name, SizeMode::Apparent));
        // Listed, but not descended into.
        let media = b.tree.node(find(&b.tree, "Media"));
        assert_eq!((media.size.apparent, media.file_count), (0, 0));
        assert_eq!(b.tree.name(b.entries[0]), "Archive", "sorted by name");

        // Old and large: ≥ 1 MiB, unchanged for more than 20 days.
        menu(&mut app, menu_index(ReportKind::OldBig));
        tick_until(&mut app, "the report", |a| browser(a).results.is_some());
        assert_eq!(rows(&app), ["Archive/old.iso", "Backup/clip-copy.mov"]);
        assert_eq!(
            browser(&app).results.as_ref().unwrap().note,
            "≥ 1 MiB ve 20 günden uzun süredir değişmemiş dosyalar."
        );
    });
}

#[test]
fn duplicates_above_the_configured_size() {
    let f = Fixture::standard();
    // The 1.5 MiB copies are below a 2 MiB minimum.
    let c = Config {
        duplicates_min_mib: 2,
        ..Config::default()
    };
    config::with(c, || {
        let mut app = open(f.root());
        menu(&mut app, menu_index(ReportKind::Duplicates));
        tick_until(&mut app, "the report", |a| browser(a).results.is_some());
        let list = browser(&app).results.as_ref().unwrap();
        assert!(list.rows.is_empty());
        assert_eq!(list.note, "2 MiB üzerinde aynı boyutta iki dosya yok.");
    });
}

#[test]
fn a_problem_shows_on_the_status_line() {
    let f = Fixture::new();
    let mut app = App::new(Some(f.root().to_path_buf()));
    app.config_notice("config.toml: sorun".into());
    tick_until(&mut app, "the scan", |a| {
        matches!(a.screen, Screen::Browser)
    });
    let status = browser(&app).status.as_ref().expect("a status line");
    assert_eq!(status.text, "config.toml: sorun");
    assert!(status.error);

    // Without a start path, the disk list shows it.
    let mut app = App::new(None);
    app.config_notice("config.toml: sorun".into());
    assert_eq!(app.message.as_deref(), Some("config.toml: sorun"));
}
