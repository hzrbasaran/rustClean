//! Finding app data folders and matching them to apps by name, bundle
//! identifier and vendor folder. Matching is a best guess.

use std::collections::HashMap;

use crate::tree::{NodeId, Tree, ROOT};

use super::find::plist_string;
use super::{child_dirs, child_named, shallow_dirs, strip_suffix_ci, App, DataDir, Platform};

/// Candidate data folders anywhere in the scan, and how many containers
/// could not be identified.
pub(super) fn data_folders(tree: &Tree, platform: Platform) -> (Vec<DataDir>, usize) {
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
            for lib in libraries(tree) {
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

/// ~/Library and /Library: a "Library" with "Application Support".
fn libraries(tree: &Tree) -> Vec<NodeId> {
    shallow_dirs(tree, 4)
        .into_iter()
        .filter(|&d| {
            let name = if d == ROOT {
                tree.root_path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
            } else {
                tree.name(d)
            };
            name == "Library" && child_named(tree, d, "Application Support").is_some()
        })
        .collect()
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
pub(super) fn container_core(key: &str) -> &str {
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

/// Subfolders of a possible vendor folder ("Google/Chrome").
pub(super) fn vendor_children(tree: &Tree, id: NodeId) -> Vec<DataDir> {
    child_dirs(tree, id)
        .map(|c| DataDir {
            id: c,
            key: tree.name(c).to_lowercase(),
        })
        .collect()
}

/// Lowercase name without spaces, dashes and underscores.
pub(super) fn normalize(name: &str) -> String {
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
pub(super) fn match_data(
    apps: &[App],
    data: &[DataDir],
    children: &dyn Fn(NodeId) -> Vec<DataDir>,
) -> Vec<Vec<NodeId>> {
    match_all(apps, data, children).0
}

/// Like `match_data`, also returning the data folders no app claimed
/// (neither the folder itself nor anything inside it as a vendor folder).
pub(super) fn match_all(
    apps: &[App],
    data: &[DataDir],
    children: &dyn Fn(NodeId) -> Vec<DataDir>,
) -> (Vec<Vec<NodeId>>, Vec<usize>) {
    let mut by_name: HashMap<String, usize> = HashMap::new();
    for (i, app) in apps.iter().enumerate() {
        by_name.entry(app.name.to_lowercase()).or_insert(i);
    }
    for (i, app) in apps.iter().enumerate() {
        for alias in &app.aliases {
            by_name.entry(alias.clone()).or_insert(i);
        }
    }
    let mut out = vec![Vec::new(); apps.len()];
    let mut unmatched = Vec::new();
    for (di, d) in data.iter().enumerate() {
        let by_bundle = bundle_owner(apps, container_core(&d.key));
        if let Some(i) = by_bundle.or_else(|| by_name.get(&d.key).copied()) {
            out[i].push(d.id);
            continue;
        }
        let mut claimed = false;
        for sub in children(d.id) {
            if let Some(i) = vendor_match(apps, &d.key, &sub.key) {
                out[i].push(sub.id);
                claimed = true;
            }
        }
        if !claimed {
            unmatched.push(di);
        }
    }
    (out, unmatched)
}

/// The app whose bundle identifier `key` is, or extends at a dot
/// (`com.foo.app.helper` belongs to `com.foo.app`, `com.foo.application`
/// does not). The most specific bundle id wins; among equals, the earliest
/// (preferred) app.
fn bundle_owner(apps: &[App], key: &str) -> Option<usize> {
    apps.iter()
        .enumerate()
        .filter_map(|(i, a)| a.bundle.as_deref().map(|b| (i, b)))
        .filter(|(_, b)| {
            key == *b
                || (key.len() > b.len() && key.starts_with(b) && key.as_bytes()[b.len()] == b'.')
        })
        .max_by_key(|&(i, b)| (b.len(), std::cmp::Reverse(i)))
        .map(|(i, _)| i)
}

/// Preference files (`Library/Preferences/<bundle id>….plist`, also in
/// `ByHost`) of each app. Matched by bundle id only: names are too vague for
/// loose files.
pub(super) fn preference_files(tree: &Tree, apps: &[App]) -> Vec<Vec<NodeId>> {
    let mut out = vec![Vec::new(); apps.len()];
    for lib in libraries(tree) {
        let Some(prefs) = child_named(tree, lib, "Preferences") else {
            continue;
        };
        let by_host = child_named(tree, prefs, "ByHost");
        for dir in std::iter::once(prefs).chain(by_host) {
            for f in tree.children(dir).filter(|&c| !tree.node(c).is_dir) {
                let Some(stem) = strip_suffix_ci(tree.name(f), ".plist") else {
                    continue;
                };
                if let Some(i) = bundle_owner(apps, &stem.to_lowercase()) {
                    out[i].push(f);
                }
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
    use std::path::Path;

    use super::*;
    use crate::apps::test_util::{app, data};
    use crate::tree::Size;

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
    fn aliases_and_unmatched() {
        let mut code = app(1, "Visual Studio Code", Some("com.microsoft.vscode"));
        code.aliases = vec!["code".into(), "electron".into()];
        let apps = [code];
        let dirs = [
            data(10, "Code"),
            data(11, "OldTool"),
            data(12, "com.removed.app"),
            data(13, "Google"),
        ];
        let children = |id: NodeId| match id {
            13 => vec![data(14, "Chrome")], // no Chrome installed
            _ => Vec::new(),
        };
        let (matched, unmatched) = match_all(&apps, &dirs, &children);
        assert_eq!(matched[0], vec![10]);
        assert_eq!(unmatched, vec![1, 2, 3]);
    }

    #[test]
    fn uuid_detection() {
        assert!(looks_like_uuid("061A356F-BE63-4E17-ACEC-907231AA9FD1"));
        assert!(!looks_like_uuid("com.apple.Safari"));
        assert!(!looks_like_uuid("061A356F-BE63-4E17-ACEC-907231AA9FDX"));
    }

    #[test]
    fn preference_files_match_by_bundle_id_only() {
        let mut t = Tree::new(Path::new("/Users/me"));
        let lib = t.push(ROOT, "Library", true, Size::default());
        t.push(lib, "Application Support", true, Size::default());
        let prefs = t.push(lib, "Preferences", true, Size::default());
        let by_host = t.push(prefs, "ByHost", true, Size::default());
        let file = |t: &mut Tree, dir, name: &str| t.push(dir, name, false, Size::default());
        let own = file(&mut t, prefs, "com.foo.app.plist");
        let helper = file(&mut t, prefs, "com.foo.app.Helper.plist");
        let host = file(
            &mut t,
            by_host,
            "com.foo.app.0A1B2C3D-0000-1111-2222-333344445555.plist",
        );
        file(&mut t, prefs, "com.foo.application.plist"); // another app
        file(&mut t, prefs, "Foo.plist"); // names are not enough
        file(&mut t, prefs, "com.foo.app.lockfile"); // not a plist
        let other = file(&mut t, prefs, "com.bar.plist");
        t.finalize();

        let apps = [
            app(1, "Foo", Some("com.foo.app")),
            app(2, "Bar", Some("com.bar")),
            app(3, "NoId", None),
        ];
        let mut found = preference_files(&t, &apps);
        found[0].sort();
        let mut want = vec![own, helper, host];
        want.sort();
        assert_eq!(found, vec![want, vec![other], vec![]]);
    }
}
