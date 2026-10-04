//! Disk images, installers and archives left in Downloads folders.

use crate::tree::{NodeId, SizeMode, Tree};

use super::{by_size, singles, walk, AgeFilter, Report};

/// What a downloaded file is, by its extension: disk images, installers and
/// archives, which are rarely needed once used.
fn download_kind(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "dmg" | "iso" | "img" | "sparseimage" => t!("disk görüntüsü", "disk image"),
        "pkg" | "mpkg" | "msi" | "exe" | "deb" | "rpm" | "appimage" => {
            t!("yükleyici", "installer")
        }
        "zip" | "xip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" => {
            t!("arşiv", "archive")
        }
        _ => return None,
    })
}

fn is_downloads(name: &str) -> bool {
    name.eq_ignore_ascii_case("Downloads")
}

pub(super) fn downloads(tree: &Tree, base: NodeId, mode: SizeMode, age: AgeFilter) -> Report {
    // The browsed folder is in Downloads, or Downloads folders are below it.
    let inside = tree
        .path_of(base)
        .components()
        .any(|c| c.as_os_str().to_str().is_some_and(is_downloads));
    let mut roots = Vec::new();
    if inside {
        roots.push(base);
    } else {
        walk(tree, base, |id| {
            let n = tree.node(id);
            if n.is_dir && is_downloads(tree.name(id)) {
                roots.push(id);
                return false;
            }
            n.is_dir
        });
    }
    if roots.is_empty() {
        return (
            Vec::new(),
            false,
            t!(
                "Bu klasörün altında Downloads yok: ev klasörünü ya da diski tarayın.",
                "No Downloads folder below this one: scan the home folder or the disk."
            ),
        );
    }
    let mut found = Vec::new();
    for root in roots {
        walk(tree, root, |id| {
            let n = tree.node(id);
            let name = tree.name(id);
            if !n.is_dir && age.ok(n.modified) {
                if let Some(kind) = download_kind(name) {
                    found.push((id, kind.to_string()));
                }
            }
            // An app's resources are not downloads.
            n.is_dir && !name.to_ascii_lowercase().ends_with(".app")
        });
    }
    let (items, truncated) = by_size(tree, mode, found);
    let rows = singles(tree, base, mode, items);
    (
        rows,
        truncated,
        t!(
            "Kurulumu bitmiş yükleyiciler genelde yeniden indirilebilir; açılmış arşivlerin içeriği yerinde durur.",
            "Installers you have used can usually be downloaded again; extracted archives keep their contents."
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::reports::test_util::{labels, sz};
    use crate::reports::{run, ReportKind};
    use crate::stats::DAY;
    use crate::tree::{SizeMode, Tree, ROOT};

    #[test]
    fn downloads_lists_installers_and_archives_in_downloads_only() {
        let now = 1000 * DAY;
        let mut t = Tree::new(Path::new("/Users/me"));
        let dl = t.push(ROOT, "Downloads", true, sz(0));
        let dmg = t.push(dl, "Tool.DMG", false, sz(500));
        t.set_times(dmg, (now - 200 * DAY) as u32, 0);
        let sub = t.push(dl, "drivers", true, sz(0));
        let pkg = t.push(sub, "driver.pkg", false, sz(300));
        t.set_times(pkg, (now - 5 * DAY) as u32, 0);
        let tgz = t.push(dl, "src.tar.gz", false, sz(100));
        t.set_times(tgz, (now - 400 * DAY) as u32, 0);
        t.push(dl, "photo.jpg", false, sz(900));
        let app = t.push(dl, "Some.app", true, sz(0));
        t.push(app, "resources.zip", false, sz(50));
        // Archives elsewhere are kept on purpose (backups, projects).
        let docs = t.push(ROOT, "Documents", true, sz(0));
        t.push(docs, "backup.zip", false, sz(800));
        t.finalize();

        let l = run(&t, ROOT, SizeMode::Disk, now, ReportKind::Downloads, 0);
        assert_eq!(
            labels(&l),
            vec![
                "Downloads/Tool.DMG",
                "Downloads/drivers/driver.pkg",
                "Downloads/src.tar.gz"
            ]
        );
        let details: Vec<&str> = l.rows.iter().map(|r| r.detail.as_str()).collect();
        assert_eq!(details, vec!["disk görüntüsü", "yükleyici", "arşiv"]);

        // Age filter.
        let l = run(&t, ROOT, SizeMode::Disk, now, ReportKind::Downloads, 180);
        assert_eq!(
            labels(&l),
            vec!["Downloads/Tool.DMG", "Downloads/src.tar.gz"]
        );

        // Browsing inside Downloads.
        let l = run(&t, sub, SizeMode::Disk, now, ReportKind::Downloads, 0);
        assert_eq!(labels(&l), vec!["driver.pkg"]);

        // No Downloads below.
        let l = run(&t, docs, SizeMode::Disk, now, ReportKind::Downloads, 0);
        assert!(l.rows.is_empty());
        assert!(l.note.contains("Downloads yok"), "{}", l.note);
    }
}
