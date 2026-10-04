//! Color themes. Screens ask for colors by role (`text`, `muted`, `warn`, …)
//! and the current theme answers: dark (the default), light, color-blind
//! friendly, or no color at all (`--no-color` / `NO_COLOR`).
//!
//! The light and color-blind themes use xterm 256-color entries, which look
//! the same in every terminal; the 16 named colors of the dark theme follow
//! the terminal's own palette.

use std::sync::atomic::{AtomicU8, Ordering};

use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeKind {
    Dark,
    Light,
    ColorBlind,
    /// No colors: `--no-color` or `NO_COLOR`.
    Mono,
}

impl ThemeKind {
    const ALL: [ThemeKind; 4] = [
        ThemeKind::Dark,
        ThemeKind::Light,
        ThemeKind::ColorBlind,
        ThemeKind::Mono,
    ];

    pub fn code(self) -> &'static str {
        match self {
            ThemeKind::Dark => "dark",
            ThemeKind::Light => "light",
            ThemeKind::ColorBlind => "colorblind",
            ThemeKind::Mono => "none",
        }
    }

    /// The themes `--theme` and the settings accept (not `none`: no color
    /// is chosen with `--no-color` or `NO_COLOR`).
    pub fn parse(s: &str) -> Option<ThemeKind> {
        match s.trim().to_ascii_lowercase().as_str() {
            "dark" => Some(ThemeKind::Dark),
            "light" => Some(ThemeKind::Light),
            "colorblind" | "color-blind" | "cb" => Some(ThemeKind::ColorBlind),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeKind::Dark => t!("Koyu", "Dark"),
            ThemeKind::Light => t!("Açık", "Light"),
            ThemeKind::ColorBlind => t!("Renk körü dostu", "Color-blind friendly"),
            ThemeKind::Mono => t!("Renksiz", "No color"),
        }
    }

    /// The next theme for `T`: dark → light → color-blind → dark.
    pub fn next(self) -> ThemeKind {
        match self {
            ThemeKind::Dark => ThemeKind::Light,
            ThemeKind::Light => ThemeKind::ColorBlind,
            ThemeKind::ColorBlind | ThemeKind::Mono => ThemeKind::Dark,
        }
    }

    pub fn theme(self) -> &'static Theme {
        match self {
            ThemeKind::Dark => &DARK,
            ThemeKind::Light => &LIGHT,
            ThemeKind::ColorBlind => &COLOR_BLIND,
            ThemeKind::Mono => &MONO,
        }
    }
}

/// Colors and styles by role.
pub struct Theme {
    /// Labels and values.
    pub text: Color,
    /// Secondary text: hints, paths, details.
    pub muted: Color,
    /// Borders of inactive panels.
    pub dim: Color,
    /// Headings, menu borders, progress.
    pub accent: Color,
    /// Folder names in lists.
    pub folder: Color,
    pub ok: Color,
    pub warn: Color,
    pub danger: Color,
    /// The selected row.
    pub selected: Style,
    /// A key in the key hints (`m`, `Enter`, …).
    pub key: Style,
    /// The "yes" key of a dangerous question.
    pub key_danger: Style,
    /// A key on a warning line.
    pub key_warn: Style,
    /// The `rustClean` badge of the title line.
    pub badge: Style,
    /// Age groups of `stats::age_label`: fresh, ≤ 30 days, ≤ 1 year, older,
    /// unknown.
    pub ages: [Color; 5],
    /// Disk usage below 75 %, from 75 %, from 90 %.
    pub usage: [Color; 3],
    /// Risk badges of cleanup actions: safe, downloaded again, data loss.
    pub risks: [Style; 3],
    /// File categories, in `stats::Category` order.
    pub categories: [Color; 8],
    /// Folder blocks in the treemap; neighbours get different ones.
    pub dirs: [Color; 12],
    /// The treemap block of "other" small entries.
    pub other_block: Color,
    /// Draw a border around every treemap block (no colors to tell them
    /// apart).
    pub framed_blocks: bool,
}

const fn fg_bg(fg: Color, bg: Color) -> Style {
    Style::new().fg(fg).bg(bg)
}

const BOLD: Modifier = Modifier::BOLD;

/// The selected row on dark terminals: white bold text on a dark gray, so
/// colored text (dates, folder names) stays readable. The colors come back
/// when the cursor moves on.
const SELECTED_ON_DARK: Style = Style::new()
    .fg(Color::White)
    .bg(Color::Indexed(238))
    .add_modifier(BOLD);

/// The original colors, tuned for dark terminals.
pub static DARK: Theme = Theme {
    text: Color::White,
    muted: Color::Gray,
    dim: Color::DarkGray,
    accent: Color::Cyan,
    folder: Color::LightBlue,
    ok: Color::Green,
    warn: Color::Yellow,
    danger: Color::Red,
    selected: SELECTED_ON_DARK,
    key: fg_bg(Color::Black, Color::Gray),
    key_danger: fg_bg(Color::Black, Color::Red).add_modifier(BOLD),
    key_warn: fg_bg(Color::Black, Color::Yellow),
    badge: fg_bg(Color::Black, Color::Cyan).add_modifier(BOLD),
    ages: [
        Color::Green,
        Color::Cyan,
        Color::Yellow,
        Color::Red,
        Color::Gray,
    ],
    usage: [Color::Green, Color::Yellow, Color::Red],
    risks: [
        fg_bg(Color::Black, Color::Green),
        fg_bg(Color::Black, Color::Yellow),
        fg_bg(Color::White, Color::Red).add_modifier(BOLD),
    ],
    categories: [
        Color::Magenta,
        Color::LightMagenta,
        Color::LightBlue,
        Color::LightYellow,
        Color::LightGreen,
        Color::LightCyan,
        Color::LightRed,
        Color::Gray,
    ],
    dirs: DIRS,
    other_block: Color::DarkGray,
    framed_blocks: false,
};

/// Folder colors in the treemap: distinct mid-dark hues (xterm 256-color
/// palette, which Terminal.app supports, unlike 24-bit color). Files keep
/// the lighter type colors, so folders and files stay apart.
const DIRS: [Color; 12] = [
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

/// For light terminals: dark text colors that keep their contrast on white.
/// Plain text uses the terminal's own color.
pub static LIGHT: Theme = Theme {
    text: Color::Reset,
    muted: Color::Indexed(242),
    dim: Color::Indexed(248),
    accent: Color::Indexed(25),
    folder: Color::Indexed(26),
    ok: Color::Indexed(28),
    warn: Color::Indexed(130),
    danger: Color::Indexed(160),
    selected: Style::new().bg(Color::Indexed(253)).add_modifier(BOLD),
    key: fg_bg(Color::Indexed(231), Color::Indexed(242)),
    key_danger: fg_bg(Color::Indexed(231), Color::Indexed(160)).add_modifier(BOLD),
    key_warn: fg_bg(Color::Indexed(16), Color::Indexed(220)),
    badge: fg_bg(Color::Indexed(231), Color::Indexed(25)).add_modifier(BOLD),
    ages: [
        Color::Indexed(28),
        Color::Indexed(31),
        Color::Indexed(130),
        Color::Indexed(160),
        Color::Indexed(242),
    ],
    usage: [Color::Indexed(28), Color::Indexed(130), Color::Indexed(160)],
    risks: [
        fg_bg(Color::Indexed(231), Color::Indexed(28)),
        fg_bg(Color::Indexed(16), Color::Indexed(220)),
        fg_bg(Color::Indexed(231), Color::Indexed(160)).add_modifier(BOLD),
    ],
    categories: [
        Color::Indexed(127),
        Color::Indexed(133),
        Color::Indexed(32),
        Color::Indexed(178),
        Color::Indexed(70),
        Color::Indexed(37),
        Color::Indexed(167),
        Color::Indexed(245),
    ],
    dirs: DIRS,
    other_block: Color::Indexed(245),
    framed_blocks: false,
};

/// The Okabe–Ito palette, distinct with red-green color blindness: blue and
/// orange instead of green and red. Every color also has text beside it
/// (a date, a percentage, a label).
pub static COLOR_BLIND: Theme = Theme {
    text: Color::White,
    muted: Color::Gray,
    dim: Color::DarkGray,
    accent: Color::Indexed(74),
    folder: Color::Indexed(117),
    ok: Color::Indexed(39),
    warn: Color::Indexed(214),
    danger: Color::Indexed(202),
    selected: SELECTED_ON_DARK,
    key: fg_bg(Color::Black, Color::Gray),
    key_danger: fg_bg(Color::Black, Color::Indexed(202)).add_modifier(BOLD),
    key_warn: fg_bg(Color::Black, Color::Indexed(214)),
    badge: fg_bg(Color::Black, Color::Indexed(74)).add_modifier(BOLD),
    ages: [
        Color::Indexed(33),
        Color::Indexed(117),
        Color::Indexed(227),
        Color::Indexed(202),
        Color::Gray,
    ],
    usage: [Color::Indexed(33), Color::Indexed(214), Color::Indexed(202)],
    risks: [
        fg_bg(Color::Indexed(231), Color::Indexed(25)),
        fg_bg(Color::Black, Color::Indexed(214)),
        fg_bg(Color::Indexed(231), Color::Indexed(166)).add_modifier(BOLD),
    ],
    categories: [
        Color::Indexed(175),
        Color::Indexed(214),
        Color::Indexed(74),
        Color::Indexed(227),
        Color::Indexed(36),
        Color::Indexed(33),
        Color::Indexed(166),
        Color::Gray,
    ],
    dirs: [
        Color::Indexed(25),
        Color::Indexed(130),
        Color::Indexed(30),
        Color::Indexed(96),
        Color::Indexed(24),
        Color::Indexed(166),
        Color::Indexed(60),
        Color::Indexed(136),
        Color::Indexed(54),
        Color::Indexed(94),
        Color::Indexed(23),
        Color::Indexed(132),
    ],
    other_block: Color::DarkGray,
    framed_blocks: false,
};

const REV: Modifier = Modifier::REVERSED;

/// No colors: the terminal's own text color, with reversed and bold text
/// where colors would mark something.
pub static MONO: Theme = Theme {
    text: Color::Reset,
    muted: Color::Reset,
    dim: Color::Reset,
    accent: Color::Reset,
    folder: Color::Reset,
    ok: Color::Reset,
    warn: Color::Reset,
    danger: Color::Reset,
    selected: Style::new().add_modifier(REV.union(BOLD)),
    key: Style::new().add_modifier(REV),
    key_danger: Style::new().add_modifier(REV.union(BOLD)),
    key_warn: Style::new().add_modifier(REV),
    badge: Style::new().add_modifier(REV.union(BOLD)),
    ages: [Color::Reset; 5],
    usage: [Color::Reset; 3],
    risks: [
        Style::new().add_modifier(REV),
        Style::new().add_modifier(REV),
        Style::new().add_modifier(REV.union(BOLD)),
    ],
    categories: [Color::Reset; 8],
    dirs: [Color::Reset; 12],
    other_block: Color::Reset,
    framed_blocks: true,
};

static CURRENT: AtomicU8 = AtomicU8::new(0);

// Test builds can pick the theme per thread, like the language.
#[cfg(test)]
thread_local! {
    static TEST_THEME: std::cell::Cell<Option<ThemeKind>> = const { std::cell::Cell::new(None) };
}

pub fn current() -> ThemeKind {
    #[cfg(test)]
    if let Some(kind) = TEST_THEME.get() {
        return kind;
    }
    ThemeKind::ALL[usize::from(CURRENT.load(Ordering::Relaxed))]
}

/// The colors of the current theme.
pub fn theme() -> &'static Theme {
    current().theme()
}

#[cfg(not(test))]
pub fn set(kind: ThemeKind) {
    let i = ThemeKind::ALL.iter().position(|&k| k == kind).unwrap_or(0);
    CURRENT.store(u8::try_from(i).unwrap_or(0), Ordering::Relaxed);
}

/// In test builds `T` switches only the thread that pressed it.
#[cfg(test)]
pub fn set(kind: ThemeKind) {
    TEST_THEME.set(Some(kind));
}

/// Runs `f` with this thread's colors from `kind` (test builds only).
#[cfg(all(test, unix))]
pub fn with_theme<R>(kind: ThemeKind, f: impl FnOnce() -> R) -> R {
    let before = TEST_THEME.replace(Some(kind));
    let out = f();
    TEST_THEME.set(before);
    out
}

/// No colors were asked for: `--no-color`, or `NO_COLOR` set to anything
/// but an empty string (<https://no-color.org>).
pub fn no_color_requested(flag: bool) -> bool {
    flag || std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty())
}

/// The theme at start: no color if asked for, else `--theme`, else the
/// saved choice, else dark.
pub fn init(flag: Option<&str>, no_color: bool) {
    let kind = if no_color_requested(no_color) {
        ThemeKind::Mono
    } else {
        flag.and_then(ThemeKind::parse)
            .or_else(|| {
                crate::settings::get("theme")
                    .as_deref()
                    .and_then(ThemeKind::parse)
            })
            .unwrap_or(ThemeKind::Dark)
    };
    set(kind);
}

/// Remembers the theme for the next start.
pub fn save(kind: ThemeKind) {
    if kind != ThemeKind::Mono {
        crate::settings::set("theme", kind.code());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_cycles() {
        assert_eq!(ThemeKind::parse("Light"), Some(ThemeKind::Light));
        assert_eq!(ThemeKind::parse("color-blind"), Some(ThemeKind::ColorBlind));
        assert_eq!(ThemeKind::parse("none"), None);
        assert_eq!(ThemeKind::parse("pink"), None);
        let mut k = ThemeKind::Dark;
        for _ in 0..3 {
            k = k.next();
        }
        assert_eq!(k, ThemeKind::Dark);
        // Each kind has its own palette.
        assert!(std::ptr::eq(ThemeKind::Light.theme(), &LIGHT));
        assert!(std::ptr::eq(ThemeKind::Mono.theme(), &MONO));
    }

    #[test]
    fn set_is_per_thread_in_tests() {
        set(ThemeKind::Light);
        assert_eq!(current(), ThemeKind::Light);
        let other = std::thread::spawn(current).join().unwrap();
        assert_eq!(other, ThemeKind::Dark);
    }
}
