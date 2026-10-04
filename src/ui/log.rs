//! The deletion log: what was moved to the trash, by day, newest first.

use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::LogView;
use crate::tree::SizeMode;

use super::format::{fmt_date, fmt_size, tilde, truncate_path};
use super::style::Themed;

/// `(date, time)` of a log entry, in the current language and time zone.
fn date_and_time(secs: u64) -> (String, String) {
    let text = fmt_date(u32::try_from(secs).unwrap_or(u32::MAX));
    match text.split_once(' ') {
        Some((d, t)) => (d.to_string(), t.to_string()),
        None => (text, String::new()),
    }
}

/// The heading line: totals of the last 30 days and of the whole log.
pub(super) fn heading(log: &LogView, mode: SizeMode, now: u64) -> String {
    let month: u64 = log
        .entries
        .iter()
        .filter(|e| now.saturating_sub(e.time) <= 30 * 86_400)
        .map(|e| e.size.get(mode))
        .sum();
    let all: u64 = log.entries.iter().map(|e| e.size.get(mode)).sum();
    tf!(
        "Silme kaydı  │  son 30 gün: {}  │  toplam {}, {}",
        "Deletion log  │  last 30 days: {}  │  {} in all, {}",
        fmt_size(month),
        crate::i18n::count(log.entries.len() as u64, "öğe", "item", "items"),
        fmt_size(all)
    )
}

pub(super) fn render_log(f: &mut Frame<'_>, log: &mut LogView, mode: SizeMode, area: Rect) {
    let width = usize::from(area.width);
    let mut lines: Vec<Line<'_>> = Vec::new();
    if log.entries.is_empty() {
        lines.push(Line::from(""));
        lines.push(
            Line::from(t!(
                "  Henüz çöp kutusuna bir şey taşınmadı.",
                "  Nothing has been moved to the trash yet."
            ))
            .muted(),
        );
    }
    // Entries are newest first, so a day's entries are next to each other.
    let mut i = 0;
    while i < log.entries.len() {
        let (day, _) = date_and_time(log.entries[i].time);
        let end = (i..log.entries.len())
            .find(|&j| date_and_time(log.entries[j].time).0 != day)
            .unwrap_or(log.entries.len());
        let total: u64 = log.entries[i..end].iter().map(|e| e.size.get(mode)).sum();
        lines.push(Line::from(vec![
            Span::raw(format!(" {day}")).accent().bold(),
            Span::raw(format!(
                "   {}, {}",
                crate::i18n::count((end - i) as u64, "öğe", "item", "items"),
                fmt_size(total)
            ))
            .muted(),
        ]));
        for e in &log.entries[i..end] {
            let (_, time) = date_and_time(e.time);
            let via = e.via.label();
            let fixed = 3 + 5 + 2 + 10 + 2 + 24 + 1;
            let path = truncate_path(
                &tilde(std::path::Path::new(&e.path)),
                width.saturating_sub(fixed),
            );
            lines.push(Line::from(vec![
                Span::raw(format!("   {time:<5}")).muted(),
                Span::raw(format!("  {:>10}", fmt_size(e.size.get(mode)))).normal(),
                Span::raw(format!("  {:<24}", super::format::clip(&via, 24))).muted(),
                Span::raw(format!(" {path}")).normal(),
            ]));
        }
        i = end;
    }
    lines.push(Line::from(""));
    lines.push(
        Line::from(t!(
            "  Öğeler çöp kutusunda: boşaltılana kadar oradan geri alınabilir.",
            "  The entries are in the trash: they can be restored from there until it is emptied."
        ))
        .muted(),
    );
    let max =
        u16::try_from(lines.len().saturating_sub(usize::from(area.height))).unwrap_or(u16::MAX);
    log.scroll = log.scroll.min(max);
    f.render_widget(Paragraph::new(lines).scroll((log.scroll, 0)), area);
}
