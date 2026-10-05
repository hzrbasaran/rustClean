//! The developer tools cleanup screen.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::app::{Hit, Mouse};
use crate::tools::{CleanAction, Status as ToolStatus};
use crate::toolsview::{confirm_word, ToolsView};

use super::format::fmt_size;
use super::style::{highlight, risk_style, Themed};
use super::theme::theme;
use super::{panel, popup, table_block_plain, table_rows, wrapped_rows, SPINNER};

/// The actions with their steps, in at most `max` rows at `width`: when
/// they do not fit, each action shows fewer steps and a "… more" line, and
/// as a last resort the list is cut with a line saying how much is left out.
fn action_lines(actions: &[CleanAction], max: usize, width: u16) -> Vec<Line<'static>> {
    let build = |shown: usize| {
        let mut lines = Vec::new();
        for a in actions {
            let (style, text) = risk_style(a.risk);
            lines.push(Line::from(vec![
                Span::styled(text, style),
                Span::raw(format!(" {}", a.label)).normal().bold(),
            ]));
            if a.manual() {
                lines.push(
                    Line::from(t!(
                        "    rustClean bunu çalıştırmaz; komutu kendiniz çalıştırın:",
                        "    rustClean does not run this; run the command yourself:",
                    ))
                    .warn(),
                );
            }
            for step in a.steps.iter().take(shown) {
                let line = Line::from(format!("    $ {}", step.describe()));
                // The command to copy reads as text, not as a hint.
                lines.push(if a.manual() {
                    line.normal()
                } else {
                    line.muted()
                });
            }
            let rest = a.steps.len().saturating_sub(shown);
            if rest > 0 {
                let more = if rest == 1 { "step" } else { "steps" };
                lines.push(
                    Line::from(tf!("    … {rest} adım daha", "    … {rest} more {more}")).muted(),
                );
            }
        }
        lines
    };
    let rows = |lines: &[Line<'_>]| -> usize { lines.iter().map(|l| wrapped_rows(l, width)).sum() };
    let most = actions.iter().map(|a| a.steps.len()).max().unwrap_or(0);
    for shown in (1..=most.max(1)).rev() {
        let lines = build(shown);
        if rows(&lines) <= max {
            return lines;
        }
    }
    let mut lines = build(1);
    let mut cut = 0;
    while lines.len() > 1 && rows(&lines) + 1 > max {
        lines.pop();
        cut += 1;
    }
    lines.push(Line::from(tf!("… {cut} satır daha", "… {cut} more lines")).muted());
    lines
}

pub(super) fn render_tools(
    f: &mut Frame<'_>,
    view: &mut ToolsView,
    tick: usize,
    area: Rect,
    mouse: &mut Mouse,
) {
    let inner = area.width.saturating_sub(2);
    let max_rows = usize::from((area.height / 2).saturating_sub(2));
    let hint = view.selected().and_then(|t| t.kind.hint());
    let hint_rows = hint.map_or(0, |h| wrapped_rows(&Line::from(h), inner));
    let action_rows = if view.run.is_some() {
        usize::from(area.height / 2)
    } else {
        view.selected().map_or(1, |t| {
            let budget = max_rows.saturating_sub(hint_rows).max(1);
            let lines = action_lines(&t.actions, budget, inner);
            lines
                .iter()
                .map(|l| wrapped_rows(l, inner))
                .sum::<usize>()
                .max(1)
                + hint_rows
        })
    };
    let [list_area, detail_area] = Layout::vertical([
        Constraint::Min(6),
        Constraint::Length((action_rows as u16 + 2).min(area.height / 2)),
    ])
    .areas(area);

    let spin = SPINNER[tick % SPINNER.len()];
    let rows = view.tools.iter().map(|t| {
        let (size, info) = match &t.status {
            ToolStatus::Measuring => (
                Line::from(format!("{spin}")).accent(),
                Span::raw(t!("ölçülüyor…", "measuring…")).accent(),
            ),
            ToolStatus::Missing(why) => (Line::from("—").muted(), Span::raw(why.clone()).muted()),
            ToolStatus::Unavailable(why) => (Line::from("—").warn(), Span::raw(why.clone()).warn()),
            ToolStatus::Ready {
                reclaimable,
                detail,
            } => (
                Line::from(fmt_size(*reclaimable)).right_aligned().bold(),
                Span::raw(detail.clone()),
            ),
        };
        let name_style = match t.status {
            ToolStatus::Missing(_) => Style::new().fg(theme().muted),
            _ => Style::new().fg(theme().text).bold(),
        };
        Row::new(vec![
            Cell::from(Span::styled(t.kind.label(), name_style)),
            Cell::from(size),
            Cell::from(info),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(34),
            Constraint::Length(12),
            Constraint::Min(10),
        ],
    )
    .header(
        Row::new([
            t!("Araç", "Tool"),
            t!("Kazanılır", "Reclaimable"),
            t!("Ayrıntı", "Details"),
        ])
        .bold()
        .underlined(),
    )
    .row_highlight_style(highlight())
    .highlight_symbol("▶ ")
    .block(table_block_plain());
    f.render_stateful_widget(table, list_area, &mut view.table);
    mouse.add_rows(
        table_rows(list_area),
        view.table.offset(),
        view.tools.len(),
        Hit::Tool,
    );

    let mut lines = Vec::new();
    match view.selected() {
        Some(t) if !t.actions.is_empty() => {
            let budget = usize::from(detail_area.height.saturating_sub(2))
                .saturating_sub(hint_rows)
                .max(1);
            lines.extend(action_lines(&t.actions, budget, inner));
        }
        Some(t) if t.status == ToolStatus::Measuring => {
            lines.push(Line::from(t!("Ölçülüyor…", "Measuring…")).accent());
        }
        _ => lines.push(
            Line::from(t!(
                "Bu araç için yapılacak bir şey yok.",
                "Nothing to do for this tool."
            ))
            .muted(),
        ),
    }
    if let Some(hint) = hint {
        lines.push(Line::from(hint).muted());
    }
    if let Some(run) = &view.run {
        // Show the end of the log.
        let visible = detail_area.height.saturating_sub(2) as usize;
        let start = run.log.len().saturating_sub(visible);
        let lines: Vec<Line<'_>> = run.log[start..]
            .iter()
            .map(|l| {
                if l.starts_with('$') {
                    Line::from(l.clone()).normal().bold()
                } else if l.contains('✗') {
                    Line::from(l.clone()).danger()
                } else if l.contains('✓') {
                    Line::from(l.clone()).success()
                } else {
                    Line::from(l.clone()).muted()
                }
            })
            .collect();
        let title = if !run.finished {
            tf!(
                " {} Çalışıyor… ",
                " {} Running… ",
                SPINNER[tick % SPINNER.len()]
            )
        } else if run.failures > 0 {
            tf!(
                " Bitti — {} adım başarısız ",
                " Done — {} steps failed ",
                run.failures
            )
        } else {
            t!(" Bitti ", " Done ").to_string()
        };
        f.render_widget(
            Paragraph::new(lines).block(panel(&title, true)),
            detail_area,
        );
    } else {
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(panel(t!(" İşlemler ", " Actions "), false)),
            detail_area,
        );
    }

    if view.picker.is_some() || view.confirm.is_some() {
        // Choosing and confirming what to clean is keyboard-only.
        mouse.clear();
    }
    if let Some(p) = &view.picker {
        let tool = &view.tools[p.tool];
        // Long lists scroll with the cursor; the popup keeps its border, the
        // two "more" lines and the key line.
        let room = usize::from(f.area().height.saturating_sub(8)).max(1);
        let n = tool.actions.len();
        let first = (p.cursor + 1)
            .saturating_sub(room)
            .min(n.saturating_sub(room));
        let last = (first + room).min(n);
        let more = |count: usize, up: bool| {
            if count == 0 {
                Line::from("")
            } else {
                let arrow = if up { "↑" } else { "↓" };
                Line::from(tf!("  {arrow} {count} daha", "  {arrow} {count} more")).muted()
            }
        };
        let mut lines = vec![more(first, true)];
        for (i, a) in tool.actions.iter().enumerate().take(last).skip(first) {
            let (style, text) = risk_style(a.risk);
            let check = if p.checked[i] { "[✓] " } else { "[ ] " };
            let row = if a.manual() {
                Line::from(vec![
                    Span::raw(if i == p.cursor { "▶ " } else { "  " }),
                    Span::raw("    "),
                    Span::styled(text, style),
                    Span::raw(format!(" {}", a.label)).muted(),
                    Span::raw(t!(" — kendiniz çalıştırın", " — run it yourself")).muted(),
                ])
            } else {
                Line::from(vec![
                    Span::raw(if i == p.cursor { "▶ " } else { "  " }),
                    Span::raw(check).bold(),
                    Span::styled(text, style),
                    Span::raw(format!(" {}", a.label)).normal(),
                ])
            };
            lines.push(if i == p.cursor {
                row.style(highlight())
            } else {
                row
            });
        }
        lines.push(more(n - last, false));
        lines.push(
            Line::from(t!(
                " Space: seç   Enter: devam   Esc: vazgeç",
                " Space: select   Enter: continue   Esc: cancel",
            ))
            .muted(),
        );
        popup(
            f,
            &tf!(
                " {} — ne temizlensin? ",
                " {} — what to clean? ",
                tool.kind.label()
            ),
            theme().accent,
            lines,
            90,
        );
    }

    if let Some(c) = &view.confirm {
        let tool = &view.tools[c.tool];
        let mut lines = vec![
            Line::from(""),
            Line::from(t!(
                "Şu komutlar sırayla çalışacak:",
                "These commands will run in order:"
            ))
            .normal(),
        ];
        // The prompt below must stay on screen, however many steps there are.
        let width = f.area().width.saturating_sub(4).min(100).saturating_sub(2);
        let hint = tool.kind.hint();
        let hint_rows = hint.map_or(0, |h| 1 + wrapped_rows(&Line::from(h), width));
        let room = usize::from(f.area().height.saturating_sub(2 + 2 + 5))
            .saturating_sub(hint_rows)
            .max(1);
        lines.extend(action_lines(&c.actions, room, width));
        if let Some(hint) = hint {
            lines.push(Line::from(""));
            lines.push(Line::from(hint).warn());
        }
        lines.push(Line::from(""));
        match &c.typed {
            Some(typed) => {
                lines.push(
                    Line::from(t!(
                        "Seçimde VERİ KAYBI olan bir işlem var. Onaylamak için „evet” yazıp Enter'a basın:",
                        "The selection includes a DATA LOSS action. Type “yes” and press Enter to confirm:",
                    ))
                        .danger()
                        .bold(),
                );
                let ok = typed.trim() == confirm_word();
                lines.push(Line::from(vec![
                    Span::raw(" > "),
                    Span::raw(format!("{typed}█")).normal().bold(),
                    Span::raw(if ok {
                        t!("   Enter: çalıştır", "   Enter: run")
                    } else {
                        ""
                    })
                    .success(),
                ]));
                lines.push(Line::from(t!(" Esc: vazgeç", " Esc: cancel")).muted());
            }
            None => lines.push(Line::from(vec![
                Span::raw(t!(" e ", " y ")).key_danger(),
                Span::raw(t!(" evet, çalıştır     ", " yes, run     ")),
                Span::raw(t!(" h ", " n ")).key(),
                Span::raw(t!(" vazgeç", " cancel")),
            ])),
        }
        popup(
            f,
            &tf!(" {} — onay ", " {} — confirm ", tool.kind.label()),
            theme().danger,
            lines,
            100,
        );
    }
}
