//! The disk list shown at start.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use ratatui::Frame;

use crate::app::{App, Hit};

use super::format::{fmt_pct, fmt_size};
use super::style::{highlight, usage_color, Themed};
use super::{bar, keys, table_rows, title};

pub(super) fn render_disks(
    f: &mut Frame<'_>,
    app: &mut App,
    header: Rect,
    body: Rect,
    footer: Rect,
) {
    f.render_widget(
        title(
            t!(
                "Diskler — taramak için bir disk seçin",
                "Disks — choose a disk to scan"
            )
            .into(),
        ),
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
                Span::raw(format!(" {:>4}", fmt_pct(ratio * 100.0, 0))),
            ])),
            Cell::from(if d.removable {
                t!("çıkarılabilir", "removable")
            } else {
                ""
            }),
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
            t!("Ad", "Name"),
            t!("Bağlama noktası", "Mount point"),
            "FS",
            t!("Kullanılan", "Used"),
            t!("Toplam", "Total"),
            t!("Doluluk", "Usage"),
            "",
        ])
        .bold()
        .underlined(),
    )
    .row_highlight_style(highlight())
    .highlight_symbol("▶ ")
    .block(Block::new().borders(Borders::TOP | Borders::BOTTOM));
    f.render_stateful_widget(table, table_area, &mut app.disk_table);
    app.mouse.add_rows(
        table_rows(table_area),
        app.disk_table.offset(),
        app.disks.len(),
        Hit::Disk,
    );

    if let Some(msg) = &app.message {
        f.render_widget(Line::from(msg.clone()).warn(), msg_area);
    } else if app.disks.is_empty() {
        f.render_widget(
            Line::from(t!("Hiç disk bulunamadı.", "No disks found.")).warn(),
            msg_area,
        );
    }

    f.render_widget(
        keys(&[
            ("↑↓", t!("seç", "select")),
            ("Enter", t!("tara", "scan")),
            ("r", t!("yenile", "refresh")),
            ("L", t!("English", "Türkçe")),
            ("T", t!("tema", "theme")),
            ("M", t!("fare", "mouse")),
            ("q", t!("çık", "quit")),
        ]),
        footer,
    );
}
