//! Rendering of the three screens.

use std::path::Path;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;

use crate::app::{App, Screen};
use crate::tree::SizeMode;

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
    let tree = &b.tree;
    let cur = tree.node(b.current);
    let mode = b.size_mode;
    let mode_label = match mode {
        SizeMode::Disk => "diskte",
        SizeMode::Apparent => "görünen",
    };

    f.render_widget(
        title(format!(
            "{}  │  {} ({mode_label})  │  {} dosya  │  sıralama: {}",
            tree.path_of(b.current).display(),
            fmt_size(cur.size.get(mode)),
            fmt_count(cur.file_count.into()),
            b.sort.label()
        )),
        header,
    );

    let [table_area, status_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(body);

    let parent_size = cur.size.get(mode).max(1);
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
        Row::new(vec![
            Cell::from(Line::from(fmt_size(size)).right_aligned()),
            Cell::from(Line::from(vec![
                Span::styled(bar(ratio, 12), Style::new().fg(Color::Cyan)),
                Span::raw(format!(" {:>5.1}%", ratio * 100.0)),
            ])),
            Cell::from(Line::from(count).right_aligned().dark_gray()),
            Cell::from(name),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(11),
            Constraint::Length(19),
            Constraint::Length(10),
            Constraint::Min(10),
        ],
    )
    .header(
        Row::new(["Boyut", "Oran", "Dosya", "Ad"])
            .bold()
            .underlined(),
    )
    .row_highlight_style(HIGHLIGHT)
    .highlight_symbol("▶ ")
    .block(Block::new().borders(Borders::TOP | Borders::BOTTOM));
    f.render_stateful_widget(table, table_area, &mut b.table);

    let mut status = vec![Span::raw(format!(
        "{} öğe tarandı, {:.1} sn",
        fmt_count(tree.len() as u64),
        b.elapsed.as_secs_f64()
    ))
    .dark_gray()];
    if b.errors > 0 {
        status.push(Span::raw(format!("   ⚠ {} öğeye erişilemedi", fmt_count(b.errors))).yellow());
    }
    if b.entries.is_empty() {
        status.push(Span::raw("   (klasör boş)").dark_gray());
    }
    f.render_widget(Line::from(status), status_area);

    f.render_widget(
        keys(&[
            ("↑↓", "gez"),
            ("Enter", "gir"),
            ("⌫", "geri"),
            ("s", "sırala"),
            ("a", "görünen/diskte"),
            ("r", "yeniden tara"),
            ("d", "diskler"),
            ("q", "çık"),
        ]),
        footer,
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
    fn bar_has_fixed_width() {
        for r in [0.0, 0.03, 0.5, 0.999, 1.0] {
            assert_eq!(bar(r, 10).chars().count(), 10);
        }
    }
}
