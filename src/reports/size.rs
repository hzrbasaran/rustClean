//! Reports by size: largest files, largest folders, old and big files.

use crate::stats::DAY;
use crate::tree::{NodeId, SizeMode, Tree};

use super::{by_size, singles, walk, AgeFilter, Report, Top};

const MIB: u64 = 1024 * 1024;

pub const OLD_BIG_SIZE: u64 = 100 * MIB;

pub const OLD_BIG_AGE: u64 = 365 * DAY;

/// A directory is skipped in "largest folders" when one subdirectory holds
/// more than this share of it: it only wraps that subdirectory.
const WRAPPER_SHARE: f64 = 0.9;

pub(super) fn largest_files(tree: &Tree, base: NodeId, mode: SizeMode, age: AgeFilter) -> Report {
    let mut top = Top::default();
    walk(tree, base, |id| {
        let n = tree.node(id);
        if !n.is_dir && age.ok(n.modified) {
            top.push(n.size.get(mode), id);
        }
        true
    });
    let (ids, truncated) = top.finish();
    let rows = singles(
        tree,
        base,
        mode,
        ids.into_iter().map(|id| (id, String::new())),
    );
    (
        rows,
        truncated,
        t!(
            "Enter: dosyanın bulunduğu klasörü aç",
            "Enter: open the file's folder"
        ),
    )
}

pub(super) fn largest_dirs(tree: &Tree, base: NodeId, mode: SizeMode, age: AgeFilter) -> Report {
    let mut top = Top::default();
    walk(tree, base, |id| {
        let n = tree.node(id);
        if !n.is_dir {
            return false;
        }
        let total = n.size.get(mode);
        let biggest_child = tree
            .children(id)
            .filter(|&c| tree.node(c).is_dir)
            .map(|c| tree.node(c).size.get(mode))
            .max()
            .unwrap_or(0);
        let wrapper = total > 0 && biggest_child as f64 > total as f64 * WRAPPER_SHARE;
        if total > 0 && !wrapper && age.ok(n.modified) {
            top.push(total, id);
        }
        true
    });
    let (ids, truncated) = top.finish();
    let rows = singles(
        tree,
        base,
        mode,
        ids.into_iter().map(|id| (id, String::new())),
    );
    let note = t!(
        "İç içe klasörler ayrıca listelenebilir; boyutları birbirini içerir.",
        "Nested folders can be listed separately; their sizes include each other."
    );
    (rows, truncated, note)
}

pub(super) fn old_big(tree: &Tree, base: NodeId, mode: SizeMode, now: u64) -> Report {
    let mut found = Vec::new();
    walk(tree, base, |id| {
        let n = tree.node(id);
        let old = n.modified != 0 && now.saturating_sub(u64::from(n.modified)) > OLD_BIG_AGE;
        if !n.is_dir && old && n.size.get(mode) >= OLD_BIG_SIZE {
            found.push((id, String::new()));
        }
        true
    });
    let (items, truncated) = by_size(tree, mode, found);
    let rows = singles(tree, base, mode, items);
    (
        rows,
        truncated,
        t!(
            "≥ 100 MiB ve 1 yıldan uzun süredir değişmemiş dosyalar.",
            "Files ≥ 100 MiB, unchanged for more than a year."
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::reports::test_util::{labels, report, sz};
    use crate::reports::{run, ReportKind};
    use crate::stats::DAY;
    use crate::tree::{SizeMode, Tree, ROOT};

    #[test]
    fn largest_files_in_order() {
        let mut t = Tree::new(Path::new("/r"));
        let d = t.push(ROOT, "d", true, sz(0));
        t.push(d, "small", false, sz(1));
        t.push(d, "big", false, sz(30));
        t.push(ROOT, "mid", false, sz(20));
        t.finalize();
        let l = report(&t, ReportKind::LargestFiles);
        assert_eq!(labels(&l), vec!["d/big", "mid", "d/small"]);
        assert!(!l.truncated);
    }

    #[test]
    fn largest_dirs_skip_wrappers() {
        let mut t = Tree::new(Path::new("/r"));
        // lib -> dev -> sim(1000): lib and dev only wrap sim.
        let lib = t.push(ROOT, "lib", true, sz(0));
        let dev = t.push(lib, "dev", true, sz(0));
        let sim = t.push(dev, "sim", true, sz(0));
        t.push(sim, "img", false, sz(1000));
        t.push(lib, "pref", false, sz(5));
        // proj holds two comparable subdirectories: a real aggregate.
        let proj = t.push(ROOT, "proj", true, sz(0));
        let a = t.push(proj, "a", true, sz(0));
        t.push(a, "f", false, sz(300));
        let b = t.push(proj, "b", true, sz(0));
        t.push(b, "f", false, sz(200));
        t.finalize();
        let l = report(&t, ReportKind::LargestDirs);
        assert_eq!(
            labels(&l),
            vec!["lib/dev/sim/", "proj/", "proj/a/", "proj/b/"]
        );
    }

    #[test]
    fn old_big_thresholds() {
        let now = 1000 * DAY;
        let mut t = Tree::new(Path::new("/r"));
        let old_big = t.push(ROOT, "old_big", false, sz(OLD_BIG_SIZE));
        t.set_times(old_big, (now - 400 * DAY) as u32, 0);
        let new_big = t.push(ROOT, "new_big", false, sz(OLD_BIG_SIZE * 2));
        t.set_times(new_big, (now - 10 * DAY) as u32, 0);
        let old_small = t.push(ROOT, "old_small", false, sz(OLD_BIG_SIZE - 1));
        t.set_times(old_small, (now - 400 * DAY) as u32, 0);
        t.push(ROOT, "unknown_date", false, sz(OLD_BIG_SIZE * 3));
        t.finalize();
        let l = run(&t, ROOT, SizeMode::Disk, now, ReportKind::OldBig, 0);
        assert_eq!(labels(&l), vec!["old_big"]);
    }
}
