//! Mouse support: the app is drawn on a test terminal, then clicked where
//! the text is, as a terminal would report it.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::{browser, find, open, press, tick_until, Fixture};
use crate::app::{App, Pane, View};
use crate::reports::{MenuItem, ReportKind};

const WIDTH: u16 = 120;
const HEIGHT: u16 = 40;

/// Draws the app like the event loop does; the screen lines.
fn draw(app: &mut App) -> Vec<Vec<String>> {
    let mut term = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    term.draw(|f| crate::ui::render(f, app)).unwrap();
    let buf = term.backend().buffer();
    (0..HEIGHT)
        .map(|y| {
            (0..WIDTH)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect()
        })
        .collect()
}

/// Draws the app and finds `text`: the cell its first character is in,
/// searching from the bottom (popups are drawn in the middle, lists above
/// the status line, so the last match is the one in front).
fn locate(app: &mut App, text: &str) -> (u16, u16) {
    let screen = draw(app);
    for (y, cells) in screen.iter().enumerate().rev() {
        for x in 0..cells.len() {
            let rest: String = cells[x..].concat();
            if rest.starts_with(text) {
                return (x as u16, y as u16);
            }
        }
    }
    let shown: Vec<String> = screen.iter().map(|l| l.concat()).collect();
    panic!("`{text}` is not on screen:\n{}", shown.join("\n"));
}

fn event(kind: MouseEventKind, (column, row): (u16, u16)) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

/// A left click at `at`, `ms` milliseconds after `start`.
fn click(app: &mut App, at: (u16, u16), start: Instant, ms: u64) {
    let down = MouseEventKind::Down(MouseButton::Left);
    app.on_mouse_at(event(down, at), start + Duration::from_millis(ms));
    app.on_tick();
}

fn double_click(app: &mut App, at: (u16, u16)) {
    let start = Instant::now();
    click(app, at, start, 0);
    draw(app);
    click(app, at, start, 150);
}

fn wheel(app: &mut App, down: bool) {
    let kind = if down {
        MouseEventKind::ScrollDown
    } else {
        MouseEventKind::ScrollUp
    };
    app.on_mouse(event(kind, (10, 10)));
}

fn mouse_on(app: &mut App) {
    press(app, KeyCode::Char('M'));
    assert!(app.mouse.on);
}

/// The name of the selected entry of the folder list.
fn selected_name(app: &App) -> String {
    let b = browser(app);
    let id = b.entries[b.table.selected().unwrap()];
    b.tree.name(id).to_string()
}

#[test]
fn m_turns_the_mouse_on_and_off() {
    let f = Fixture::standard();
    let mut app = open(f.root());
    assert!(!app.mouse.on, "off by default");
    // Tests run in Turkish.
    press(&mut app, KeyCode::Char('M'));
    assert!(app.mouse.on);
    let status = &browser(&app).status.as_ref().unwrap().text;
    assert!(status.starts_with("Fare açık"), "{status}");
    press(&mut app, KeyCode::Char('M'));
    assert!(!app.mouse.on);
    let status = &browser(&app).status.as_ref().unwrap().text;
    assert_eq!(status, "Fare kapalı (M: aç).");
    // With the mouse off, clicks are ignored.
    let at = locate(&mut app, "Downloads/");
    let before = browser(&app).table.selected();
    click(&mut app, at, Instant::now(), 0);
    assert_eq!(browser(&app).table.selected(), before);
}

#[test]
fn click_selects_and_double_click_opens_a_folder() {
    let f = Fixture::standard();
    let mut app = open(f.root());
    mouse_on(&mut app);
    assert_ne!(selected_name(&app), "Downloads");

    let at = locate(&mut app, "Downloads/");
    click(&mut app, at, Instant::now(), 0);
    assert_eq!(selected_name(&app), "Downloads");
    assert_eq!(
        browser(&app).current,
        crate::tree::ROOT,
        "one click only selects"
    );

    double_click(&mut app, at);
    let b = browser(&app);
    assert_eq!(b.current, find(&b.tree, "Downloads"));

    // Two slow clicks are two single clicks.
    press(&mut app, KeyCode::Backspace);
    let at = locate(&mut app, "Downloads/");
    let start = Instant::now();
    click(&mut app, at, start, 0);
    draw(&mut app);
    click(&mut app, at, start, 2000);
    assert_eq!(browser(&app).current, crate::tree::ROOT);
}

#[test]
fn clicks_outside_the_list_do_nothing() {
    let f = Fixture::standard();
    let mut app = open(f.root());
    mouse_on(&mut app);
    draw(&mut app);
    let before = browser(&app).table.selected();
    // The title, the column headings and the bottom line.
    for row in [0, 1, 2, HEIGHT - 1] {
        click(&mut app, (40, row), Instant::now(), 0);
    }
    assert_eq!(browser(&app).table.selected(), before);
    assert_eq!(browser(&app).current, crate::tree::ROOT);
}

#[test]
fn wheel_moves_the_selection() {
    let f = Fixture::standard();
    let mut app = open(f.root());
    mouse_on(&mut app);
    draw(&mut app);
    assert_eq!(browser(&app).table.selected(), Some(0));
    wheel(&mut app, true);
    wheel(&mut app, true);
    assert_eq!(browser(&app).table.selected(), Some(2));
    wheel(&mut app, false);
    assert_eq!(browser(&app).table.selected(), Some(1));
}

#[test]
fn treemap_blocks_are_clicked_and_opened() {
    let f = Fixture::standard();
    let mut app = open(f.root());
    mouse_on(&mut app);
    press(&mut app, KeyCode::Char('t'));
    assert_eq!(browser(&app).view, View::Map);

    let at = locate(&mut app, "Media/");
    click(&mut app, at, Instant::now(), 0);
    assert_eq!(selected_name(&app), "Media");

    double_click(&mut app, at);
    let b = browser(&app);
    assert_eq!(b.current, find(&b.tree, "Media"));
}

#[test]
fn menu_items_are_clicked_and_run() {
    let f = Fixture::standard();
    let mut app = open(f.root());
    mouse_on(&mut app);
    press(&mut app, KeyCode::Char('m'));
    let item = MenuItem::ALL
        .iter()
        .position(|m| matches!(m, MenuItem::Report(ReportKind::Downloads)))
        .unwrap();

    let at = locate(&mut app, MenuItem::ALL[item].label());
    // Behind the menu, the folder list is covered.
    let before = browser(&app).table.selected();
    click(&mut app, (1, 3), Instant::now(), 0);
    assert_eq!(browser(&app).table.selected(), before);

    click(&mut app, at, Instant::now(), 0);
    assert_eq!(browser(&app).report_menu, Some(item));

    double_click(&mut app, at);
    tick_until(&mut app, "the report", |a| browser(a).results.is_some());
    assert_eq!(browser(&app).report_menu, None);

    // A row of the result list.
    let at = locate(&mut app, "photos.zip");
    click(&mut app, at, Instant::now(), 0);
    let b = browser(&app);
    let r = b.results.as_ref().unwrap();
    assert!(r.rows[r.table.selected().unwrap()]
        .label
        .ends_with("photos.zip"));
}

#[test]
fn summary_lists_take_clicks() {
    let f = Fixture::standard();
    let mut app = open(f.root());
    mouse_on(&mut app);
    press(&mut app, KeyCode::Char('i'));

    // "Media/" and its padding: the fullest folders list, not a file path.
    let at = locate(&mut app, "Media/   ");
    click(&mut app, at, Instant::now(), 0);
    let d = browser(&app).dashboard.as_ref().unwrap();
    assert_eq!(d.focus, Pane::Dirs);
    let (id, _) = d.stats.top_dirs[d.dirs.selected().unwrap()];
    assert_eq!(browser(&app).tree.name(id), "Media");

    double_click(&mut app, at);
    let b = browser(&app);
    assert!(b.dashboard.is_none());
    assert_eq!(b.current, find(&b.tree, "Media"));
}

#[test]
fn a_confirmation_takes_no_mouse_input() {
    let f = Fixture::standard();
    let mut app = open(f.root());
    mouse_on(&mut app);
    let at = locate(&mut app, "Downloads/");
    click(&mut app, at, Instant::now(), 0);
    press(&mut app, KeyCode::Char('x'));
    assert!(browser(&app).confirm.is_some());

    // Click and double-click all over the screen, and turn the wheel.
    draw(&mut app);
    for y in (0..HEIGHT).step_by(2) {
        for x in (0..WIDTH).step_by(4) {
            double_click(&mut app, (x, y));
        }
    }
    wheel(&mut app, true);
    wheel(&mut app, false);
    let b = browser(&app);
    assert!(b.confirm.is_some(), "still waiting for the keyboard");
    assert!(b.deleting.is_none());
    assert!(f.path("Downloads/setup.dmg").exists());

    press(&mut app, KeyCode::Esc);
    assert!(browser(&app).confirm.is_none());
    assert!(f.path("Downloads/setup.dmg").exists());
}

#[test]
fn wheel_scrolls_the_help_and_clicks_leave_it_open() {
    let f = Fixture::standard();
    let mut app = open(f.root());
    mouse_on(&mut app);
    press(&mut app, KeyCode::Char('?'));
    draw(&mut app);
    wheel(&mut app, true);
    assert_eq!(app.help, Some(3));
    click(&mut app, (WIDTH / 2, HEIGHT / 2), Instant::now(), 0);
    assert_eq!(app.help, Some(3), "a click neither closes nor scrolls");
    wheel(&mut app, false);
    assert_eq!(app.help, Some(0));
}

#[test]
fn wheel_scrolls_the_deletion_log() {
    let f = Fixture::standard();
    let mut app = open(f.root());
    mouse_on(&mut app);
    app.browser.as_mut().unwrap().deletion_log = Some(crate::app::LogView {
        entries: Vec::new(),
        scroll: 0,
    });
    wheel(&mut app, true);
    wheel(&mut app, true);
    assert_eq!(browser(&app).deletion_log.as_ref().unwrap().scroll, 6);
}

#[test]
fn disks_are_clicked_and_scanned() {
    use crate::app::Screen;
    use crate::disks::DiskInfo;
    let f = Fixture::standard();
    let disk = |name: &str| DiskInfo {
        name: name.into(),
        mount_point: f.root().to_path_buf(),
        fs_type: "apfs".into(),
        total: 1 << 30,
        available: 1 << 29,
        removable: false,
    };
    let mut app = App::new(None);
    app.disks = vec![disk("First disk"), disk("Second disk")];
    app.disk_table.select(Some(0));
    mouse_on(&mut app);

    let at = locate(&mut app, "Second disk");
    click(&mut app, at, Instant::now(), 0);
    assert_eq!(app.disk_table.selected(), Some(1));
    double_click(&mut app, at);
    assert!(!matches!(app.screen, Screen::DiskSelect));
    tick_until(&mut app, "the scan", |a| {
        matches!(a.screen, Screen::Browser)
    });
}
