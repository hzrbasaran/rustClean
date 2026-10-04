//! The developer tools cleanup screen.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};
use ratatui::Frame;

use crate::tools::Status as ToolStatus;
use crate::toolsview::{confirm_word, ToolsView};

use super::format::fmt_size;
use super::style::{risk_style, HIGHLIGHT};
use super::{panel, popup, table_block_plain, SPINNER};

pub(super) fn render_tools(f: &mut Frame<'_>, view: &mut ToolsView, tick: usize, area: Rect) {
    let action_rows: u16 = if view.run.is_some() {
        area.height / 2
    } else {
        view.selected().map_or(1, |t| {
            t.actions
                .iter()
                .map(|a| 1 + a.steps.len() as u16)
                .sum::<u16>()
                .max(1)
        })
    };
    let [list_area, detail_area] = Layout::vertical([
        Constraint::Min(6),
        Constraint::Length((action_rows + 2).min(area.height / 2)),
    ])
    .areas(area);

    let spin = SPINNER[tick % SPINNER.len()];
    let rows = view.tools.iter().map(|t| {
        let (size, info) = match &t.status {
            ToolStatus::Measuring => (
                Line::from(format!("{spin}")).cyan(),
                Span::raw(t!("ölçülüyor…", "measuring…")).cyan(),
            ),
            ToolStatus::Missing(why) => (Line::from("—").gray(), Span::raw(why.clone()).gray()),
            ToolStatus::Unavailable(why) => {
                (Line::from("—").yellow(), Span::raw(why.clone()).yellow())
            }
            ToolStatus::Ready {
                reclaimable,
                detail,
            } => (
                Line::from(fmt_size(*reclaimable)).right_aligned().bold(),
                Span::raw(detail.clone()),
            ),
        };
        let name_style = match t.status {
            ToolStatus::Missing(_) => Style::new().fg(Color::Gray),
            _ => Style::new().fg(Color::White).bold(),
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
    .row_highlight_style(HIGHLIGHT)
    .highlight_symbol("▶ ")
    .block(table_block_plain());
    f.render_stateful_widget(table, list_area, &mut view.table);

    let mut lines = Vec::new();
    match view.selected() {
        Some(t) if !t.actions.is_empty() => {
            for a in &t.actions {
                let (style, text) = risk_style(a.risk);
                lines.push(Line::from(vec![
                    Span::styled(text, style),
                    Span::raw(format!(" {}", a.label)).white().bold(),
                ]));
                for step in &a.steps {
                    lines.push(Line::from(format!("    $ {}", step.describe())).gray());
                }
            }
        }
        Some(t) if t.status == ToolStatus::Measuring => {
            lines.push(Line::from(t!("Ölçülüyor…", "Measuring…")).cyan());
        }
        _ => lines.push(
            Line::from(t!(
                "Bu araç için yapılacak bir şey yok.",
                "Nothing to do for this tool."
            ))
            .gray(),
        ),
    }
    if let Some(run) = &view.run {
        // Show the end of the log.
        let visible = detail_area.height.saturating_sub(2) as usize;
        let start = run.log.len().saturating_sub(visible);
        let lines: Vec<Line<'_>> = run.log[start..]
            .iter()
            .map(|l| {
                if l.starts_with('$') {
                    Line::from(l.clone()).white().bold()
                } else if l.contains('✗') {
                    Line::from(l.clone()).red()
                } else if l.contains('✓') {
                    Line::from(l.clone()).green()
                } else {
                    Line::from(l.clone()).gray()
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
            Paragraph::new(lines).block(panel(t!(" İşlemler ", " Actions "), false)),
            detail_area,
        );
    }

    if let Some(p) = &view.picker {
        let tool = &view.tools[p.tool];
        let mut lines = vec![Line::from("")];
        for (i, a) in tool.actions.iter().enumerate() {
            let (style, text) = risk_style(a.risk);
            let check = if p.checked[i] { "[✓] " } else { "[ ] " };
            let row = Line::from(vec![
                Span::raw(if i == p.cursor { "▶ " } else { "  " }),
                Span::raw(check).bold(),
                Span::styled(text, style),
                Span::raw(format!(" {}", a.label)).white(),
            ]);
            lines.push(if i == p.cursor {
                row.style(HIGHLIGHT)
            } else {
                row
            });
        }
        lines.push(Line::from(""));
        lines.push(
            Line::from(t!(
                " Space: seç   Enter: devam   Esc: vazgeç",
                " Space: select   Enter: continue   Esc: cancel",
            ))
            .gray(),
        );
        popup(
            f,
            &tf!(
                " {} — ne temizlensin? ",
                " {} — what to clean? ",
                tool.kind.label()
            ),
            Color::Cyan,
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
            .white(),
        ];
        for a in &c.actions {
            let (style, text) = risk_style(a.risk);
            lines.push(Line::from(vec![
                Span::styled(text, style),
                Span::raw(format!(" {}", a.label)).white().bold(),
            ]));
            for step in &a.steps {
                lines.push(Line::from(format!("    $ {}", step.describe())).gray());
            }
        }
        lines.push(Line::from(""));
        match &c.typed {
            Some(typed) => {
                lines.push(
                    Line::from(t!(
                        "Seçimde VERİ KAYBI olan bir işlem var. Onaylamak için „evet” yazıp Enter'a basın:",
                        "The selection includes a DATA LOSS action. Type “yes” and press Enter to confirm:",
                    ))
                        .red()
                        .bold(),
                );
                let ok = typed.trim() == confirm_word();
                lines.push(Line::from(vec![
                    Span::raw(" > "),
                    Span::raw(format!("{typed}█")).white().bold(),
                    Span::raw(if ok {
                        t!("   Enter: çalıştır", "   Enter: run")
                    } else {
                        ""
                    })
                    .green(),
                ]));
                lines.push(Line::from(t!(" Esc: vazgeç", " Esc: cancel")).gray());
            }
            None => lines.push(Line::from(vec![
                Span::raw(t!(" e ", " y ")).black().on_red().bold(),
                Span::raw(t!(" evet, çalıştır     ", " yes, run     ")),
                Span::raw(t!(" h ", " n ")).black().on_gray(),
                Span::raw(t!(" vazgeç", " cancel")),
            ])),
        }
        popup(
            f,
            &tf!(" {} — onay ", " {} — confirm ", tool.kind.label()),
            Color::Red,
            lines,
            100,
        );
    }
}
