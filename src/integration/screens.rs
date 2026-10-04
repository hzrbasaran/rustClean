//! Screen snapshots: every screen drawn on a fixed-size test terminal, in
//! Turkish and English, stored with `insta` under `snapshots/`. A snapshot
//! holds the text as shown and the colors and styles of each line, so both
//! wording and readability regressions show up in review.
//!
//! Everything that could change between runs is fixed: the tree is built by
//! hand (`/Users/demo`, set dates, distinct sizes), the clock is stopped and
//! dates are shown in UTC, and disks, tools and system data are fake. Paths
//! use `/`, so these run on Unix only.
//!
//! After an intended change: `cargo insta review`, or
//! `INSTA_UPDATE=always cargo test` and review the `.snap` diff.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;

use crate::app::{App, Browser, FailureDialog, Screen, UninstallDialog};
use crate::delete::Failure;
use crate::disks::DiskInfo;
use crate::history::{Header, Saved};
use crate::i18n::{with_lang, Lang};
use crate::scanner::{ScanProgress, ScanResult};
use crate::system::{Container, SystemInfo, Volume};
use crate::tools::{CleanAction, Risk, Status, Step, Tool, ToolKind};
use crate::toolsview::ToolsView;
use crate::tree::{NodeId, Size, Tree, ROOT};
use crate::ui::with_fixed_now;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 30;
/// 2026-10-01 12:00 UTC.
const NOW: u64 = 1_790_856_000;
const DAY: u64 = 86_400;
const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// A home folder with something for every report. Disk sizes are a little
/// larger than apparent ones, as allocation rounds up.
fn demo_tree() -> Tree {
    let mut t = Tree::new(Path::new("/Users/demo"));
    let mut dirs: Vec<(String, NodeId)> = Vec::new();
    let mut dir = |t: &mut Tree, rel: &str| -> NodeId {
        let mut parent = ROOT;
        let mut path = String::new();
        for name in rel.split('/') {
            path = if path.is_empty() {
                name.into()
            } else {
                format!("{path}/{name}")
            };
            parent = match dirs.iter().find(|(p, _)| *p == path) {
                Some(&(_, id)) => id,
                None => {
                    let id = t.push(parent, name, true, Size::default());
                    dirs.push((path.clone(), id));
                    id
                }
            };
        }
        parent
    };
    let files: &[(&str, &str, u64, u64)] = &[
        ("Movies", "holiday-2024.mp4", 2 * GIB + 300 * MIB, 580),
        ("Movies", "screen-recording.mov", 900 * MIB, 40),
        ("Downloads", "Xcode_16.xip", 3 * GIB, 400),
        ("Downloads", "setup.dmg", 450 * MIB, 200),
        ("Downloads", "photos.zip", 120 * MIB, 12),
        ("Downloads", "report.pdf", 3 * MIB, 2),
        ("Projects/web", "package.json", 2_100, 3),
        ("Projects/web", "README.md", 2_200, 3),
        ("Projects/web/node_modules/react", "index.js", 310 * MIB, 3),
        ("Projects/api", "Cargo.toml", 900, 120),
        ("Projects/api/target/debug", "api", 820 * MIB, 120),
        (
            "Library/Caches/com.example.browser",
            "cache.db",
            610 * MIB,
            1,
        ),
        (
            "Library/Application Support/com.example.sketchpad",
            "state.db",
            55 * MIB,
            20,
        ),
        (
            "Applications/Sketchpad.app/Contents/MacOS",
            "Sketchpad",
            150 * MIB,
            60,
        ),
        ("Documents", "taxes-2025.pdf", 4 * MIB + 7, 300),
        ("Documents", "notes.md", 12_345, 1),
        ("Documents", "README.md", 1_100, 90),
    ];
    for &(parent, name, size, age_days) in files {
        let p = dir(&mut t, parent);
        let disk = size.div_ceil(4096) * 4096;
        let id = t.push(
            p,
            name,
            false,
            Size {
                apparent: size,
                disk,
            },
        );
        let when = u32::try_from(NOW - age_days * DAY).unwrap();
        t.set_times(id, when, when - 3600);
    }
    for &(_, id) in &dirs {
        let when = u32::try_from(NOW - 10 * DAY).unwrap();
        t.set_times(id, when, when);
    }
    t.finalize();
    t
}

/// The app on the browser screen over the demo tree.
fn app() -> App {
    let mut app = App::new(None);
    app.disks = disks();
    app.browser = Some(Browser::new(ScanResult {
        tree: demo_tree(),
        errors: 2,
        elapsed: Duration::from_millis(1234),
    }));
    app.screen = Screen::Browser;
    app
}

fn disks() -> Vec<DiskInfo> {
    vec![
        DiskInfo {
            name: "Macintosh HD".into(),
            mount_point: "/".into(),
            fs_type: "apfs".into(),
            total: 994 * GIB,
            available: 270 * GIB,
            removable: false,
        },
        DiskInfo {
            name: "Backup".into(),
            mount_point: "/Volumes/Backup".into(),
            fs_type: "exfat".into(),
            total: 2000 * GIB,
            available: 150 * GIB,
            removable: true,
        },
    ]
}

fn browser(app: &mut App) -> &mut Browser {
    app.browser.as_mut().unwrap()
}

fn find(app: &mut App, rel: &str) -> NodeId {
    let t = &browser(app).tree;
    rel.split('/').fold(ROOT, |dir, name| {
        t.children(dir).find(|&c| t.name(c) == name).unwrap()
    })
}

fn press(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    // Reports wait one frame so "preparing" can be drawn.
    for _ in 0..3 {
        app.on_tick();
    }
}

/// Opens menu item `index` (`m`, ↓ `index` times, Enter).
fn menu(app: &mut App, index: usize) {
    press(app, KeyCode::Char('m'));
    for _ in 0..index {
        press(app, KeyCode::Down);
    }
    press(app, KeyCode::Enter);
}

/// The screen as text, then the styled runs of each line.
fn render(app: &mut App) -> String {
    let mut term = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    term.draw(|f| crate::ui::render(f, app)).unwrap();
    describe(term.backend().buffer())
}

fn describe(buf: &Buffer) -> String {
    let mut text = String::new();
    let mut styles = String::new();
    for y in 0..buf.area.height {
        let mut line = String::new();
        let mut runs: Vec<(u16, u16, String)> = Vec::new();
        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            line.push_str(cell.symbol());
            let style = style_name(cell.fg, cell.bg, cell.modifier);
            match runs.last_mut() {
                Some((_, len, s)) if *s == style => *len += 1,
                _ => runs.push((x, 1, style)),
            }
        }
        text.push_str(line.trim_end());
        text.push('\n');
        let runs: Vec<String> = runs
            .into_iter()
            .filter(|(_, _, s)| !s.is_empty())
            .map(|(x, len, s)| format!("{x}+{len} {s}"))
            .collect();
        if !runs.is_empty() {
            styles.push_str(&format!("{y:2}: {}\n", runs.join(" | ")));
        }
    }
    format!("{text}--- styles (column+width foreground/background modifiers) ---\n{styles}")
}

fn style_name(fg: Color, bg: Color, m: Modifier) -> String {
    let color = |c: Color| match c {
        Color::Reset => String::new(),
        c => format!("{c:?}"),
    };
    let (fg, bg) = (color(fg), color(bg));
    if fg.is_empty() && bg.is_empty() && m.is_empty() {
        return String::new();
    }
    let mut s = format!(
        "{}/{}",
        if fg.is_empty() { "-" } else { &fg },
        if bg.is_empty() { "-" } else { &bg }
    );
    if !m.is_empty() {
        s.push_str(&format!(" {m:?}"));
    }
    s
}

/// Draws the screen `build` sets up, in both languages, and compares each
/// with its stored snapshot.
fn snap(name: &str, build: impl Fn() -> App) {
    for (lang, code) in [(Lang::Tr, "tr"), (Lang::En, "en")] {
        let screen = with_lang(lang, || {
            with_fixed_now(NOW, || {
                let mut app = build();
                render(&mut app)
            })
        });
        insta::assert_snapshot!(format!("{name}-{code}"), screen);
    }
}

#[test]
fn disk_list() {
    snap("disks", || {
        let mut app = app();
        app.screen = Screen::DiskSelect;
        app
    });
}

#[test]
fn scanning() {
    snap("scanning", || {
        let mut app = app();
        app.screen = Screen::Scanning;
        app.scan_root = PathBuf::from("/Users/demo");
        app.progress = ScanProgress {
            files: 123_456,
            dirs: 7_890,
            bytes: 42 * GIB,
            errors: 3,
            current: PathBuf::from("/Users/demo/Library/Caches/com.example.browser"),
        };
        app
    });
}

#[test]
fn folder_list() {
    snap("browser", app);
}

#[test]
fn folder_list_inside_a_folder() {
    snap("browser-downloads", || {
        let mut app = app();
        let downloads = find(&mut app, "Downloads");
        let b = browser(&mut app);
        let at = b.entries.iter().position(|&e| e == downloads).unwrap();
        b.table.select(Some(at));
        press(&mut app, KeyCode::Enter);
        app
    });
}

#[test]
fn treemap_by_kind_and_by_age() {
    snap("treemap", || {
        let mut app = app();
        press(&mut app, KeyCode::Char('t'));
        app
    });
    snap("treemap-age", || {
        let mut app = app();
        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Char('c'));
        app
    });
}

#[test]
fn report_menu() {
    snap("menu", || {
        let mut app = app();
        press(&mut app, KeyCode::Char('m'));
        app
    });
}

#[test]
fn reports() {
    snap("report-largest-files", || {
        let mut app = app();
        menu(&mut app, 0);
        app
    });
    snap("report-dev-junk", || {
        let mut app = app();
        menu(&mut app, 5);
        app
    });
    snap("report-downloads-30-days", || {
        let mut app = app();
        menu(&mut app, 8);
        press(&mut app, KeyCode::Char('f'));
        app
    });
}

/// Opened in one language, then `L`: the list is built again in the other
/// (`-tr` starts in Turkish and ends in English).
#[test]
fn report_after_switching_language() {
    snap("report-after-language-switch", || {
        let mut app = app();
        menu(&mut app, 8);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char('L'));
        app
    });
}

#[test]
fn group_report_and_its_members() {
    snap("report-repeated-names", || {
        let mut app = app();
        menu(&mut app, 2);
        app
    });
    snap("report-repeated-names-drill", || {
        let mut app = app();
        menu(&mut app, 2);
        press(&mut app, KeyCode::Enter);
        app
    });
}

#[test]
fn dashboard() {
    snap("dashboard", || {
        let mut app = app();
        press(&mut app, KeyCode::Char('i'));
        // The real disk would show; the demo has none.
        browser(&mut app).dashboard.as_mut().unwrap().disk = None;
        app
    });
}

#[test]
fn basket() {
    snap("basket", || {
        let mut app = app();
        for rel in ["Downloads/Xcode_16.xip", "Projects/api/target"] {
            let id = find(&mut app, rel);
            let b = browser(&mut app);
            b.basket.add(&b.tree, id);
        }
        press(&mut app, KeyCode::Char('S'));
        app
    });
}

#[test]
fn search_box() {
    snap("search", || {
        let mut app = app();
        press(&mut app, KeyCode::Char('/'));
        for c in "*.dmg".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        app
    });
}

#[test]
fn delete_confirmation() {
    snap("confirm", || {
        let mut app = app();
        let ids = vec![
            find(&mut app, "Downloads/setup.dmg"),
            find(&mut app, "Projects/web/node_modules"),
        ];
        browser(&mut app).confirm = Some(ids);
        app
    });
}

#[test]
fn uninstall_dialog() {
    snap("uninstall", || {
        let mut app = app();
        let items = vec![
            (find(&mut app, "Applications/Sketchpad.app"), true),
            (
                find(
                    &mut app,
                    "Library/Application Support/com.example.sketchpad",
                ),
                true,
            ),
            (find(&mut app, "Library/Caches/com.example.browser"), false),
        ];
        browser(&mut app).uninstall = Some(UninstallDialog {
            label: "Sketchpad".into(),
            items,
            cursor: 1,
            running: true,
        });
        app
    });
}

#[test]
fn failures_dialog() {
    snap("failures", || {
        let mut app = app();
        browser(&mut app).failures = Some(FailureDialog {
            items: vec![
                Failure::new(
                    "/Users/demo/Library/Containers/com.example.locked".into(),
                    "Operation not permitted (os error 1)",
                ),
                Failure::new(
                    "/Users/demo/Downloads/gone.zip".into(),
                    "No such file or directory (os error 2)",
                ),
            ],
            moved: 3,
            size: "1.2 GiB".into(),
            scroll: 0,
        });
        app
    });
}

#[test]
fn saved_scan_picker() {
    snap("scan-picker", || {
        let mut app = app();
        let header = |days: u64, gib: u64| Header {
            root: "/Users/demo".into(),
            time: NOW - days * DAY,
            total: Size {
                apparent: gib * GIB,
                disk: gib * GIB,
            },
            files: 1000,
        };
        let saved = vec![
            Saved {
                file: "a.rcs".into(),
                header: header(1, 9),
            },
            Saved {
                file: "b.rcs".into(),
                header: header(30, 12),
            },
        ];
        browser(&mut app).snapshot_picker = Some((saved, 0));
        app
    });
}

#[test]
fn developer_tools() {
    snap("tools", || {
        let mut app = app();
        let tool = |kind, status| Tool {
            kind,
            status,
            actions: Vec::new(),
        };
        let mut docker = tool(
            ToolKind::Docker,
            Status::Ready {
                reclaimable: 6 * GIB,
                detail: "3 images, 2 volumes".into(),
            },
        );
        docker.actions = vec![CleanAction {
            label: "docker system prune".into(),
            risk: Risk::Redownload,
            steps: vec![Step::Command(vec![
                "/usr/local/bin/docker".into(),
                "system".into(),
                "prune".into(),
            ])],
        }];
        let tools = vec![
            docker,
            tool(
                ToolKind::Npm,
                Status::Ready {
                    reclaimable: 800 * MIB,
                    detail: "~/.npm".into(),
                },
            ),
            tool(ToolKind::Cargo, Status::Measuring),
            tool(ToolKind::Pip, Status::Missing("not installed".into())),
            tool(
                ToolKind::Gradle,
                Status::Unavailable("daemon not running".into()),
            ),
        ];
        browser(&mut app).tools = Some(ToolsView::with_tools(tools));
        app
    });
}

#[test]
fn system_data() {
    snap("system", || {
        let mut app = app();
        let volume = |name: &str, role: &str, gib: u64| Volume {
            name: name.into(),
            role: role.into(),
            used: gib * GIB,
        };
        browser(&mut app).system = Some(SystemInfo {
            main: Some(Container {
                reference: "disk3".into(),
                total: 994 * GIB,
                free: 270 * GIB,
                volumes: vec![
                    volume("Macintosh HD - Data", "Data", 640),
                    volume("Macintosh HD", "System", 11),
                    volume("Preboot", "Preboot", 7),
                    volume("VM", "VM", 4),
                ],
            }),
            simulators: Vec::new(),
            snapshots: vec!["com.apple.TimeMachine.2026-10-01-110000.local".into()],
            swap: Some((4 * GIB, GIB)),
            sleepimage: Some(GIB),
            problems: Vec::new(),
        });
        app
    });
}
