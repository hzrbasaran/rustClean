//! Interface language (Turkish / English).
//!
//! Texts stay next to the code that uses them, in both languages:
//! `t!("Tamam", "OK")` for plain text and `tf!("{n} öğe", "{n} items")`
//! for formatted text. The language is global: background threads (tool
//! measurements) build texts too.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Tr,
    En,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::Tr => "tr",
            Lang::En => "en",
        }
    }

    pub fn parse(s: &str) -> Option<Lang> {
        match s.trim().to_ascii_lowercase().get(..2)? {
            "tr" => Some(Lang::Tr),
            "en" => Some(Lang::En),
            _ => None,
        }
    }

    pub fn other(self) -> Lang {
        match self {
            Lang::Tr => Lang::En,
            Lang::En => Lang::Tr,
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

// Test builds can pick the language per thread, so tests that render in
// English do not change what tests running beside them see.
#[cfg(test)]
thread_local! {
    static TEST_LANG: std::cell::Cell<Option<Lang>> = const { std::cell::Cell::new(None) };
}

/// Runs `f` with this thread's texts in `lang` (test builds only; the
/// screen snapshots, which run on Unix, use it).
#[cfg(all(test, unix))]
pub fn with_lang<R>(lang: Lang, f: impl FnOnce() -> R) -> R {
    let before = TEST_LANG.replace(Some(lang));
    let out = f();
    TEST_LANG.set(before);
    out
}

pub fn lang() -> Lang {
    #[cfg(test)]
    if let Some(lang) = TEST_LANG.get() {
        return lang;
    }
    if LANG.load(Ordering::Relaxed) == 1 {
        Lang::En
    } else {
        Lang::Tr
    }
}

#[cfg(not(test))]
pub fn set_lang(lang: Lang) {
    LANG.store(u8::from(lang == Lang::En), Ordering::Relaxed);
}

/// In test builds `L` switches only the thread that pressed it.
#[cfg(test)]
pub fn set_lang(lang: Lang) {
    TEST_LANG.set(Some(lang));
}

/// Picks the text for the current language.
pub fn pick<T>(tr: T, en: T) -> T {
    match lang() {
        Lang::Tr => tr,
        Lang::En => en,
    }
}

/// `t!("Türkçe", "English")`: a `&'static str` in the current language.
#[macro_export]
macro_rules! t {
    ($tr:expr, $en:expr $(,)?) => {
        $crate::i18n::pick($tr, $en)
    };
}

/// `tf!("{n} öğe", "{n} items", ...)`: `format!` in the current language.
#[macro_export]
macro_rules! tf {
    ($tr:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        match $crate::i18n::lang() {
            $crate::i18n::Lang::Tr => format!($tr $(, $arg)*),
            $crate::i18n::Lang::En => format!($en $(, $arg)*),
        }
    };
}

fn settings_file() -> Option<PathBuf> {
    Some(crate::paths::data_dir()?.join("settings"))
}

/// The saved choice, if any.
fn saved() -> Option<Lang> {
    let text = std::fs::read_to_string(settings_file()?).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("lang="))
        .and_then(Lang::parse)
}

/// Remembers the choice for the next start.
pub fn save(lang: Lang) {
    if let Some(file) = settings_file() {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, format!("lang={}\n", lang.code()));
    }
}

/// The system's preferred language.
fn system() -> Option<Lang> {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(lang) = std::env::var(var).ok().as_deref().and_then(Lang::parse) {
            return Some(lang);
        }
    }
    #[cfg(target_os = "macos")]
    {
        // First entry of `defaults read -g AppleLanguages`, e.g. "en-TR".
        let out = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleLanguages"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let first = text
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .find(|s| !s.is_empty())?;
        return Some(Lang::parse(first).unwrap_or(Lang::En));
    }
    #[allow(unreachable_code)]
    None
}

/// Chooses the language at start: command line, saved choice, system.
pub fn init(cli: Option<&str>) {
    let lang = cli
        .and_then(Lang::parse)
        .or_else(saved)
        .or_else(system)
        .unwrap_or(Lang::En);
    set_lang(lang);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codes() {
        assert_eq!(Lang::parse("tr"), Some(Lang::Tr));
        assert_eq!(Lang::parse("tr_TR.UTF-8"), Some(Lang::Tr));
        assert_eq!(Lang::parse("en-TR"), Some(Lang::En));
        assert_eq!(Lang::parse("EN"), Some(Lang::En));
        assert_eq!(Lang::parse("C.UTF-8"), None);
        assert_eq!(Lang::parse("de"), None);
        assert_eq!(Lang::Tr.other(), Lang::En);
    }
}
