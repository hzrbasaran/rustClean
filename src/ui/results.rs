//! Result lists: reports, search results, the basket.

use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};
use ratatui::Frame;

use crate::app::{Hit, Mouse};
use crate::lists::ResultList;
use crate::tree::Tree;

use super::format::{fmt_count, fmt_size, now_secs, wrap_indented};
use super::style::{date_cell, highlight, Themed};
use super::theme::theme;
use super::{table_block, table_rows, DATE_WIDTH, WIDE};

pub(super) fn render_results(
    f: &mut Frame<'_>,
    tree: &Tree,
    r: &mut ResultList,
    checks: &[bool],
    area: Rect,
    mouse: &mut Mouse,
) {
    // An empty report explains why (nothing scanned there, no access…):
    // the note gets the whole area, as it may not fit the status line.
    if r.rows.is_empty() && !r.note.is_empty() {
        let lines: Vec<Line<'_>> =
            wrap_indented(&r.note, usize::from(area.width.saturating_sub(1)), " ", " ")
                .into_iter()
                .map(|l| Line::from(l).normal())
                .collect();
        let text = Paragraph::new(lines).block(table_block());
        f.render_widget(text, area);
        return;
    }
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
            Span::raw("[✓]").success().bold()
        } else {
            Span::raw("[ ]").muted()
        };
        // Groups show their member count and newest member; single entries
        // their file count (directories) and own dates.
        let (count, modified, created) = if row.group {
            let newest = row.nodes.iter().map(|&id| tree.node(id).modified).max();
            (
                crate::i18n::count(row.nodes.len() as u64, "adet", "item", "items"),
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
            Style::new().fg(theme().folder).bold()
        } else {
            Style::new()
        };
        let mut cells = vec![
            Cell::from(check),
            Cell::from(Line::from(fmt_size(row.size())).right_aligned()),
            Cell::from(Line::from(count).right_aligned().muted()),
            date_cell(modified, now),
        ];
        if wide {
            cells.push(date_cell(created, now));
        }
        cells.push(Cell::from(Span::styled(row.label.clone(), label_style)));
        if has_detail {
            cells.push(Cell::from(Span::raw(row.detail.clone()).muted()));
        }
        Row::new(cells)
    });

    let mut widths = vec![
        Constraint::Length(3),
        Constraint::Length(11),
        Constraint::Length(10),
        Constraint::Length(DATE_WIDTH),
    ];
    let mut header = vec![
        "",
        t!("Boyut", "Size"),
        t!("Adet", "Count"),
        t!("Son değişiklik", "Modified"),
    ];
    if wide {
        widths.push(Constraint::Length(DATE_WIDTH));
        header.push(t!("Oluşturma", "Created"));
    }
    // The location matters most: it gets at least 30 columns, details
    // whatever is left up to their length.
    widths.push(Constraint::Min(30));
    header.push(t!("Konum", "Location"));
    if has_detail {
        widths.push(Constraint::Max(detail_width.max(7)));
        header.push(t!("Ayrıntı", "Details"));
    }

    let table = Table::new(rows, widths)
        .header(Row::new(header).bold().underlined())
        .row_highlight_style(highlight())
        .highlight_symbol("▶ ")
        .block(table_block());
    f.render_stateful_widget(table, area, &mut r.table);
    mouse.add_rows(
        table_rows(area),
        r.table.offset(),
        r.rows.len(),
        Hit::Result,
    );
}
