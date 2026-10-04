//! File names that repeat most often.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use xxhash_rust::xxh3::xxh3_64;

use crate::lists::{Row, RowSize};
use crate::tree::{NodeId, SizeMode, Tree};

use super::{walk, AgeFilter, Report, LIMIT};

/// Case-insensitive hash of a file name; ASCII names (nearly all) are
/// lowercased on the stack.
fn name_hash(name: &str) -> u64 {
    let mut buf = [0u8; 255];
    if name.is_ascii() && name.len() <= buf.len() {
        let lower = &mut buf[..name.len()];
        lower.copy_from_slice(name.as_bytes());
        lower.make_ascii_lowercase();
        xxh3_64(lower)
    } else {
        xxh3_64(name.to_lowercase().as_bytes())
    }
}

/// The keys are already hashes; hashing them again would only cost time.
#[derive(Default)]
struct IdentityHasher(u64);

impl Hasher for IdentityHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 << 8) | u64::from(b);
        }
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}

type HashKeyed<V> = HashMap<u64, V, BuildHasherDefault<IdentityHasher>>;

pub(super) fn repeated_names(tree: &Tree, base: NodeId, mode: SizeMode, age: AgeFilter) -> Report {
    // Pass 1: count per name (by hash, to keep memory small on big disks).
    let mut counts: HashKeyed<(u32, u64)> = HashKeyed::default();
    walk(tree, base, |id| {
        let n = tree.node(id);
        if !n.is_dir && age.ok(n.modified) {
            let e = counts.entry(name_hash(tree.name(id))).or_default();
            e.0 += 1;
            e.1 += n.size.get(mode);
        }
        true
    });
    let mut repeated: Vec<(u64, u32, u64)> = counts
        .into_iter()
        .filter(|(_, (count, _))| *count >= 2)
        .map(|(h, (count, size))| (h, count, size))
        .collect();
    repeated.sort_by_key(|&(h, count, size)| (Reverse(count), Reverse(size), h));
    let truncated = repeated.len() > LIMIT;
    repeated.truncate(LIMIT);
    let order: HashKeyed<usize> = repeated
        .iter()
        .enumerate()
        .map(|(i, &(h, ..))| (h, i))
        .collect();

    // Pass 2: collect the members of the shown names.
    let mut members: Vec<Vec<(NodeId, u64)>> = vec![Vec::new(); repeated.len()];
    walk(tree, base, |id| {
        let n = tree.node(id);
        if !n.is_dir && age.ok(n.modified) {
            if let Some(&i) = order.get(&name_hash(tree.name(id))) {
                members[i].push((id, n.size.get(mode)));
            }
        }
        true
    });
    let rows = members
        .into_iter()
        .filter(|m| m.len() >= 2)
        .map(|mut m| {
            m.sort_by_key(|&(id, size)| (Reverse(size), id));
            let label = tree.name(m[0].0).to_string();
            Row::group(label, String::new(), m, RowSize::Sum, 2)
        })
        .collect();
    (
        rows,
        truncated,
        t!(
            "Enter: bu adı taşıyan dosyaları listele",
            "Enter: list the files with this name"
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::reports::test_util::{labels, report, sz};
    use crate::reports::ReportKind;

    use crate::tree::{Tree, ROOT};

    #[test]
    fn repeated_names_group_case_insensitively() {
        let mut t = Tree::new(Path::new("/r"));
        let a = t.push(ROOT, "a", true, sz(0));
        let b = t.push(ROOT, "b", true, sz(0));
        let c = t.push(ROOT, "c", true, sz(0));
        for dir in [a, b, c] {
            t.push(dir, "package.json", false, sz(10));
        }
        t.push(a, "README.md", false, sz(5));
        t.push(b, "readme.MD", false, sz(7));
        t.push(c, "unique.txt", false, sz(100));
        t.finalize();

        let l = report(&t, ReportKind::RepeatedNames);
        assert_eq!(labels(&l), vec!["package.json", "readme.MD"]);
        assert!(l.rows.iter().all(|r| r.group));
        assert_eq!(l.rows[0].nodes.len(), 3);
        assert_eq!(l.rows[0].size(), 30);
        assert_eq!(l.rows[1].size(), 12);
    }
}
