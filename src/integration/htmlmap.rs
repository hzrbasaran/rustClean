//! `w`: the treemap page of the current folder.

use std::fs;

use crossterm::event::KeyCode;

use super::{browser, open, press, Fixture};
use crate::app::View;

/// The pages written into `dir`, sorted by name.
fn pages(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn w_writes_the_page_of_the_current_folder() {
    let f = Fixture::standard();
    let out = tempfile::tempdir().unwrap();
    let mut app = open(f.root());
    app.browser.as_mut().unwrap().html_dir = Some(out.path().to_path_buf());

    // From the treemap, inside Projects.
    press(&mut app, KeyCode::Char('t'));
    let b = browser(&app);
    let at = b
        .entries
        .iter()
        .position(|&id| b.tree.name(id) == "Projects")
        .unwrap();
    app.browser.as_mut().unwrap().table.select(Some(at));
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('w'));
    assert_eq!(browser(&app).view, View::Map, "the map stays open");

    let names = pages(out.path());
    assert_eq!(names.len(), 1, "{names:?}");
    let name = &names[0];
    assert!(name.starts_with("rustclean-treemap-") && name.ends_with(".html"));
    // rustclean-treemap-YYYYMMDD-HHMMSS.html
    assert_eq!(name.len(), "rustclean-treemap-20261005-120000.html".len());

    let status = browser(&app).status.as_ref().expect("a status line");
    assert!(!status.error);
    assert!(status.text.starts_with("Harita sayfası kaydedildi: "));
    assert!(status.text.replace('\\', "/").ends_with(name.as_str()));

    let page = fs::read_to_string(out.path().join(name)).unwrap();
    assert!(page.contains("\"n\":\"Projects\""));
    assert!(page.contains("\"n\":\"rusty\""));
    assert!(page.contains("\"n\":\"index.js\""));
    // Only the current folder.
    assert!(!page.contains("setup.dmg"));
    assert!(!page.contains("https://") && !page.contains("http://"));

    // Again (also from the list): a second file, never the same one.
    press(&mut app, KeyCode::Char('t'));
    press(&mut app, KeyCode::Char('w'));
    assert_eq!(pages(out.path()).len(), 2);
}

#[test]
fn an_unwritable_folder_is_reported() {
    let f = Fixture::standard();
    let mut app = open(f.root());
    let missing = f.path("no/such/folder");
    app.browser.as_mut().unwrap().html_dir = Some(missing);
    press(&mut app, KeyCode::Char('w'));
    let status = browser(&app).status.as_ref().expect("a status line");
    assert!(status.error);
    assert!(
        status.text.starts_with("Harita sayfası yazılamadı"),
        "{}",
        status.text
    );
}
