//! "Applications and their data" report.
//!
//! Applications and their data folders are found anywhere in the scan (an app in /Applications keeps its data in
//! ~/Library). Matching data to apps is by name and bundle identifier, so
//! it is a best guess; the report says so.
//!
//! `find` locates the apps, `data` finds their data folders and matches them,
//! and `orphans` lists data whose app is gone. `App` and `DataDir` stay here
//! so all three can use their private fields.

use std::collections::HashMap;
use std::path::Path;

use crate::lists::{ResultList, Row, RowSize};
use crate::reports::{ReportKind, LIMIT};
use crate::tree::{NodeId, SizeMode, Tree, ROOT};
use crate::ui::fmt_size;

mod data;
mod find;
mod orphans;

pub use orphans::orphans;

use data::{data_folders, match_data, preference_files, vendor_children};
use find::find_apps;

#[derive(Debug)]
struct App {
    id: NodeId,
    name: String,
    /// Lowercased bundle identifier (macOS).
    bundle: Option<String>,
    /// Other lowercased names its data may use: CFBundleName and
    /// CFBundleExecutable ("Code" for Visual Studio Code).
    aliases: Vec<String>,
}

/// Apps are listed from the whole scan, wherever the user is browsing:
/// "which programs take the most space" is a question about the disk.
pub fn run(tree: &Tree, mode: SizeMode) -> ResultList {
    let base = ROOT;
    let platform = Platform::current();
    let mut apps = find_apps(tree, base, platform);
    // When the same app exists more than once (an updater's or installer's
    // copy), its data belongs to the installed one: prefer copies inside an
    // "Applications" folder, then the shallowest.
    apps.sort_by_key(|a| {
        let installed = tree
            .parent(a.id)
            .is_some_and(|p| tree.name(p) == "Applications");
        (!installed, tree.path_of(a.id).components().count())
    });
    let (data_dirs, unreadable) = data_folders(tree, platform);
    let mut matched = match_data(&apps, &data_dirs, &|id| vendor_children(tree, id));
    if platform == Platform::MacOs {
        for (m, prefs) in matched.iter_mut().zip(preference_files(tree, &apps)) {
            m.extend(prefs);
        }
    }
    let mut name_counts: HashMap<&str, usize> = HashMap::new();
    for app in &apps {
        *name_counts.entry(app.name.as_str()).or_default() += 1;
    }

    let size = |id: NodeId| tree.node(id).size.get(mode);
    let mut rows: Vec<Row> = apps
        .iter()
        .zip(matched)
        .map(|(app, data)| {
            let app_size = size(app.id);
            let data_size: u64 = data.iter().map(|&d| size(d)).sum();
            let detail = if data_dirs.is_empty() {
                tf!(
                    "uygulama {} · veri taranmadı",
                    "app {} · data not scanned",
                    fmt_size(app_size)
                )
            } else {
                tf!(
                    "uygulama {} · veri {}",
                    "app {} · data {}",
                    fmt_size(app_size),
                    fmt_size(data_size)
                )
            };
            let mut members = vec![(app.id, app_size)];
            members.extend(data.iter().map(|&d| (d, size(d))));
            // Tell apart several copies of the same app by their folder.
            let label = if name_counts[app.name.as_str()] > 1 {
                let parent = tree.parent(app.id).unwrap_or(ROOT);
                let place = crate::lists::relative_label(tree, base, parent);
                let place = place.trim_end_matches('/');
                let place = if place.is_empty() { "." } else { place };
                format!("{} ({place})", app.name)
            } else {
                app.name.clone()
            };
            Row::group(label, detail, members, RowSize::Sum, 1)
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.size()));
    let truncated = rows.len() > LIMIT;
    rows.truncate(LIMIT);

    let mut list = ResultList::new(ReportKind::Apps.label().to_string(), base, rows);
    list.truncated = truncated;
    list.report = Some(ReportKind::Apps);
    list.note = if apps.is_empty() {
        t!(
            "Bu taramada uygulama yok. Uygulamalar için diski tarayın: d → Macintosh HD (Windows: C:\\, Linux: /).",
            "No applications in this scan. Scan the disk to see them: d → Macintosh HD (Windows: C:\\, Linux: /)."
        )
        .into()
    } else if data_dirs.is_empty() {
        t!(
            "Uygulama verileri taranmamış: ev klasörünü ya da diski tarayın.",
            "App data was not scanned: scan the home folder or the disk."
        )
        .into()
    } else {
        let mut note =
            t!(
                "Veri eşleştirmesi ad ve paket kimliğine göre tahmindir; silmeden önce Enter ile gruba bakın.",
                "Data is matched by name and bundle id, so it is a guess; press Enter to check a group before deleting."
            )
            .to_string();
        if unreadable > 0 {
            note.push_str(&tf!(
                " {unreadable} kapsayıcı okunamadı (Tam Disk Erişimi).",
                " {unreadable} containers could not be read (Full Disk Access)."
            ));
        }
        note
    };
    list
}

/// Whether the app at `path` may be uninstalled (moved to the trash with
/// its data). macOS only; the caller checks the platform.
pub fn uninstall_check(path: &Path) -> Result<(), &'static str> {
    let is_bundle = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| strip_suffix_ci(n, ".app"))
        .is_some();
    if !is_bundle {
        return Err(t!(
            "Bu satır bir uygulama değil.",
            "This row is not an application."
        ));
    }
    if path.starts_with("/System") {
        return Err(t!(
            "Sistem uygulaması kaldırılamaz.",
            "System applications cannot be removed."
        ));
    }
    Ok(())
}

/// Whether a running process was started from inside the app bundle.
pub fn is_running(app: &Path) -> bool {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    sys.processes()
        .values()
        .filter_map(|p| p.exe())
        .any(|exe| exe.starts_with(app))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Platform {
    MacOs,
    Windows,
    Linux,
}

impl Platform {
    fn current() -> Self {
        if cfg!(target_os = "macos") {
            Platform::MacOs
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }
}

/// Directories directly below `id`, by name.
fn child_dirs<'a>(tree: &'a Tree, id: NodeId) -> impl Iterator<Item = NodeId> + 'a {
    tree.children(id).filter(|&c| tree.node(c).is_dir)
}

fn child_named(tree: &Tree, id: NodeId, name: &str) -> Option<NodeId> {
    child_dirs(tree, id).find(|&c| tree.name(c).eq_ignore_ascii_case(name))
}

/// The tree's directories, breadth first, down to `depth` levels below the
/// root (system data folders sit near the top).
fn shallow_dirs(tree: &Tree, depth: usize) -> Vec<NodeId> {
    let mut out = vec![ROOT];
    let mut level = vec![ROOT];
    for _ in 0..depth {
        level = level.iter().flat_map(|&d| child_dirs(tree, d)).collect();
        out.extend(&level);
    }
    out
}

fn strip_suffix_ci<'a>(name: &'a str, suffix: &str) -> Option<&'a str> {
    let cut = name.len().checked_sub(suffix.len())?;
    (name.is_char_boundary(cut) && name[cut..].eq_ignore_ascii_case(suffix) && cut > 0)
        .then(|| &name[..cut])
}

/// A folder that may hold an app's data, with the name to match on.
struct DataDir {
    id: NodeId,
    /// Lowercased name (or container identifier) used for matching.
    key: String,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::apps::test_util::{app, data};
    use crate::tree::Size;

    #[test]
    fn mac_report_groups_bundle_and_data() {
        let s = |n| Size {
            apparent: n,
            disk: n,
        };
        let mut t = Tree::new(Path::new("/"));
        let apps_dir = t.push(ROOT, "Applications", true, s(0));
        let foo = t.push(apps_dir, "Foo.app", true, s(0));
        let contents = t.push(foo, "Contents", true, s(0));
        t.push(contents, "Foo", false, s(300));
        let bar = t.push(apps_dir, "Bar.app", true, s(0));
        t.push(bar, "bin", false, s(50));
        let users = t.push(ROOT, "Users", true, s(0));
        let me = t.push(users, "me", true, s(0));
        let lib = t.push(me, "Library", true, s(0));
        let support = t.push(lib, "Application Support", true, s(0));
        let foo_data = t.push(support, "Foo", true, s(0));
        t.push(foo_data, "db", false, s(1000));
        let caches = t.push(lib, "Caches", true, s(0));
        let foo_cache = t.push(caches, "foo", true, s(0));
        t.push(foo_cache, "c", false, s(20));
        t.finalize();

        let (dirs, unreadable) = data_folders(&t, Platform::MacOs);
        assert_eq!(unreadable, 0);
        let apps = find_apps(&t, ROOT, Platform::MacOs);
        let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Bar", "Foo"]);
        // Data goes to the first of several copies with the same bundle id.
        let copies = [
            app(1, "Foo", Some("com.foo")),
            app(2, "Foo", Some("com.foo")),
        ];
        assert_eq!(
            match_data(&copies, &[data(9, "com.foo")], &|_| Vec::new()),
            vec![vec![9], vec![]]
        );
        let m = match_data(&apps, &dirs, &|_| Vec::new());
        assert_eq!(m[1], vec![foo_data, foo_cache]);
        assert!(m[0].is_empty());
    }

    #[test]
    fn uninstall_refuses_system_apps_and_non_apps() {
        assert!(uninstall_check(Path::new("/Applications/Foo.app")).is_ok());
        assert!(uninstall_check(Path::new("/Users/me/Applications/Foo.APP")).is_ok());
        assert!(uninstall_check(Path::new("/System/Applications/Chess.app")).is_err());
        assert!(uninstall_check(Path::new("/Users/me/Library/Caches/foo")).is_err());
    }

    #[test]
    fn running_apps_are_detected_by_executable_path() {
        let exe = std::env::current_exe().unwrap();
        assert!(is_running(exe.parent().unwrap()));
        assert!(!is_running(Path::new("/no/such/Thing.app")));
    }

    #[test]
    fn windows_and_linux_apps() {
        let s = Size::default();
        let mut t = Tree::new(Path::new("C:\\"));
        let pf = t.push(ROOT, "Program Files", true, s);
        let app_dir = t.push(pf, "Foo", true, s);
        let users = t.push(ROOT, "Users", true, s);
        let me = t.push(users, "me", true, s);
        let appdata = t.push(me, "AppData", true, s);
        let local = t.push(appdata, "Local", true, s);
        let foo_local = t.push(local, "foo", true, s);
        t.finalize();
        let apps = find_apps(&t, ROOT, Platform::Windows);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].id, app_dir);
        let (dirs, _) = data_folders(&t, Platform::Windows);
        assert_eq!(
            match_data(&apps, &dirs, &|_| Vec::new())[0],
            vec![foo_local]
        );

        let mut t = Tree::new(Path::new("/"));
        let opt = t.push(ROOT, "opt", true, s);
        let tool = t.push(opt, "tool", true, s);
        let home = t.push(ROOT, "home", true, s);
        let me = t.push(home, "me", true, s);
        let config = t.push(me, ".config", true, s);
        let tool_cfg = t.push(config, "tool", true, s);
        t.finalize();
        let apps = find_apps(&t, ROOT, Platform::Linux);
        assert_eq!(apps[0].id, tool);
        let (dirs, _) = data_folders(&t, Platform::Linux);
        assert_eq!(match_data(&apps, &dirs, &|_| Vec::new())[0], vec![tool_cfg]);
    }
}

#[cfg(test)]
mod test_util {
    use super::*;

    pub(super) fn app(id: NodeId, name: &str, bundle: Option<&str>) -> App {
        App {
            id,
            name: name.into(),
            bundle: bundle.map(str::to_string),
            aliases: Vec::new(),
        }
    }

    pub(super) fn data(id: NodeId, key: &str) -> DataDir {
        DataDir {
            id,
            key: key.to_lowercase(),
        }
    }
}
