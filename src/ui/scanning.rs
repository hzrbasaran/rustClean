//! Progress while a scan runs.

use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;

use super::format::{fmt_count, fmt_size, truncate_path};
use super::style::Themed;
use super::{keys, title, SPINNER};

pub(super) fn render_scanning(
    f: &mut Frame<'_>,
    app: &mut App,
    header: Rect,
    body: Rect,
    footer: Rect,
) {
    f.render_widget(
        title(tf!(
            "Taranıyor: {}",
            "Scanning: {}",
            app.scan_root.display()
        )),
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
        Line::from(tf!("  {spin} Taranıyor…", "  {spin} Scanning…"))
            .accent()
            .bold(),
        Line::from(""),
        stat_line(t!("Dosya", "Files"), &fmt_count(p.files)),
        stat_line(t!("Klasör", "Folders"), &fmt_count(p.dirs)),
        stat_line(t!("Boyut", "Size"), &fmt_size(p.bytes)),
        stat_line(t!("Erişilemeyen", "Inaccessible"), &fmt_count(p.errors)),
        stat_line(
            t!("Süre", "Time"),
            &tf!("{:.1} sn", "{:.1} s", elapsed.as_secs_f64()),
        ),
        Line::from(""),
        stat_line(t!("Şu an", "Now"), &truncate_path(&p.current, width)),
    ];
    f.render_widget(Paragraph::new(lines), body);
    f.render_widget(
        keys(&[("Esc", t!("iptal", "cancel")), ("q", t!("çık", "quit"))]),
        footer,
    );
}

fn stat_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw(format!("  {label:<13}")).muted(),
        Span::raw(value.to_string()),
    ])
}
