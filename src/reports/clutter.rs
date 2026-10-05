//! Clutter: empty folders, broken symbolic links and temporary files, in
//! three groups.
//!
//! Each kind is harmless to remove only where nothing relies on it, so the
//! search skips, as everywhere in rustClean, whatever might still matter:
//! hidden folders (a fresh repository's `.git/refs` is empty and needed),
//! app and package bundles, `Library` / `AppData` (apps expect their empty
//! folders), build output and dependencies (`target`, `Pods`, `vendor`…),
//! system folders, the standard folders of the home directory, and
//! temporary files touched in the last day (they may be in use).

use std::path::{Path, PathBuf};

use crate::lists::Row;
use crate::lists::RowSize;
use crate::stats::DAY;
use crate::tree::{NodeId, SizeMode, Tree};

use super::dev_junk::dev_junk_kind;
use super::{AgeFilter, Report};

/// Folder names whose contents are an app's, a package manager's or the
/// system's own business.
const KEEP_OUT: [&str; 5] = [
    "Library",
    "AppData",
    "node_modules",
    "vendor",
    "System Volume Information",
];

/// Extensions of folders that are packages: an app, a library, a document.
const BUNDLES: [&str; 19] = [
    "app",
    "appex",
    "bundle",
    "framework",
    "xcframework",
    "dsym",
    "kext",
    "plugin",
    "xpc",
    "photoslibrary",
    "photolibrary",
    "musiclibrary",
    "tvlibrary",
    "imovielibrary",
    "logicx",
    "band",
    "xcodeproj",
    "xcworkspace",
    "playground",
];

/// System folders, by absolute path: never searched.
const SYSTEM: [&str; 24] = [
    "/System",
    "/Library",
    "/private",
    "/usr",
    "/bin",
    "/sbin",
    "/etc",
    "/var",
    "/opt",
    "/dev",
    "/cores",
    "/Volumes",
    "/Applications",
    "/proc",
    "/sys",
    "/run",
    "/boot",
    "/lib",
    "/lib64",
    "/snap",
    "/srv",
    "C:\\Windows",
    "C:\\Program Files",
    "C:\\ProgramData",
];

/// Folders of the home directory that the system and apps expect.
const HOME_FOLDERS: [&str; 12] = [
    "Applications",
    "Desktop",
    "Documents",
    "Downloads",
    "Movies",
    "Music",
    "Pictures",
    "Public",
    "Sites",
    "Templates",
    "Videos",
    "Library",
];

/// Whether the search goes into folder `id` (also used by the similar
/// images search).
pub(crate) fn searched(tree: &Tree, id: NodeId, path: &Path) -> bool {
    let name = tree.name(id);
    if name.starts_with('.') || KEEP_OUT.contains(&name) {
        return false;
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    if ext.is_some_and(|e| BUNDLES.contains(&e.as_str())) {
        return false;
    }
    // Build output and dependencies (Cargo `target`, `Pods`, `.venv`…): their
    // tools manage them, and the developer junk report lists them.
    if dev_junk_kind(tree, id).is_some() {
        return false;
    }
    !SYSTEM.iter().any(|s| path == Path::new(s))
}

/// Whether `id` holds nothing at all: no file, no folder below it that the
/// search keeps out (an empty `.app` or `.git` still matters), and no folder
/// whose contents were not read (it may hold anything).
fn plainly_empty(tree: &Tree, id: NodeId, path: &Path) -> bool {
    if tree.node(id).file_count != 0 || tree.node(id).unread {
        return false;
    }
    let mut stack: Vec<(NodeId, PathBuf)> = vec![(id, path.to_path_buf())];
    while let Some((dir, p)) = stack.pop() {
        for c in tree.children(dir) {
            let cp = p.join(tree.name(c));
            if tree.node(c).unread || !searched(tree, c, &cp) {
                return false;
            }
            stack.push((c, cp));
        }
    }
    true
}

/// What a temporary file is, by its name.
fn temp_kind(name: &str) -> Option<TempKind> {
    let lower = name.to_ascii_lowercase();
    if lower == ".ds_store" || lower == "thumbs.db" {
        return Some(TempKind::Recreated);
    }
    let ext = lower.rsplit_once('.').map(|(_, e)| e);
    match ext {
        Some("tmp" | "temp") => Some(TempKind::Temp),
        Some("crdownload" | "part" | "download") => Some(TempKind::Download),
        _ if name.starts_with("~$") => Some(TempKind::Lock),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TempKind {
    /// `.DS_Store`, `Thumbs.db`: the system writes them again when needed.
    Recreated,
    Temp,
    /// An Office lock file (`~$…`), present while a document is open.
    Lock,
    /// An unfinished download.
    Download,
}

/// Temporary files touched more recently than this may be in use.
const IN_USE: u64 = DAY;

pub(super) fn clutter(
    tree: &Tree,
    base: NodeId,
    mode: SizeMode,
    age: AgeFilter,
    now: u64,
) -> Report {
    clutter_in(tree, base, mode, age, now, dirs::home_dir().as_deref())
}

/// `clutter` with the home directory given, for tests.
fn clutter_in(
    tree: &Tree,
    base: NodeId,
    mode: SizeMode,
    age: AgeFilter,
    now: u64,
    home: Option<&Path>,
) -> Report {
    let mut empty = Vec::new();
    let mut links = Vec::new();
    let mut temps = Vec::new();
    // Depth first, skipping what `searched` keeps out; paths are built as
    // the walk goes down.
    let mut stack: Vec<(NodeId, PathBuf)> =
        tree.children(base).map(|c| (c, tree.path_of(c))).collect();
    while let Some((id, path)) = stack.pop() {
        let n = tree.node(id);
        if n.is_dir {
            if !searched(tree, id, &path) {
                continue;
            }
            let standard = home == path.parent() && HOME_FOLDERS.contains(&tree.name(id));
            if !standard && plainly_empty(tree, id, &path) {
                // The topmost empty folder; what is below is empty too.
                if age.ok(n.modified) {
                    empty.push(id);
                }
                continue;
            }
            stack.extend(tree.children(id).map(|c| {
                let p = path.join(tree.name(c));
                (c, p)
            }));
        } else if n.is_link {
            // Checked now: only links are looked up on disk.
            let gone =
                std::fs::metadata(&path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
            if gone && age.ok(n.modified) {
                links.push(id);
            }
        } else if let Some(kind) = temp_kind(tree.name(id)) {
            let idle = now.saturating_sub(u64::from(n.modified)) >= IN_USE;
            if (kind == TempKind::Recreated || (n.modified != 0 && idle)) && age.ok(n.modified) {
                temps.push(id);
            }
        }
    }

    let size = |id: NodeId| tree.node(id).size.get(mode);
    let group = |label: &str, detail: String, mut ids: Vec<NodeId>| {
        ids.sort_by_key(|&id| std::cmp::Reverse(size(id)));
        let members = ids.into_iter().map(|id| (id, size(id))).collect();
        Row::group(label.to_string(), detail, members, RowSize::Sum, 1)
    };
    let mut rows = Vec::new();
    if !temps.is_empty() {
        rows.push(group(
            t!("Geçici dosyalar", "Temporary files"),
            t!(
                ".DS_Store, Thumbs.db, *.tmp, ~$… kilitleri, yarım indirmeler; 1 günden eski",
                ".DS_Store, Thumbs.db, *.tmp, ~$… locks, unfinished downloads; over a day old"
            )
            .into(),
            temps,
        ));
    }
    if !empty.is_empty() {
        rows.push(group(
            t!("Boş klasörler", "Empty folders"),
            t!(
                "altında hiç dosya yok; yalnızca en üstteki listelenir",
                "no file anywhere below; only the topmost is listed"
            )
            .into(),
            empty,
        ));
    }
    if !links.is_empty() {
        rows.push(group(
            t!("Kırık bağlantılar", "Broken links"),
            t!(
                "sembolik bağlantı; hedefi artık yok",
                "symbolic links whose target is gone"
            )
            .into(),
            links,
        ));
    }
    (
        rows,
        false,
        t!(
            "Enter: grubu aç · Space: hepsini sepete. Gizli klasörler, paketler, Library ve sistem klasörleri aranmaz.",
            "Enter: open a group · Space: all to the basket. Hidden folders, bundles, Library and system folders are not searched.",
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{Size, ROOT};

    const NOW: u64 = 1_790_856_000;

    /// A tree under `/Users/demo` from `(path, is_dir, age in days)`.
    fn tree(entries: &[(&str, bool, u64)]) -> Tree {
        let mut t = Tree::new(Path::new("/Users/demo"));
        let mut made: Vec<(String, NodeId)> = Vec::new();
        for &(path, is_dir, age) in entries {
            let mut parent = ROOT;
            let parts: Vec<&str> = path.split('/').collect();
            for (i, name) in parts.iter().enumerate() {
                let sofar = parts[..=i].join("/");
                let last = i == parts.len() - 1;
                parent = match made.iter().find(|(p, _)| *p == sofar) {
                    Some(&(_, id)) => id,
                    None => {
                        let dir = !last || is_dir;
                        let size = if dir {
                            Size::default()
                        } else {
                            Size {
                                apparent: 10,
                                disk: 10,
                            }
                        };
                        let id = t.push(parent, name, dir, size);
                        let when = u32::try_from(NOW - age * DAY).unwrap();
                        t.set_times(id, when, when);
                        made.push((sofar, id));
                        id
                    }
                };
            }
        }
        t.finalize();
        t
    }

    fn names(t: &Tree, report: &Report, group: &str) -> Vec<String> {
        let Some(row) = report.0.iter().find(|r| r.label == group) else {
            return Vec::new();
        };
        let mut out: Vec<String> = row
            .nodes
            .iter()
            .map(|&id| {
                let p = t.path_of(id);
                // `/` on every platform.
                p.strip_prefix("/Users/demo")
                    .unwrap()
                    .display()
                    .to_string()
                    .replace('\\', "/")
            })
            .collect();
        out.sort();
        out
    }

    fn run(t: &Tree, min_days: u32) -> Report {
        let age = AgeFilter::new(NOW, min_days);
        clutter_in(
            t,
            ROOT,
            SizeMode::Apparent,
            age,
            NOW,
            Some(Path::new("/Users/demo")),
        )
    }

    #[test]
    fn empty_folders_skip_what_might_matter() {
        let t = tree(&[
            ("Empty/Nested/Deeper", true, 50),
            ("Notes/keep.txt", false, 50),
            ("Notes/Sub", true, 50),
            ("Project/.git/refs/heads", true, 50),
            ("Project/main.rs", false, 50),
            ("Tools/Thing.app/Contents/Resources", true, 50),
            ("Library/Caches/com.example", true, 50),
            ("Documents", true, 50),
            ("Pictures/Old", true, 50),
            ("Projects/web/node_modules/pkg/empty", true, 50),
            ("Repo/.git/objects", true, 50),
            ("Rusty/Cargo.toml", false, 50),
            ("Rusty/target/debug/incremental", true, 50),
            ("App/Podfile", false, 50),
            ("App/Pods/Kit/Kit.xcframework/ios/dSYMs", true, 50),
            ("Gems/Gemfile", false, 50),
            ("Gems/vendor/bundle/doc", true, 50),
            ("Kits/Lib.xcframework/ios-arm64/dSYMs", true, 50),
            ("Kits/notes.md", false, 50),
            ("Projects/web/package.json", false, 50),
        ]);
        let r = run(&t, 0);
        assert_eq!(
            names(&t, &r, "Boş klasörler"),
            // Only the topmost of an empty subtree; ~/Documents is a
            // standard folder, but an empty folder inside ~/Pictures is not.
            ["Empty", "Notes/Sub", "Pictures"]
                .map(String::from)
                .to_vec()
                .into_iter()
                .filter(|p| p != "Pictures")
                .chain(["Pictures/Old".to_string()])
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn unread_folders_are_never_empty() {
        let mut t = tree(&[
            ("Locked", true, 50),
            ("Outer/Inner", true, 50),
            ("Plain", true, 50),
        ]);
        // An unreadable or skipped folder may hold anything, and so may a
        // folder around one.
        for path in ["Locked", "Outer/Inner"] {
            let id = path.split('/').fold(ROOT, |dir, name| {
                t.children(dir).find(|&c| t.name(c) == name).unwrap()
            });
            t.set_unread(id);
        }
        assert_eq!(names(&t, &run(&t, 0), "Boş klasörler"), ["Plain"]);
    }

    #[test]
    fn temporary_files_need_a_quiet_day() {
        let t = tree(&[
            ("Work/report.tmp", false, 3),
            ("Work/fresh.tmp", false, 0),
            ("Work/~$budget.xlsx", false, 2),
            ("Work/~$open.docx", false, 0),
            ("Work/.DS_Store", false, 0),
            ("Downloads/movie.mp4.part", false, 0),
            ("Downloads/old.iso.crdownload", false, 5),
            ("Work/notes.txt", false, 9),
            ("Apps/Thing.app/Contents/cache.tmp", false, 9),
            (".cache/x.tmp", false, 9),
        ]);
        let r = run(&t, 0);
        assert_eq!(
            names(&t, &r, "Geçici dosyalar"),
            [
                "Downloads/old.iso.crdownload",
                "Work/.DS_Store",
                "Work/report.tmp",
                "Work/~$budget.xlsx",
            ]
        );
        // The age filter applies on top.
        let r = run(&t, 4);
        assert_eq!(
            names(&t, &r, "Geçici dosyalar"),
            ["Downloads/old.iso.crdownload"]
        );
    }

    #[test]
    fn system_folders_are_not_searched() {
        let mut t = Tree::new(Path::new("/"));
        for dir in ["System", "private", "usr", "Users"] {
            let d = t.push(ROOT, dir, true, Size::default());
            t.push(d, "Empty", true, Size::default());
        }
        t.finalize();
        let age = AgeFilter::new(NOW, 0);
        let r = clutter_in(&t, ROOT, SizeMode::Apparent, age, NOW, None);
        let row = &r.0[0];
        let paths: Vec<_> = row.nodes.iter().map(|&id| t.path_of(id)).collect();
        assert_eq!(paths, [PathBuf::from("/Users")]);
    }

    #[test]
    fn nothing_found_gives_no_rows() {
        let t = tree(&[("Notes/keep.txt", false, 5)]);
        assert!(run(&t, 0).0.is_empty());
    }
}
