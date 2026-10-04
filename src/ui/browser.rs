//! The browser screen: header, status, key hints, and the folder listing.
//! It hands the body to the other views (map, results, dashboard, …).

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Row, Table};
use ratatui::Frame;

use crate::app::{App, Browser, View};
use crate::tree::SizeMode;

use super::dashboard::render_dashboard;
use super::dialogs::{render_confirm, render_failures, render_uninstall};
use super::format::{fmt_count, fmt_pct, fmt_size, now_secs};
use super::map::render_map;
use super::menus::{render_report_menu, render_snapshot_picker};
use super::results::render_results;
use super::style::{date_cell, HIGHLIGHT};
use super::system::render_system;
use super::tools::render_tools;
use super::{bar, keys, table_block, title, DATE_WIDTH, SPINNER, WIDE};

pub(super) fn render_browser(
    f: &mut Frame<'_>,
    app: &mut App,
    header: Rect,
    body: Rect,
    footer: Rect,
) {
    let Some(b) = &mut app.browser else { return };
    let mode = b.size_mode;
    let mode_label = match mode {
        SizeMode::Disk => t!("diskte", "on disk"),
        SizeMode::Apparent => t!("görünen", "apparent"),
    };

    let heading = if let Some(view) = &b.tools {
        tf!(
            "Geliştirici araçları temizliği  │  geri kazanılabilir: {}{}",
            "Developer tools cleanup  │  reclaimable: {}{}",
            fmt_size(view.reclaimable()),
            if view.measuring() {
                t!(" (ölçülüyor…)", " (measuring…)")
            } else {
                ""
            }
        )
    } else if b.system.is_some() {
        t!("Sistem verileri", "System data").to_string()
    } else if let Some(d) = &b.dashboard {
        tf!(
            "Özet — {}  │  {mode_label}",
            "Summary — {}  │  {mode_label}",
            b.tree.path_of(d.base).display()
        )
    } else if let Some(r) = &b.results {
        let shown = if r.truncated {
            t!("ilk ", "first ")
        } else {
            ""
        };
        tf!(
            "{} — {}  │  {shown}{} satır, {} ({mode_label})",
            "{} — {}  │  {shown}{} rows, {} ({mode_label})",
            r.title,
            b.tree.path_of(r.base).display(),
            fmt_count(r.rows.len() as u64),
            fmt_size(r.total_size()),
        )
    } else {
        let cur = b.tree.node(b.current);
        tf!(
            "{}  │  {} ({mode_label})  │  {} dosya  │  sıralama: {}",
            "{}  │  {} ({mode_label})  │  {} files  │  sort: {}",
            b.tree.path_of(b.current).display(),
            fmt_size(cur.size.get(mode)),
            fmt_count(cur.file_count.into()),
            b.sort.label()
        )
    };
    let heading = if b.basket.is_empty() {
        heading
    } else {
        // First, so long paths cannot push it off screen.
        tf!(
            "🧺 {} öğe, {} (S)  │  {heading}",
            "🧺 {} items, {} (S)  │  {heading}",
            fmt_count(b.basket.len() as u64),
            fmt_size(b.basket.size(&b.tree, mode))
        )
    };
    f.render_widget(title(heading), header);

    let [table_area, status_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(body);

    if let Some(view) = &mut b.tools {
        render_tools(f, view, app.tick, table_area);
    } else if let Some(sys) = &b.system {
        render_system(f, b, sys, table_area);
    } else if let Some(d) = &mut b.dashboard {
        render_dashboard(f, &b.tree, d, mode, b.errors, table_area);
    } else if let Some(r) = &b.results {
        let checks: Vec<bool> = r.rows.iter().map(|row| b.row_in_basket(r, row)).collect();
        if let Some(r) = &mut b.results {
            render_results(f, &b.tree, r, &checks, table_area);
        }
    } else if b.view == View::Map {
        render_map(f, b, table_area);
    } else {
        render_entries(f, b, table_area);
    }

    let status = if let Some(r) = &b.rescan {
        let spin = SPINNER[app.tick % SPINNER.len()];
        let p = &r.progress;
        Line::from(vec![
            Span::raw(tf!(
                "{spin} Klasör yeniden taranıyor — {} dosya, {}",
                "{spin} Rescanning the folder — {} files, {}",
                fmt_count(p.files),
                fmt_size(p.bytes)
            ))
            .cyan(),
            Span::raw(t!("   Esc: iptal", "   Esc: cancel")).gray(),
        ])
    } else if let Some(job) = &b.dup_job {
        use std::sync::atomic::Ordering::Relaxed;
        let p = &job.progress;
        let (done, total) = (p.done.load(Relaxed), p.total.load(Relaxed));
        let spin = SPINNER[app.tick % SPINNER.len()];
        let step = match p.stage.load(Relaxed) {
            0 => t!(
                "aynı boyuttaki dosyalar bulunuyor",
                "finding files of equal size"
            )
            .to_string(),
            1 => tf!(
                "1/2 dosya uçları karşılaştırılıyor: {} / {}",
                "1/2 comparing file edges: {} / {}",
                fmt_count(done),
                fmt_count(total)
            ),
            _ => tf!(
                "2/2 içerikler okunuyor: {} / {}",
                "2/2 reading contents: {} / {}",
                fmt_size(done),
                fmt_size(total)
            ),
        };
        Line::from(vec![
            Span::raw(tf!(
                "{spin} Kopyalar aranıyor — {step}",
                "{spin} Looking for duplicates — {step}"
            ))
            .cyan(),
            Span::raw(t!("   Esc: iptal", "   Esc: cancel")).gray(),
        ])
    } else if let Some(input) = &b.input {
        Line::from(vec![
            Span::raw(t!(" Ara: ", " Find: "))
                .black()
                .on_yellow()
                .bold(),
            Span::raw(format!(" {input}")),
            Span::raw("█").slow_blink(),
            Span::raw(t!(
                "   örn: deneme · deneme* · *.log",
                "   e.g. test · test* · *.log"
            ))
            .gray(),
        ])
    } else if let Some(st) = &b.status {
        let style = if st.error {
            Style::new().fg(Color::Red)
        } else {
            Style::new().fg(Color::Green)
        };
        Line::from(Span::styled(st.text.clone(), style))
    } else if let Some(view) = &b.tools {
        match &view.run {
            Some(run) if run.finished => Line::from(t!(
                "Bitti. Araç yeniden ölçülüyor; ana listedeki boyutlar için gezginde r ile yeniden tarayın.",
                "Done. Measuring the tool again; rescan with r in the browser to update the main list.",
            ))
            .green(),
            Some(_) => Line::from(t!("Komutlar çalışıyor, lütfen bekleyin…", "Commands are running, please wait…")).cyan(),
            None => Line::from(t!(
                "Ölçüm yalnızca okur. Hiçbir komut siz onaylamadan çalışmaz.",
                "Measuring only reads. No command runs without your confirmation.",
            ))
            .gray(),
        }
    } else if b.system.is_some() {
        Line::from(t!(
            "Bu ekran yalnızca bilgi verir; hiçbir şeyi değiştirmez.",
            "This screen only shows information; it changes nothing.",
        ))
        .gray()
    } else if b.dashboard.is_some() {
        Line::from(t!(
            "„En dolu klasörler” alt klasörleri saymaz: yerin asıl durduğu klasörleri gösterir.",
            "“Fullest folders” ignores subfolders: it shows where the space actually sits.",
        ))
        .gray()
    } else if let Some(r) = &b.results {
        let text = if r.rows.is_empty() {
            t!("Sonuç yok.", "No results.")
        } else {
            r.note.as_str()
        };
        Line::from(text).gray()
    } else {
        let mut spans = vec![Span::raw(tf!(
            "{} öğe tarandı, {:.1} sn",
            "{} items scanned, {:.1} s",
            fmt_count(b.tree.len() as u64),
            b.elapsed.as_secs_f64()
        ))
        .gray()];
        if b.errors > 0 {
            spans.push(
                Span::raw(tf!(
                    "   ⚠ {} öğeye erişilemedi",
                    "   ⚠ {} items inaccessible",
                    fmt_count(b.errors)
                ))
                .yellow(),
            );
        }
        if b.trashed.get(mode) > 0 {
            spans.push(
                Span::raw(tf!(
                    "   🗑 bu oturumda çöpe taşınan: {}",
                    "   🗑 moved to trash this session: {}",
                    fmt_size(b.trashed.get(mode))
                ))
                .green(),
            );
        }
        if b.entries.is_empty() {
            spans.push(Span::raw(t!("   (klasör boş)", "   (empty folder)")).gray());
        }
        Line::from(spans)
    };
    f.render_widget(status, status_area);

    let footer_keys: &[(&str, &str)] = if b.rescan.is_some() {
        &[("Esc", t!("iptal", "cancel")), ("q", t!("çık", "quit"))]
    } else if b.failures.is_some() {
        &[
            ("↑↓", t!("kaydır", "scroll")),
            ("Esc", t!("kapat", "close")),
        ]
    } else if b.dup_job.is_some() {
        &[("Esc", t!("iptal", "cancel")), ("q", t!("çık", "quit"))]
    } else if b.report_menu.is_some() || b.snapshot_picker.is_some() {
        &[
            ("↑↓", t!("seç", "select")),
            ("Enter", t!("çalıştır", "run")),
            ("Esc", t!("kapat", "close")),
        ]
    } else if b.uninstall.is_some() {
        &[
            ("↑↓", t!("gez", "move")),
            ("Space", t!("işaretle", "check")),
            ("t", t!("tümü", "all")),
            (t!("e", "y"), t!("kaldır", "uninstall")),
            (t!("h / Esc", "n / Esc"), t!("vazgeç", "cancel")),
        ]
    } else if b.confirm.is_some() {
        &[
            (t!("e", "y"), t!("evet, çöpe taşı", "yes, move to trash")),
            (t!("h / Esc", "n / Esc"), t!("vazgeç", "cancel")),
        ]
    } else if b.deleting.is_some() {
        &[("q", t!("çık", "quit"))]
    } else if b.input.is_some() {
        &[
            ("Enter", t!("ara", "find")),
            ("Esc", t!("vazgeç", "cancel")),
        ]
    } else if let Some(view) = &b.tools {
        if view.running() {
            &[("…", t!("komutlar çalışıyor", "commands running"))]
        } else if view.confirm.is_some() || view.picker.is_some() {
            &[("Esc", t!("vazgeç", "cancel"))]
        } else {
            &[
                ("↑↓", t!("seç", "select")),
                ("Enter", t!("temizle", "clean")),
                ("r", t!("yeniden ölç", "measure again")),
                ("m", t!("menü", "menu")),
                ("Esc", t!("geri", "back")),
                ("q", t!("çık", "quit")),
            ]
        }
    } else if b.system.is_some() {
        &[
            ("r", t!("yenile", "refresh")),
            ("m", t!("menü", "menu")),
            ("Esc", t!("geri", "back")),
            ("q", t!("çık", "quit")),
        ]
    } else if b.dashboard.is_some() {
        &[
            ("Tab", t!("liste değiştir", "switch list")),
            ("↑↓", t!("gez", "move")),
            ("Enter", t!("konuma git", "go to location")),
            ("Space", t!("sepete", "to basket")),
            ("x", t!("çöpe taşı", "move to trash")),
            ("a", t!("görünen/diskte", "apparent/on disk")),
            ("Esc", t!("geri", "back")),
            ("q", t!("çık", "quit")),
        ]
    } else if b.apps_report().is_some() {
        &[
            ("↑↓", t!("gez", "move")),
            ("u", t!("uygulamayı kaldır", "uninstall app")),
            ("Space", t!("sepete", "to basket")),
            ("x", t!("çöpe taşı", "move to trash")),
            ("S", t!("sepet", "basket")),
            ("Enter", t!("aç / konuma git", "open / go to")),
            ("m", t!("raporlar", "reports")),
            ("Esc", t!("geri", "back")),
            ("q", t!("çık", "quit")),
        ]
    } else if b.results.is_some() {
        &[
            ("↑↓", t!("gez", "move")),
            ("Space", t!("sepete", "to basket")),
            ("t", t!("tümü", "all")),
            ("f", t!("yaş filtresi", "age filter")),
            ("x", t!("çöpe taşı", "move to trash")),
            ("S", t!("sepet", "basket")),
            ("Enter", t!("aç / konuma git", "open / go to")),
            ("/", t!("ara", "find")),
            ("m", t!("raporlar", "reports")),
            ("Esc", t!("geri", "back")),
            ("q", t!("çık", "quit")),
        ]
    } else if b.view == View::Map {
        &[
            ("←↑↓→", t!("blok seç", "select block")),
            ("Enter", t!("gir", "open")),
            ("⌫", t!("geri", "back")),
            ("c", t!("renk", "color")),
            ("t/Esc", t!("liste", "list")),
            ("Space", t!("sepete", "to basket")),
            ("x", t!("çöpe taşı", "move to trash")),
            ("S", t!("sepet", "basket")),
            ("R", t!("klasörü yenile", "refresh folder")),
            ("m", t!("raporlar", "reports")),
            ("q", t!("çık", "quit")),
        ]
    } else {
        &[
            ("↑↓", t!("gez", "move")),
            ("Enter", t!("gir", "open")),
            ("⌫", t!("geri", "back")),
            ("t", t!("harita", "map")),
            ("/", t!("ara", "find")),
            ("m", t!("raporlar", "reports")),
            ("i", t!("özet", "summary")),
            ("s", t!("sırala", "sort")),
            ("a", t!("görünen/diskte", "apparent/on disk")),
            ("Space", t!("sepete", "to basket")),
            ("x", t!("çöpe taşı", "move to trash")),
            ("S", t!("sepet", "basket")),
            ("R", t!("klasörü yenile", "refresh folder")),
            ("r", t!("tümünü tara", "rescan all")),
            ("d", t!("diskler", "disks")),
            ("q", t!("çık", "quit")),
        ]
    };
    f.render_widget(keys(footer_keys), footer);

    if let Some(sel) = b.report_menu {
        render_report_menu(f, sel, f.area());
    }
    if let Some((saved, sel)) = &b.snapshot_picker {
        let total = b.tree.node(crate::tree::ROOT).size.disk;
        render_snapshot_picker(f, saved, *sel, total, f.area());
    }
    if let Some(ids) = &b.confirm {
        render_confirm(f, &b.tree, ids, mode, f.area());
    }
    if let Some(d) = &b.uninstall {
        render_uninstall(f, &b.tree, d, mode, f.area());
    }
    if let Some(dialog) = &mut b.failures {
        render_failures(f, dialog, f.area());
    }
}

fn render_entries(f: &mut Frame<'_>, b: &mut Browser, area: Rect) {
    let tree = &b.tree;
    let mode = b.size_mode;
    let now = now_secs();
    let wide = area.width >= WIDE;
    let parent_size = tree.node(b.current).size.get(mode).max(1);
    let rows = b.entries.iter().map(|&id| {
        let n = tree.node(id);
        let size = n.size.get(mode);
        let ratio = size as f64 / parent_size as f64;
        let name = if n.is_dir {
            Span::styled(
                format!("{}/", tree.name(id)),
                Style::new().fg(Color::LightBlue).bold(),
            )
        } else {
            Span::raw(tree.name(id).to_string())
        };
        let count = if n.is_dir {
            fmt_count(n.file_count.into())
        } else {
            String::new()
        };
        let in_basket = if b.basket.covers(tree, id) {
            Span::raw("✓").green().bold()
        } else {
            Span::raw(" ")
        };
        let mut cells = vec![
            Cell::from(in_basket),
            Cell::from(Line::from(fmt_size(size)).right_aligned()),
            Cell::from(Line::from(vec![
                Span::styled(bar(ratio, 12), Style::new().fg(Color::Cyan)),
                Span::raw(format!(" {:>6}", fmt_pct(ratio * 100.0, 1))),
            ])),
            Cell::from(Line::from(count).right_aligned().gray()),
            date_cell(n.modified, now),
        ];
        if wide {
            cells.push(date_cell(n.created, now));
        }
        cells.push(Cell::from(name));
        Row::new(cells)
    });

    let mut widths = vec![
        Constraint::Length(1),
        Constraint::Length(11),
        Constraint::Length(19),
        Constraint::Length(10),
        Constraint::Length(DATE_WIDTH),
    ];
    let mut header = vec![
        "",
        t!("Boyut", "Size"),
        t!("Oran", "Share"),
        t!("Dosya", "Files"),
        t!("Son değişiklik", "Modified"),
    ];
    if wide {
        widths.push(Constraint::Length(DATE_WIDTH));
        header.push(t!("Oluşturma", "Created"));
    }
    widths.push(Constraint::Min(10));
    header.push(t!("Ad", "Name"));

    let table = Table::new(rows, widths)
        .header(Row::new(header).bold().underlined())
        .row_highlight_style(HIGHLIGHT)
        .highlight_symbol("▶ ")
        .block(table_block());
    f.render_stateful_widget(table, area, &mut b.table);
}
