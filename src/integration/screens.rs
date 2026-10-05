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
use crate::reports::ReportKind;
use crate::scanner::{ScanProgress, ScanResult};
use crate::system::{Container, SystemInfo, Volume};
use crate::tools::{CleanAction, Risk, Status, Step, Tool, ToolKind};
use crate::toolsview::ToolsView;
use crate::tree::{NodeId, Size, Tree, ROOT};
use crate::ui::theme::{with_theme, ThemeKind};
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
    demo_tree_with(&[])
}

/// The demo tree with more `(folder, file, size, age in days)` entries.
fn demo_tree_with(extra: &[(&str, &str, u64, u64)]) -> Tree {
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
    for &(parent, name, size, age_days) in files.iter().chain(extra) {
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
    app_over(demo_tree())
}

fn app_over(tree: Tree) -> App {
    let mut app = App::new(None);
    app.disks = disks();
    app.browser = Some(Browser::new(ScanResult {
        tree,
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
    snap_in(ThemeKind::Dark, name, &build);
}

fn snap_in(theme: ThemeKind, name: &str, build: &impl Fn() -> App) {
    for (lang, code) in [(Lang::Tr, "tr"), (Lang::En, "en")] {
        let screen = with_theme(theme, || {
            with_lang(lang, || {
                with_fixed_now(NOW, || {
                    let mut app = build();
                    render(&mut app)
                })
            })
        });
        insta::assert_snapshot!(format!("{name}-{code}"), screen);
    }
}

/// The same screen in the light, color-blind and no-color themes (the dark
/// one is covered by `snap`).
fn snap_themes(name: &str, build: impl Fn() -> App) {
    for theme in [ThemeKind::Light, ThemeKind::ColorBlind, ThemeKind::Mono] {
        snap_in(theme, &format!("{name}-{}", theme.code()), &build);
    }
}

#[test]
fn themes() {
    snap_themes("theme-browser", app);
    snap_themes("theme-treemap", || {
        let mut app = app();
        press(&mut app, KeyCode::Char('t'));
        app
    });
    snap_themes("theme-report", || {
        let mut app = app();
        menu(&mut app, 0);
        app
    });
    snap_themes("theme-dashboard", || {
        let mut app = app();
        press(&mut app, KeyCode::Char('i'));
        browser(&mut app).dashboard.as_mut().unwrap().disk = None;
        app
    });
    snap_themes("theme-tools", tools_app);
}

/// `T` cycles dark → light → color-blind → dark and says so.
#[test]
fn t_switches_the_theme() {
    use crate::ui::theme;
    let mut app = app();
    with_lang(Lang::En, || {
        press(&mut app, KeyCode::Char('T'));
        assert_eq!(theme::current(), ThemeKind::Light);
        let status = &browser(&mut app).status.as_ref().unwrap().text;
        assert_eq!(status, "Theme: Light (T: change)");
        press(&mut app, KeyCode::Char('T'));
        assert_eq!(theme::current(), ThemeKind::ColorBlind);
        press(&mut app, KeyCode::Char('T'));
        assert_eq!(theme::current(), ThemeKind::Dark);
        // Without colors, T leaves them off.
        theme::set(ThemeKind::Mono);
        press(&mut app, KeyCode::Char('T'));
        assert_eq!(theme::current(), ThemeKind::Mono);
        let status = &browser(&mut app).status.as_ref().unwrap().text;
        assert!(status.starts_with("Colors are off"), "{status}");
    });
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

/// Three backups of two devices; their details stand in for `Info.plist`.
#[test]
fn device_backups() {
    use crate::reports::{with_fake_backups, BackupInfo, MenuItem};
    use std::collections::HashMap;

    const BACKUP: &str = "Library/Application Support/MobileSync/Backup";
    let folders: [(&str, &str, &str, &str, u64, bool, u64); 3] = [
        (
            "00008110-000A1B2C3D4E5F60",
            "Demo iPhone",
            "iPhone 13 Pro",
            "17.5.1",
            12,
            true,
            38 * GIB,
        ),
        (
            "00008110-000A1B2C3D4E5F60-20250610-081500",
            "Demo iPhone",
            "iPhone 13 Pro",
            "16.7",
            480,
            true,
            31 * GIB,
        ),
        (
            "00008027-0011223344556677",
            "Studio iPad",
            "iPad13,4",
            "18.0",
            200,
            false,
            12 * GIB,
        ),
    ];
    let files: Vec<(String, u64, u64)> = folders
        .iter()
        .map(|&(dir, .., days, _, size)| (format!("{BACKUP}/{dir}"), size, days))
        .collect();
    let extra: Vec<(&str, &str, u64, u64)> = files
        .iter()
        .map(|(dir, size, days)| (dir.as_str(), "Manifest.db", *size, *days))
        .collect();
    let infos: HashMap<String, BackupInfo> = folders
        .iter()
        .map(|&(dir, device, model, version, days, encrypted, _)| {
            let id = dir.split('-').take(2).collect::<Vec<_>>().join("-");
            let info = BackupInfo {
                device: Some(device.into()),
                model: Some(model.into()),
                product_type: Some(model.into()),
                version: Some(version.into()),
                date: Some(u32::try_from(NOW - days * DAY).unwrap()),
                encrypted,
                device_id: Some(id),
            };
            (dir.to_string(), info)
        })
        .collect();
    let index = MenuItem::ALL
        .iter()
        .position(|&i| i == MenuItem::Report(ReportKind::DeviceBackups))
        .unwrap();
    snap("report-device-backups", || {
        with_fake_backups(infos.clone(), || {
            let mut app = app_over(demo_tree_with(&extra));
            menu(&mut app, index);
            app
        })
    });
    // Without backups, the note says why in place of the list.
    snap("report-device-backups-empty", || {
        let mut app = app();
        menu(&mut app, index);
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
    snap("tools", tools_app);
}

/// The tools screen with made-up measurements.
fn tools_app() -> App {
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
}

#[test]
fn linux_and_windows_tools() {
    snap("tools-linux", linux_tools_app);
    snap("tools-windows", windows_tools_app);
}

/// A made-up tool row with its actions.
fn tool_row(kind: ToolKind, status: Status, actions: Vec<CleanAction>) -> Tool {
    Tool {
        kind,
        status,
        actions,
    }
}

fn ready(gib: u64, detail: &str) -> Status {
    Status::Ready {
        reclaimable: gib * GIB,
        detail: detail.into(),
    }
}

/// The Linux system caches, with the snap revisions selected: their
/// commands need root, so they are shown for the user to run.
fn linux_tools_app() -> App {
    use crate::tools::linux::{self, SnapRevision};
    let mut app = app();
    let revs = [("core20", "2015"), ("firefox", "4173")].map(|(name, rev)| SnapRevision {
        name: name.into(),
        rev: rev.into(),
    });
    let not_installed = || Status::Missing("not installed".into());
    let tools = vec![
        tool_row(
            ToolKind::AptCache,
            ready(1, "/var/cache/apt"),
            vec![linux::apt_action()],
        ),
        tool_row(ToolKind::DnfCache, not_installed(), Vec::new()),
        tool_row(ToolKind::PacmanCache, not_installed(), Vec::new()),
        tool_row(
            ToolKind::Journal,
            ready(2, "all journals"),
            vec![linux::journal_action()],
        ),
        tool_row(
            ToolKind::Snaps,
            ready(1, "core20 (2015) · firefox (4173)"),
            vec![linux::snap_action(&revs, GIB)],
        ),
    ];
    let mut view = ToolsView::with_tools(tools);
    view.table.select(Some(4));
    browser(&mut app).tools = Some(view);
    app
}

/// The Windows folders, with the update cache selected: only `%TEMP%` is
/// cleaned by rustClean, the rest is shown as commands.
fn windows_tools_app() -> App {
    use crate::tools::windows;
    let mut app = app();
    let temp = r"C:\Users\demo\AppData\Local\Temp";
    let download = r"C:\Windows\SoftwareDistribution\Download";
    let tools = vec![
        tool_row(
            ToolKind::WinTemp,
            ready(3, temp),
            vec![windows::temp_action(PathBuf::from(temp))],
        ),
        tool_row(
            ToolKind::WinSystemTemp,
            Status::Unavailable("needs administrator rights to measure".into()),
            vec![windows::system_temp_action(Path::new(r"C:\Windows\Temp"))],
        ),
        tool_row(
            ToolKind::WinUpdate,
            ready(2, download),
            vec![windows::update_action(Path::new(download))],
        ),
        tool_row(
            ToolKind::RecycleBin,
            ready(5, "C: 5.0 GiB"),
            vec![windows::recycle_action()],
        ),
    ];
    let mut view = ToolsView::with_tools(tools);
    view.table.select(Some(2));
    browser(&mut app).tools = Some(view);
    app
}

/// Commands that need root or an administrator are never offered to run:
/// Enter on such a tool opens nothing, and Space cannot check them.
#[test]
fn manual_commands_cannot_be_run() {
    let mut app = windows_tools_app();
    press(&mut app, KeyCode::Enter);
    let view = browser(&mut app).tools.as_ref().unwrap();
    assert!(view.picker.is_none() && view.confirm.is_none());

    // %TEMP% goes through the normal flow.
    let tools = browser(&mut app).tools.as_mut().unwrap();
    tools.table.select(Some(0));
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char(' '));
    press(&mut app, KeyCode::Enter);
    let view = browser(&mut app).tools.as_ref().unwrap();
    let confirm = view.confirm.as_ref().expect("asks before trashing");
    assert!(matches!(confirm.actions[0].steps[0], Step::TrashEach(_)));

    // A tool mixing both: only the runnable action can be chosen.
    let mut app = windows_tools_app();
    let tools = browser(&mut app).tools.as_mut().unwrap();
    tools.tools[0]
        .actions
        .push(crate::tools::windows::recycle_action());
    tools.table.select(Some(0));
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char(' '));
    let view = browser(&mut app).tools.as_ref().unwrap();
    assert_eq!(view.picker.as_ref().unwrap().checked, [false, false]);
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

/// Sets up a screen to draw.
type Builder = fn() -> App;

/// Writes every theme of a few screens as a colored HTML page, to compare
/// themes in a browser:
/// `RUSTCLEAN_PREVIEW=preview.html cargo test theme_preview -- --ignored`
#[test]
#[ignore = "writes a file; run on demand"]
fn theme_preview() {
    let out = std::env::var("RUSTCLEAN_PREVIEW").unwrap_or_else(|_| "theme-preview.html".into());
    let screens: [(&str, Builder); 5] = [
        ("browser", app),
        ("treemap", || {
            let mut app = app();
            press(&mut app, KeyCode::Char('t'));
            app
        }),
        ("report", || {
            let mut app = app();
            menu(&mut app, 0);
            app
        }),
        ("dashboard", || {
            let mut app = app();
            press(&mut app, KeyCode::Char('i'));
            browser(&mut app).dashboard.as_mut().unwrap().disk = None;
            app
        }),
        ("tools", tools_app),
    ];
    let themes = [
        ThemeKind::Dark,
        ThemeKind::Light,
        ThemeKind::ColorBlind,
        ThemeKind::Mono,
    ];
    let mut html = String::from(
        "<!doctype html><meta charset=utf-8><title>rustClean themes</title><style>\
         body{font-family:system-ui;margin:16px;background:#888}\
         h2{margin:24px 0 8px}.row{display:flex;gap:16px;flex-wrap:wrap}\
         figure{margin:0}figcaption{font-weight:600;margin:4px 0}\
         pre{font:12px/1.25 Menlo,monospace;margin:0;padding:6px;white-space:pre}\
         </style><h1>rustClean themes</h1>",
    );
    for (name, build) in screens {
        html.push_str(&format!("<h2>{name}</h2><div class=row>"));
        for theme in themes {
            let light = theme == ThemeKind::Light;
            let buf = with_theme(theme, || {
                with_lang(Lang::Tr, || {
                    with_fixed_now(NOW, || {
                        let mut app = build();
                        let mut term = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
                        term.draw(|f| crate::ui::render(f, &mut app)).unwrap();
                        term.backend().buffer().clone()
                    })
                })
            });
            html.push_str(&format!(
                "<figure><figcaption>{}</figcaption>{}</figure>",
                theme.code(),
                buffer_html(&buf, light)
            ));
        }
        html.push_str("</div>");
    }
    std::fs::write(&out, html).unwrap();
    println!("wrote {out}");
}

/// The buffer as a `<pre>` with colored spans, on a dark or light terminal.
fn buffer_html(buf: &Buffer, light: bool) -> String {
    let (default_fg, default_bg) = if light {
        ("#1d1d1f", "#ffffff")
    } else {
        ("#e5e5e5", "#1e1e1e")
    };
    let mut out = format!("<pre style=\"color:{default_fg};background:{default_bg}\">");
    for y in 0..buf.area.height {
        // Neighbouring cells of the same style share a span.
        let mut runs: Vec<(String, String)> = Vec::new();
        for x in 0..buf.area.width {
            let c = &buf[(x, y)];
            let mut fg = css(c.fg).unwrap_or(default_fg.into());
            let mut bg = css(c.bg).unwrap_or(default_bg.into());
            if c.modifier.contains(Modifier::REVERSED) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let bold = if c.modifier.contains(Modifier::BOLD) {
                ";font-weight:bold"
            } else {
                ""
            };
            let style = format!("color:{fg};background:{bg}{bold}");
            let sym = match c.symbol() {
                "<" => "&lt;",
                ">" => "&gt;",
                "&" => "&amp;",
                s => s,
            };
            match runs.last_mut() {
                Some((last, text)) if *last == style => text.push_str(sym),
                _ => runs.push((style, sym.to_string())),
            }
        }
        for (style, text) in runs {
            out.push_str(&format!("<span style=\"{style}\">{text}</span>"));
        }
        out.push('\n');
    }
    out.push_str("</pre>");
    out
}

/// A terminal color as CSS (xterm's palette); `None` for the default.
fn css(c: Color) -> Option<String> {
    const NAMED: [&str; 16] = [
        "#000000", "#cd0000", "#00cd00", "#cdcd00", "#0000ee", "#cd00cd", "#00cdcd", "#e5e5e5",
        "#7f7f7f", "#ff0000", "#00ff00", "#ffff00", "#5c5cff", "#ff00ff", "#00ffff", "#ffffff",
    ];
    let named = |i: usize| Some(NAMED[i].to_string());
    match c {
        Color::Reset => None,
        Color::Black => named(0),
        Color::Red => named(1),
        Color::Green => named(2),
        Color::Yellow => named(3),
        Color::Blue => named(4),
        Color::Magenta => named(5),
        Color::Cyan => named(6),
        Color::Gray => named(7),
        Color::DarkGray => named(8),
        Color::LightRed => named(9),
        Color::LightGreen => named(10),
        Color::LightYellow => named(11),
        Color::LightBlue => named(12),
        Color::LightMagenta => named(13),
        Color::LightCyan => named(14),
        Color::White => named(15),
        Color::Indexed(n @ 0..=15) => named(usize::from(n)),
        Color::Indexed(n @ 16..=231) => {
            let i = n - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * u32::from(v) };
            Some(format!(
                "#{:02x}{:02x}{:02x}",
                level(i / 36),
                level((i / 6) % 6),
                level(i % 6)
            ))
        }
        Color::Indexed(n) => {
            let v = 8 + 10 * u32::from(n - 232);
            Some(format!("#{v:02x}{v:02x}{v:02x}"))
        }
        Color::Rgb(r, g, b) => Some(format!("#{r:02x}{g:02x}{b:02x}")),
    }
}

#[test]
fn help_screen() {
    snap("help-browser", || {
        let mut app = app();
        press(&mut app, KeyCode::Char('?'));
        app
    });
    snap("help-report", || {
        let mut app = app();
        menu(&mut app, 0);
        press(&mut app, KeyCode::Char('?'));
        app
    });
    snap("help-disks", || {
        let mut app = app();
        app.screen = Screen::DiskSelect;
        press(&mut app, KeyCode::Char('?'));
        app
    });
    snap("help-scrolled", || {
        let mut app = app();
        press(&mut app, KeyCode::Char('?'));
        press(&mut app, KeyCode::PageDown);
        app
    });
}

#[test]
fn help_keys() {
    let mut app = app();
    press(&mut app, KeyCode::Char('?'));
    assert_eq!(app.help, Some(0));
    // Scrolling stays within the text once drawn.
    for _ in 0..50 {
        press(&mut app, KeyCode::PageDown);
    }
    render(&mut app);
    let max = app.help.unwrap();
    assert!(max > 0 && max < 500, "{max}");
    press(&mut app, KeyCode::Home);
    assert_eq!(app.help, Some(0));
    // Any other key closes it and does nothing else.
    press(&mut app, KeyCode::Char('m'));
    assert_eq!(app.help, None);
    assert!(browser(&mut app).report_menu.is_none());
    // Over a dialog: the dialog stays open behind the help.
    let id = find(&mut app, "Downloads/setup.dmg");
    browser(&mut app).confirm = Some(vec![id]);
    press(&mut app, KeyCode::Char('?'));
    press(&mut app, KeyCode::Esc);
    assert!(browser(&mut app).confirm.is_some());
    // In the search box `?` is a wildcard, not help.
    browser(&mut app).confirm = None;
    press(&mut app, KeyCode::Char('/'));
    press(&mut app, KeyCode::Char('?'));
    assert_eq!(app.help, None);
    assert_eq!(browser(&mut app).input.as_deref(), Some("?"));
}

/// Sets up a screen to draw; closures that capture nothing are fine too.
type BoxedBuilder = Box<dyn Fn() -> App>;

/// Every key the bottom line names is in the help of that screen.
#[test]
fn help_lists_every_key_of_the_bottom_line() {
    use crate::ui::help::{global_keys, topic};
    let tokens = |k: &str| -> Vec<String> {
        k.split(|c: char| c.is_whitespace() || c == '/')
            .filter(|t| !t.is_empty() && *t != "…")
            .map(str::to_string)
            .collect()
    };
    let screens: Vec<(&str, BoxedBuilder)> = vec![
        ("list", Box::new(app)),
        (
            "map",
            Box::new(|| {
                let mut app = app();
                press(&mut app, KeyCode::Char('t'));
                app
            }),
        ),
        (
            "report",
            Box::new(|| {
                let mut app = app();
                menu(&mut app, 0);
                app
            }),
        ),
        (
            "apps",
            Box::new(|| {
                let mut app = app();
                menu(&mut app, 3);
                app
            }),
        ),
        (
            "basket",
            Box::new(|| {
                let mut app = app();
                let id = find(&mut app, "Downloads/setup.dmg");
                let b = browser(&mut app);
                b.basket.add(&b.tree, id);
                press(&mut app, KeyCode::Char('S'));
                app
            }),
        ),
        (
            "dashboard",
            Box::new(|| {
                let mut app = app();
                press(&mut app, KeyCode::Char('i'));
                app
            }),
        ),
        (
            "menu",
            Box::new(|| {
                let mut app = app();
                press(&mut app, KeyCode::Char('m'));
                app
            }),
        ),
        (
            "confirm",
            Box::new(|| {
                let mut app = app();
                let id = find(&mut app, "Downloads/setup.dmg");
                browser(&mut app).confirm = Some(vec![id]);
                app
            }),
        ),
        (
            "uninstall",
            Box::new(|| {
                let mut app = app();
                let id = find(&mut app, "Applications/Sketchpad.app");
                browser(&mut app).uninstall = Some(UninstallDialog {
                    label: "Sketchpad".into(),
                    items: vec![(id, true)],
                    cursor: 0,
                    running: false,
                });
                app
            }),
        ),
        ("tools", Box::new(tools_app)),
        (
            "log",
            Box::new(|| {
                let mut app = app();
                browser(&mut app).deletion_log = Some(crate::app::LogView {
                    entries: Vec::new(),
                    scroll: 0,
                });
                app
            }),
        ),
        (
            "system",
            Box::new(|| {
                let mut app = app();
                press(&mut app, KeyCode::Char('m'));
                browser(&mut app).report_menu = None;
                browser(&mut app).system = Some(SystemInfo {
                    main: None,
                    simulators: Vec::new(),
                    snapshots: Vec::new(),
                    swap: None,
                    sleepimage: None,
                    problems: Vec::new(),
                });
                app
            }),
        ),
    ];
    for lang in [Lang::Tr, Lang::En] {
        with_lang(lang, || {
            for (name, build) in &screens {
                let mut app = build();
                let t = topic(&app);
                let known: Vec<String> = t
                    .keys()
                    .into_iter()
                    .chain(global_keys())
                    .flat_map(|(k, _)| tokens(k))
                    .collect();
                let footer = crate::ui::footer_keys(browser(&mut app));
                for (k, desc) in footer {
                    for tok in tokens(k) {
                        assert!(
                            known.contains(&tok),
                            "{lang:?} {name} ({t:?}): `{tok}` ({desc}) is not in the help"
                        );
                    }
                }
            }
        });
    }
}

#[test]
fn deletion_log() {
    use crate::app::LogView;
    use crate::trashlog::{Entry, Via};
    let entry = |hours_ago: u64, path: &str, gib_tenths: u64, via: Via| Entry {
        time: NOW - hours_ago * 3600,
        path: path.into(),
        size: Size {
            apparent: gib_tenths * GIB / 10,
            disk: gib_tenths * GIB / 10,
        },
        via,
    };
    snap("deletion-log", || {
        let mut app = app();
        browser(&mut app).deletion_log = Some(LogView {
            entries: vec![
                entry(1, "/Users/demo/Downloads/Xcode_16.xip", 30, Via::Basket),
                entry(
                    2,
                    "/Users/demo/Downloads/setup.dmg",
                    4,
                    Via::Report(ReportKind::Downloads),
                ),
                entry(3, "/Users/demo/.npm/_cacache", 8, Via::Tool(ToolKind::Npm)),
                entry(
                    30,
                    "/Users/demo/Applications/Sketchpad.app",
                    2,
                    Via::Uninstall("Sketchpad".into()),
                ),
                entry(
                    30,
                    "/Users/demo/Library/Application Support/com.example.sketchpad",
                    1,
                    Via::Uninstall("Sketchpad".into()),
                ),
                entry(800, "/Users/demo/Movies/old.mov", 12, Via::List),
            ],
            scroll: 0,
        });
        app
    });
    snap("deletion-log-empty", || {
        let mut app = app();
        browser(&mut app).deletion_log = Some(LogView {
            entries: Vec::new(),
            scroll: 0,
        });
        app
    });
}
