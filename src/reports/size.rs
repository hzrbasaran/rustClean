//! Reports by size: largest files, largest folders, old and big files.

use crate::stats::DAY;
use crate::tree::{NodeId, SizeMode, Tree};

use super::{by_size, singles, walk, AgeFilter, Report, Top};

const MIB: u64 = 1024 * 1024;

/// "Old and large files" without a configuration: at least this size…
pub const OLD_BIG_SIZE: u64 = 100 * MIB;

/// …and unchanged for longer than this.
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

/// Files of at least `old_big_min_mib`, unchanged for more than
/// `old_big_min_days` (configuration). The note is `old_big_note`.
pub(super) fn old_big(tree: &Tree, base: NodeId, mode: SizeMode, now: u64) -> Report {
    let config = crate::config::get();
    let (min_size, min_age) = (config.old_big_min_size(), config.old_big_min_age());
    let mut found = Vec::new();
    walk(tree, base, |id| {
        let n = tree.node(id);
        let old = n.modified != 0 && now.saturating_sub(u64::from(n.modified)) > min_age;
        if !n.is_dir && old && n.size.get(mode) >= min_size {
            found.push((id, String::new()));
        }
        true
    });
    let (items, truncated) = by_size(tree, mode, found);
    let rows = singles(tree, base, mode, items);
    (rows, truncated, "")
}

/// The note of "old and large files", with the configured thresholds.
pub(super) fn old_big_note() -> String {
    let config = crate::config::get();
    let size = crate::config::fmt_mib(config.old_big_min_mib);
    let (_, than, en) = age_words(config.old_big_min_days);
    crate::i18n::pick(
        format!("≥ {size} ve {than} uzun süredir değişmemiş dosyalar."),
        format!("Files ≥ {size}, unchanged for more than {en}."),
    )
}

/// "1 yıldır" / "a year", "90 gündür" / "90 days", for the menu line.
pub(super) fn age_for(days: u64) -> String {
    let (since, _, en) = age_words(days);
    crate::i18n::pick(since, en)
}

/// An age in whole years when it is one, else in days: the Turkish "for"
/// ("1 yıldır") and "than" ("1 yıldan") forms, and the English one.
fn age_words(days: u64) -> (String, String, String) {
    if days.is_multiple_of(365) {
        let y = days / 365;
        let en = if y == 1 {
            "a year".into()
        } else {
            format!("{y} years")
        };
        (format!("{y} yıldır"), format!("{y} yıldan"), en)
    } else {
        let en = if days == 1 {
            "a day".into()
        } else {
            format!("{days} days")
        };
        (format!("{days} gündür"), format!("{days} günden"), en)
    }
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
        assert_eq!(
            l.note,
            "≥ 100 MiB ve 1 yıldan uzun süredir değişmemiş dosyalar."
        );
    }

    #[test]
    fn old_big_follows_the_configuration() {
        let now = 1000 * DAY;
        let mut t = Tree::new(Path::new("/r"));
        let a = t.push(ROOT, "five_mib_40_days", false, sz(5 * MIB));
        t.set_times(a, (now - 40 * DAY) as u32, 0);
        let b = t.push(ROOT, "four_mib_400_days", false, sz(4 * MIB));
        t.set_times(b, (now - 400 * DAY) as u32, 0);
        let c = t.push(ROOT, "big_20_days", false, sz(OLD_BIG_SIZE));
        t.set_times(c, (now - 20 * DAY) as u32, 0);
        t.finalize();
        let config = crate::config::Config {
            old_big_min_mib: 5,
            old_big_min_days: 30,
            ..Default::default()
        };
        crate::config::with(config, || {
            let l = run(&t, ROOT, SizeMode::Disk, now, ReportKind::OldBig, 0);
            assert_eq!(labels(&l), vec!["five_mib_40_days"]);
            assert_eq!(
                l.note,
                "≥ 5 MiB ve 30 günden uzun süredir değişmemiş dosyalar."
            );
            assert_eq!(
                ReportKind::OldBig.description(),
                "5 MiB'tan büyük, 30 gündür değişmemiş dosyalar"
            );
            crate::i18n::set_lang(crate::i18n::Lang::En);
            let note = old_big_note();
            let description = ReportKind::OldBig.description();
            crate::i18n::set_lang(crate::i18n::Lang::Tr);
            assert_eq!(note, "Files ≥ 5 MiB, unchanged for more than 30 days.");
            assert_eq!(description, "Files over 5 MiB, unchanged for 30 days");
        });
        // Whole years and GiB read naturally.
        let config = crate::config::Config {
            old_big_min_mib: 2048,
            old_big_min_days: 730,
            ..Default::default()
        };
        crate::config::with(config, || {
            assert_eq!(
                ReportKind::OldBig.description(),
                "2 GiB'tan büyük, 2 yıldır değişmemiş dosyalar"
            );
        });
        assert_eq!(
            ReportKind::OldBig.description(),
            "100 MiB'tan büyük, 1 yıldır değişmemiş dosyalar"
        );
    }
}
