//! Colors and styles shared by the screens.

use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::Span;
use ratatui::widgets::Cell;

use crate::stats;
use crate::tools::Risk;

use super::format::fmt_date;

pub(super) const HIGHLIGHT: Style = Style::new()
    .bg(Color::DarkGray)
    .add_modifier(Modifier::BOLD);

/// Colors of the age groups of `stats::age_label`: fresh is green,
/// untouched for over a year is red.
pub(super) const AGE_COLORS: [Color; 5] = [
    Color::Green,
    Color::Cyan,
    Color::Yellow,
    Color::Red,
    Color::Gray,
];

/// Color for a timestamp of the given age in seconds.
pub(super) fn age_color(age: u64) -> Color {
    AGE_COLORS[stats::age_group(age)]
}

pub(super) fn date_cell(secs: u32, now: u64) -> Cell<'static> {
    Cell::from(date_span(secs, now))
}

/// Text color that stays readable on `bg`.
pub(super) fn text_on(bg: Color) -> Color {
    match bg {
        Color::Blue | Color::Magenta | Color::Red | Color::DarkGray | Color::Black => Color::White,
        Color::Indexed(n) if indexed_luma(n) < 140.0 => Color::White,
        _ => Color::Black,
    }
}

/// Perceived brightness (0–255) of an xterm 256-color palette entry.
fn indexed_luma(n: u8) -> f64 {
    const LEVELS: [f64; 6] = [0.0, 95.0, 135.0, 175.0, 215.0, 255.0];
    let (r, g, b) = match n {
        16..=231 => {
            let i = usize::from(n - 16);
            (LEVELS[i / 36], LEVELS[(i / 6) % 6], LEVELS[i % 6])
        }
        232..=255 => {
            let v = 8.0 + 10.0 * f64::from(n - 232);
            (v, v, v)
        }
        _ => (128.0, 128.0, 128.0),
    };
    0.299 * r + 0.587 * g + 0.114 * b
}

/// Folder colors in the treemap: distinct mid-dark hues (xterm 256-color
/// palette, which Terminal.app supports, unlike 24-bit color). Files keep
/// the lighter type colors, so folders and files stay apart.
pub(super) const DIR_COLORS: [Color; 12] = [
    Color::Indexed(25),  // blue
    Color::Indexed(130), // orange
    Color::Indexed(29),  // green
    Color::Indexed(90),  // magenta
    Color::Indexed(31),  // teal
    Color::Indexed(124), // red
    Color::Indexed(60),  // slate
    Color::Indexed(64),  // olive
    Color::Indexed(54),  // purple
    Color::Indexed(94),  // brown
    Color::Indexed(23),  // dark teal
    Color::Indexed(89),  // wine
];

pub(super) const CATEGORY_COLORS: [Color; 8] = [
    Color::Magenta,
    Color::LightMagenta,
    Color::LightBlue,
    Color::LightYellow,
    Color::LightGreen,
    Color::LightCyan,
    Color::LightRed,
    Color::Gray,
];

pub(super) fn risk_style(risk: Risk) -> (Style, &'static str) {
    match risk {
        Risk::Safe => (
            Style::new().fg(Color::Black).bg(Color::Green),
            t!(" güvenli ", " safe "),
        ),
        Risk::Redownload => (
            Style::new().fg(Color::Black).bg(Color::Yellow),
            t!(" yeniden indirilir ", " re-downloaded "),
        ),
        Risk::DataLoss => (
            Style::new().fg(Color::White).bg(Color::Red).bold(),
            t!(" VERİ KAYBI ", " DATA LOSS "),
        ),
    }
}

pub(super) fn usage_color(ratio: f64) -> Color {
    match ratio {
        r if r >= 0.9 => Color::Red,
        r if r >= 0.75 => Color::Yellow,
        _ => Color::Green,
    }
}

pub(super) fn date_span(secs: u32, now: u64) -> Span<'static> {
    if secs == 0 {
        return Span::raw("—").gray();
    }
    let age = now.saturating_sub(u64::from(secs));
    Span::styled(fmt_date(secs), Style::new().fg(age_color(age)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::DAY;

    #[test]
    fn colors_by_age() {
        assert_eq!(age_color(0), Color::Green);
        assert_eq!(age_color(7 * DAY), Color::Green);
        assert_eq!(age_color(8 * DAY), Color::Cyan);
        assert_eq!(age_color(100 * DAY), Color::Yellow);
        assert_eq!(age_color(400 * DAY), Color::Red);
    }

    #[test]
    fn folder_colors_get_readable_text() {
        // Dark palette entries get white text, light ones black.
        for c in DIR_COLORS {
            assert_eq!(text_on(c), Color::White, "{c:?}");
        }
        assert_eq!(text_on(Color::Indexed(231)), Color::Black); // white
        assert_eq!(text_on(Color::Indexed(16)), Color::White); // black
        assert_eq!(text_on(Color::LightYellow), Color::Black);
    }
}
