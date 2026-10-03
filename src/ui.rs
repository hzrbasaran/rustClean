//! Rendering of the three screens.

use std::path::Path;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::app::{App, Browser, Dashboard, MapColor, Pane, Screen, View};
use crate::lists::ResultList;
use crate::reports::MenuItem;
use crate::stats;
use crate::system::{self, SystemInfo};
use crate::tools::{Risk, Status as ToolStatus};
use crate::toolsview::{ToolsView, CONFIRM_WORD};
use crate::tree::{NodeId, SizeMode, Tree};
use crate::treemap::Slot;

const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const HIGHLIGHT: Style = Style::new()
    .bg(Color::DarkGray)
    .add_modifier(Modifier::BOLD);

pub fn render(f: &mut Frame, app: &mut App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(f.area());

    match app.screen {
        Screen::DiskSelect => render_disks(f, app, header, body, footer),
        Screen::Scanning => render_scanning(f, app, header, body, footer),
        Screen::Browser => render_browser(f, app, header, body, footer),
    }
}

fn title(text: String) -> Line<'static> {
    Line::from(vec![
        Span::raw(" rustClean ").black().on_cyan().bold(),
        Span::raw(" "),
        Span::raw(text),
    ])
}

fn keys(pairs: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (k, desc) in pairs {
        spans.push(Span::raw(format!(" {k} ")).black().on_gray());
        spans.push(Span::raw(format!(" {desc}  ")));
    }
    Line::from(spans)
}

fn render_disks(f: &mut Frame, app: &mut App, header: Rect, body: Rect, footer: Rect) {
    f.render_widget(
        title("Diskler — taramak için bir disk seçin".into()),
        header,
    );

    let [table_area, msg_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(body);

    let rows = app.disks.iter().map(|d| {
        let ratio = d.usage_ratio();
        let color = usage_color(ratio);
        Row::new(vec![
            Cell::from(d.name.clone()),
            Cell::from(d.mount_point.display().to_string()),
            Cell::from(d.fs_type.clone()),
            Cell::from(Line::from(fmt_size(d.used())).right_aligned()),
            Cell::from(Line::from(fmt_size(d.total)).right_aligned()),
            Cell::from(Line::from(vec![
                Span::styled(bar(ratio, 20), Style::new().fg(color)),
                Span::raw(format!(" {:>3.0}%", ratio * 100.0)),
            ])),
            Cell::from(if d.removable { "çıkarılabilir" } else { "" }),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(20),
            Constraint::Percentage(25),
            Constraint::Length(8),
            Constraint::Length(11),
            Constraint::Length(11),
            Constraint::Length(26),
            Constraint::Length(13),
        ],
    )
    .header(
        Row::new([
            "Ad",
            "Bağlama noktası",
            "FS",
            "Kullanılan",
            "Toplam",
            "Doluluk",
            "",
        ])
        .bold()
        .underlined(),
    )
    .row_highlight_style(HIGHLIGHT)
    .highlight_symbol("▶ ")
    .block(Block::new().borders(Borders::TOP | Borders::BOTTOM));
    f.render_stateful_widget(table, table_area, &mut app.disk_table);

    if let Some(msg) = &app.message {
        f.render_widget(Line::from(msg.clone()).yellow(), msg_area);
    } else if app.disks.is_empty() {
        f.render_widget(Line::from("Hiç disk bulunamadı.").yellow(), msg_area);
    }

    f.render_widget(
        keys(&[
            ("↑↓", "seç"),
            ("Enter", "tara"),
            ("r", "yenile"),
            ("q", "çık"),
        ]),
        footer,
    );
}

fn render_scanning(f: &mut Frame, app: &mut App, header: Rect, body: Rect, footer: Rect) {
    f.render_widget(
        title(format!("Taranıyor: {}", app.scan_root.display())),
        header,
    );

    let p = &app.progress;
    let elapsed = app
        .scan
        .as_ref()
        .map(|s| s.started.elapsed())
        .unwrap_or_default();
    let spin = SPINNER[app.tick % SPINNER.len()];
    let width = body.width.saturating_sub(16) as usize;
    let lines = vec![
        Line::from(""),
        Line::from(format!("  {spin} Taranıyor…")).cyan().bold(),
        Line::from(""),
        stat_line("Dosya", &fmt_count(p.files)),
        stat_line("Klasör", &fmt_count(p.dirs)),
        stat_line("Boyut", &fmt_size(p.bytes)),
        stat_line("Erişilemeyen", &fmt_count(p.errors)),
        stat_line("Süre", &format!("{:.1} sn", elapsed.as_secs_f64())),
        Line::from(""),
        stat_line("Şu an", &truncate_path(&p.current, width)),
    ];
    f.render_widget(Paragraph::new(lines), body);
    f.render_widget(keys(&[("Esc", "iptal"), ("q", "çık")]), footer);
}

fn stat_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw(format!("  {label:<13}")).gray(),
        Span::raw(value.to_string()),
    ])
}

fn render_browser(f: &mut Frame, app: &mut App, header: Rect, body: Rect, footer: Rect) {
    let Some(b) = &mut app.browser else { return };
    let mode = b.size_mode;
    let mode_label = match mode {
        SizeMode::Disk => "diskte",
        SizeMode::Apparent => "görünen",
    };

    let heading = if let Some(view) = &b.tools {
        format!(
            "Geliştirici araçları temizliği  │  geri kazanılabilir: {}{}",
            fmt_size(view.reclaimable()),
            if view.measuring() {
                " (ölçülüyor…)"
            } else {
                ""
            }
        )
    } else if b.system.is_some() {
        "Sistem verileri".to_string()
    } else if let Some(d) = &b.dashboard {
        format!(
            "Özet — {}  │  {mode_label}",
            b.tree.path_of(d.base).display()
        )
    } else if let Some(r) = &b.results {
        let shown = if r.truncated { "ilk " } else { "" };
        format!(
            "{} — {}  │  {shown}{} satır, {} ({mode_label})",
            r.title,
            b.tree.path_of(r.base).display(),
            fmt_count(r.rows.len() as u64),
            fmt_size(r.total_size()),
        )
    } else {
        let cur = b.tree.node(b.current);
        format!(
            "{}  │  {} ({mode_label})  │  {} dosya  │  sıralama: {}",
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
        format!(
            "🧺 {} öğe, {} (S)  │  {heading}",
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

    let status = if let Some(job) = &b.dup_job {
        use std::sync::atomic::Ordering::Relaxed;
        let p = &job.progress;
        let (done, total) = (p.done.load(Relaxed), p.total.load(Relaxed));
        let spin = SPINNER[app.tick % SPINNER.len()];
        let step = match p.stage.load(Relaxed) {
            0 => "aynı boyuttaki dosyalar bulunuyor".to_string(),
            1 => format!(
                "1/2 dosya uçları karşılaştırılıyor: {} / {}",
                fmt_count(done),
                fmt_count(total)
            ),
            _ => format!(
                "2/2 içerikler okunuyor: {} / {}",
                fmt_size(done),
                fmt_size(total)
            ),
        };
        Line::from(vec![
            Span::raw(format!("{spin} Kopyalar aranıyor — {step}")).cyan(),
            Span::raw("   Esc: iptal").gray(),
        ])
    } else if let Some(input) = &b.input {
        Line::from(vec![
            Span::raw(" Ara: ").black().on_yellow().bold(),
            Span::raw(format!(" {input}")),
            Span::raw("█").slow_blink(),
            Span::raw("   örn: deneme · deneme* · *.log").gray(),
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
            Some(run) if run.finished => Line::from(
                "Bitti. Araç yeniden ölçülüyor; ana listedeki boyutlar için gezginde r ile yeniden tarayın.",
            )
            .green(),
            Some(_) => Line::from("Komutlar çalışıyor, lütfen bekleyin…").cyan(),
            None => Line::from("Ölçüm yalnızca okur. Hiçbir komut siz onaylamadan çalışmaz.").gray(),
        }
    } else if b.system.is_some() {
        Line::from("Bu ekran yalnızca bilgi verir; hiçbir şeyi değiştirmez.").gray()
    } else if b.dashboard.is_some() {
        Line::from(
            "„En dolu klasörler” alt klasörleri saymaz: yerin asıl durduğu klasörleri gösterir.",
        )
        .gray()
    } else if let Some(r) = &b.results {
        let text = if r.rows.is_empty() {
            "Sonuç yok."
        } else {
            r.note.as_str()
        };
        Line::from(text).gray()
    } else {
        let mut spans = vec![Span::raw(format!(
            "{} öğe tarandı, {:.1} sn",
            fmt_count(b.tree.len() as u64),
            b.elapsed.as_secs_f64()
        ))
        .gray()];
        if b.errors > 0 {
            spans.push(
                Span::raw(format!("   ⚠ {} öğeye erişilemedi", fmt_count(b.errors))).yellow(),
            );
        }
        if b.trashed.get(mode) > 0 {
            spans.push(
                Span::raw(format!(
                    "   🗑 bu oturumda çöpe taşınan: {}",
                    fmt_size(b.trashed.get(mode))
                ))
                .green(),
            );
        }
        if b.entries.is_empty() {
            spans.push(Span::raw("   (klasör boş)").gray());
        }
        Line::from(spans)
    };
    f.render_widget(status, status_area);

    let footer_keys: &[(&str, &str)] = if b.dup_job.is_some() {
        &[("Esc", "iptal"), ("q", "çık")]
    } else if b.report_menu.is_some() || b.snapshot_picker.is_some() {
        &[("↑↓", "seç"), ("Enter", "çalıştır"), ("Esc", "kapat")]
    } else if b.confirm.is_some() {
        &[("e", "evet, çöpe taşı"), ("h / Esc", "vazgeç")]
    } else if b.deleting.is_some() {
        &[("q", "çık")]
    } else if b.input.is_some() {
        &[("Enter", "ara"), ("Esc", "vazgeç")]
    } else if let Some(view) = &b.tools {
        if view.running() {
            &[("…", "komutlar çalışıyor")]
        } else if view.confirm.is_some() || view.picker.is_some() {
            &[("Esc", "vazgeç")]
        } else {
            &[
                ("↑↓", "seç"),
                ("Enter", "temizle"),
                ("r", "yeniden ölç"),
                ("m", "menü"),
                ("Esc", "geri"),
                ("q", "çık"),
            ]
        }
    } else if b.system.is_some() {
        &[
            ("r", "yenile"),
            ("m", "menü"),
            ("Esc", "geri"),
            ("q", "çık"),
        ]
    } else if b.dashboard.is_some() {
        &[
            ("Tab", "liste değiştir"),
            ("↑↓", "gez"),
            ("Enter", "konuma git"),
            ("Space", "sepete"),
            ("x", "çöpe taşı"),
            ("a", "görünen/diskte"),
            ("Esc", "geri"),
            ("q", "çık"),
        ]
    } else if b.results.is_some() {
        &[
            ("↑↓", "gez"),
            ("Space", "sepete"),
            ("t", "tümü"),
            ("f", "yaş filtresi"),
            ("x", "çöpe taşı"),
            ("S", "sepet"),
            ("Enter", "aç / konuma git"),
            ("/", "ara"),
            ("m", "raporlar"),
            ("Esc", "geri"),
            ("q", "çık"),
        ]
    } else if b.view == View::Map {
        &[
            ("←↑↓→", "blok seç"),
            ("Enter", "gir"),
            ("⌫", "geri"),
            ("c", "renk"),
            ("t/Esc", "liste"),
            ("Space", "sepete"),
            ("x", "çöpe taşı"),
            ("S", "sepet"),
            ("m", "raporlar"),
            ("q", "çık"),
        ]
    } else {
        &[
            ("↑↓", "gez"),
            ("Enter", "gir"),
            ("⌫", "geri"),
            ("t", "harita"),
            ("/", "ara"),
            ("m", "raporlar"),
            ("i", "özet"),
            ("s", "sırala"),
            ("a", "görünen/diskte"),
            ("Space", "sepete"),
            ("x", "çöpe taşı"),
            ("S", "sepet"),
            ("r", "yeniden tara"),
            ("d", "diskler"),
            ("q", "çık"),
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
}

fn render_snapshot_picker(
    f: &mut Frame,
    saved: &[crate::history::Saved],
    selected: usize,
    total_now: u64,
    area: Rect,
) {
    let width = area.width.saturating_sub(4).min(84);
    let now = now_secs();
    let mut lines = vec![Line::from("")];
    for (i, s) in saved.iter().enumerate() {
        let h = &s.header;
        let style = if i == selected {
            HIGHLIGHT
        } else {
            Style::new()
        };
        lines.push(
            Line::from(vec![
                Span::raw(if i == selected { "▶ " } else { "  " }),
                Span::raw(fmt_date(h.time.min(u64::from(u32::MAX)) as u32))
                    .white()
                    .bold(),
                Span::raw(format!("  {:<16}", fmt_ago(now.saturating_sub(h.time)))).gray(),
                Span::raw(format!("toplam {:>10}", fmt_size(h.total.disk))),
                Span::raw(format!(
                    "   şimdiye göre {}",
                    fmt_delta(total_now, h.total.disk)
                ))
                .yellow(),
            ])
            .style(style),
        );
    }
    lines.push(Line::from(""));
    lines.push(Line::from(" Karşılaştırma bulunduğunuz klasörün altında yapılır.").gray());
    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(
                    Span::raw(" Hangi taramayla karşılaştırılsın? ")
                        .white()
                        .bold(),
                )
                .border_style(Style::new().fg(Color::Cyan)),
        ),
        popup,
    );
}

fn render_report_menu(f: &mut Frame, selected: usize, area: Rect) {
    let width = area.width.saturating_sub(4).min(84);
    let mut lines = Vec::new();
    for (i, item) in MenuItem::ALL.iter().enumerate() {
        if i == 0 || item.is_tool() != MenuItem::ALL[i - 1].is_tool() {
            let heading = if item.is_tool() {
                " Araçlar"
            } else {
                " Raporlar"
            };
            lines.push(Line::from(""));
            lines.push(Line::from(heading).cyan().bold());
        }
        let marker = if i == selected { "▶ " } else { "  " };
        let style = if i == selected {
            HIGHLIGHT
        } else {
            Style::new()
        };
        lines.push(
            Line::from(vec![
                Span::raw(marker),
                Span::raw(format!("{:>2}. ", i + 1)).gray(),
                Span::raw(item.label()).white().bold(),
            ])
            .style(style),
        );
    }
    lines.push(Line::from(""));
    lines.push(Line::from(format!(" {}", MenuItem::ALL[selected].description())).yellow());
    lines.push(
        Line::from(" Raporlar bulunduğunuz klasörün altında çalışır (uygulamalar: tüm tarama).")
            .gray(),
    );

    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(Span::raw(" Menü ").white().bold())
                .border_style(Style::new().fg(Color::Cyan)),
        ),
        popup,
    );
}

/// Terminal width from which the creation date column is shown.
const WIDE: u16 = 110;
const DATE_WIDTH: u16 = 16;

fn render_entries(f: &mut Frame, b: &mut Browser, area: Rect) {
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
                Style::new().fg(Color::Blue).bold(),
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
                Span::raw(format!(" {:>5.1}%", ratio * 100.0)),
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
    let mut header = vec!["", "Boyut", "Oran", "Dosya", "Son değişiklik"];
    if wide {
        widths.push(Constraint::Length(DATE_WIDTH));
        header.push("Oluşturma");
    }
    widths.push(Constraint::Min(10));
    header.push("Ad");

    let table = Table::new(rows, widths)
        .header(Row::new(header).bold().underlined())
        .row_highlight_style(HIGHLIGHT)
        .highlight_symbol("▶ ")
        .block(table_block());
    f.render_stateful_widget(table, area, &mut b.table);
}

fn render_results(f: &mut Frame, tree: &Tree, r: &mut ResultList, checks: &[bool], area: Rect) {
    let now = now_secs();
    let wide = area.width >= WIDE;
    let detail_width = r
        .rows
        .iter()
        .map(|row| row.detail.chars().count())
        .max()
        .unwrap_or(0)
        .min(44) as u16;
    let has_detail = detail_width > 0;
    let rows = r.rows.iter().enumerate().map(|(i, row)| {
        let check = if checks[i] {
            Span::raw("[✓]").green().bold()
        } else {
            Span::raw("[ ]").gray()
        };
        // Groups show their member count and newest member; single entries
        // their file count (directories) and own dates.
        let (count, modified, created) = if row.group {
            let newest = row.nodes.iter().map(|&id| tree.node(id).modified).max();
            (
                format!("{} adet", fmt_count(row.nodes.len() as u64)),
                newest.unwrap_or(0),
                0,
            )
        } else {
            let n = tree.node(row.nodes[0]);
            let count = if n.is_dir {
                fmt_count(n.file_count.into())
            } else {
                String::new()
            };
            (count, n.modified, n.created)
        };
        let label_style = if row.group || tree.node(row.nodes[0]).is_dir {
            Style::new().fg(Color::Blue).bold()
        } else {
            Style::new()
        };
        let mut cells = vec![
            Cell::from(check),
            Cell::from(Line::from(fmt_size(row.size())).right_aligned()),
            Cell::from(Line::from(count).right_aligned().gray()),
            date_cell(modified, now),
        ];
        if wide {
            cells.push(date_cell(created, now));
        }
        cells.push(Cell::from(Span::styled(row.label.clone(), label_style)));
        if has_detail {
            cells.push(Cell::from(Span::raw(row.detail.clone()).gray()));
        }
        Row::new(cells)
    });

    let mut widths = vec![
        Constraint::Length(3),
        Constraint::Length(11),
        Constraint::Length(10),
        Constraint::Length(DATE_WIDTH),
    ];
    let mut header = vec!["", "Boyut", "Adet", "Son değişiklik"];
    if wide {
        widths.push(Constraint::Length(DATE_WIDTH));
        header.push("Oluşturma");
    }
    widths.push(Constraint::Min(10));
    header.push("Konum");
    if has_detail {
        widths.push(Constraint::Length(detail_width.max(7)));
        header.push("Ayrıntı");
    }

    let table = Table::new(rows, widths)
        .header(Row::new(header).bold().underlined())
        .row_highlight_style(HIGHLIGHT)
        .highlight_symbol("▶ ")
        .block(table_block());
    f.render_stateful_widget(table, area, &mut r.table);
}

/// Top/bottom rule with the date color legend on the top edge.
fn table_block() -> Block<'static> {
    let label = Style::new().fg(Color::White);
    let mut legend = vec![Span::styled(" Tarih: ", label.bold())];
    for (color, text) in AGE_COLORS.iter().zip(stats::AGE_LABELS).take(4) {
        legend.push(Span::styled("██", Style::new().fg(*color)));
        legend.push(Span::styled(format!(" {text}   "), label));
    }
    Block::new()
        .borders(Borders::TOP | Borders::BOTTOM)
        .title_top(Line::from(legend).right_aligned())
}

/// Colors of the age groups in `stats::AGE_LABELS`: fresh is green,
/// untouched for over a year is red.
const AGE_COLORS: [Color; 5] = [
    Color::Green,
    Color::Cyan,
    Color::Yellow,
    Color::Red,
    Color::Gray,
];

/// Color for a timestamp of the given age in seconds.
fn age_color(age: u64) -> Color {
    AGE_COLORS[stats::age_group(age)]
}

fn date_cell(secs: u32, now: u64) -> Cell<'static> {
    Cell::from(date_span(secs, now))
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Local date and time, e.g. "03.10.2026 14:22".
pub fn fmt_date(secs: u32) -> String {
    chrono::DateTime::from_timestamp(i64::from(secs), 0)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%d.%m.%Y %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "—".into())
}

/// Text color that stays readable on `bg`.
fn text_on(bg: Color) -> Color {
    match bg {
        Color::Blue | Color::Magenta | Color::Red | Color::DarkGray | Color::Black => Color::White,
        Color::Indexed(n) if indexed_luma(n) < 140.0 => Color::White,
        _ => Color::Black,
    }
}

/// Perceived brightness (0–255) of an xterm 256-color palette entry.
fn indexed_luma(n: u8) -> f64 {
    const LEVELS: [f64; 6] = [0.0, 95.0, 135.0, 175.0, 215.0, 255.0];
    let (r, g, b) = match n {
        16..=231 => {
            let i = usize::from(n - 16);
            (LEVELS[i / 36], LEVELS[(i / 6) % 6], LEVELS[i % 6])
        }
        232..=255 => {
            let v = 8.0 + 10.0 * f64::from(n - 232);
            (v, v, v)
        }
        _ => (128.0, 128.0, 128.0),
    };
    0.299 * r + 0.587 * g + 0.114 * b
}

/// Folder colors in the treemap: distinct mid-dark hues (xterm 256-color
/// palette, which Terminal.app supports, unlike 24-bit color). Files keep
/// the lighter type colors, so folders and files stay apart.
const DIR_COLORS: [Color; 12] = [
    Color::Indexed(25),  // blue
    Color::Indexed(130), // orange
    Color::Indexed(29),  // green
    Color::Indexed(90),  // magenta
    Color::Indexed(31),  // teal
    Color::Indexed(124), // red
    Color::Indexed(60),  // slate
    Color::Indexed(64),  // olive
    Color::Indexed(54),  // purple
    Color::Indexed(94),  // brown
    Color::Indexed(23),  // dark teal
    Color::Indexed(89),  // wine
];

/// Cuts `s` to at most `width` characters.
fn clip(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        s.to_string()
    } else if width <= 1 {
        s.chars().take(width).collect()
    } else {
        let mut out: String = s.chars().take(width - 1).collect();
        out.push('…');
        out
    }
}

fn render_map(f: &mut Frame, b: &mut Browser, area: Rect) {
    let [map_area, legend_area] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
    b.map_area = map_area;
    let blocks = b.map_blocks();
    let selected = b.selected_block();
    let (tree, mode) = (&b.tree, b.size_mode);
    let total = tree.node(b.current).size.get(mode).max(1);
    let now = now_secs();

    for (slot, rect) in &blocks {
        let (bg, name, size, in_basket) = match *slot {
            Slot::Item(i) => {
                let id = b.entries[i];
                let n = tree.node(id);
                let bg = match b.map_color {
                    MapColor::Age => age_color(now.saturating_sub(u64::from(n.modified))),
                    // Entries are in size order, so neighbours get different colors.
                    MapColor::Kind if n.is_dir => DIR_COLORS[i % DIR_COLORS.len()],
                    MapColor::Kind => CATEGORY_COLORS[stats::Category::of(tree.name(id)) as usize],
                };
                let mut name = tree.name(id).to_string();
                if n.is_dir {
                    name.push('/');
                }
                (bg, name, n.size.get(mode), b.basket.covers(tree, id))
            }
            Slot::Other { count } => {
                let shown: Vec<usize> = blocks
                    .iter()
                    .filter_map(|(s, _)| match s {
                        Slot::Item(i) => Some(*i),
                        _ => None,
                    })
                    .collect();
                let size: u64 = (0..b.entries.len())
                    .filter(|i| !shown.contains(i))
                    .map(|i| tree.node(b.entries[i]).size.get(mode))
                    .sum();
                (
                    Color::DarkGray,
                    format!("diğer ({count} küçük öğe — Enter: listede gör)"),
                    size,
                    false,
                )
            }
        };
        let fg = text_on(bg);
        let is_selected = selected == Some(*slot);
        let style = Style::new().bg(bg).fg(fg);
        let mut inner = *rect;
        if is_selected && rect.width >= 3 && rect.height >= 3 {
            f.render_widget(
                Block::bordered()
                    .border_type(ratatui::widgets::BorderType::Thick)
                    .border_style(Style::new().fg(Color::White).bg(bg).bold())
                    .style(style),
                *rect,
            );
            inner = rect.inner(ratatui::layout::Margin::new(1, 1));
        } else {
            f.render_widget(Block::new().style(style), *rect);
        }
        let width = usize::from(inner.width);
        let mark = if in_basket { "✓ " } else { "" };
        let mut lines = vec![Line::from(clip(&format!("{mark}{name}"), width)).bold()];
        if inner.height >= 2 {
            let pct = size as f64 / total as f64 * 100.0;
            lines.push(Line::from(clip(
                &format!("{} · %{pct:.1}", fmt_size(size)),
                width,
            )));
        }
        let text_style = if is_selected && inner == *rect {
            // Too small for a border: show the selection by inverting.
            Style::new().bg(Color::White).fg(Color::Black)
        } else {
            style
        };
        f.render_widget(Paragraph::new(lines).style(text_style), inner);
    }

    let mut legend = vec![
        Span::raw(" t / Esc ").black().on_gray(),
        Span::raw(" listeye dön   ").white(),
        Span::raw("Renk: ").white().bold(),
    ];
    match b.map_color {
        MapColor::Kind => {
            for color in DIR_COLORS.iter().take(4) {
                legend.push(Span::styled("█", Style::new().fg(*color)));
            }
            legend.push(Span::raw(" klasörler (her biri ayrı)  ").white());
            for (cat, color) in stats::Category::ALL.iter().zip(CATEGORY_COLORS) {
                legend.push(Span::styled("██", Style::new().fg(color)));
                legend.push(Span::raw(format!(" {}  ", cat.label())).white());
            }
            legend.push(Span::raw("  (c: yaşa göre)").gray());
        }
        MapColor::Age => {
            for (color, text) in AGE_COLORS.iter().zip(stats::AGE_LABELS).take(4) {
                legend.push(Span::styled("██", Style::new().fg(*color)));
                legend.push(Span::raw(format!(" {text}  ")).white());
            }
            legend.push(Span::raw("  (c: türe göre)").gray());
        }
    }
    f.render_widget(Line::from(legend), legend_area);
}

const CATEGORY_COLORS: [Color; 8] = [
    Color::Magenta,
    Color::LightMagenta,
    Color::LightBlue,
    Color::LightYellow,
    Color::LightGreen,
    Color::LightCyan,
    Color::LightRed,
    Color::Gray,
];

fn render_dashboard(
    f: &mut Frame,
    tree: &Tree,
    d: &mut Dashboard,
    mode: SizeMode,
    scan_errors: u64,
    area: Rect,
) {
    let s = &d.stats;
    let age_rows = if s.ages[stats::AGE_UNKNOWN].files > 0 {
        5
    } else {
        4
    };
    let mid_rows = s.categories.len().max(age_rows) as u16;
    let [top, mid, bottom] = Layout::vertical([
        Constraint::Length(6),
        Constraint::Length(mid_rows + 2),
        Constraint::Min(5),
    ])
    .areas(area);
    let [general, disk_area] = halves(top);
    let [types, ages] = halves(mid);
    let [files, dirs] = halves(bottom);

    // General
    let now = now_secs();
    let base = tree.node(d.base);
    let mut lines = vec![
        Line::from(vec![
            Span::raw("Toplam boyut   ").white(),
            Span::raw(fmt_size(s.size)).bold(),
        ]),
        Line::from(vec![
            Span::raw("İçerik         ").white(),
            Span::raw(format!(
                "{} dosya · {} klasör",
                fmt_count(s.files),
                fmt_count(s.dirs)
            )),
        ]),
        Line::from(vec![
            Span::raw("En yeni değişiklik  ").white(),
            date_span(base.modified, now),
        ]),
    ];
    if scan_errors > 0 {
        lines.push(
            Line::from(format!(
                "⚠ Taramada {} öğeye erişilemedi",
                fmt_count(scan_errors)
            ))
            .yellow(),
        );
    }
    f.render_widget(
        Paragraph::new(lines).block(panel(" Genel ", false)),
        general,
    );

    // Disk
    let disk_lines = if let Some(disk) = &d.disk {
        let ratio = disk.usage_ratio();
        let width = (disk_area.width as usize).saturating_sub(2 + 6).max(4);
        let share = if disk.total > 0 {
            s.size as f64 / disk.total as f64 * 100.0
        } else {
            0.0
        };
        vec![
            Line::from(vec![
                Span::styled(bar(ratio, width), Style::new().fg(usage_color(ratio))),
                Span::raw(format!(" %{:.0}", ratio * 100.0)).bold(),
            ]),
            Line::from(format!(
                "{} / {} kullanılıyor · {} boş",
                fmt_size(disk.used()),
                fmt_size(disk.total),
                fmt_size(disk.available)
            )),
            Line::from(vec![
                Span::raw("Disk payı  ").white(),
                Span::raw(if share > 0.0 && share < 0.1 {
                    "< %0.1".to_string()
                } else {
                    format!("%{share:.1}")
                })
                .bold(),
            ]),
        ]
    } else {
        vec![Line::from("Disk bilgisi bulunamadı.").gray()]
    };
    let disk_title = d
        .disk
        .as_ref()
        .map_or(" Disk ".to_string(), |x| format!(" Disk: {} ", x.name));
    f.render_widget(
        Paragraph::new(disk_lines).block(panel(&disk_title, false)),
        disk_area,
    );

    // File types
    let inner = types.width.saturating_sub(2) as usize;
    let bar_w = inner.saturating_sub(20 + 11 + 6 + 2).max(4);
    let total = s.size.max(1) as f64;
    let lines: Vec<Line> = s
        .categories
        .iter()
        .map(|&(cat, b)| {
            let ratio = b.size as f64 / total;
            Line::from(vec![
                Span::raw(format!("{:<19}", cat.label())).white(),
                Span::styled(
                    bar(ratio, bar_w),
                    Style::new().fg(CATEGORY_COLORS[cat as usize]),
                ),
                Span::raw(format!(" {:>10}", fmt_size(b.size))),
                Span::raw(format!(" {:>4.0}%", ratio * 100.0)).gray(),
            ])
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).block(panel(" Dosya türleri ", false)),
        types,
    );

    // Ages
    let inner = ages.width.saturating_sub(2) as usize;
    let bar_w = inner.saturating_sub(11 + 11 + 6 + 2).max(4);
    let lines: Vec<Line> = (0..age_rows)
        .map(|i| {
            let b = s.ages[i];
            let ratio = b.size as f64 / total;
            Line::from(vec![
                Span::raw(format!("{:<11}", stats::AGE_LABELS[i])).white(),
                Span::styled(bar(ratio, bar_w), Style::new().fg(AGE_COLORS[i])),
                Span::raw(format!(" {:>10}", fmt_size(b.size))),
                Span::raw(format!(" {:>4.0}%", ratio * 100.0)).gray(),
            ])
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).block(panel(" Yaş (son değişiklik) ", false)),
        ages,
    );

    // Largest files and fullest directories
    let base_path = tree.path_of(d.base);
    let rel = |id: NodeId| {
        if id == d.base {
            return "(bu klasör)".to_string();
        }
        let path = tree.path_of(id);
        path.strip_prefix(&base_path)
            .unwrap_or(&path)
            .display()
            .to_string()
    };

    let focus_files = d.focus == Pane::Files;
    let rows = s.top_files.iter().map(|&id| {
        let n = tree.node(id);
        Row::new(vec![
            Cell::from(Line::from(fmt_size(n.size.get(mode))).right_aligned()),
            date_cell(n.modified, now),
            Cell::from(rel(id)),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(DATE_WIDTH),
            Constraint::Min(10),
        ],
    )
    .block(panel(" En büyük dosyalar ", focus_files));
    let table = if focus_files {
        table.row_highlight_style(HIGHLIGHT).highlight_symbol("▶ ")
    } else {
        table
    };
    f.render_stateful_widget(table, files, &mut d.files);

    let focus_dirs = d.focus == Pane::Dirs;
    let rows = s.top_dirs.iter().map(|&(id, size)| {
        Row::new(vec![
            Cell::from(Line::from(fmt_size(size)).right_aligned()),
            Cell::from(Span::styled(
                if id == d.base {
                    rel(id)
                } else {
                    format!("{}/", rel(id))
                },
                Style::new().fg(Color::Blue).bold(),
            )),
        ])
    });
    let table = Table::new(rows, [Constraint::Length(10), Constraint::Min(10)])
        .block(panel(" En dolu klasörler (doğrudan içerik) ", focus_dirs));
    let table = if focus_dirs {
        table.row_highlight_style(HIGHLIGHT).highlight_symbol("▶ ")
    } else {
        table
    };
    f.render_stateful_widget(table, dirs, &mut d.dirs);
}

fn risk_style(risk: Risk) -> (Style, &'static str) {
    match risk {
        Risk::Safe => (Style::new().fg(Color::Black).bg(Color::Green), " güvenli "),
        Risk::Redownload => (
            Style::new().fg(Color::Black).bg(Color::Yellow),
            " yeniden indirilir ",
        ),
        Risk::DataLoss => (
            Style::new().fg(Color::White).bg(Color::Red).bold(),
            " VERİ KAYBI ",
        ),
    }
}

fn render_tools(f: &mut Frame, view: &mut ToolsView, tick: usize, area: Rect) {
    let action_rows: u16 = if view.run.is_some() {
        area.height / 2
    } else {
        view.selected().map_or(1, |t| {
            t.actions
                .iter()
                .map(|a| 1 + a.steps.len() as u16)
                .sum::<u16>()
                .max(1)
        })
    };
    let [list_area, detail_area] = Layout::vertical([
        Constraint::Min(6),
        Constraint::Length((action_rows + 2).min(area.height / 2)),
    ])
    .areas(area);

    let spin = SPINNER[tick % SPINNER.len()];
    let rows = view.tools.iter().map(|t| {
        let (size, info) = match &t.status {
            ToolStatus::Measuring => (
                Line::from(format!("{spin}")).cyan(),
                Span::raw("ölçülüyor…").cyan(),
            ),
            ToolStatus::Missing(why) => (Line::from("—").gray(), Span::raw(why.clone()).gray()),
            ToolStatus::Unavailable(why) => {
                (Line::from("—").yellow(), Span::raw(why.clone()).yellow())
            }
            ToolStatus::Ready {
                reclaimable,
                detail,
            } => (
                Line::from(fmt_size(*reclaimable)).right_aligned().bold(),
                Span::raw(detail.clone()),
            ),
        };
        let name_style = match t.status {
            ToolStatus::Missing(_) => Style::new().fg(Color::Gray),
            _ => Style::new().fg(Color::White).bold(),
        };
        Row::new(vec![
            Cell::from(Span::styled(t.kind.label(), name_style)),
            Cell::from(size),
            Cell::from(info),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(34),
            Constraint::Length(12),
            Constraint::Min(10),
        ],
    )
    .header(
        Row::new(["Araç", "Kazanılır", "Ayrıntı"])
            .bold()
            .underlined(),
    )
    .row_highlight_style(HIGHLIGHT)
    .highlight_symbol("▶ ")
    .block(table_block_plain());
    f.render_stateful_widget(table, list_area, &mut view.table);

    let mut lines = Vec::new();
    match view.selected() {
        Some(t) if !t.actions.is_empty() => {
            for a in &t.actions {
                let (style, text) = risk_style(a.risk);
                lines.push(Line::from(vec![
                    Span::styled(text, style),
                    Span::raw(format!(" {}", a.label)).white().bold(),
                ]));
                for step in &a.steps {
                    lines.push(Line::from(format!("    $ {}", step.describe())).gray());
                }
            }
        }
        Some(t) if t.status == ToolStatus::Measuring => lines.push(Line::from("Ölçülüyor…").cyan()),
        _ => lines.push(Line::from("Bu araç için yapılacak bir şey yok.").gray()),
    }
    if let Some(run) = &view.run {
        // Show the end of the log.
        let visible = detail_area.height.saturating_sub(2) as usize;
        let start = run.log.len().saturating_sub(visible);
        let lines: Vec<Line> = run.log[start..]
            .iter()
            .map(|l| {
                if l.starts_with('$') {
                    Line::from(l.clone()).white().bold()
                } else if l.contains('✗') {
                    Line::from(l.clone()).red()
                } else if l.contains('✓') {
                    Line::from(l.clone()).green()
                } else {
                    Line::from(l.clone()).gray()
                }
            })
            .collect();
        let title = if !run.finished {
            format!(" {} Çalışıyor… ", SPINNER[tick % SPINNER.len()])
        } else if run.failures > 0 {
            format!(" Bitti — {} adım başarısız ", run.failures)
        } else {
            " Bitti ".to_string()
        };
        f.render_widget(
            Paragraph::new(lines).block(panel(&title, true)),
            detail_area,
        );
    } else {
        f.render_widget(
            Paragraph::new(lines).block(panel(" İşlemler ", false)),
            detail_area,
        );
    }

    if let Some(p) = &view.picker {
        let tool = &view.tools[p.tool];
        let mut lines = vec![Line::from("")];
        for (i, a) in tool.actions.iter().enumerate() {
            let (style, text) = risk_style(a.risk);
            let check = if p.checked[i] { "[✓] " } else { "[ ] " };
            let row = Line::from(vec![
                Span::raw(if i == p.cursor { "▶ " } else { "  " }),
                Span::raw(check).bold(),
                Span::styled(text, style),
                Span::raw(format!(" {}", a.label)).white(),
            ]);
            lines.push(if i == p.cursor {
                row.style(HIGHLIGHT)
            } else {
                row
            });
        }
        lines.push(Line::from(""));
        lines.push(Line::from(" Space: seç   Enter: devam   Esc: vazgeç").gray());
        popup(
            f,
            &format!(" {} — ne temizlensin? ", tool.kind.label()),
            Color::Cyan,
            lines,
            90,
        );
    }

    if let Some(c) = &view.confirm {
        let tool = &view.tools[c.tool];
        let mut lines = vec![
            Line::from(""),
            Line::from("Şu komutlar sırayla çalışacak:").white(),
        ];
        for a in &c.actions {
            let (style, text) = risk_style(a.risk);
            lines.push(Line::from(vec![
                Span::styled(text, style),
                Span::raw(format!(" {}", a.label)).white().bold(),
            ]));
            for step in &a.steps {
                lines.push(Line::from(format!("    $ {}", step.describe())).gray());
            }
        }
        lines.push(Line::from(""));
        match &c.typed {
            Some(typed) => {
                lines.push(
                    Line::from("Seçimde VERİ KAYBI olan bir işlem var. Onaylamak için „evet” yazıp Enter'a basın:")
                        .red()
                        .bold(),
                );
                let ok = typed.trim() == CONFIRM_WORD;
                lines.push(Line::from(vec![
                    Span::raw(" > "),
                    Span::raw(format!("{typed}█")).white().bold(),
                    Span::raw(if ok { "   Enter: çalıştır" } else { "" }).green(),
                ]));
                lines.push(Line::from(" Esc: vazgeç").gray());
            }
            None => lines.push(Line::from(vec![
                Span::raw(" e ").black().on_red().bold(),
                Span::raw(" evet, çalıştır     "),
                Span::raw(" h ").black().on_gray(),
                Span::raw(" vazgeç"),
            ])),
        }
        popup(
            f,
            &format!(" {} — onay ", tool.kind.label()),
            Color::Red,
            lines,
            100,
        );
    }
}

/// A centered bordered box sized to its lines.
fn popup(f: &mut Frame, title: &str, color: Color, lines: Vec<Line>, max_width: u16) {
    let area = f.area();
    let width = area.width.saturating_sub(4).min(max_width);
    let height = (lines.len() as u16 + 2).min(area.height);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .title(Span::raw(title.to_string()).white().bold())
                .border_style(Style::new().fg(color)),
        ),
        rect,
    );
}

fn table_block_plain() -> Block<'static> {
    Block::new().borders(Borders::TOP | Borders::BOTTOM)
}

fn render_system(f: &mut Frame, b: &Browser, sys: &SystemInfo, area: Rect) {
    let vol_rows = sys.main.as_ref().map_or(1, |c| c.volumes.len()) as u16;
    let [top, mid, bottom] = Layout::vertical([
        Constraint::Length(vol_rows + 4),
        Constraint::Length(9),
        Constraint::Min(3),
    ])
    .areas(area);
    let [gap_area, other_area] = halves(mid);
    let line = |label: &str, value: String| {
        Line::from(vec![
            Span::raw(format!("{label:<30}")).white(),
            Span::raw(value).bold(),
        ])
    };

    // APFS container and its volumes
    let mut lines = Vec::new();
    let title = match &sys.main {
        Some(c) => {
            let used = c.total.saturating_sub(c.free);
            let ratio = used as f64 / c.total.max(1) as f64;
            let bar_w = (top.width as usize).saturating_sub(60).max(10);
            lines.push(Line::from(vec![
                Span::raw(format!(
                    "Toplam {}  ·  kullanılan {}  ·  boş {}   ",
                    fmt_size(c.total),
                    fmt_size(used),
                    fmt_size(c.free)
                )),
                Span::styled(bar(ratio, bar_w), Style::new().fg(usage_color(ratio))),
                Span::raw(format!(" %{:.0}", ratio * 100.0)).bold(),
            ]));
            lines.push(Line::from(""));
            let mut vols = c.volumes.clone();
            vols.sort_by_key(|v| std::cmp::Reverse(v.used));
            for v in vols {
                let ratio = v.used as f64 / c.total.max(1) as f64;
                lines.push(Line::from(vec![
                    Span::raw(format!("{:<26}", v.name)).white().bold(),
                    Span::raw(format!("{:<38}", system::role_label(&v.role))).gray(),
                    Span::raw(format!("{:>11}  ", fmt_size(v.used))),
                    Span::styled(bar(ratio, 20), Style::new().fg(Color::Cyan)),
                ]));
            }
            format!(" APFS kapsayıcısı ({}) ", c.reference)
        }
        None => {
            lines.push(Line::from("APFS bilgisi alınamadı.").yellow());
            " APFS kapsayıcısı ".to_string()
        }
    };
    f.render_widget(Paragraph::new(lines).block(panel(&title, false)), top);

    // What the scan cannot see
    let scanned = b.tree.node(crate::tree::ROOT).size.disk;
    let mut lines = Vec::new();
    match sys.scannable_used() {
        Some(expected) if b.tree.root_path() == std::path::Path::new("/") => {
            lines.push(line("Sistem + Veri bölümleri", fmt_size(expected)));
            lines.push(line("Taramanın bulduğu", fmt_size(scanned)));
            if expected >= scanned {
                lines.push(line("Taramanın göremediği", fmt_size(expected - scanned)).yellow());
                lines.push(Line::from("Olası nedenler:").gray());
                if b.errors > 0 {
                    lines.push(Line::from(format!(
                        " • erişilemeyen {} öğe — terminale Tam Disk Erişimi verin",
                        fmt_count(b.errors)
                    )));
                }
                if !sys.snapshots.is_empty() {
                    lines.push(Line::from(format!(
                        " • {} Time Machine yerel anlık görüntüsü",
                        sys.snapshots.len()
                    )));
                }
                lines.push(Line::from(
                    " • silinebilir (purgeable) alan ve dosya sistemi meta verisi",
                ));
            } else {
                lines.push(line("Fazla sayılan", fmt_size(scanned - expected)));
                lines.push(
                    Line::from(
                        "APFS klonları ve kopyalar blok paylaşsa da tarama onları ayrı sayar.",
                    )
                    .gray(),
                );
            }
        }
        _ => {
            lines.push(Line::from(
                "Bu karşılaştırma için diskin kökünü (/) tarayın:",
            ));
            lines.push(Line::from("d → Macintosh HD").cyan());
        }
    }
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel(" Taramanın göremediği ", false)),
        gap_area,
    );

    // Snapshots and memory files
    let mut lines = vec![line(
        "Time Machine anlık görüntüsü",
        format!("{} adet", sys.snapshots.len()),
    )];
    for name in sys.snapshots.iter().take(2) {
        lines.push(Line::from(format!("  {name}")).gray());
    }
    if let Some(first) = sys.snapshots.first() {
        let date = first
            .trim_start_matches("com.apple.TimeMachine.")
            .trim_end_matches(".local");
        lines.push(
            Line::from(format!(
                "  silmek için: sudo tmutil deletelocalsnapshots {date}"
            ))
            .cyan(),
        );
    }
    if let Some((total, used)) = sys.swap {
        lines.push(line(
            "Takas (swap)",
            format!("{} / {}", fmt_size(used), fmt_size(total)),
        ));
    }
    if let Some(sleep) = sys.sleepimage {
        lines.push(line("Uyku görüntüsü (sleepimage)", fmt_size(sleep)));
    }
    f.render_widget(
        Paragraph::new(lines).block(panel(" Anlık görüntüler ve bellek ", false)),
        other_area,
    );

    // Simulator runtimes and problems
    let mut lines = Vec::new();
    if !sys.simulators.is_empty() {
        let total: u64 = sys.simulators.iter().map(|c| c.used()).sum();
        lines.push(line(
            "Simülatör çalışma zamanları",
            format!("{} imaj, {}", sys.simulators.len(), fmt_size(total)),
        ));
        lines.push(
            Line::from("  Ayrı disk imajlarında durur, taramada görünmez. Temizlik: m → Geliştirici araçları temizliği")
                .gray(),
        );
    }
    for p in &sys.problems {
        lines.push(Line::from(format!("⚠ {p}")).yellow());
    }
    f.render_widget(Paragraph::new(lines).block(panel(" Diğer ", false)), bottom);
}

fn halves(area: Rect) -> [Rect; 2] {
    Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area)
}

/// Bordered panel; the focused one gets a highlighted border.
fn panel(title: &str, focused: bool) -> Block<'static> {
    let border = if focused {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    Block::bordered()
        .title(Span::raw(title.to_string()).white().bold())
        .border_style(border)
}

fn usage_color(ratio: f64) -> Color {
    match ratio {
        r if r >= 0.9 => Color::Red,
        r if r >= 0.75 => Color::Yellow,
        _ => Color::Green,
    }
}

fn date_span(secs: u32, now: u64) -> Span<'static> {
    if secs == 0 {
        return Span::raw("—").gray();
    }
    let age = now.saturating_sub(u64::from(secs));
    Span::styled(fmt_date(secs), Style::new().fg(age_color(age)))
}

fn render_confirm(f: &mut Frame, tree: &Tree, ids: &[NodeId], mode: SizeMode, area: Rect) {
    const LISTED: usize = 6;
    let width = area.width.saturating_sub(4).min(76);
    let inner_width = width.saturating_sub(4) as usize;
    let size: u64 = ids.iter().map(|&id| tree.node(id).size.get(mode)).sum();
    let files: u64 = ids
        .iter()
        .map(|&id| u64::from(tree.node(id).file_count))
        .sum();

    let mut lines = vec![Line::from("")];
    if let [id] = ids {
        lines.push(Line::from(truncate_path(&tree.path_of(*id), inner_width)).bold());
    } else {
        lines.push(Line::from(format!("{} öğe", fmt_count(ids.len() as u64))).bold());
        lines.push(Line::from(""));
        for &id in ids.iter().take(LISTED) {
            lines.push(Line::from(truncate_path(&tree.path_of(id), inner_width)).gray());
        }
        if ids.len() > LISTED {
            lines.push(
                Line::from(format!(
                    "… ve {} öğe daha",
                    fmt_count((ids.len() - LISTED) as u64)
                ))
                .gray(),
            );
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(format!("Toplam boyut: {}", fmt_size(size))));
    if files > 0 {
        lines.push(Line::from(format!("İçerdiği dosya: {}", fmt_count(files))));
    }
    lines.extend([
        Line::from(""),
        Line::from("Çöp kutusuna taşınacak.").gray(),
        Line::from(""),
        Line::from(vec![
            Span::raw(" e ").black().on_red().bold(),
            Span::raw(" evet, taşı     "),
            Span::raw(" h ").black().on_gray(),
            Span::raw(" vazgeç"),
        ]),
    ]);

    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .centered()
            .block(
                Block::bordered()
                    .title(" Çöp kutusuna taşınsın mı? ")
                    .border_style(Style::new().fg(Color::Red)),
            ),
        popup,
    );
}

/// Horizontal bar of `width` cells using eighth-block characters.
fn bar(ratio: f64, width: usize) -> String {
    const PARTS: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
    let eighths = (ratio.clamp(0.0, 1.0) * width as f64 * 8.0).round() as usize;
    let full = eighths / 8;
    let mut s = "█".repeat(full);
    if full < width {
        s.push(PARTS[eighths % 8]);
        s.push_str(&" ".repeat(width - full - 1));
    }
    s
}

/// Signed size difference: "+1.2 GiB", "−300.0 MiB", "değişmedi".
pub fn fmt_delta(now: u64, then: u64) -> String {
    match now.cmp(&then) {
        std::cmp::Ordering::Greater => format!("+{}", fmt_size(now - then)),
        std::cmp::Ordering::Less => format!("−{}", fmt_size(then - now)),
        std::cmp::Ordering::Equal => "değişmedi".into(),
    }
}

/// "az önce", "5 dakika önce", "3 saat önce", "2 gün önce".
pub fn fmt_ago(secs: u64) -> String {
    match secs {
        s if s < 60 => "az önce".into(),
        s if s < 3600 => format!("{} dakika önce", s / 60),
        s if s < 86_400 => format!("{} saat önce", s / 3600),
        s => format!("{} gün önce", s / 86_400),
    }
}

/// Human readable size, 1024-based.
pub fn fmt_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Integer with thousands separators: 1234567 -> "1.234.567".
fn fmt_count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

fn truncate_path(path: &Path, max: usize) -> String {
    let s = path.display().to_string();
    let len = s.chars().count();
    if len <= max || max < 2 {
        return s;
    }
    let tail: String = s.chars().skip(len - (max - 1)).collect();
    format!("…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::DAY;

    #[test]
    fn formats_sizes() {
        assert_eq!(fmt_size(0), "0 B");
        assert_eq!(fmt_size(1023), "1023 B");
        assert_eq!(fmt_size(1536), "1.5 KiB");
        assert_eq!(fmt_size(5 * 1024 * 1024 * 1024), "5.0 GiB");
    }

    #[test]
    fn formats_deltas_and_ages() {
        assert_eq!(fmt_delta(3072, 1024), "+2.0 KiB");
        assert_eq!(fmt_delta(1024, 3072), "−2.0 KiB");
        assert_eq!(fmt_delta(5, 5), "değişmedi");
        assert_eq!(fmt_ago(30), "az önce");
        assert_eq!(fmt_ago(600), "10 dakika önce");
        assert_eq!(fmt_ago(7200), "2 saat önce");
        assert_eq!(fmt_ago(3 * 86_400 + 5), "3 gün önce");
    }

    #[test]
    fn formats_counts() {
        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(999), "999");
        assert_eq!(fmt_count(1000), "1.000");
        assert_eq!(fmt_count(1234567), "1.234.567");
    }

    #[test]
    fn colors_by_age() {
        assert_eq!(age_color(0), Color::Green);
        assert_eq!(age_color(7 * DAY), Color::Green);
        assert_eq!(age_color(8 * DAY), Color::Cyan);
        assert_eq!(age_color(100 * DAY), Color::Yellow);
        assert_eq!(age_color(400 * DAY), Color::Red);
    }

    #[test]
    fn formats_dates() {
        // 2001-09-09 01:46 UTC: 8 or 9 September depending on the time zone.
        let d = fmt_date(1_000_000_000);
        assert_eq!(d.chars().count(), 16, "{d}");
        assert!(d.contains(".09.2001 "), "{d}");
    }

    #[test]
    fn folder_colors_get_readable_text() {
        // Dark palette entries get white text, light ones black.
        for c in DIR_COLORS {
            assert_eq!(text_on(c), Color::White, "{c:?}");
        }
        assert_eq!(text_on(Color::Indexed(231)), Color::Black); // white
        assert_eq!(text_on(Color::Indexed(16)), Color::White); // black
        assert_eq!(text_on(Color::LightYellow), Color::Black);
    }

    #[test]
    fn bar_has_fixed_width() {
        for r in [0.0, 0.03, 0.5, 0.999, 1.0] {
            assert_eq!(bar(r, 10).chars().count(), 10);
        }
    }
}
