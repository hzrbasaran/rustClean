//! The report menu and the saved scan picker.

use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{Hit, Mouse};
use crate::reports::MenuItem;

use super::format::{fmt_ago, fmt_date, fmt_delta, fmt_size, now_secs};
use super::style::{highlight, Themed};
use super::theme::theme;

pub(super) fn render_snapshot_picker(
    f: &mut Frame<'_>,
    saved: &[crate::history::Saved],
    selected: usize,
    total_now: u64,
    area: Rect,
    mouse: &mut Mouse,
) {
    let width = area.width.saturating_sub(4).min(84);
    let now = now_secs();
    let mut lines = vec![Line::from("")];
    // The line each saved scan is on.
    let mut rows = Vec::new();
    for (i, s) in saved.iter().enumerate() {
        rows.push(lines.len());
        let h = &s.header;
        let style = if i == selected {
            highlight()
        } else {
            Style::new()
        };
        lines.push(
            Line::from(vec![
                Span::raw(if i == selected { "▶ " } else { "  " }),
                Span::raw(fmt_date(h.time.min(u64::from(u32::MAX)) as u32))
                    .normal()
                    .bold(),
                Span::raw(format!("  {:<16}", fmt_ago(now.saturating_sub(h.time)))).muted(),
                Span::raw(tf!("toplam {:>10}", "total {:>10}", fmt_size(h.total.disk))),
                Span::raw(tf!(
                    "   şimdiye göre {}",
                    "   vs. now {}",
                    fmt_delta(total_now, h.total.disk)
                ))
                .warn(),
            ])
            .style(style),
        );
    }
    lines.push(Line::from(""));
    lines.push(
        Line::from(t!(
            " Karşılaştırma bulunduğunuz klasörün altında yapılır.",
            " The comparison covers the folder you are in.",
        ))
        .muted(),
    );
    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    record_lines(mouse, popup, &rows, Hit::Snapshot);
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(
                    Span::raw(t!(
                        " Hangi taramayla karşılaştırılsın? ",
                        " Compare with which scan? "
                    ))
                    .normal()
                    .bold(),
                )
                .border_style(Style::new().fg(theme().accent)),
        ),
        popup,
    );
}

pub(super) fn render_report_menu(
    f: &mut Frame<'_>,
    selected: usize,
    area: Rect,
    mouse: &mut Mouse,
) {
    let width = area.width.saturating_sub(4).min(84);
    let mut lines = Vec::new();
    // The line each item is on.
    let mut rows = Vec::new();
    for (i, item) in MenuItem::ALL.iter().enumerate() {
        if i == 0 || item.is_tool() != MenuItem::ALL[i - 1].is_tool() {
            let heading = if item.is_tool() {
                t!(" Araçlar", " Tools")
            } else {
                t!(" Raporlar", " Reports")
            };
            lines.push(Line::from(""));
            lines.push(Line::from(heading).accent().bold());
        }
        rows.push(lines.len());
        let marker = if i == selected { "▶ " } else { "  " };
        let style = if i == selected {
            highlight()
        } else {
            Style::new()
        };
        lines.push(
            Line::from(vec![
                Span::raw(marker),
                Span::raw(format!("{:>2}. ", i + 1)).muted(),
                Span::raw(item.label()).normal().bold(),
            ])
            .style(style),
        );
    }
    lines.push(Line::from(""));
    lines.push(Line::from(format!(" {}", MenuItem::ALL[selected].description())).warn());
    lines.push(
        Line::from(t!(
            " Raporlar bulunduğunuz klasörün altında çalışır (uygulamalar: tüm tarama). L: English",
            " Reports cover the folder you are in (apps: the whole scan). L: Türkçe",
        ))
        .muted(),
    );

    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    record_lines(mouse, popup, &rows, Hit::Menu);
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(Span::raw(t!(" Menü ", " Menu ")).normal().bold())
                .border_style(Style::new().fg(theme().accent)),
        ),
        popup,
    );
}

/// Makes the lines of a bordered popup clickable: `rows[i]` is the line of
/// item `i`. The popup covers what is behind it, so that stops being clickable.
fn record_lines(mouse: &mut Mouse, popup: Rect, rows: &[usize], hit: impl Fn(usize) -> Hit) {
    mouse.clear();
    let inner = popup.inner(ratatui::layout::Margin::new(1, 1));
    for (i, &line) in rows.iter().enumerate() {
        let Ok(line) = u16::try_from(line) else { break };
        if line >= inner.height {
            break;
        }
        mouse.add(
            Rect {
                y: inner.y + line,
                height: 1,
                ..inner
            },
            hit(i),
        );
    }
}
