//! Rendering: picks the screen to draw and holds the helpers the screens
//! share (title and key lines, popups, panels, bars).

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, Screen};
use crate::stats;

mod browser;
#[cfg(all(test, unix))]
pub use browser::footer_keys;
mod dashboard;
mod dialogs;
mod disks;
mod format;
pub mod help;
mod log;
mod map;
mod menus;
mod results;
mod scanning;
mod style;
mod system;
pub mod theme;
mod tools;

// Formatting helpers used outside `ui`.
#[cfg(all(test, unix))]
pub use format::with_fixed_now;
pub use format::{fmt_ago, fmt_count, fmt_date, fmt_delta, fmt_size, now_secs};

use browser::render_browser;
use disks::render_disks;
use scanning::render_scanning;
use style::Themed;
use theme::theme;

const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

pub fn render(f: &mut Frame<'_>, app: &mut App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(f.area());

    app.mouse.clear();
    match app.screen {
        Screen::DiskSelect => render_disks(f, app, header, body, footer),
        Screen::Scanning => render_scanning(f, app, header, body, footer),
        Screen::Browser => render_browser(f, app, header, body, footer),
    }
    if let Some(mut scroll) = app.help {
        // Nothing behind the help is clickable.
        app.mouse.clear();
        help::render_help(f, app, &mut scroll);
        app.help = Some(scroll);
    }
}

fn title(text: String) -> Line<'static> {
    Line::from(vec![
        Span::raw(" rustClean ").badge(),
        Span::raw(" "),
        Span::raw(text),
    ])
}

/// The key hints of the bottom line, after `?  help`.
fn keys(pairs: &[(&str, &str)]) -> Line<'static> {
    let mut all = vec![("?", t!("yardım", "help"))];
    all.extend_from_slice(pairs);
    keys_without_help(&all)
}

/// Key hints without `?`, for when `?` is typed as text (the search box).
fn keys_without_help(pairs: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (k, desc) in pairs {
        spans.push(Span::raw(format!(" {k} ")).key());
        spans.push(Span::raw(format!(" {desc}  ")));
    }
    Line::from(spans)
}

/// Terminal width from which the creation date column is shown.
const WIDE: u16 = 110;

const DATE_WIDTH: u16 = 16;

/// Top/bottom rule with the date color legend on the top edge.
fn table_block() -> Block<'static> {
    let label = Style::new().fg(theme().text);
    let mut legend = vec![Span::styled(t!(" Tarih: ", " Date: "), label.bold())];
    for (color, text) in theme().ages.iter().zip((0..4).map(stats::age_label)) {
        legend.push(Span::styled("██", Style::new().fg(*color)));
        legend.push(Span::styled(format!(" {text}   "), label));
    }
    Block::new()
        .borders(Borders::TOP | Borders::BOTTOM)
        .title_top(Line::from(legend).right_aligned())
}

/// A centered bordered box sized to its lines.
fn popup(f: &mut Frame<'_>, title: &str, color: Color, lines: Vec<Line<'_>>, max_width: u16) {
    let area = f.area();
    let width = area.width.saturating_sub(4).min(max_width);
    let height = (lines.len() as u16 + 2).min(area.height);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .title(Span::raw(title.to_string()).normal().bold())
                .border_style(Style::new().fg(color)),
        ),
        rect,
    );
}

/// Where the rows of a table drawn with `table_block` (or
/// `table_block_plain`) and a header line are, for mouse clicks.
fn table_rows(area: Rect) -> Rect {
    Rect {
        y: area.y.saturating_add(2),
        height: area.height.saturating_sub(3),
        ..area
    }
}

/// Where the rows of a table without a header inside `panel` are.
fn panel_rows(area: Rect) -> Rect {
    area.inner(ratatui::layout::Margin::new(1, 1))
}

fn table_block_plain() -> Block<'static> {
    Block::new().borders(Borders::TOP | Borders::BOTTOM)
}

fn halves(area: Rect) -> [Rect; 2] {
    Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area)
}

/// Bordered panel; the focused one gets a highlighted border.
fn panel(title: &str, focused: bool) -> Block<'static> {
    let border = if focused {
        Style::new().fg(theme().accent)
    } else {
        Style::new().fg(theme().dim)
    };
    Block::bordered()
        .title(Span::raw(title.to_string()).normal().bold())
        .border_style(border)
}

/// Horizontal bar of `width` cells using eighth-block characters.
fn bar(ratio: f64, width: usize) -> String {
    const PARTS: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
    let eighths = (ratio.clamp(0.0, 1.0) * width as f64 * 8.0).round() as usize;
    let full = eighths / 8;
    let mut s = "█".repeat(full);
    if full < width {
        s.push(PARTS[eighths % 8]);
        s.push_str(&" ".repeat(width - full - 1));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_has_fixed_width() {
        for r in [0.0, 0.03, 0.5, 0.999, 1.0] {
            assert_eq!(bar(r, 10).chars().count(), 10);
        }
    }
}
