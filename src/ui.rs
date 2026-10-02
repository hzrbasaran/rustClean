//! Rendering of the three screens.

use std::path::Path;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::app::{App, Browser, Dashboard, Pane, Screen};
use crate::lists::ResultList;
use crate::reports::ReportKind;
use crate::stats;
use crate::tree::{NodeId, SizeMode, Tree};

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
        Span::raw(format!("  {label:<13}")).dark_gray(),
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

    let heading = if let Some(d) = &b.dashboard {
        format!(
            "Özet — {}  │  {mode_label}",
            b.tree.path_of(d.base).display()
        )
    } else if let Some(r) = &b.results {
        let shown = if r.truncated { "ilk " } else { "" };
        format!(
            "{} — {}  │  {shown}{} satır, {} ({mode_label})  │  seçili: {} ({})",
            r.title,
            b.tree.path_of(r.base).display(),
            fmt_count(r.rows.len() as u64),
            fmt_size(r.total_size()),
            fmt_count(r.checked.iter().filter(|&&c| c).count() as u64),
            fmt_size(r.checked_size()),
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
    f.render_widget(title(heading), header);

    let [table_area, status_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(body);

    if let Some(d) = &mut b.dashboard {
        render_dashboard(f, &b.tree, d, mode, b.errors, table_area);
    } else if let Some(r) = &mut b.results {
        render_results(f, &b.tree, r, table_area);
    } else {
        render_entries(f, b, table_area);
    }

    let status = if let Some(input) = &b.input {
        Line::from(vec![
            Span::raw(" Ara: ").black().on_yellow().bold(),
            Span::raw(format!(" {input}")),
            Span::raw("█").slow_blink(),
            Span::raw("   örn: deneme · deneme* · *.log").dark_gray(),
        ])
    } else if let Some(st) = &b.status {
        let style = if st.error {
            Style::new().fg(Color::Red)
        } else {
            Style::new().fg(Color::Green)
        };
        Line::from(Span::styled(st.text.clone(), style))
    } else if b.dashboard.is_some() {
        Line::from(
            "„En dolu klasörler” alt klasörleri saymaz: yerin asıl durduğu klasörleri gösterir.",
        )
        .dark_gray()
    } else if let Some(r) = &b.results {
        let text = if r.rows.is_empty() {
            "Sonuç yok."
        } else {
            r.note.as_str()
        };
        Line::from(text).dark_gray()
    } else {
        let mut spans = vec![Span::raw(format!(
            "{} öğe tarandı, {:.1} sn",
            fmt_count(b.tree.len() as u64),
            b.elapsed.as_secs_f64()
        ))
        .dark_gray()];
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
            spans.push(Span::raw("   (klasör boş)").dark_gray());
        }
        Line::from(spans)
    };
    f.render_widget(status, status_area);

    let footer_keys: &[(&str, &str)] = if b.report_menu.is_some() {
        &[("↑↓", "seç"), ("Enter", "çalıştır"), ("Esc", "kapat")]
    } else if b.confirm.is_some() {
        &[("e", "evet, çöpe taşı"), ("h / Esc", "vazgeç")]
    } else if b.deleting.is_some() {
        &[("q", "çık")]
    } else if b.input.is_some() {
        &[("Enter", "ara"), ("Esc", "vazgeç")]
    } else if b.dashboard.is_some() {
        &[
            ("Tab", "liste değiştir"),
            ("↑↓", "gez"),
            ("Enter", "konuma git"),
            ("x", "çöpe taşı"),
            ("a", "görünen/diskte"),
            ("Esc", "geri"),
            ("q", "çık"),
        ]
    } else if b.results.is_some() {
        &[
            ("↑↓", "gez"),
            ("Space", "seç"),
            ("t", "tümü"),
            ("x", "seçilileri çöpe taşı"),
            ("Enter", "aç / konuma git"),
            ("/", "ara"),
            ("m", "raporlar"),
            ("Esc", "geri"),
            ("q", "çık"),
        ]
    } else {
        &[
            ("↑↓", "gez"),
            ("Enter", "gir"),
            ("⌫", "geri"),
            ("/", "ara"),
            ("m", "raporlar"),
            ("i", "özet"),
            ("s", "sırala"),
            ("a", "görünen/diskte"),
            ("x", "çöpe taşı"),
            ("r", "yeniden tara"),
            ("d", "diskler"),
            ("q", "çık"),
        ]
    };
    f.render_widget(keys(footer_keys), footer);

    if let Some(sel) = b.report_menu {
        render_report_menu(f, sel, f.area());
    }
    if let Some(ids) = &b.confirm {
        render_confirm(f, &b.tree, ids, mode, f.area());
    }
}

fn render_report_menu(f: &mut Frame, selected: usize, area: Rect) {
    let width = area.width.saturating_sub(4).min(84);
    let mut lines = vec![Line::from("")];
    for (i, kind) in ReportKind::ALL.iter().enumerate() {
        let marker = if i == selected { "▶ " } else { "  " };
        let style = if i == selected {
            HIGHLIGHT
        } else {
            Style::new()
        };
        lines.push(
            Line::from(vec![
                Span::raw(marker),
                Span::raw(format!("{}. ", i + 1)).dark_gray(),
                Span::raw(kind.label()).white().bold(),
            ])
            .style(style),
        );
        lines.push(Line::from(format!("       {}", kind.description())).dark_gray());
    }
    lines.push(Line::from(""));
    lines.push(Line::from(" Raporlar bulunduğunuz klasörün altında çalışır.").dark_gray());

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
                .title(Span::raw(" Raporlar ").white().bold())
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
        let mut cells = vec![
            Cell::from(Line::from(fmt_size(size)).right_aligned()),
            Cell::from(Line::from(vec![
                Span::styled(bar(ratio, 12), Style::new().fg(Color::Cyan)),
                Span::raw(format!(" {:>5.1}%", ratio * 100.0)),
            ])),
            Cell::from(Line::from(count).right_aligned().dark_gray()),
            date_cell(n.modified, now),
        ];
        if wide {
            cells.push(date_cell(n.created, now));
        }
        cells.push(Cell::from(name));
        Row::new(cells)
    });

    let mut widths = vec![
        Constraint::Length(11),
        Constraint::Length(19),
        Constraint::Length(10),
        Constraint::Length(DATE_WIDTH),
    ];
    let mut header = vec!["Boyut", "Oran", "Dosya", "Son değişiklik"];
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

fn render_results(f: &mut Frame, tree: &Tree, r: &mut ResultList, area: Rect) {
    let now = now_secs();
    let wide = area.width >= WIDE;
    let has_detail = r.rows.iter().any(|row| !row.detail.is_empty());
    let rows = r.rows.iter().enumerate().map(|(i, row)| {
        let check = if r.checked[i] {
            Span::raw("[✓]").green().bold()
        } else {
            Span::raw("[ ]").dark_gray()
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
            Cell::from(Line::from(count).right_aligned().dark_gray()),
            date_cell(modified, now),
        ];
        if wide {
            cells.push(date_cell(created, now));
        }
        cells.push(Cell::from(Span::styled(row.label.clone(), label_style)));
        if has_detail {
            cells.push(Cell::from(Span::raw(row.detail.clone()).dark_gray()));
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
        widths.push(Constraint::Length(30));
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
    Color::DarkGray,
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
fn fmt_date(secs: u32) -> String {
    chrono::DateTime::from_timestamp(i64::from(secs), 0)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%d.%m.%Y %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "—".into())
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
        vec![Line::from("Disk bilgisi bulunamadı.").dark_gray()]
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
                Span::raw(format!(" {:>4.0}%", ratio * 100.0)).dark_gray(),
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
                Span::raw(format!(" {:>4.0}%", ratio * 100.0)).dark_gray(),
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
        return Span::raw("—").dark_gray();
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
            lines.push(Line::from(truncate_path(&tree.path_of(id), inner_width)).dark_gray());
        }
        if ids.len() > LISTED {
            lines.push(
                Line::from(format!(
                    "… ve {} öğe daha",
                    fmt_count((ids.len() - LISTED) as u64)
                ))
                .dark_gray(),
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
        Line::from("Çöp kutusuna taşınacak.").dark_gray(),
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
    fn bar_has_fixed_width() {
        for r in [0.0, 0.03, 0.5, 0.999, 1.0] {
            assert_eq!(bar(r, 10).chars().count(), 10);
        }
    }
}
