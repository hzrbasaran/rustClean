//! "Applications and their data" report.
//!
//! Applications and their data folders are found anywhere in the scan (an app in /Applications keeps its data in
//! ~/Library). Matching data to apps is by name and bundle identifier, so
//! it is a best guess; the report says so.

use std::collections::HashMap;
use std::path::Path;

use crate::lists::{ResultList, Row, RowSize};
use crate::reports::{ReportKind, LIMIT};
use crate::tree::{NodeId, SizeMode, Tree, ROOT};
use crate::ui::fmt_size;

#[derive(Debug)]
struct App {
    id: NodeId,
    name: String,
    /// Lowercased bundle identifier (macOS).
    bundle: Option<String>,
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
    let vendor_children = |id: NodeId| {
        child_dirs(tree, id)
            .map(|c| DataDir {
                id: c,
                key: tree.name(c).to_lowercase(),
            })
            .collect()
    };
    let matched = match_data(&apps, &data_dirs, &vendor_children);
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
                format!("uygulama {} · veri taranmadı", fmt_size(app_size))
            } else {
                format!(
                    "uygulama {} · veri {}",
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
    list.note = if apps.is_empty() {
        "Bu taramada uygulama yok. Uygulamalar için diski tarayın: d → Macintosh HD (Windows: C:\\, Linux: /)."
            .into()
    } else if data_dirs.is_empty() {
        "Uygulama verileri taranmamış: ev klasörünü ya da diski tarayın.".into()
    } else {
        let mut note =
            "Veri eşleştirmesi ad ve paket kimliğine göre tahmindir; silmeden önce Enter ile gruba bakın."
                .to_string();
        if unreadable > 0 {
            note.push_str(&format!(
                " {unreadable} kapsayıcı okunamadı (Tam Disk Erişimi)."
            ));
        }
        note
    };
    list
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

fn find_apps(tree: &Tree, base: NodeId, platform: Platform) -> Vec<App> {
    let mut apps = Vec::new();
    let mut stack = vec![base];
    while let Some(dir) = stack.pop() {
        for c in child_dirs(tree, dir) {
            let name = tree.name(c);
            match platform {
                Platform::MacOs => {
                    if let Some(stem) = strip_suffix_ci(name, ".app") {
                        apps.push(App {
                            id: c,
                            name: stem.to_string(),
                            bundle: bundle_id(&tree.path_of(c)),
                        });
                        continue; // never look inside bundles
                    }
                }
                Platform::Windows => {
                    if name.eq_ignore_ascii_case("Program Files")
                        || name.eq_ignore_ascii_case("Program Files (x86)")
                    {
                        apps.extend(child_dirs(tree, c).map(|a| App {
                            id: a,
                            name: tree.name(a).to_string(),
                            bundle: None,
                        }));
                        continue;
                    }
                }
                Platform::Linux => {
                    if name == "opt" && tree.parent(c) == Some(ROOT) {
                        apps.extend(child_dirs(tree, c).map(|a| App {
                            id: a,
                            name: tree.name(a).to_string(),
                            bundle: None,
                        }));
                        continue;
                    }
                }
            }
            stack.push(c);
        }
    }
    // An app in Program Files whose own folder is the browsed one.
    if platform != Platform::MacOs {
        if let Some(parent) = tree.parent(base) {
            let pname = tree.name(parent);
            if pname.eq_ignore_ascii_case("Program Files")
                || pname.eq_ignore_ascii_case("Program Files (x86)")
            {
                apps.push(App {
                    id: base,
                    name: tree.name(base).to_string(),
                    bundle: None,
                });
            }
        }
    }
    apps
}

fn strip_suffix_ci<'a>(name: &'a str, suffix: &str) -> Option<&'a str> {
    let cut = name.len().checked_sub(suffix.len())?;
    (name.is_char_boundary(cut) && name[cut..].eq_ignore_ascii_case(suffix) && cut > 0)
        .then(|| &name[..cut])
}

fn bundle_id(app: &Path) -> Option<String> {
    plist_string(&app.join("Contents/Info.plist"), "CFBundleIdentifier")
}

fn plist_string(path: &Path, key: &str) -> Option<String> {
    let value = plist::Value::from_file(path).ok()?;
    let s = value.as_dictionary()?.get(key)?.as_string()?;
    Some(s.to_lowercase())
}

/// A folder that may hold an app's data, with the name to match on.
struct DataDir {
    id: NodeId,
    /// Lowercased name (or container identifier) used for matching.
    key: String,
}

/// Candidate data folders anywhere in the scan, and how many containers
/// could not be identified.
fn data_folders(tree: &Tree, platform: Platform) -> (Vec<DataDir>, usize) {
    let mut out = Vec::new();
    let mut unreadable = 0;
    let add_children = |dir: NodeId, out: &mut Vec<DataDir>| {
        for c in child_dirs(tree, dir) {
            out.push(DataDir {
                id: c,
                key: tree.name(c).to_lowercase(),
            });
        }
    };
    match platform {
        Platform::MacOs => {
            const KINDS: [&str; 7] = [
                "Application Support",
                "Caches",
                "Group Containers",
                "Logs",
                "HTTPStorages",
                "WebKit",
                "Saved Application State",
            ];
            // ~/Library and /Library: a "Library" with "Application Support".
            let libraries = shallow_dirs(tree, 4).into_iter().filter(|&d| {
                let name = if d == ROOT {
                    tree.root_path()
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                } else {
                    tree.name(d)
                };
                name == "Library" && child_named(tree, d, "Application Support").is_some()
            });
            for lib in libraries {
                for kind in KINDS {
                    if let Some(dir) = child_named(tree, lib, kind) {
                        add_children(dir, &mut out);
                    }
                }
                // Sandboxed apps; newer macOS names some containers by UUID
                // and stores the bundle id in a metadata file.
                if let Some(containers) = child_named(tree, lib, "Containers") {
                    for c in child_dirs(tree, containers) {
                        let name = tree.name(c);
                        let key = if looks_like_uuid(name) {
                            let meta = tree
                                .path_of(c)
                                .join(".com.apple.containermanagerd.metadata.plist");
                            match plist_string(&meta, "MCMMetadataIdentifier") {
                                Some(id) => id,
                                None => {
                                    unreadable += 1;
                                    continue;
                                }
                            }
                        } else {
                            name.to_lowercase()
                        };
                        out.push(DataDir { id: c, key });
                    }
                }
            }
        }
        Platform::Windows => {
            for appdata in shallow_dirs(tree, 4)
                .into_iter()
                .filter(|&d| tree.name(d).eq_ignore_ascii_case("AppData"))
            {
                for kind in ["Local", "LocalLow", "Roaming"] {
                    if let Some(dir) = child_named(tree, appdata, kind) {
                        add_children(dir, &mut out);
                    }
                }
            }
        }
        Platform::Linux => {
            for home in shallow_dirs(tree, 3) {
                for sub in [".config", ".cache"] {
                    if let Some(dir) = child_named(tree, home, sub) {
                        add_children(dir, &mut out);
                    }
                }
                if let Some(share) =
                    child_named(tree, home, ".local").and_then(|l| child_named(tree, l, "share"))
                {
                    add_children(share, &mut out);
                }
            }
        }
    }
    (out, unreadable)
}

fn looks_like_uuid(name: &str) -> bool {
    name.len() == 36
        && name.char_indices().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

/// Strips a team identifier ("ABCDE12345.") and "group(s)." prefix from a
/// (lowercased) container name.
fn container_core(key: &str) -> &str {
    let mut key = key;
    if let Some((team, rest)) = key.split_once('.') {
        if team.len() == 10 && team.chars().all(|c| c.is_ascii_alphanumeric()) {
            key = rest;
        }
    }
    for prefix in ["group.", "groups."] {
        if let Some(rest) = key.strip_prefix(prefix) {
            return rest;
        }
    }
    key
}

/// Lowercase name without spaces, dashes and underscores.
fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_'))
        .flat_map(char::to_lowercase)
        .collect()
}

/// For each app, the data folders that belong to it. Each folder goes to
/// at most one app: the one with the most specific match. Folders that
/// match no app are treated as vendor folders ("Google", "Microsoft") and
/// their subfolders are matched by name: "Google/Chrome" belongs to
/// "Google Chrome", "Google/AndroidStudio2024.1" to "Android Studio".
fn match_data(
    apps: &[App],
    data: &[DataDir],
    children: &dyn Fn(NodeId) -> Vec<DataDir>,
) -> Vec<Vec<NodeId>> {
    let mut by_name: HashMap<String, usize> = HashMap::new();
    for (i, app) in apps.iter().enumerate() {
        by_name.entry(app.name.to_lowercase()).or_insert(i);
    }
    let mut out = vec![Vec::new(); apps.len()];
    for d in data {
        let core = container_core(&d.key);
        let by_bundle = apps
            .iter()
            .enumerate()
            .filter_map(|(i, a)| a.bundle.as_deref().map(|b| (i, b)))
            .filter(|(_, b)| {
                core == *b
                    || (core.len() > b.len()
                        && core.starts_with(b)
                        && core.as_bytes()[b.len()] == b'.')
            })
            // Most specific bundle id; among equals, the earliest (preferred) app.
            .max_by_key(|&(i, b)| (b.len(), std::cmp::Reverse(i)))
            .map(|(i, _)| i);
        if let Some(i) = by_bundle.or_else(|| by_name.get(&d.key).copied()) {
            out[i].push(d.id);
            continue;
        }
        for sub in children(d.id) {
            if let Some(i) = vendor_match(apps, &d.key, &sub.key) {
                out[i].push(sub.id);
            }
        }
    }
    out
}

/// The app a folder `vendor/sub` belongs to, if any.
fn vendor_match(apps: &[App], vendor: &str, sub: &str) -> Option<usize> {
    let vendor = normalize(vendor);
    let sub = normalize(sub);
    apps.iter().position(|app| {
        let full = normalize(&app.name);
        let short = full.strip_prefix(vendor.as_str()).unwrap_or(&full);
        let hit = [full.as_str(), short].into_iter().any(|name| {
            !name.is_empty()
                && sub.strip_prefix(name).is_some_and(|rest| {
                    rest.is_empty() || rest.starts_with(|c: char| c.is_ascii_digit() || c == '.')
                })
        });
        hit
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::Size;

    fn app(id: NodeId, name: &str, bundle: Option<&str>) -> App {
        App {
            id,
            name: name.into(),
            bundle: bundle.map(str::to_string),
        }
    }

    fn data(id: NodeId, key: &str) -> DataDir {
        DataDir {
            id,
            key: key.to_lowercase(),
        }
    }

    #[test]
    fn matches_by_bundle_name_and_team_prefix() {
        let apps = [
            app(1, "Xcode", Some("com.apple.dt.xcode")),
            app(2, "Slack", Some("com.tinyspeck.slackmacgap")),
            app(3, "Notes", None),
        ];
        let dirs = [
            data(10, "com.apple.dt.Xcode"),
            data(11, "com.apple.dt.XcodeCloud"), // a different app
            data(12, "S8EX82NJP6.com.tinyspeck.slackmacgap.helper"),
            data(13, "Slack"),
            data(14, "243LU875E5.groups.com.tinyspeck.slackmacgap"),
            data(15, "notes"),
            data(16, "com.unrelated"),
        ];
        let m = match_data(&apps, &dirs, &|_| Vec::new());
        assert_eq!(m[0], vec![10]);
        assert_eq!(m[1], vec![12, 13, 14]);
        assert_eq!(m[2], vec![15]);
    }

    #[test]
    fn matches_inside_vendor_folders() {
        let apps = [
            app(1, "Google Chrome", Some("com.google.chrome")),
            app(2, "Android Studio", Some("com.google.android.studio")),
            app(3, "Microsoft Teams", None),
        ];
        let dirs = [data(10, "Google"), data(20, "Microsoft")];
        let children = |id: NodeId| match id {
            10 => vec![
                data(11, "Chrome"),
                data(12, "AndroidStudio2024.1"),
                data(13, "Chrome for Testing"),
                data(14, "Chrome-headless"),
            ],
            20 => vec![data(21, "Teams"), data(22, "Office")],
            _ => Vec::new(),
        };
        let m = match_data(&apps, &dirs, &children);
        assert_eq!(m[0], vec![11]);
        assert_eq!(m[1], vec![12]);
        assert_eq!(m[2], vec![21]);
    }

    #[test]
    fn uuid_detection() {
        assert!(looks_like_uuid("061A356F-BE63-4E17-ACEC-907231AA9FD1"));
        assert!(!looks_like_uuid("com.apple.Safari"));
        assert!(!looks_like_uuid("061A356F-BE63-4E17-ACEC-907231AA9FDX"));
    }

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
