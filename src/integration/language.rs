//! `L` switches the language of an open result list too: it is built again
//! from its source, with the cursor on the same entry. Tests start in
//! Turkish; in test builds `L` only switches the thread that pressed it.

use crossterm::event::KeyCode;

use super::{browser, find, menu, now, open, press, rows, scan, tick_until, Fixture};
use crate::app::App;
use crate::history;
use crate::lists::ResultList;
use crate::reports::ReportKind;

fn list(app: &App) -> &ResultList {
    browser(app).results.as_ref().expect("a result list")
}

fn open_report(f: &Fixture, index: usize, kind: ReportKind) -> App {
    let mut app = open(f.root());
    menu(&mut app, index);
    tick_until(&mut app, "the report", |a| {
        browser(a)
            .results
            .as_ref()
            .is_some_and(|r| r.report() == Some(kind))
    });
    app
}

/// The entry under the cursor of the open list.
fn cursor(app: &App) -> Vec<String> {
    let r = list(app);
    let row = r.selected_row().expect("a selected row");
    let t = &browser(app).tree;
    row.nodes.iter().map(|&id| t.name(id).to_string()).collect()
}

#[test]
fn a_report_follows_the_language_and_keeps_the_cursor() {
    let f = Fixture::standard();
    let mut app = open_report(&f, 8, ReportKind::Downloads);
    assert!(
        list(&app).title.starts_with("İndirilen kurulum"),
        "{}",
        list(&app).title
    );
    press(&mut app, KeyCode::Down);
    assert_eq!(cursor(&app), ["photos.zip"]);
    let turkish_detail = list(&app).rows[1].detail.clone();

    press(&mut app, KeyCode::Char('L'));
    let r = list(&app);
    assert!(
        r.title.starts_with("Installers and archives"),
        "{}",
        r.title
    );
    assert!(r.note.starts_with("Installers"), "{}", r.note);
    assert_ne!(r.rows[1].detail, turkish_detail);
    assert_eq!(cursor(&app), ["photos.zip"]);

    // And back.
    press(&mut app, KeyCode::Char('L'));
    assert!(list(&app).title.starts_with("İndirilen kurulum"));
    assert_eq!(list(&app).rows[1].detail, turkish_detail);
    assert_eq!(cursor(&app), ["photos.zip"]);
}

#[test]
fn the_age_filter_is_kept() {
    let f = Fixture::standard();
    let mut app = open_report(&f, 5, ReportKind::DevJunk);
    press(&mut app, KeyCode::Char('f'));
    assert_eq!(list(&app).min_age_days(), 30);

    press(&mut app, KeyCode::Char('L'));
    let r = list(&app);
    assert_eq!(r.min_age_days(), 30);
    assert!(r.title.ends_with("untouched for ≥ 30 days"), "{}", r.title);
    assert_eq!(rows(&app), ["Projects/rusty/target/"]);
}

#[test]
fn an_open_group_is_opened_again() {
    let f = Fixture::standard();
    let mut app = open_report(&f, 2, ReportKind::RepeatedNames);
    let at = rows(&app)
        .iter()
        .position(|l| l.eq_ignore_ascii_case("readme.md"))
        .unwrap();
    for _ in 0..at {
        press(&mut app, KeyCode::Down);
    }
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Down);
    let before = cursor(&app);
    let member_rows = rows(&app);

    press(&mut app, KeyCode::Char('L'));
    let r = list(&app);
    assert!(r.parent.is_some(), "still inside the group");
    assert!(
        r.title.starts_with("Most repeated file names › "),
        "{}",
        r.title
    );
    assert_eq!(rows(&app), member_rows);
    assert_eq!(cursor(&app), before);

    // Back to the group list, in English, on the same group.
    press(&mut app, KeyCode::Esc);
    assert!(list(&app).title.starts_with("Most repeated"));
    assert!(rows(&app)[list(&app).table.selected().unwrap()].eq_ignore_ascii_case("readme.md"));
}

#[test]
fn search_results_are_searched_again() {
    let f = Fixture::standard();
    let mut app = open(f.root());
    press(&mut app, KeyCode::Char('/'));
    for c in "*.mov".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    press(&mut app, KeyCode::Enter);
    assert!(
        list(&app).title.starts_with("Arama"),
        "{}",
        list(&app).title
    );
    let found = rows(&app);
    assert_eq!(found.len(), 3);

    press(&mut app, KeyCode::Char('L'));
    assert_eq!(list(&app).title, "Search “*.mov”");
    assert_eq!(rows(&app), found);
}

#[test]
fn duplicates_are_rebuilt_without_a_new_search() {
    let f = Fixture::standard();
    let mut app = open(f.root());
    menu(&mut app, 9);
    tick_until(&mut app, "the duplicate search", |a| {
        browser(a).dup_job.is_none() && browser(a).results.is_some()
    });
    let groups = rows(&app);

    press(&mut app, KeyCode::Char('L'));
    assert!(browser(&app).dup_job.is_none(), "no new search");
    let r = list(&app);
    assert!(r.title.starts_with("Duplicate files"), "{}", r.title);
    assert!(r.rows[0].detail.contains("copies"), "{}", r.rows[0].detail);
    assert_eq!(rows(&app), groups);
}

#[test]
fn changes_are_compared_again() {
    let mut f = Fixture::standard();
    let before = scan(f.root()).tree;
    let dir = history::dir_for(before.root_path()).unwrap();
    history::save(&dir, &history::capture(&before, now() - 3600)).unwrap();
    f.file("New/blob", 3 * super::MIB, b'W', 0);

    let mut app = open(f.root());
    tick_until(&mut app, "the scan to be saved", |_| {
        history::list(&dir).len() == 2
    });
    menu(&mut app, 10);
    press(&mut app, KeyCode::Enter);
    assert!(
        list(&app).title.starts_with("Değişenler"),
        "{}",
        list(&app).title
    );

    press(&mut app, KeyCode::Char('L'));
    let r = list(&app);
    assert!(
        r.title.starts_with("Changes (since the scan of"),
        "{}",
        r.title
    );
    assert!(r.rows[0].detail.starts_with("new"), "{}", r.rows[0].detail);
}

#[test]
fn the_basket_view_follows_too() {
    let f = Fixture::standard();
    let mut app = open(f.root());
    let b = app.browser.as_mut().unwrap();
    let id = find(&b.tree, "Downloads/setup.dmg");
    b.basket.add(&b.tree, id);
    press(&mut app, KeyCode::Char('S'));
    assert_eq!(list(&app).title, "Sepet");

    press(&mut app, KeyCode::Char('L'));
    assert!(list(&app).is_basket());
    assert_eq!(list(&app).title, "Basket");
    assert_eq!(rows(&app), ["Downloads/setup.dmg"]);
}
