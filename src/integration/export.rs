//! `o`: saving the list on screen as CSV or JSON.

use std::fs;
use std::path::{Path, PathBuf};

use crossterm::event::KeyCode;

use super::*;
use crate::reports::{MenuItem, ReportKind};

/// The app over the standard fixture, saving into `out`.
fn app_saving_to(f: &Fixture, out: &Path) -> App {
    let mut app = open(f.root());
    app.browser.as_mut().unwrap().export_dir = Some(out.to_path_buf());
    app
}

fn saved(out: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(out)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    files
}

fn menu_index(item: MenuItem) -> usize {
    MenuItem::ALL.iter().position(|&i| i == item).unwrap()
}

fn status(app: &App) -> String {
    browser(app)
        .status
        .as_ref()
        .map(|s| s.text.clone())
        .unwrap_or_default()
}

#[test]
fn the_folder_list_as_csv() {
    let f = Fixture::standard();
    let out = tempfile::tempdir().unwrap();
    let mut app = app_saving_to(&f, out.path());
    press(&mut app, KeyCode::Char('o'));
    assert!(browser(&app).export_prompt.is_some());
    press(&mut app, KeyCode::Char('c'));
    assert!(browser(&app).export_prompt.is_none());

    let files = saved(out.path());
    assert_eq!(files.len(), 1);
    let name = files[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with("rustclean-folder-") && name.ends_with(".csv"),
        "{name}"
    );
    assert!(status(&app).contains(&name), "{}", status(&app));
    let text = fs::read_to_string(&files[0]).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], crate::export::COLUMNS.join(","));
    // One line per entry of the root, largest first, with full paths.
    assert_eq!(lines.len(), 1 + browser(&app).entries.len());
    let b = browser(&app);
    let first = b.tree.path_of(b.entries[0]).to_string_lossy().into_owned();
    assert!(Path::new(&first).is_absolute());
    assert!(lines[1].starts_with(&format!("{first},")), "{}", lines[1]);
}

#[test]
fn a_report_with_groups_as_json() {
    let f = Fixture::standard();
    let out = tempfile::tempdir().unwrap();
    let mut app = app_saving_to(&f, out.path());
    menu(
        &mut app,
        menu_index(MenuItem::Report(ReportKind::RepeatedNames)),
    );
    tick_until(&mut app, "the report", |a| browser(a).results.is_some());
    press(&mut app, KeyCode::Char('o'));
    press(&mut app, KeyCode::Char('j'));

    let files = saved(out.path());
    let name = files[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("rustclean-repeated-names-"), "{name}");
    let v: serde_json::Value = serde_json::from_slice(&fs::read(&files[0]).unwrap()).unwrap();
    let entries = v["entries"].as_array().unwrap();
    // README.md ×2 (Readme.md counts too, by case) — every member is listed
    // with the group's label.
    assert!(entries.len() >= 2, "{v}");
    assert!(entries.iter().all(|e| e["group"].is_string()), "{v}");
    let paths: Vec<String> = entries
        .iter()
        .map(|e| e["path"].as_str().unwrap().replace('\\', "/"))
        .collect();
    assert!(
        paths.iter().any(|p| p.ends_with("Projects/web/README.md")),
        "{paths:?}"
    );
}

#[test]
fn the_summary_lists() {
    let f = Fixture::standard();
    let out = tempfile::tempdir().unwrap();
    let mut app = app_saving_to(&f, out.path());
    press(&mut app, KeyCode::Char('i'));
    press(&mut app, KeyCode::Char('o'));
    press(&mut app, KeyCode::Char('c'));
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('o'));
    press(&mut app, KeyCode::Char('c'));
    let names: Vec<String> = saved(out.path())
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(
        names[0].starts_with("rustclean-summary-files-"),
        "{names:?}"
    );
    assert!(
        names[1].starts_with("rustclean-summary-folders-"),
        "{names:?}"
    );
    // The summary is still open.
    assert!(browser(&app).dashboard.is_some());
}

#[test]
fn esc_cancels_and_nothing_is_overwritten() {
    let f = Fixture::standard();
    let out = tempfile::tempdir().unwrap();
    let mut app = app_saving_to(&f, out.path());
    press(&mut app, KeyCode::Char('t')); // the treemap saves the folder too
    press(&mut app, KeyCode::Char('o'));
    press(&mut app, KeyCode::Esc);
    assert!(saved(out.path()).is_empty());
    // Esc only closed the question: still on the map.
    assert_eq!(browser(&app).view, crate::app::View::Map);

    // Saving again never replaces the first file.
    for _ in 0..2 {
        press(&mut app, KeyCode::Char('o'));
        press(&mut app, KeyCode::Char('c'));
    }
    let files = saved(out.path());
    assert_eq!(files.len(), 2);
    let a = fs::read_to_string(&files[0]).unwrap();
    let b = fs::read_to_string(&files[1]).unwrap();
    assert_eq!(a, b);
}

#[test]
fn an_empty_list_is_not_saved() {
    let f = Fixture::standard();
    let out = tempfile::tempdir().unwrap();
    let mut app = app_saving_to(&f, out.path());
    press(&mut app, KeyCode::Char('/'));
    for c in "nothing-like-this".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    press(&mut app, KeyCode::Enter);
    assert!(rows(&app).is_empty());
    press(&mut app, KeyCode::Char('o'));
    assert!(browser(&app).export_prompt.is_none());
    assert!(browser(&app).status.as_ref().unwrap().error);
    assert!(saved(out.path()).is_empty());
}
