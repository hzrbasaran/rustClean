//! Colors and styles shared by the screens.

use ratatui::style::{Color, Style, Stylize};
use ratatui::text::Span;
use ratatui::widgets::Cell;

use crate::stats;
use crate::tools::Risk;

use super::format::fmt_date;
use super::theme::theme;

/// The style of the selected row.
pub(super) fn highlight() -> Style {
    theme().selected
}

/// Color of an age group of `stats::age_label`: fresh is green, untouched
/// for over a year is red (in the dark theme).
pub(super) fn age_colors() -> [Color; 5] {
    theme().ages
}

/// Color for a timestamp of the given age in seconds.
pub(super) fn age_color(age: u64) -> Color {
    theme().ages[stats::age_group(age)]
}

pub(super) fn date_cell(secs: u32, now: u64) -> Cell<'static> {
    Cell::from(date_span(secs, now))
}

/// Text color that stays readable on `bg`.
pub(super) fn text_on(bg: Color) -> Color {
    match bg {
        // No background color: the terminal's own text color.
        Color::Reset => Color::Reset,
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

/// Treemap color of folder number `i`.
pub(super) fn dir_color(i: usize) -> Color {
    let dirs = theme().dirs;
    dirs[i % dirs.len()]
}

/// Color of a file category (`stats::Category as usize`).
pub(super) fn category_color(i: usize) -> Color {
    theme().categories[i]
}

pub(super) fn risk_style(risk: Risk) -> (Style, &'static str) {
    let risks = theme().risks;
    match risk {
        Risk::Safe => (risks[0], t!(" güvenli ", " safe ")),
        Risk::Redownload => (risks[1], t!(" yeniden indirilir ", " re-downloaded ")),
        Risk::DataLoss => (risks[2], t!(" VERİ KAYBI ", " DATA LOSS ")),
    }
}

pub(super) fn usage_color(ratio: f64) -> Color {
    let usage = theme().usage;
    match ratio {
        r if r >= 0.9 => usage[2],
        r if r >= 0.75 => usage[1],
        _ => usage[0],
    }
}

pub(super) fn date_span(secs: u32, now: u64) -> Span<'static> {
    if secs == 0 {
        return Span::raw("—").muted();
    }
    let age = now.saturating_sub(u64::from(secs));
    Span::styled(fmt_date(secs), Style::new().fg(age_color(age)))
}

/// Colors by role, for anything ratatui can style (spans, lines, cells).
pub(super) trait Themed<'a, T>: Stylize<'a, T> + Sized {
    /// Labels and values.
    fn normal(self) -> T {
        self.fg(theme().text)
    }
    /// Secondary text.
    fn muted(self) -> T {
        self.fg(theme().muted)
    }
    fn accent(self) -> T {
        self.fg(theme().accent)
    }
    fn success(self) -> T {
        self.fg(theme().ok)
    }
    fn warn(self) -> T {
        self.fg(theme().warn)
    }
    fn danger(self) -> T {
        self.fg(theme().danger)
    }
    /// A key in the key hints (`m`, `Enter`, …).
    fn key(self) -> T
    where
        T: Stylize<'a, T>,
    {
        apply(self, theme().key)
    }
    /// The "yes" key of a dangerous question.
    fn key_danger(self) -> T
    where
        T: Stylize<'a, T>,
    {
        apply(self, theme().key_danger)
    }
    /// A key on a warning line.
    fn key_warn(self) -> T
    where
        T: Stylize<'a, T>,
    {
        apply(self, theme().key_warn)
    }
    /// The `rustClean` badge of the title line.
    fn badge(self) -> T
    where
        T: Stylize<'a, T>,
    {
        apply(self, theme().badge)
    }
}

/// Gives `x` the colors and modifiers of `style`.
fn apply<'a, T: Stylize<'a, T>, U: Stylize<'a, T>>(x: U, style: Style) -> T {
    x.fg(style.fg.unwrap_or(Color::Reset))
        .bg(style.bg.unwrap_or(Color::Reset))
        .add_modifier(style.add_modifier)
}

impl<'a, T, U: Stylize<'a, T>> Themed<'a, T> for U {}

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
        for c in super::super::theme::DARK.dirs {
            assert_eq!(text_on(c), Color::White, "{c:?}");
        }
        assert_eq!(text_on(Color::Indexed(231)), Color::Black); // white
        assert_eq!(text_on(Color::Indexed(16)), Color::White); // black
        assert_eq!(text_on(Color::LightYellow), Color::Black);
    }
}
