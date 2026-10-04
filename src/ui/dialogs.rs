//! Dialogs: delete confirmation, uninstall, failures.

use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::FailureDialog;
use crate::tree::{NodeId, SizeMode, Tree};

use super::format::{fmt_count, fmt_size, tilde, truncate_path, wrap_indented};
use super::style::Themed;
use super::theme::theme;

/// The entries a deletion could not move, with full, wrapped errors.
pub(super) fn render_failures(f: &mut Frame<'_>, d: &mut FailureDialog, area: Rect) {
    let width = area.width.saturating_sub(6).min(110);
    let height = area.height.saturating_sub(4).max(8);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw(tf!(
                " ✗ {} öğe taşınamadı",
                " ✗ {} items could not be moved",
                d.items.len()
            ))
            .danger()
            .bold(),
            Span::raw(tf!(
                "   ·   ✓ {} öğe taşındı ({})",
                "   ·   ✓ {} items moved ({})",
                d.moved,
                d.size
            ))
            .success(),
        ]),
        Line::from(""),
    ];
    // Every line is wrapped here, with continuation lines indented under
    // their text, so each fits the box.
    let inner_width = usize::from(width.saturating_sub(2)).max(1);
    let wrap = |text: &str, first: &str, rest: &str| wrap_indented(text, inner_width, first, rest);
    // Errors sharing a hint get it once, after the last of them.
    for (i, item) in d.items.iter().enumerate() {
        let name = std::path::Path::new(&item.path)
            .file_name()
            .map_or(item.path.clone(), |n| n.to_string_lossy().into_owned());
        let number = format!(" {}. ", i + 1);
        let indent = " ".repeat(number.chars().count());
        for l in wrap(&name, &number, &indent) {
            lines.push(Line::from(l).normal().bold());
        }
        for l in wrap(&item.path, "    ", "    ") {
            lines.push(Line::from(l).muted());
        }
        for l in wrap(&item.error, "    ", "    ") {
            lines.push(Line::from(l).normal());
        }
        let next_hint = d.items.get(i + 1).and_then(|n| n.hint);
        if let Some(hint) = item.hint.filter(|h| Some(*h) != next_hint) {
            const ARROW: &str = "    → ";
            for (n, l) in wrap(hint, ARROW, "      ").into_iter().enumerate() {
                lines.push(match l.strip_prefix(ARROW) {
                    Some(text) if n == 0 => Line::from(vec![
                        Span::raw(ARROW).warn().bold(),
                        Span::raw(text.to_string()).warn(),
                    ]),
                    _ => Line::from(l).warn(),
                });
            }
        }
        lines.push(Line::from(""));
    }

    // Keep scrolling within the text.
    let total: usize = lines
        .iter()
        .map(|l| l.width().div_ceil(inner_width).max(1))
        .sum();
    let visible = usize::from(height.saturating_sub(3));
    let max_scroll = total.saturating_sub(visible) as u16;
    d.scroll = d.scroll.min(max_scroll);

    let footer = if max_scroll > 0 {
        t!(
            " ↑↓ kaydır · Esc / Enter: kapat ",
            " ↑↓ scroll · Esc / Enter: close "
        )
    } else {
        t!(" Esc / Enter: kapat ", " Esc / Enter: close ")
    };
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((d.scroll, 0))
            .block(
                Block::bordered()
                    .title(
                        Span::raw(t!(" Taşınamayan öğeler ", " Items not moved "))
                            .normal()
                            .bold(),
                    )
                    .title_bottom(Line::from(footer).centered().muted())
                    .border_style(Style::new().fg(theme().danger)),
            ),
        rect,
    );
}

pub(super) fn render_confirm(
    f: &mut Frame<'_>,
    tree: &Tree,
    ids: &[NodeId],
    mode: SizeMode,
    area: Rect,
) {
    const LISTED: usize = 6;
    let width = area.width.saturating_sub(4).min(76);
    let inner_width = width.saturating_sub(4) as usize;
    let size: u64 = ids.iter().map(|&id| tree.node(id).size.get(mode)).sum();
    let files: u64 = ids
        .iter()
        .map(|&id| u64::from(tree.node(id).file_count))
        .sum();

    let mut lines = vec![Line::from("")];
    if let [id] = ids {
        lines.push(Line::from(truncate_path(&tree.path_of(*id), inner_width)).bold());
    } else {
        lines.push(Line::from(tf!("{} öğe", "{} items", fmt_count(ids.len() as u64))).bold());
        lines.push(Line::from(""));
        for &id in ids.iter().take(LISTED) {
            lines.push(Line::from(truncate_path(&tree.path_of(id), inner_width)).muted());
        }
        if ids.len() > LISTED {
            lines.push(
                Line::from(tf!(
                    "… ve {} öğe daha",
                    "… and {} more",
                    fmt_count((ids.len() - LISTED) as u64)
                ))
                .muted(),
            );
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(tf!(
        "Toplam boyut: {}",
        "Total size: {}",
        fmt_size(size)
    )));
    if files > 0 {
        lines.push(Line::from(tf!(
            "İçerdiği dosya: {}",
            "Files inside: {}",
            fmt_count(files)
        )));
    }
    lines.extend([
        Line::from(""),
        Line::from(t!("Çöp kutusuna taşınacak.", "Will be moved to the trash.")).muted(),
        Line::from(""),
        Line::from(vec![
            Span::raw(t!(" e ", " y ")).key_danger(),
            Span::raw(t!(" evet, taşı     ", " yes, move     ")),
            Span::raw(t!(" h ", " n ")).key(),
            Span::raw(t!(" vazgeç", " cancel")),
        ]),
    ]);

    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .centered()
            .block(
                Block::bordered()
                    .title(t!(" Çöp kutusuna taşınsın mı? ", " Move to the trash? "))
                    .border_style(Style::new().fg(theme().danger)),
            ),
        popup,
    );
}

/// The app and its data to uninstall, each with a check box.
pub(super) fn render_uninstall(
    f: &mut Frame<'_>,
    tree: &Tree,
    d: &crate::app::UninstallDialog,
    mode: SizeMode,
    area: Rect,
) {
    let width = area.width.saturating_sub(4).min(100);
    let inner = width.saturating_sub(4) as usize;
    let size_of = |id: NodeId| tree.node(id).size.get(mode);
    let chosen: Vec<NodeId> = d.chosen();
    let total: u64 = chosen.iter().map(|&id| size_of(id)).sum();

    let mut tail = vec![
        Line::from(""),
        Line::from(tf!(
            "Seçili: {} / {} öğe · {}",
            "Selected: {} of {} items · {}",
            chosen.len(),
            d.items.len(),
            fmt_size(total)
        ))
        .bold(),
        Line::from(t!(
            "Veriler ad ve paket kimliğiyle eşleştirildi (tahmin): listeyi kontrol edin.",
            "Data was matched by name and bundle id (a guess): check the list.",
        ))
        .muted(),
    ];
    if d.running {
        tail.push(
            Line::from(t!(
                "⚠ Uygulama şu an açık: önce kapatın.",
                "⚠ The app is running: quit it first.",
            ))
            .danger()
            .bold(),
        );
    }
    tail.extend([
        Line::from(t!("Çöp kutusuna taşınacak.", "Will be moved to the trash.")).muted(),
        Line::from(""),
        Line::from(vec![
            Span::raw(t!(" e ", " y ")).key_danger(),
            Span::raw(t!(" kaldır     ", " uninstall     ")),
            Span::raw(" Space ").key(),
            Span::raw(t!(" işaretle     ", " check     ")),
            Span::raw(t!(" h ", " n ")).key(),
            Span::raw(t!(" vazgeç", " cancel")),
        ]),
    ]);

    // Borders, a blank line on top, the tail; the rest lists the items.
    let room = (area.height as usize).saturating_sub(tail.len() + 4).max(3);
    let shown = d.items.len().min(room);
    let offset = (d.cursor + 1).saturating_sub(shown);
    let mut lines = vec![Line::from("")];
    for (i, &(id, checked)) in d.items.iter().enumerate().skip(offset).take(shown) {
        let mark = if checked { "[✓] " } else { "[ ] " };
        let size = fmt_size(size_of(id));
        let room = inner.saturating_sub(4 + size.len() + 2);
        let path = truncate_path(&tilde(&tree.path_of(id)), room);
        let pad = inner.saturating_sub(4 + path.chars().count() + size.chars().count());
        let mut line = Line::from(format!("{mark}{path}{}{size}", " ".repeat(pad)));
        if !checked {
            line = line.muted();
        }
        if i == d.cursor {
            line = line.reversed();
        }
        lines.push(line);
    }
    if d.items.len() > shown {
        lines.push(
            Line::from(tf!(
                "{}–{} / {} (↑↓ kaydır)",
                "{}–{} of {} (↑↓ to scroll)",
                offset + 1,
                offset + shown,
                d.items.len()
            ))
            .muted(),
        );
    }
    lines.extend(tail);

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
                .padding(ratatui::widgets::Padding::horizontal(1))
                .title(tf!(" Kaldır: {} ", " Uninstall: {} ", d.label))
                .border_style(Style::new().fg(theme().danger)),
        ),
        popup,
    );
}
