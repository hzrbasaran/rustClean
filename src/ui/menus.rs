//! The report menu and the saved scan picker.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crate::reports::MenuItem;

use super::format::{fmt_ago, fmt_date, fmt_delta, fmt_size, now_secs};
use super::style::HIGHLIGHT;

pub(super) fn render_snapshot_picker(
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
                Span::raw(tf!("toplam {:>10}", "total {:>10}", fmt_size(h.total.disk))),
                Span::raw(tf!(
                    "   şimdiye göre {}",
                    "   vs. now {}",
                    fmt_delta(total_now, h.total.disk)
                ))
                .yellow(),
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
                .title(
                    Span::raw(t!(
                        " Hangi taramayla karşılaştırılsın? ",
                        " Compare with which scan? "
                    ))
                    .white()
                    .bold(),
                )
                .border_style(Style::new().fg(Color::Cyan)),
        ),
        popup,
    );
}

pub(super) fn render_report_menu(f: &mut Frame, selected: usize, area: Rect) {
    let width = area.width.saturating_sub(4).min(84);
    let mut lines = Vec::new();
    for (i, item) in MenuItem::ALL.iter().enumerate() {
        if i == 0 || item.is_tool() != MenuItem::ALL[i - 1].is_tool() {
            let heading = if item.is_tool() {
                t!(" Araçlar", " Tools")
            } else {
                t!(" Raporlar", " Reports")
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
        Line::from(t!(
            " Raporlar bulunduğunuz klasörün altında çalışır (uygulamalar: tüm tarama). L: English",
            " Reports cover the folder you are in (apps: the whole scan). L: Türkçe",
        ))
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
                .title(Span::raw(t!(" Menü ", " Menu ")).white().bold())
                .border_style(Style::new().fg(Color::Cyan)),
        ),
        popup,
    );
}
