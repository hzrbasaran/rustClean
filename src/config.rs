//! The configuration file: `config.toml` in the data directory, read once at
//! start. Every key is optional; a missing key keeps today's behavior.
//!
//! ```toml
//! [scan]
//! exclude = ["~/Library/Containers", "/Volumes/Backup"]
//! [reports]
//! old_big_min_mib = 100
//! old_big_min_days = 365
//! duplicates_min_mib = 1
//! [view]
//! size = "disk"      # or "apparent"
//! sort = "size"      # size | name | count | modified
//! ```
//!
//! A broken file never stops the program: invalid TOML means the defaults,
//! an invalid value means the default for that key, and an unknown key is
//! ignored. Each case is a `Problem`, shown on the status line (or on stderr
//! for `--summary` and `--config`). Theme and language are not here: `T` and
//! `L` save them in the `settings` file.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::app::SortMode;
use crate::tree::SizeMode;

const MIB: u64 = 1024 * 1024;

/// The file name inside the data directory.
const FILE_NAME: &str = "config.toml";

/// The effective settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Folders the scanner does not descend into (`~` expanded).
    pub exclude: Vec<PathBuf>,
    /// "Old and large files": the smallest size, in MiB.
    pub old_big_min_mib: u64,
    /// "Old and large files": unchanged for more than this many days.
    pub old_big_min_days: u64,
    /// The smallest file the duplicate search reads, in MiB.
    pub duplicates_min_mib: u64,
    /// The size shown when a scan opens.
    pub size: SizeMode,
    /// The order of the folder list when a scan opens.
    pub sort: SortMode,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            exclude: Vec::new(),
            old_big_min_mib: crate::reports::OLD_BIG_SIZE / MIB,
            old_big_min_days: crate::reports::OLD_BIG_AGE / crate::stats::DAY,
            duplicates_min_mib: crate::duplicates::MIN_SIZE / MIB,
            size: SizeMode::Disk,
            sort: SortMode::Size,
        }
    }
}

impl Config {
    /// In bytes. Values are checked when read, so this does not overflow.
    pub fn old_big_min_size(&self) -> u64 {
        self.old_big_min_mib.saturating_mul(MIB)
    }

    /// In seconds.
    pub fn old_big_min_age(&self) -> u64 {
        self.old_big_min_days.saturating_mul(crate::stats::DAY)
    }

    /// In bytes.
    pub fn duplicates_min_size(&self) -> u64 {
        self.duplicates_min_mib.saturating_mul(MIB)
    }
}

/// What a value should have been.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expected {
    Section,
    PathList,
    Path,
    PositiveInt,
    OneOf(&'static [&'static str]),
}

impl Expected {
    fn text(self) -> String {
        match self {
            Expected::Section => t!(
                "bir bölüm olmalı ([scan] gibi)",
                "must be a section (like [scan])"
            )
            .into(),
            Expected::PathList => t!("bir yol listesi olmalı", "must be a list of paths").into(),
            Expected::Path => t!(
                "tam bir yol olmalı ya da ~ ile başlamalı",
                "must be an absolute path or start with ~"
            )
            .into(),
            Expected::PositiveInt => t!(
                "1 ya da daha büyük bir tam sayı olmalı",
                "must be a whole number of 1 or more"
            )
            .into(),
            Expected::OneOf(values) => tf!(
                "şunlardan biri olmalı: {}",
                "must be one of: {}",
                values.join(", ")
            ),
        }
    }
}

/// Something wrong with the file. None of them stops the program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The file exists but could not be read: the defaults are used.
    Unreadable(String),
    /// Not valid TOML: the defaults are used.
    Syntax {
        line: usize,
        column: usize,
        message: String,
    },
    /// A key rustClean does not know, as `section.key`: ignored.
    UnknownKey(String),
    /// A known key with a wrong value: its default is used.
    BadValue { key: String, expected: Expected },
}

impl Problem {
    /// The message in the current language, naming the file. The file comes
    /// last, so a narrow status line still shows what is wrong.
    pub fn text(&self, file: &str) -> String {
        match self {
            Problem::Unreadable(err) => tf!(
                "Ayar dosyası okunamadı ({err}), varsayılanlar kullanılıyor: {file}",
                "Could not read the config file ({err}), using the defaults: {file}"
            ),
            Problem::Syntax {
                line,
                column,
                message,
            } => tf!(
                "Ayar dosyası geçersiz, varsayılanlar kullanılıyor (satır {line}, sütun {column}: {message}): {file}",
                "Invalid config file, using the defaults (line {line}, column {column}: {message}): {file}"
            ),
            Problem::UnknownKey(key) => tf!(
                "Bilinmeyen ayar yok sayıldı: {key} ({file})",
                "Unknown key ignored: {key} ({file})"
            ),
            Problem::BadValue { key, expected } => tf!(
                "Ayar {key} {}; varsayılan kullanılıyor ({file})",
                "Setting {key} {}; using the default ({file})",
                expected.text()
            ),
        }
    }
}

/// The result of reading the file.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub config: Config,
    /// Where the file is (or would be); `None` without a data directory.
    pub path: Option<PathBuf>,
    /// Whether the file exists.
    pub found: bool,
    pub problems: Vec<Problem>,
}

impl Loaded {
    /// Every problem as a message, with the file's full path (for stderr).
    pub fn messages(&self) -> Vec<String> {
        let file = self.path.as_deref().unwrap_or(Path::new(FILE_NAME));
        let file = file.display().to_string();
        self.problems.iter().map(|p| p.text(&file)).collect()
    }

    /// One line for the status line: the first problem and how many more,
    /// with the home folder shortened to `~`.
    pub fn status(&self) -> Option<String> {
        let first = self.problems.first()?;
        let file = self.path.as_deref().unwrap_or(Path::new(FILE_NAME));
        let mut line = first.text(&tilde(file, dirs::home_dir().as_deref()));
        let more = self.problems.len() - 1;
        if more > 0 {
            line.push_str(&tf!(
                " (+{more} uyarı daha: rustclean --config)",
                " (+{more} more: rustclean --config)"
            ));
        }
        Some(line)
    }
}

/// Where the configuration file is.
pub fn path() -> Option<PathBuf> {
    Some(crate::paths::data_dir()?.join(FILE_NAME))
}

/// Reads the configuration file from the data directory.
pub fn load() -> Loaded {
    let path = path();
    let home = dirs::home_dir();
    let mut loaded = match &path {
        Some(p) => load_from(p, home.as_deref()),
        None => Loaded {
            config: Config::default(),
            path: None,
            found: false,
            problems: Vec::new(),
        },
    };
    loaded.path = path;
    loaded
}

/// Reads `file`; `home` replaces a leading `~`. A missing file is not a
/// problem: it means the defaults.
pub fn load_from(file: &Path, home: Option<&Path>) -> Loaded {
    let (config, found, problems) = match std::fs::read_to_string(file) {
        Ok(text) => {
            let (config, problems) = parse(&text, home);
            (config, true, problems)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::default(), false, vec![]),
        Err(e) => (
            Config::default(),
            true,
            vec![Problem::Unreadable(e.to_string())],
        ),
    };
    Loaded {
        config,
        path: Some(file.to_path_buf()),
        found,
        problems,
    }
}

/// Reads the settings from TOML text. Invalid TOML gives the defaults and
/// one problem; otherwise each bad or unknown key is a problem of its own
/// and the other keys still apply.
pub fn parse(text: &str, home: Option<&Path>) -> (Config, Vec<Problem>) {
    let mut config = Config::default();
    let mut problems = Vec::new();
    let table = match text.parse::<toml::Table>() {
        Ok(t) => t,
        Err(e) => {
            let (line, column) = position(text, e.span().map_or(0, |s| s.start));
            problems.push(Problem::Syntax {
                line,
                column,
                message: e.message().trim().replace(char::is_whitespace, " "),
            });
            return (config, problems);
        }
    };
    for (section, value) in &table {
        let keys: &[&str] = match section.as_str() {
            "scan" => &["exclude"],
            "reports" => &["old_big_min_mib", "old_big_min_days", "duplicates_min_mib"],
            "view" => &["size", "sort"],
            _ => {
                problems.push(Problem::UnknownKey(section.clone()));
                continue;
            }
        };
        let Some(entries) = value.as_table() else {
            problems.push(Problem::BadValue {
                key: section.clone(),
                expected: Expected::Section,
            });
            continue;
        };
        for (key, value) in entries {
            let name = format!("{section}.{key}");
            if !keys.contains(&key.as_str()) {
                problems.push(Problem::UnknownKey(name));
                continue;
            }
            let bad = |expected| Problem::BadValue {
                key: name.clone(),
                expected,
            };
            match key.as_str() {
                "exclude" => match value.as_array() {
                    Some(items) => {
                        for item in items {
                            match item.as_str().and_then(|p| expand(p, home)) {
                                Some(p) => config.exclude.push(p),
                                None => problems.push(bad(Expected::Path)),
                            }
                        }
                    }
                    None => problems.push(bad(Expected::PathList)),
                },
                "old_big_min_mib" | "old_big_min_days" | "duplicates_min_mib" => {
                    let unit = if key == "old_big_min_days" {
                        crate::stats::DAY
                    } else {
                        MIB
                    };
                    let n = value
                        .as_integer()
                        .and_then(|n| u64::try_from(n).ok())
                        .filter(|&n| n >= 1 && n.checked_mul(unit).is_some());
                    match (n, key.as_str()) {
                        (None, _) => problems.push(bad(Expected::PositiveInt)),
                        (Some(n), "old_big_min_mib") => config.old_big_min_mib = n,
                        (Some(n), "old_big_min_days") => config.old_big_min_days = n,
                        (Some(n), _) => config.duplicates_min_mib = n,
                    }
                }
                "size" => match value.as_str().and_then(parse_size) {
                    Some(mode) => config.size = mode,
                    None => problems.push(bad(Expected::OneOf(SIZES))),
                },
                _ => match value.as_str().and_then(parse_sort) {
                    Some(sort) => config.sort = sort,
                    None => problems.push(bad(Expected::OneOf(SORTS))),
                },
            }
        }
    }
    (config, problems)
}

const SIZES: &[&str] = &["disk", "apparent"];
const SORTS: &[&str] = &["size", "name", "count", "modified"];

fn parse_size(s: &str) -> Option<SizeMode> {
    match s {
        "disk" => Some(SizeMode::Disk),
        "apparent" => Some(SizeMode::Apparent),
        _ => None,
    }
}

fn size_code(mode: SizeMode) -> &'static str {
    match mode {
        SizeMode::Disk => "disk",
        SizeMode::Apparent => "apparent",
    }
}

fn parse_sort(s: &str) -> Option<SortMode> {
    match s {
        "size" => Some(SortMode::Size),
        "name" => Some(SortMode::Name),
        "count" => Some(SortMode::Count),
        "modified" => Some(SortMode::Modified),
        _ => None,
    }
}

fn sort_code(sort: SortMode) -> &'static str {
    match sort {
        SortMode::Size => "size",
        SortMode::Name => "name",
        SortMode::Count => "count",
        SortMode::Modified => "modified",
    }
}

/// `~` or `~/…` under `home`; other paths must be absolute. A relative path
/// would depend on where rustClean was started, so it is refused.
fn expand(p: &str, home: Option<&Path>) -> Option<PathBuf> {
    let rest = if p == "~" {
        Some("")
    } else {
        p.strip_prefix("~/")
            .or_else(|| p.strip_prefix("~\\").filter(|_| cfg!(windows)))
    };
    let path = match rest {
        Some(rest) => home?.join(rest),
        None => PathBuf::from(p),
    };
    path.is_absolute().then_some(path)
}

/// `path` with `home` written as `~`.
fn tilde(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => Path::new("~").join(rest).display().to_string(),
        None => path.display().to_string(),
    }
}

/// 1-based line and column of byte `offset`.
fn position(text: &str, offset: usize) -> (usize, usize) {
    let before = text.get(..offset).unwrap_or(text);
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    (line, column)
}

/// The effective settings as a TOML file, for `rustclean --config`.
pub fn to_toml(config: &Config) -> String {
    let quote = |p: &Path| {
        let s = p
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        format!("\"{s}\"")
    };
    let exclude: Vec<String> = config.exclude.iter().map(|p| quote(p)).collect();
    format!(
        "[scan]\n\
         exclude = [{}]\n\
         \n\
         [reports]\n\
         old_big_min_mib = {}\n\
         old_big_min_days = {}\n\
         duplicates_min_mib = {}\n\
         \n\
         [view]\n\
         size = \"{}\"\n\
         sort = \"{}\"\n",
        exclude.join(", "),
        config.old_big_min_mib,
        config.old_big_min_days,
        config.duplicates_min_mib,
        size_code(config.size),
        sort_code(config.sort),
    )
}

static CONFIG: OnceLock<Arc<Config>> = OnceLock::new();

// Test builds can set the configuration per thread, so a test does not
// change what the tests running beside it see.
#[cfg(test)]
thread_local! {
    static TEST_CONFIG: std::cell::RefCell<Option<Arc<Config>>> = const { std::cell::RefCell::new(None) };
}

/// Sets the configuration for the whole run (once, at start).
pub fn init(config: Config) {
    let _ = CONFIG.set(Arc::new(config));
}

/// The configuration in effect: the defaults until `init`.
pub fn get() -> Arc<Config> {
    #[cfg(test)]
    if let Some(c) = TEST_CONFIG.with_borrow(Clone::clone) {
        return c;
    }
    CONFIG.get().cloned().unwrap_or_default()
}

/// Runs `f` with this thread's configuration set to `config` (test builds).
#[cfg(test)]
pub fn with<R>(config: Config, f: impl FnOnce() -> R) -> R {
    let before = TEST_CONFIG.replace(Some(Arc::new(config)));
    let out = f();
    TEST_CONFIG.set(before);
    out
}

/// What the scanner does not descend into: other volumes' mount points and
/// the excluded folders.
pub fn scan_skip() -> Vec<PathBuf> {
    let mut skip = crate::disks::all_mount_points();
    skip.extend(get().exclude.iter().cloned());
    skip
}

/// A size in whole MiB as text: "100 MiB", "2 GiB".
pub fn fmt_mib(mib: u64) -> String {
    let n = crate::ui::fmt_count;
    if mib >= 1024 && mib.is_multiple_of(1024) {
        format!("{} GiB", n(mib / 1024))
    } else {
        format!("{} MiB", n(mib))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\Users\me")
        } else {
            PathBuf::from("/home/me")
        }
    }

    fn parse_ok(text: &str) -> Config {
        let (config, problems) = parse(text, Some(&home()));
        assert_eq!(problems, vec![], "{text}");
        config
    }

    #[test]
    fn defaults_without_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_from(&dir.path().join(FILE_NAME), Some(&home()));
        assert!(!loaded.found);
        assert!(loaded.problems.is_empty());
        assert_eq!(loaded.config, Config::default());
        assert_eq!(loaded.status(), None);
        // The defaults are today's constants.
        let d = Config::default();
        assert_eq!(d.old_big_min_size(), crate::reports::OLD_BIG_SIZE);
        assert_eq!(d.old_big_min_age(), crate::reports::OLD_BIG_AGE);
        assert_eq!(d.duplicates_min_size(), crate::duplicates::MIN_SIZE);
        assert_eq!((d.size, d.sort), (SizeMode::Disk, SortMode::Size));
        // An empty file is the defaults too.
        assert_eq!(parse_ok(""), Config::default());
    }

    #[test]
    fn reads_every_key() {
        let abs = if cfg!(windows) {
            r"D:\Backup"
        } else {
            "/Volumes/Backup"
        };
        let text = format!(
            "[scan]\nexclude = ['~/Library/Containers', '{abs}']\n\
             [reports]\nold_big_min_mib = 500\nold_big_min_days = 30\nduplicates_min_mib = 10\n\
             [view]\nsize = \"apparent\"\nsort = \"modified\"\n"
        );
        let c = parse_ok(&text);
        assert_eq!(
            c.exclude,
            vec![home().join("Library/Containers"), PathBuf::from(abs)]
        );
        assert_eq!(c.old_big_min_size(), 500 * MIB);
        assert_eq!(c.old_big_min_age(), 30 * crate::stats::DAY);
        assert_eq!(c.duplicates_min_size(), 10 * MIB);
        assert_eq!(c.size, SizeMode::Apparent);
        assert_eq!(c.sort, SortMode::Modified);
        for (code, sort) in [
            ("size", SortMode::Size),
            ("name", SortMode::Name),
            ("count", SortMode::Count),
        ] {
            assert_eq!(parse_ok(&format!("[view]\nsort = '{code}'")).sort, sort);
        }
        assert_eq!(parse_ok("[view]\nsize = 'disk'").size, SizeMode::Disk);
        // One key alone leaves the others at their defaults.
        let c = parse_ok("[reports]\nold_big_min_days = 90");
        assert_eq!(c.old_big_min_days, 90);
        assert_eq!(c.old_big_min_mib, Config::default().old_big_min_mib);
    }

    #[test]
    fn expands_the_home_folder() {
        let h = home();
        assert_eq!(expand("~", Some(&h)), Some(h.clone()));
        assert_eq!(expand("~/a/b", Some(&h)), Some(h.join("a/b")));
        // Without a home folder, or relative: refused.
        assert_eq!(expand("~/a", None), None);
        assert_eq!(expand("a/b", Some(&h)), None);
        assert_eq!(expand("~other/a", Some(&h)), None);
        // And back, for the status line.
        let shown = tilde(&h.join("data").join(FILE_NAME), Some(&h));
        assert_eq!(shown.replace('\\', "/"), "~/data/config.toml");
        let elsewhere = PathBuf::from(if cfg!(windows) {
            r"D:\c.toml"
        } else {
            "/c.toml"
        });
        assert_eq!(tilde(&elsewhere, Some(&h)), elsewhere.display().to_string());
        let (c, problems) = parse("[scan]\nexclude = ['relative', 3, '~/ok']", Some(&h));
        assert_eq!(c.exclude, vec![h.join("ok")]);
        assert_eq!(problems.len(), 2);
    }

    #[test]
    fn invalid_file_gives_the_defaults_and_the_position() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE_NAME);
        std::fs::write(
            &file,
            "[view]\nsort = \"name\"\n[reports]\nold_big_min_mib = = 3\n",
        )
        .unwrap();
        let loaded = load_from(&file, Some(&home()));
        assert!(loaded.found);
        assert_eq!(loaded.config, Config::default(), "nothing applies");
        let [Problem::Syntax { line, column, .. }] = loaded.problems.as_slice() else {
            panic!("{:?}", loaded.problems);
        };
        assert_eq!((*line, *column), (4, 19));
        let status = loaded.status().unwrap();
        assert!(loaded.messages()[0].ends_with(&file.display().to_string()));
        assert!(status.ends_with("config.toml"), "{status}");
        assert!(status.contains("satır 4, sütun 19"), "{status}");
        crate::i18n::set_lang(crate::i18n::Lang::En);
        let status = loaded.status().unwrap();
        crate::i18n::set_lang(crate::i18n::Lang::Tr);
        assert!(status.contains("line 4, column 19"), "{status}");
    }

    #[test]
    fn bad_values_keep_their_default_only() {
        let (c, problems) = parse(
            "[reports]\nold_big_min_mib = 0\nold_big_min_days = -1\nduplicates_min_mib = 2.5\n\
             [view]\nsize = 'huge'\nsort = 'name'\n",
            Some(&home()),
        );
        let d = Config::default();
        assert_eq!(c.old_big_min_mib, d.old_big_min_mib);
        assert_eq!(c.old_big_min_days, d.old_big_min_days);
        assert_eq!(c.duplicates_min_mib, d.duplicates_min_mib);
        assert_eq!(c.size, d.size);
        assert_eq!(c.sort, SortMode::Name, "the valid key still applies");
        assert_eq!(problems.len(), 4);
        // Too large to count in bytes.
        let (_, problems) = parse(&format!("[reports]\nold_big_min_mib = {}", i64::MAX), None);
        assert_eq!(problems.len(), 1);
        // A section that is not a table.
        let (_, problems) = parse("view = 3", None);
        assert_eq!(
            problems,
            vec![Problem::BadValue {
                key: "view".into(),
                expected: Expected::Section
            }]
        );
    }

    #[test]
    fn unknown_keys_warn_without_failing() {
        let (c, problems) = parse(
            "theme = 'dark'\n[view]\nsort = 'count'\ncolour = 1\n[colors]\na = 1\n",
            None,
        );
        assert_eq!(c.sort, SortMode::Count);
        assert_eq!(
            problems,
            vec![
                Problem::UnknownKey("colors".into()),
                Problem::UnknownKey("theme".into()),
                Problem::UnknownKey("view.colour".into()),
            ]
        );
        let loaded = Loaded {
            config: c,
            path: Some(PathBuf::from("c.toml")),
            found: true,
            problems,
        };
        assert_eq!(
            loaded.status().unwrap(),
            "Bilinmeyen ayar yok sayıldı: colors (c.toml) (+2 uyarı daha: rustclean --config)"
        );
    }

    #[test]
    fn printed_values_read_back() {
        let c = Config {
            exclude: vec![home().join("a \"b\""), home().join("c")],
            old_big_min_mib: 2048,
            old_big_min_days: 10,
            duplicates_min_mib: 5,
            size: SizeMode::Apparent,
            sort: SortMode::Count,
        };
        assert_eq!(parse_ok(&to_toml(&c)), c);
        assert_eq!(parse_ok(&to_toml(&Config::default())), Config::default());
    }

    #[test]
    fn the_documented_example_is_the_defaults() {
        let usage = include_str!("../docs/USAGE.md");
        let section = &usage[usage.find("## Configuration file").unwrap()..];
        let start = section.find("```toml\n").unwrap() + "```toml\n".len();
        let example = &section[start..start + section[start..].find("```").unwrap()];
        assert_eq!(parse_ok(example), Config::default());
        // The commented-out line is valid too ("/Volumes/…" is not an
        // absolute path on Windows).
        let line = example
            .lines()
            .find_map(|l| l.strip_prefix("# exclude = "))
            .unwrap();
        if cfg!(unix) {
            let c = parse_ok(&format!("[scan]\nexclude = {line}"));
            assert_eq!(c.exclude.len(), 2);
        }
    }

    #[test]
    fn sizes_in_mib() {
        assert_eq!(fmt_mib(1), "1 MiB");
        assert_eq!(fmt_mib(100), "100 MiB");
        assert_eq!(fmt_mib(2048), "2 GiB");
        assert_eq!(fmt_mib(1536), "1.536 MiB");
    }

    #[test]
    fn thread_override() {
        let c = Config {
            old_big_min_mib: 7,
            ..Config::default()
        };
        assert_eq!(with(c, || get().old_big_min_mib), 7);
        assert_eq!(*get(), Config::default());
    }
}
