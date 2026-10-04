//! Integration tests: real folders in a temp directory, scanned by the real
//! scanner and driven through `App` the way the interface drives it.
//!
//! Test builds keep history and settings under the temp directory
//! (`paths::data_dir`) and move "deleted" entries into a test trash folder
//! (`delete::test_trash`), so nothing here touches the user's data.

mod apps;
mod clutter;
mod deletion;
mod history;
mod language;
mod reports;
mod scan;
mod screens;

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, Browser, Screen};
use crate::scanner;
use crate::tree::{NodeId, Tree, ROOT};

pub const KIB: u64 = 1024;
pub const MIB: u64 = 1024 * KIB;
const DAY: u64 = 86_400;

/// A folder tree in a temp directory. Every file gets its own size, so
/// "largest first" lists have no ties and a fixed order.
pub struct Fixture {
    dir: tempfile::TempDir,
    /// Sum of the sizes written, for checking the scan.
    pub bytes: u64,
    pub files: u64,
}

impl Fixture {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
            bytes: 0,
            files: 0,
        }
    }

    /// The fixture used by most tests:
    ///
    /// ```text
    /// Archive/old.iso                 100 MiB + 1, 400 days old (sparse)
    /// Media/clip.mov                  1.5 MiB, content A
    /// Media/other.mov                 1.5 MiB, content B (same size only)
    /// Backup/clip-copy.mov            1.5 MiB, content A, 30 days old
    /// Downloads/setup.dmg             300 KiB, 400 days old
    /// Downloads/photos.zip            200 KiB, 10 days old
    /// Downloads/notes.txt              50 KiB
    /// Projects/rusty/{Cargo.toml, target/debug/app 700 KiB}, 200 days old
    /// Projects/web/{package.json, node_modules/pkg/index.js 600 KiB}
    /// Projects/loose/target/x          40 KiB (no Cargo.toml: not junk)
    /// Library/Caches/com.example.app/blob  400 KiB
    /// README.md in Projects/rusty, Projects/web and Docs (as Readme.md)
    /// ```
    pub fn standard() -> Self {
        let mut f = Self::new();
        f.sparse("Archive/old.iso", 100 * MIB + 1, 400);
        f.file("Media/clip.mov", 1536 * KIB, b'A', 0);
        f.file("Media/other.mov", 1536 * KIB, b'B', 0);
        f.file("Backup/clip-copy.mov", 1536 * KIB, b'A', 30);
        f.file("Downloads/setup.dmg", 300 * KIB, b'd', 400);
        f.file("Downloads/photos.zip", 200 * KIB, b'z', 10);
        f.file("Downloads/notes.txt", 50 * KIB, b't', 0);
        f.file("Projects/rusty/Cargo.toml", 101, b'c', 200);
        f.file("Projects/rusty/target/debug/app", 700 * KIB, b'r', 200);
        f.file("Projects/rusty/README.md", 102, b'm', 200);
        f.file("Projects/web/package.json", 103, b'p', 5);
        f.file("Projects/web/node_modules/pkg/index.js", 600 * KIB, b'j', 5);
        f.file("Projects/web/README.md", 104, b'n', 5);
        f.file("Projects/loose/target/x", 40 * KIB, b'x', 0);
        f.file("Library/Caches/com.example.app/blob", 400 * KIB, b'b', 0);
        f.file("Docs/Readme.md", 105, b'o', 0);
        f
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.root().join(rel)
    }

    /// Writes `len` bytes of `fill` and makes the file `age_days` old. Files
    /// with the same size and fill have the same content.
    pub fn file(&mut self, rel: &str, len: u64, fill: u8, age_days: u64) {
        let path = self.path(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let data = vec![fill; usize::try_from(len).unwrap()];
        fs::write(&path, data).unwrap();
        self.added(&path, len, age_days);
    }

    /// A file of `len` bytes that takes (almost) no disk space.
    pub fn sparse(&mut self, rel: &str, len: u64, age_days: u64) {
        let path = self.path(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        File::create(&path).unwrap().set_len(len).unwrap();
        self.added(&path, len, age_days);
    }

    fn added(&mut self, path: &Path, len: u64, age_days: u64) {
        set_age(path, age_days);
        self.bytes += len;
        self.files += 1;
    }

    pub fn remove(&mut self, rel: &str) {
        fs::remove_dir_all(self.path(rel)).unwrap();
    }
}

/// Sets the modification time of a file to `days` ago.
pub fn set_age(path: &Path, days: u64) {
    let when = SystemTime::now() - Duration::from_secs(days * DAY);
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

/// Scans `root` with the real scanner.
pub fn scan(root: &Path) -> scanner::ScanResult {
    scanner::scan(root, Vec::new(), &Arc::default(), |_| {}).unwrap()
}

/// The node at `rel` ('/'-separated) below the root.
pub fn find(tree: &Tree, rel: &str) -> NodeId {
    rel.split('/').fold(ROOT, |dir, name| {
        tree.children(dir)
            .find(|&c| tree.name(c) == name)
            .unwrap_or_else(|| panic!("{rel}: {name} not found"))
    })
}

pub fn now() -> u64 {
    crate::ui::now_secs()
}

/// The app after scanning `root`, on the browser screen.
pub fn open(root: &Path) -> App {
    let mut app = App::new(Some(root.to_path_buf()));
    tick_until(&mut app, "the scan", |a| {
        matches!(a.screen, Screen::Browser)
    });
    app
}

pub fn press(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    app.on_tick();
}

/// Opens menu item `index` (`m`, then ↓ `index` times, Enter).
pub fn menu(app: &mut App, index: usize) {
    press(app, KeyCode::Char('m'));
    for _ in 0..index {
        press(app, KeyCode::Down);
    }
    press(app, KeyCode::Enter);
}

/// Ticks the app like the event loop does until `done`, for at most 20 s.
pub fn tick_until(app: &mut App, what: &str, done: impl Fn(&App) -> bool) {
    let start = Instant::now();
    while !done(app) {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}"
        );
        app.on_tick();
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn browser(app: &App) -> &Browser {
    app.browser.as_ref().expect("browser screen")
}

/// Row labels of the open result list, with '/' separators.
pub fn rows(app: &App) -> Vec<String> {
    let r = browser(app).results.as_ref().expect("a result list");
    r.rows.iter().map(|r| r.label.replace('\\', "/")).collect()
}
