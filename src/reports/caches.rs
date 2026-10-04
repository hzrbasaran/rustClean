//! Per-app cache folders.

use crate::tree::{NodeId, SizeMode, Tree};

use super::{by_size, singles, walk, AgeFilter, Report};

pub(super) fn caches(tree: &Tree, base: NodeId, mode: SizeMode, age: AgeFilter) -> Report {
    const CONTAINERS: [&str; 2] = ["caches", ".cache"];
    const SELF: [&str; 6] = [
        "cache",
        "code cache",
        "gpucache",
        "cacheddata",
        "shadercache",
        "dawncache",
    ];
    let mut found = Vec::new();
    let mut visit_dir = |id: NodeId| -> bool {
        if !tree.node(id).is_dir {
            return false;
        }
        let name = tree.name(id).to_lowercase();
        if CONTAINERS.contains(&name.as_str()) {
            found.extend(
                tree.children(id)
                    .map(|c| (c, tf!("{} içinde", "in {}", tree.name(id)))),
            );
            false
        } else if SELF.contains(&name.as_str()) {
            found.push((id, t!("önbellek", "cache").to_string()));
            false
        } else {
            true
        }
    };
    // The browsed directory itself may be a cache container.
    if visit_dir(base) {
        walk(tree, base, &mut visit_dir);
    }
    found.retain(|(id, _)| tree.node(*id).size.get(mode) > 0 && age.ok(tree.node(*id).modified));
    let (items, truncated) = by_size(tree, mode, found);
    let rows = singles(tree, base, mode, items);
    (
        rows,
        truncated,
        t!(
            "Önbellekler silinince uygulamalar gerektiğinde yeniden oluşturur.",
            "Apps recreate caches when needed after they are deleted."
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::reports::test_util::{labels, report, sz};
    use crate::reports::{run, ReportKind};

    use crate::tree::{SizeMode, Tree, ROOT};

    #[test]
    fn caches_list_per_app_entries() {
        let mut t = Tree::new(Path::new("/r"));
        let lib = t.push(ROOT, "Library", true, sz(0));
        let caches = t.push(lib, "Caches", true, sz(0));
        let a = t.push(caches, "com.a", true, sz(0));
        t.push(a, "blob", false, sz(70));
        let b = t.push(caches, "com.b", true, sz(0));
        t.push(b, "blob", false, sz(30));
        let app = t.push(ROOT, "Chrome", true, sz(0));
        let gpu = t.push(app, "GPUCache", true, sz(0));
        t.push(gpu, "data", false, sz(40));
        t.finalize();

        let l = report(&t, ReportKind::Caches);
        assert_eq!(
            labels(&l),
            vec![
                "Library/Caches/com.a/",
                "Chrome/GPUCache/",
                "Library/Caches/com.b/"
            ]
        );

        // Opened directly on a cache container, its entries are listed.
        let l = run(&t, caches, SizeMode::Disk, 0, ReportKind::Caches, 0);
        assert_eq!(labels(&l), vec!["com.a/", "com.b/"]);
    }
}
