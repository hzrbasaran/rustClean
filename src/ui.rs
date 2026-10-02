//! Rendering of the three screens.

use std::path::Path;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::app::{App, Browser, Screen, SearchResults};
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
        let color = match ratio {
            r if r >= 0.9 => Color::Red,
            r if r >= 0.75 => Color::Yellow,
            _ => Color::Green,
        };
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

    let heading = if let Some(r) = &b.results {
        let tree = &b.tree;
        let total: u64 = r.items.iter().map(|&id| tree.node(id).size.get(mode)).sum();
        let checked: u64 = r
            .checked_ids()
            .iter()
            .map(|&id| tree.node(id).size.get(mode))
            .sum();
        format!(
            "Arama „{}” — {}  │  {} sonuç, {} ({mode_label})  │  seçili: {} ({})",
            r.pattern,
            tree.path_of(r.base).display(),
            fmt_count(r.items.len() as u64),
            fmt_size(total),
            fmt_count(r.checked.iter().filter(|&&c| c).count() as u64),
            fmt_size(checked),
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

    if let Some(r) = &mut b.results {
        render_results(f, &b.tree, r, mode, table_area);
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
    } else if let Some(r) = &b.results {
        let text = if r.items.is_empty() {
            "Eşleşen öğe yok."
        } else {
            "Eşleşen klasörlerin içi ayrıca listelenmez; klasörle birlikte taşınır."
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

    let footer_keys: &[(&str, &str)] = if b.confirm.is_some() {
        &[("e", "evet, çöpe taşı"), ("h / Esc", "vazgeç")]
    } else if b.deleting.is_some() {
        &[("q", "çık")]
    } else if b.input.is_some() {
        &[("Enter", "ara"), ("Esc", "vazgeç")]
    } else if b.results.is_some() {
        &[
            ("↑↓", "gez"),
            ("Space", "seç"),
            ("t", "tümü"),
            ("x", "seçilileri çöpe taşı"),
            ("Enter", "konuma git"),
            ("/", "yeni arama"),
            ("Esc", "listeye dön"),
            ("q", "çık"),
        ]
    } else {
        &[
            ("↑↓", "gez"),
            ("Enter", "gir"),
            ("⌫", "geri"),
            ("/", "ara"),
            ("s", "sırala"),
            ("a", "görünen/diskte"),
            ("x", "çöpe taşı"),
            ("r", "yeniden tara"),
            ("d", "diskler"),
            ("q", "çık"),
        ]
    };
    f.render_widget(keys(footer_keys), footer);

    if let Some(ids) = &b.confirm {
        render_confirm(f, &b.tree, ids, mode, f.area());
    }
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

fn render_results(f: &mut Frame, tree: &Tree, r: &mut SearchResults, mode: SizeMode, area: Rect) {
    let now = now_secs();
    let wide = area.width >= WIDE;
    let rows = r.items.iter().enumerate().map(|(i, &id)| {
        let n = tree.node(id);
        let check = if r.checked[i] {
            Span::raw("[✓]").green().bold()
        } else {
            Span::raw("[ ]").dark_gray()
        };
        let label = if n.is_dir {
            Span::styled(r.labels[i].clone(), Style::new().fg(Color::Blue).bold())
        } else {
            Span::raw(r.labels[i].clone())
        };
        let count = if n.is_dir {
            fmt_count(n.file_count.into())
        } else {
            String::new()
        };
        let mut cells = vec![
            Cell::from(check),
            Cell::from(Line::from(fmt_size(n.size.get(mode))).right_aligned()),
            Cell::from(Line::from(count).right_aligned().dark_gray()),
            date_cell(n.modified, now),
        ];
        if wide {
            cells.push(date_cell(n.created, now));
        }
        cells.push(Cell::from(label));
        Row::new(cells)
    });

    let mut widths = vec![
        Constraint::Length(3),
        Constraint::Length(11),
        Constraint::Length(10),
        Constraint::Length(DATE_WIDTH),
    ];
    let mut header = vec!["", "Boyut", "Dosya", "Son değişiklik"];
    if wide {
        widths.push(Constraint::Length(DATE_WIDTH));
        header.push("Oluşturma");
    }
    widths.push(Constraint::Min(10));
    header.push("Konum");

    let table = Table::new(rows, widths)
        .header(Row::new(header).bold().underlined())
        .row_highlight_style(HIGHLIGHT)
        .highlight_symbol("▶ ")
        .block(table_block());
    f.render_stateful_widget(table, area, &mut r.table);
}

/// Top/bottom rule with the date color legend on the top edge.
fn table_block() -> Block<'static> {
    let mut legend = vec![Span::raw(" tarih: ").dark_gray()];
    for (age, label) in [
        (0, "7 gün"),
        (8 * DAY, "30 gün"),
        (31 * DAY, "1 yıl"),
        (366 * DAY, "daha eski"),
    ] {
        legend.push(Span::styled("● ", Style::new().fg(age_color(age))));
        legend.push(Span::raw(format!("{label}  ")).dark_gray());
    }
    Block::new()
        .borders(Borders::TOP | Borders::BOTTOM)
        .title_top(Line::from(legend).right_aligned())
}

const DAY: u64 = 24 * 60 * 60;

/// Color for a timestamp of the given age in seconds: fresh is green,
/// untouched for over a year is red.
fn age_color(age: u64) -> Color {
    match age {
        a if a <= 7 * DAY => Color::Green,
        a if a <= 30 * DAY => Color::Cyan,
        a if a <= 365 * DAY => Color::Yellow,
        _ => Color::Red,
    }
}

fn date_cell(secs: u32, now: u64) -> Cell<'static> {
    if secs == 0 {
        return Cell::from(Span::raw("—").dark_gray());
    }
    let age = now.saturating_sub(u64::from(secs));
    Cell::from(Span::styled(
        fmt_date(secs),
        Style::new().fg(age_color(age)),
    ))
}

fn now_secs() -> u64 {
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
