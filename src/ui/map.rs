//! The treemap view of a folder.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::Frame;

use crate::app::{Browser, MapColor};
use crate::stats;
use crate::treemap::Slot;

use super::format::{clip, fmt_pct, fmt_size, now_secs};
use super::style::{age_color, category_color, dir_color, text_on, Themed};
use super::theme::theme;

pub(super) fn render_map(f: &mut Frame<'_>, b: &mut Browser, area: Rect) {
    let [map_area, legend_area] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
    b.map_area = map_area;
    let blocks = b.map_blocks();
    let selected = b.selected_block();
    let (tree, mode) = (&b.tree, b.size_mode);
    let total = tree.node(b.current).size.get(mode).max(1);
    let now = now_secs();

    for (slot, rect) in &blocks {
        let (bg, name, size, in_basket) = match *slot {
            Slot::Item(i) => {
                let id = b.entries[i];
                let n = tree.node(id);
                let bg = match b.map_color {
                    MapColor::Age => age_color(now.saturating_sub(u64::from(n.modified))),
                    // Entries are in size order, so neighbours get different colors.
                    MapColor::Kind if n.is_dir => dir_color(i),
                    MapColor::Kind => category_color(stats::Category::of(tree.name(id)) as usize),
                };
                let mut name = tree.name(id).to_string();
                if n.is_dir {
                    name.push('/');
                }
                (bg, name, n.size.get(mode), b.basket.covers(tree, id))
            }
            Slot::Other { count } => {
                let shown: Vec<usize> = blocks
                    .iter()
                    .filter_map(|(s, _)| match s {
                        Slot::Item(i) => Some(*i),
                        _ => None,
                    })
                    .collect();
                let size: u64 = (0..b.entries.len())
                    .filter(|i| !shown.contains(i))
                    .map(|i| tree.node(b.entries[i]).size.get(mode))
                    .sum();
                (
                    theme().other_block,
                    tf!(
                        "diğer ({count} küçük öğe — Enter: listede gör)",
                        "other ({count} small items — Enter: show in list)"
                    ),
                    size,
                    false,
                )
            }
        };
        let fg = text_on(bg);
        let is_selected = selected == Some(*slot);
        let style = Style::new().bg(bg).fg(fg);
        let mut inner = *rect;
        // Without colors every block gets a border, so blocks stay apart.
        let framed = theme().framed_blocks;
        if (is_selected || framed) && rect.width >= 3 && rect.height >= 3 {
            let (kind, border) = if is_selected {
                let white = if framed { Color::Reset } else { Color::White };
                (BorderType::Thick, Style::new().fg(white).bg(bg).bold())
            } else {
                (BorderType::Plain, style)
            };
            f.render_widget(
                Block::bordered()
                    .border_type(kind)
                    .border_style(border)
                    .style(style),
                *rect,
            );
            inner = rect.inner(ratatui::layout::Margin::new(1, 1));
        } else {
            f.render_widget(Block::new().style(style), *rect);
        }
        let width = usize::from(inner.width);
        let mark = if in_basket { "✓ " } else { "" };
        let mut lines = vec![Line::from(clip(&format!("{mark}{name}"), width)).bold()];
        if inner.height >= 2 {
            let pct = size as f64 / total as f64 * 100.0;
            lines.push(Line::from(clip(
                &format!("{} · {}", fmt_size(size), fmt_pct(pct, 1)),
                width,
            )));
        }
        let text_style = if is_selected && inner == *rect {
            // Too small for a border: show the selection by inverting.
            if framed {
                style.add_modifier(Modifier::REVERSED)
            } else {
                Style::new().bg(Color::White).fg(Color::Black)
            }
        } else {
            style
        };
        f.render_widget(Paragraph::new(lines).style(text_style), inner);
    }

    let mut legend = vec![
        Span::raw(" t / Esc ").key(),
        Span::raw(t!(" listeye dön   ", " back to list   ")).normal(),
        Span::raw(t!("Renk: ", "Color: ")).normal().bold(),
    ];
    match b.map_color {
        MapColor::Kind => {
            for color in theme().dirs.iter().take(4) {
                legend.push(Span::styled("█", Style::new().fg(*color)));
            }
            legend.push(
                Span::raw(t!(
                    " klasörler (her biri ayrı)  ",
                    " folders (each its own)  "
                ))
                .normal(),
            );
            for (cat, color) in stats::Category::ALL.iter().zip(theme().categories) {
                legend.push(Span::styled("██", Style::new().fg(color)));
                legend.push(Span::raw(format!(" {}  ", cat.label())).normal());
            }
            legend.push(Span::raw(t!("  (c: yaşa göre)", "  (c: by age)")).muted());
        }
        MapColor::Age => {
            for (color, text) in theme().ages.iter().zip((0..4).map(stats::age_label)) {
                legend.push(Span::styled("██", Style::new().fg(*color)));
                legend.push(Span::raw(format!(" {text}  ")).normal());
            }
            legend.push(Span::raw(t!("  (c: türe göre)", "  (c: by type)")).muted());
        }
    }
    f.render_widget(Line::from(legend), legend_area);
}
