//! Text formatting of sizes, counts, dates, ages and paths.

use std::path::Path;

use crate::i18n::{lang, Lang};

// Test builds can stop the clock per thread, for screens that show ages.
#[cfg(test)]
thread_local! {
    static FIXED_NOW: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// Runs `f` with this thread's clock fixed at `now`, and dates shown in UTC
/// instead of the local time zone (test builds only; the screen snapshots,
/// which run on Unix, use it).
#[cfg(all(test, unix))]
pub fn with_fixed_now<R>(now: u64, f: impl FnOnce() -> R) -> R {
    let before = FIXED_NOW.replace(Some(now));
    let out = f();
    FIXED_NOW.set(before);
    out
}

pub fn now_secs() -> u64 {
    #[cfg(test)]
    if let Some(now) = FIXED_NOW.get() {
        return now;
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Local date and time, e.g. "03.10.2026 14:22".
pub fn fmt_date(secs: u32) -> String {
    fmt_date_in(lang(), secs)
}

/// "03.10.2026 14:22" in Turkish, "2026-10-03 14:22" in English.
fn fmt_date_in(lang: Lang, secs: u32) -> String {
    let pattern = match lang {
        Lang::Tr => "%d.%m.%Y %H:%M",
        Lang::En => "%Y-%m-%d %H:%M",
    };
    #[cfg(test)]
    if FIXED_NOW.get().is_some() {
        return chrono::DateTime::from_timestamp(i64::from(secs), 0)
            .map_or_else(|| "—".into(), |t| t.format(pattern).to_string());
    }
    chrono::DateTime::from_timestamp(i64::from(secs), 0).map_or_else(
        || "—".into(),
        |t| t.with_timezone(&chrono::Local).format(pattern).to_string(),
    )
}

/// A percentage: "%51.0" in Turkish, "51.0%" in English.
pub fn fmt_pct(value: f64, decimals: usize) -> String {
    match lang() {
        Lang::Tr => format!("%{value:.decimals$}"),
        Lang::En => format!("{value:.decimals$}%"),
    }
}

/// Wraps `text` at spaces to lines of at most `width` characters: the first
/// line starts with `first`, the others with `rest` (a hanging indent).
/// Words longer than a line, such as paths, are split.
pub(super) fn wrap_indented(text: &str, width: usize, first: &str, rest: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = first.to_string();
    // Nothing but the prefix on `line` yet.
    let mut fresh = true;
    for word in text.split(' ').filter(|w| !w.is_empty()) {
        let mut word: Vec<char> = word.chars().collect();
        loop {
            let room = width.saturating_sub(line.chars().count() + usize::from(!fresh));
            if word.len() <= room {
                if !fresh {
                    line.push(' ');
                }
                line.extend(&word);
                fresh = false;
                break;
            }
            if fresh {
                // Longer than a whole line: fill this one with its start.
                let take = room.max(1);
                line.extend(word.drain(..take));
            }
            lines.push(std::mem::replace(&mut line, rest.to_string()));
            fresh = true;
            if word.is_empty() {
                break;
            }
        }
    }
    if !fresh || lines.is_empty() {
        lines.push(line);
    }
    lines
}

/// Cuts `s` to at most `width` characters.
pub(super) fn clip(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        s.to_string()
    } else if width <= 1 {
        s.chars().take(width).collect()
    } else {
        let mut out: String = s.chars().take(width - 1).collect();
        out.push('…');
        out
    }
}

/// The path with the home folder shortened to `~`.
pub fn tilde(path: &Path) -> std::path::PathBuf {
    match dirs::home_dir().and_then(|h| path.strip_prefix(h).ok().map(Path::to_path_buf)) {
        Some(rest) => Path::new("~").join(rest),
        None => path.to_path_buf(),
    }
}

/// Signed size difference: "+1.2 GiB", "−300.0 MiB", "değişmedi".
pub fn fmt_delta(now: u64, then: u64) -> String {
    match now.cmp(&then) {
        std::cmp::Ordering::Greater => format!("+{}", fmt_size(now - then)),
        std::cmp::Ordering::Less => format!("−{}", fmt_size(then - now)),
        std::cmp::Ordering::Equal => t!("değişmedi", "unchanged").into(),
    }
}

/// "az önce" / "just now", "5 dakika önce" / "5 minutes ago", …
pub fn fmt_ago(secs: u64) -> String {
    fmt_ago_in(lang(), secs)
}

fn fmt_ago_in(lang: Lang, secs: u64) -> String {
    let (n, tr, en) = match secs {
        s if s < 60 => {
            return match lang {
                Lang::Tr => "az önce".into(),
                Lang::En => "just now".into(),
            }
        }
        s if s < 3600 => (s / 60, "dakika", "minute"),
        s if s < 86_400 => (s / 3600, "saat", "hour"),
        s => (s / 86_400, "gün", "day"),
    };
    match lang {
        Lang::Tr => format!("{n} {tr} önce"),
        Lang::En => format!("{n} {en}{} ago", if n == 1 { "" } else { "s" }),
    }
}

/// Human readable size, 1024-based.
pub fn fmt_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Integer with thousands separators: 1234567 -> "1.234.567".
pub fn fmt_count(n: u64) -> String {
    fmt_count_in(lang(), n)
}

/// Thousands separators: "1.234.567" in Turkish, "1,234,567" in English.
fn fmt_count_in(lang: Lang, n: u64) -> String {
    let sep = match lang {
        Lang::Tr => '.',
        Lang::En => ',',
    };
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

pub(super) fn truncate_path(path: &Path, max: usize) -> String {
    let s = path.display().to_string();
    let len = s.chars().count();
    if len <= max || max < 2 {
        return s;
    }
    let tail: String = s.chars().skip(len - (max - 1)).collect();
    format!("…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;

    #[test]
    fn formats_sizes() {
        assert_eq!(fmt_size(0), "0 B");
        assert_eq!(fmt_size(1023), "1023 B");
        assert_eq!(fmt_size(1536), "1.5 KiB");
        assert_eq!(fmt_size(5 * 1024 * 1024 * 1024), "5.0 GiB");
    }

    #[test]
    fn formats_deltas_and_ages() {
        assert_eq!(fmt_delta(3072, 1024), "+2.0 KiB");
        assert_eq!(fmt_delta(1024, 3072), "−2.0 KiB");
        assert_eq!(fmt_delta(5, 5), "değişmedi");
        assert_eq!(fmt_ago(30), "az önce");
        assert_eq!(fmt_ago(600), "10 dakika önce");
        assert_eq!(fmt_ago(7200), "2 saat önce");
        assert_eq!(fmt_ago(3 * 86_400 + 5), "3 gün önce");
    }

    #[test]
    fn formats_in_english() {
        assert_eq!(fmt_count_in(Lang::En, 1234567), "1,234,567");
        assert_eq!(fmt_ago_in(Lang::En, 30), "just now");
        assert_eq!(fmt_ago_in(Lang::En, 60), "1 minute ago");
        assert_eq!(fmt_ago_in(Lang::En, 7200), "2 hours ago");
        assert_eq!(fmt_ago_in(Lang::Tr, 7200), "2 saat önce");
        let d = fmt_date_in(Lang::En, 1_000_000_000);
        assert!(d.starts_with("2001-09-0"), "{d}");
    }

    #[test]
    fn formats_counts() {
        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(999), "999");
        assert_eq!(fmt_count(1000), "1.000");
        assert_eq!(fmt_count(1234567), "1.234.567");
    }

    #[test]
    fn wraps_with_a_hanging_indent() {
        let lines = wrap_indented("one two three four", 12, "  → ", "    ");
        assert_eq!(lines, ["  → one two", "    three", "    four"]);
        assert!(lines.iter().all(|l| l.chars().count() <= 12));
        // A path without spaces is split; nothing is lost.
        let path = "/Users/demo/Library/Containers/com.example.locked";
        let lines = wrap_indented(path, 20, "    ", "    ");
        assert!(lines.iter().all(|l| l.chars().count() <= 20), "{lines:?}");
        let joined: String = lines.iter().map(|l| &l[4..]).collect();
        assert_eq!(joined, path);
        // Short text stays on one line; empty text keeps the prefix.
        assert_eq!(wrap_indented("ok", 20, "> ", "  "), ["> ok"]);
        assert_eq!(wrap_indented("", 20, "> ", "  "), ["> "]);
        // A width below the prefix still ends.
        assert!(!wrap_indented("abc def", 2, "    ", "    ").is_empty());
    }

    #[test]
    fn formats_dates() {
        // 2001-09-09 01:46 UTC: 8 or 9 September depending on the time zone.
        let d = fmt_date(1_000_000_000);
        assert_eq!(d.chars().count(), 16, "{d}");
        assert!(d.contains(".09.2001 "), "{d}");
    }
}
