//! The summary dashboard of a folder.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};
use ratatui::Frame;

use crate::app::{Dashboard, Pane};
use crate::stats;
use crate::tree::{NodeId, SizeMode, Tree};

use super::format::{fmt_count, fmt_pct, fmt_size, now_secs};
use super::style::{date_cell, date_span, usage_color, AGE_COLORS, CATEGORY_COLORS, HIGHLIGHT};
use super::{bar, halves, panel, DATE_WIDTH};

pub(super) fn render_dashboard(
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
            Span::raw(format!("{:<15}", t!("Toplam boyut", "Total size"))).white(),
            Span::raw(fmt_size(s.size)).bold(),
        ]),
        Line::from(vec![
            Span::raw(format!("{:<15}", t!("İçerik", "Contents"))).white(),
            Span::raw(tf!(
                "{} dosya · {} klasör",
                "{} files · {} folders",
                fmt_count(s.files),
                fmt_count(s.dirs)
            )),
        ]),
        Line::from(vec![
            Span::raw(format!("{}  ", t!("En yeni değişiklik", "Latest change"))).white(),
            date_span(base.modified, now),
        ]),
    ];
    if scan_errors > 0 {
        lines.push(
            Line::from(tf!(
                "⚠ Taramada {} öğeye erişilemedi",
                "⚠ {} items were inaccessible during the scan",
                fmt_count(scan_errors)
            ))
            .yellow(),
        );
    }
    f.render_widget(
        Paragraph::new(lines).block(panel(t!(" Genel ", " General "), false)),
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
                Span::raw(format!(" {}", fmt_pct(ratio * 100.0, 0))).bold(),
            ]),
            Line::from(tf!(
                "{} / {} kullanılıyor · {} boş",
                "{} / {} used · {} free",
                fmt_size(disk.used()),
                fmt_size(disk.total),
                fmt_size(disk.available)
            )),
            Line::from(vec![
                Span::raw(t!("Disk payı  ", "Disk share  ")).white(),
                Span::raw(if share > 0.0 && share < 0.1 {
                    format!("< {}", fmt_pct(0.1, 1))
                } else {
                    fmt_pct(share, 1)
                })
                .bold(),
            ]),
        ]
    } else {
        vec![Line::from(t!("Disk bilgisi bulunamadı.", "No disk information.")).gray()]
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
                Span::raw(format!(" {:>5}", fmt_pct(ratio * 100.0, 0))).gray(),
            ])
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).block(panel(t!(" Dosya türleri ", " File types "), false)),
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
                Span::raw(format!("{:<11}", stats::age_label(i))).white(),
                Span::styled(bar(ratio, bar_w), Style::new().fg(AGE_COLORS[i])),
                Span::raw(format!(" {:>10}", fmt_size(b.size))),
                Span::raw(format!(" {:>5}", fmt_pct(ratio * 100.0, 0))).gray(),
            ])
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).block(panel(
            t!(" Yaş (son değişiklik) ", " Age (last modified) "),
            false,
        )),
        ages,
    );

    // Largest files and fullest directories
    let base_path = tree.path_of(d.base);
    let rel = |id: NodeId| {
        if id == d.base {
            return t!("(bu klasör)", "(this folder)").to_string();
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
    .block(panel(
        t!(" En büyük dosyalar ", " Largest files "),
        focus_files,
    ));
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
                Style::new().fg(Color::LightBlue).bold(),
            )),
        ])
    });
    let table = Table::new(rows, [Constraint::Length(10), Constraint::Min(10)]).block(panel(
        t!(
            " En dolu klasörler (doğrudan içerik) ",
            " Fullest folders (direct contents) "
        ),
        focus_dirs,
    ));
    let table = if focus_dirs {
        table.row_highlight_style(HIGHLIGHT).highlight_symbol("▶ ")
    } else {
        table
    };
    f.render_stateful_widget(table, dirs, &mut d.dirs);
}
