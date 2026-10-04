//! Every report on a real folder tree, called directly and through the menu.

use crossterm::event::KeyCode;

use super::{browser, find, menu, now, open, press, rows, scan, tick_until, Fixture};
use crate::duplicates::{self, Progress};
use crate::lists::ResultList;
use crate::reports::{self, ReportKind};
use crate::tree::{SizeMode, Tree, ROOT};

fn run(t: &Tree, kind: ReportKind, mode: SizeMode, min_age_days: u32) -> ResultList {
    reports::run(t, ROOT, mode, now(), kind, min_age_days)
}

fn labels(list: &ResultList) -> Vec<String> {
    list.rows
        .iter()
        .map(|r| r.label.replace('\\', "/"))
        .collect()
}

#[test]
fn largest_files_and_folders() {
    let f = Fixture::standard();
    let t = scan(f.root()).tree;
    let files = labels(&run(&t, ReportKind::LargestFiles, SizeMode::Apparent, 0));
    assert_eq!(files[0], "Archive/old.iso");
    assert_eq!(files.len(), usize::try_from(f.files).unwrap());
    // Thirty days or older: the backup copy, the old projects and downloads.
    let old = labels(&run(&t, ReportKind::LargestFiles, SizeMode::Apparent, 30));
    assert!(old.contains(&"Backup/clip-copy.mov".into()), "{old:?}");
    assert!(!old.contains(&"Media/clip.mov".into()), "{old:?}");

    let dirs = labels(&run(&t, ReportKind::LargestDirs, SizeMode::Apparent, 0));
    assert_eq!(dirs[0], "Archive/");
    assert!(dirs.contains(&"Media/".into()), "{dirs:?}");
}

#[test]
fn old_and_big_files() {
    let f = Fixture::standard();
    let t = scan(f.root()).tree;
    let list = run(&t, ReportKind::OldBig, SizeMode::Apparent, 0);
    assert_eq!(labels(&list), ["Archive/old.iso"]);
}

#[test]
fn downloads_lists_images_installers_and_archives() {
    let f = Fixture::standard();
    let t = scan(f.root()).tree;
    let all = labels(&run(&t, ReportKind::Downloads, SizeMode::Apparent, 0));
    assert_eq!(all, ["Downloads/setup.dmg", "Downloads/photos.zip"]);
}

#[test]
fn dev_junk_needs_project_markers_and_uses_project_age() {
    let f = Fixture::standard();
    let t = scan(f.root()).tree;
    let all = labels(&run(&t, ReportKind::DevJunk, SizeMode::Apparent, 0));
    assert_eq!(
        all,
        ["Projects/rusty/target/", "Projects/web/node_modules/"],
        "a target folder without Cargo.toml is not junk"
    );
    // The Rust project was last touched 200 days ago, the web one 5.
    let stale = labels(&run(&t, ReportKind::DevJunk, SizeMode::Apparent, 90));
    assert_eq!(stale, ["Projects/rusty/target/"]);
}

#[test]
fn caches_list_per_app_folders() {
    let f = Fixture::standard();
    let t = scan(f.root()).tree;
    let list = labels(&run(&t, ReportKind::Caches, SizeMode::Apparent, 0));
    assert_eq!(list, ["Library/Caches/com.example.app/"]);
}

#[test]
fn repeated_names_ignore_case() {
    let f = Fixture::standard();
    let t = scan(f.root()).tree;
    let list = run(&t, ReportKind::RepeatedNames, SizeMode::Apparent, 0);
    let readme = list
        .rows
        .iter()
        .find(|r| r.label.eq_ignore_ascii_case("readme.md"))
        .expect("a README group");
    assert_eq!(readme.nodes.len(), 3);
}

#[test]
fn duplicates_need_the_same_content() {
    let f = Fixture::standard();
    let t = scan(f.root()).tree;
    let cands = duplicates::candidates(&t, ROOT);
    // Three files share the size 1.5 MiB; the 100 MiB one is alone.
    assert_eq!(cands.len(), 3);
    let groups = duplicates::find_groups(&cands, &Default::default(), &Progress::default());
    assert_eq!(groups.len(), 1);
    let mut names: Vec<String> = groups[0].iter().map(|&id| t.name(id).into()).collect();
    names.sort();
    assert_eq!(names, ["clip-copy.mov", "clip.mov"]);
}

#[test]
fn reports_open_from_the_menu() {
    let f = Fixture::standard();
    let mut app = open(f.root());
    // Menu item 8: Downloads.
    menu(&mut app, 8);
    tick_until(&mut app, "the report", |a| {
        browser(a)
            .results
            .as_ref()
            .is_some_and(|r| r.report() == Some(ReportKind::Downloads))
    });
    assert_eq!(rows(&app), ["Downloads/setup.dmg", "Downloads/photos.zip"]);

    // `f` re-runs it with the next age step (30 days).
    press(&mut app, KeyCode::Char('f'));
    tick_until(&mut app, "the filtered report", |a| {
        browser(a)
            .results
            .as_ref()
            .is_some_and(|r| r.min_age_days() == 30)
    });
    assert_eq!(rows(&app), ["Downloads/setup.dmg"]);
}

#[test]
fn duplicates_open_from_the_menu_and_keep_the_oldest() {
    let f = Fixture::standard();
    let mut app = open(f.root());
    // Menu item 9: duplicates, searched in the background.
    menu(&mut app, 9);
    tick_until(&mut app, "the duplicate search", |a| {
        browser(a).dup_job.is_none() && browser(a).results.is_some()
    });
    // One group, named after one of its copies.
    let groups = rows(&app);
    assert_eq!(groups.len(), 1);
    assert!(
        ["clip.mov", "clip-copy.mov"].contains(&groups[0].as_str()),
        "{groups:?}"
    );

    // Space adds every copy but the oldest. Both were created within the
    // same second here, so only "exactly one stays" is checked.
    press(&mut app, KeyCode::Char(' '));
    let b = browser(&app);
    let copies = [
        find(&b.tree, "Media/clip.mov"),
        find(&b.tree, "Backup/clip-copy.mov"),
    ];
    let added = b.basket.items().to_vec();
    assert_eq!(added.len(), 1);
    assert!(copies.contains(&added[0]));

    // Enter shows both copies and names the one that is kept. Space moved
    // the cursor past the only row; drawing would pull it back, and these
    // tests do not draw.
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Enter);
    let mut members = rows(&app);
    members.sort();
    assert_eq!(members, ["Backup/clip-copy.mov", "Media/clip.mov"]);
    let keep = browser(&app).results.as_ref().unwrap().keep.unwrap();
    assert!(copies.contains(&keep) && keep != added[0]);
}
