//! Finding installed applications: in the scan, and on disk for the
//! leftovers report.

use std::path::{Path, PathBuf};

use crate::tree::{NodeId, Tree, ROOT};

use super::{child_dirs, strip_suffix_ci, App, Platform};

pub(super) fn find_apps(tree: &Tree, base: NodeId, platform: Platform) -> Vec<App> {
    let mut apps = Vec::new();
    let mut stack = vec![base];
    while let Some(dir) = stack.pop() {
        for c in child_dirs(tree, dir) {
            let name = tree.name(c);
            match platform {
                Platform::MacOs => {
                    if let Some(stem) = strip_suffix_ci(name, ".app") {
                        let (bundle, aliases) = bundle_info(&tree.path_of(c));
                        apps.push(App {
                            id: c,
                            name: stem.to_string(),
                            bundle,
                            aliases,
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
                            aliases: Vec::new(),
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
                            aliases: Vec::new(),
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
                    aliases: Vec::new(),
                });
            }
        }
    }
    apps
}

/// Bundle identifier and alternative names from the app's Info.plist.
fn bundle_info(app: &Path) -> (Option<String>, Vec<String>) {
    let Some(dict) = plist::Value::from_file(app.join("Contents/Info.plist"))
        .ok()
        .and_then(|v| v.into_dictionary())
    else {
        return (None, Vec::new());
    };
    let get = |k: &str| {
        dict.get(k)
            .and_then(|v| v.as_string())
            .map(str::to_lowercase)
    };
    let aliases = ["CFBundleName", "CFBundleExecutable"]
        .iter()
        .filter_map(|k| get(k))
        .filter(|a| a.len() > 2)
        .collect();
    (get("CFBundleIdentifier"), aliases)
}

pub(super) fn plist_string(path: &Path, key: &str) -> Option<String> {
    let value = plist::Value::from_file(path).ok()?;
    let s = value.as_dictionary()?.get(key)?.as_string()?;
    Some(s.to_lowercase())
}

/// Apps installed in `dirs` (and their subfolders, e.g. Utilities), read
/// from disk: only names and bundle ids are needed, not sizes. They are not
/// in the tree, so their `id` is a placeholder.
pub(super) fn installed_on_disk(dirs: &[PathBuf]) -> Vec<App> {
    const DEPTH: usize = 3;
    let mut out = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = dirs.iter().map(|d| (d.clone(), 0)).collect();
    while let Some((dir, depth)) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if !kind.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(stem) = strip_suffix_ci(&name, ".app") {
                let (bundle, aliases) = bundle_info(&entry.path());
                out.push(App {
                    id: ROOT,
                    name: stem.to_string(),
                    bundle,
                    aliases,
                });
            } else if depth + 1 < DEPTH {
                stack.push((entry.path(), depth + 1));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn reads_installed_apps_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path();
        let foo = apps.join("Foo.app/Contents");
        std::fs::create_dir_all(&foo).unwrap();
        std::fs::write(
            foo.join("Info.plist"),
            r#"<?xml version="1.0"?><plist version="1.0"><dict>
            <key>CFBundleIdentifier</key><string>com.example.Foo</string>
            <key>CFBundleName</key><string>FooApp</string></dict></plist>"#,
        )
        .unwrap();
        std::fs::create_dir_all(apps.join("Utilities/Bar.app/Contents")).unwrap();
        // Bundles are not entered: an app inside an app is not listed.
        std::fs::create_dir_all(apps.join("Foo.app/Contents/Helpers/Inner.app")).unwrap();

        let mut found = installed_on_disk(&[apps.to_path_buf(), apps.join("missing")]);
        found.sort_by(|a, b| a.name.cmp(&b.name));
        let names: Vec<&str> = found.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Bar", "Foo"]);
        assert_eq!(found[1].bundle.as_deref(), Some("com.example.foo"));
        assert_eq!(found[1].aliases, vec!["fooapp".to_string()]);
    }
}
