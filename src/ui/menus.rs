//! The report menu and the saved scan picker.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
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
    scroll: &mut usize,
    area: Rect,
    mouse: &mut Mouse,
) {
    let width = area.width.saturating_sub(4).min(84);
    let inner_width = width.saturating_sub(2);
    // The list: headings and items. `rows[i]` is the line of item `i`.
    let mut list = Vec::new();
    let mut rows = Vec::new();
    for (i, item) in MenuItem::ALL.iter().enumerate() {
        if i == 0 || item.is_tool() != MenuItem::ALL[i - 1].is_tool() {
            let heading = if item.is_tool() {
                t!(" Araçlar", " Tools")
            } else {
                t!(" Raporlar", " Reports")
            };
            if i > 0 {
                list.push(Line::from(""));
            }
            list.push(Line::from(heading).accent().bold());
        }
        rows.push(list.len());
        let marker = if i == selected { "▶ " } else { "  " };
        let style = if i == selected {
            highlight()
        } else {
            Style::new()
        };
        list.push(
            Line::from(vec![
                Span::raw(marker),
                Span::raw(format!("{:>2}. ", i + 1)).muted(),
                Span::raw(item.label()).normal().bold(),
            ])
            .style(style),
        );
    }
    // Below the list, always shown: what the selected item does.
    let footer = vec![
        Line::from(MenuItem::ALL[selected].description()).warn(),
        Line::from(t!(
            "Raporlar bulunduğunuz klasörün altında çalışır (uygulamalar: tüm tarama). L: English",
            "Reports cover the folder you are in (apps: the whole scan). L: Türkçe",
        ))
        .muted(),
    ];
    let footer_rows: usize = footer
        .iter()
        .map(|l| super::wrapped_rows(l, inner_width.saturating_sub(2)))
        .sum();

    // The screen's first and last lines (the title and the keys) stay
    // visible. One row above and one below the list: blank, or how much of
    // it is hidden.
    let room = usize::from(area.height.saturating_sub(4));
    let fits = list.len() + 2 + footer_rows <= room;
    // A blank line between a cut list's last row and the description.
    let gap = usize::from(!fits);
    let shown = list
        .len()
        .min(room.saturating_sub(footer_rows + 2 + gap))
        .max(3.min(list.len()));
    let first = menu_window(&rows, list.len(), shown, selected, scroll);
    // A heading or blank line at the bottom of the view goes with the
    // items after it, which are not shown.
    let mut end = first + shown;
    while end > first + 1 && !rows.contains(&(end - 1)) {
        end -= 1;
    }
    let above = rows.iter().filter(|&&l| l < first).count();
    let below = rows.iter().filter(|&&l| l >= end).count();
    let more = |n: usize, arrow: &str| -> Line<'static> {
        if n == 0 {
            Line::from("")
        } else {
            let hidden = crate::i18n::count(n as u64, "öğe daha", "more item", "more items");
            Line::from(format!(" {arrow} {hidden}")).muted()
        }
    };

    let height = ((shown + 2 + gap + footer_rows) as u16 + 2).min(area.height.saturating_sub(2));
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    // Lines of the visible items inside the popup; hidden ones get none.
    let visible: Vec<usize> = rows
        .iter()
        .map(|&l| {
            if (first..end).contains(&l) {
                l - first + 1
            } else {
                usize::MAX
            }
        })
        .collect();
    record_lines(mouse, popup, &visible, Hit::Menu);
    f.render_widget(Clear, popup);
    let block = Block::bordered()
        .title(Span::raw(t!(" Menü ", " Menu ")).normal().bold())
        .border_style(Style::new().fg(theme().accent));
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let [top, body, bottom, _, notes] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(shown as u16),
        Constraint::Length(1),
        Constraint::Length(gap as u16),
        Constraint::Min(0),
    ])
    .areas(inner);
    f.render_widget(more(above, "▲"), top);
    let window: Vec<Line<'_>> = list.into_iter().take(end).skip(first).collect();
    f.render_widget(Paragraph::new(window), body);
    f.render_widget(more(below, "▼"), bottom);
    // Indented like the list, also where a long text wraps.
    let notes = notes.inner(ratatui::layout::Margin::new(1, 0));
    f.render_widget(Paragraph::new(footer).wrap(Wrap { trim: false }), notes);
}

/// The first line of the menu list to show in `shown` rows, so the selected
/// item is in view: the view moves only when the selection reaches an edge,
/// and the first item of a group brings its heading along.
fn menu_window(
    rows: &[usize],
    total: usize,
    shown: usize,
    selected: usize,
    scroll: &mut usize,
) -> usize {
    let line = rows[selected];
    // The heading (and the blank line before it) of a group's first item.
    let top = if selected == 0 {
        0
    } else if rows[selected - 1] + 1 < line {
        rows[selected - 1] + 1
    } else {
        line
    };
    let mut first = (*scroll).min(total.saturating_sub(shown));
    if top < first {
        first = top;
    }
    if line >= first + shown {
        first = line + 1 - shown;
    }
    *scroll = first;
    first
}

/// Makes the lines of a bordered popup clickable: `rows[i]` is the line of
/// item `i`. The popup covers what is behind it, so that stops being clickable.
fn record_lines(mouse: &mut Mouse, popup: Rect, rows: &[usize], hit: impl Fn(usize) -> Hit) {
    mouse.clear();
    let inner = popup.inner(ratatui::layout::Margin::new(1, 1));
    for (i, &line) in rows.iter().enumerate() {
        let Ok(line) = u16::try_from(line) else {
            continue;
        };
        if line >= inner.height {
            continue;
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

#[cfg(test)]
mod tests {
    use super::menu_window;

    #[test]
    fn the_menu_view_moves_only_at_its_edges() {
        // A heading on line 0, items 0–3 on lines 1–4, a blank line and a
        // heading on 5–6, items 4–5 on lines 7–8. Four lines are shown.
        let rows = [1, 2, 3, 4, 7, 8];
        let mut scroll = 0;
        assert_eq!(menu_window(&rows, 9, 4, 0, &mut scroll), 0);
        assert_eq!(
            menu_window(&rows, 9, 4, 3, &mut scroll),
            1,
            "down to the edge"
        );
        assert_eq!(menu_window(&rows, 9, 4, 2, &mut scroll), 1, "inside: stays");
        assert_eq!(menu_window(&rows, 9, 4, 5, &mut scroll), 5);
        // The first item of a group brings its heading and blank line along.
        assert_eq!(menu_window(&rows, 9, 4, 4, &mut scroll), 5);
        assert_eq!(menu_window(&rows, 9, 4, 1, &mut scroll), 2);
        assert_eq!(
            menu_window(&rows, 9, 4, 0, &mut scroll),
            0,
            "the top heading too"
        );
        // A stale offset (the menu opened again) is pulled back into range.
        let mut stale = 50;
        assert_eq!(menu_window(&rows, 9, 4, 5, &mut stale), 5);
    }
}
