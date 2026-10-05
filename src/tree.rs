//! Arena-based file tree produced by a scan.
//!
//! Nodes live in a flat `Vec`; children always have a larger id than their
//! parent, which lets size aggregation run in a single reverse pass. A full
//! disk can hold tens of millions of entries, so nodes are kept small: ids
//! are `u32`, children form a sibling-linked list and all names share one
//! byte buffer.

use std::collections::{HashMap, HashSet};
use std::ops::{AddAssign, SubAssign};
use std::path::{Path, PathBuf};

pub type NodeId = u32;
pub const ROOT: NodeId = 0;
const NONE: NodeId = NodeId::MAX;

/// Which size to show: the file length, or the space allocated on disk
/// (what `du` reports; differs for sparse, compressed and tiny files).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeMode {
    Apparent,
    Disk,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Size {
    pub apparent: u64,
    pub disk: u64,
}

impl Size {
    pub fn get(self, mode: SizeMode) -> u64 {
        match mode {
            SizeMode::Apparent => self.apparent,
            SizeMode::Disk => self.disk,
        }
    }
}

impl AddAssign for Size {
    fn add_assign(&mut self, rhs: Self) {
        self.apparent += rhs.apparent;
        self.disk += rhs.disk;
    }
}

impl SubAssign for Size {
    fn sub_assign(&mut self, rhs: Self) {
        self.apparent = self.apparent.saturating_sub(rhs.apparent);
        self.disk = self.disk.saturating_sub(rhs.disk);
    }
}

#[derive(Debug)]
pub struct Node {
    /// Own size for files. For directories, their own size plus that of all
    /// descendants (after `finalize`).
    pub size: Size,
    name_start: u64,
    /// 1 for files; number of files below for directories (after `finalize`).
    pub file_count: u32,
    /// Last modification, seconds since the Unix epoch (0 = unknown). For
    /// directories, the newest modification of anything below (after
    /// `finalize`).
    pub modified: u32,
    /// Creation time, seconds since the Unix epoch (0 = unknown).
    pub created: u32,
    parent: NodeId,
    first_child: NodeId,
    next_sibling: NodeId,
    name_len: u16,
    pub is_dir: bool,
    /// A symbolic link (never followed; counted as a file).
    pub is_link: bool,
    /// A directory whose contents were not read: it could not be opened, is
    /// on another file system, or was skipped. It may hold anything.
    pub unread: bool,
}

#[derive(Debug)]
pub struct Tree {
    nodes: Vec<Node>,
    names: String,
    root_path: PathBuf,
    /// APFS clones: `(clone id, private size)` of files sharing blocks.
    /// Few files are clones, so this stays small.
    clones: HashMap<NodeId, (u64, u64)>,
}

impl Tree {
    pub fn new(root_path: &Path) -> Self {
        let mut tree = Self {
            nodes: Vec::new(),
            names: String::new(),
            root_path: root_path.to_path_buf(),
            clones: HashMap::new(),
        };
        tree.add(
            NONE,
            &root_path.display().to_string(),
            true,
            Size::default(),
        );
        tree
    }

    pub fn push(&mut self, parent: NodeId, name: &str, is_dir: bool, size: Size) -> NodeId {
        let id = self.add(parent, name, is_dir, size);
        let p = &mut self.nodes[parent as usize];
        let old_first = std::mem::replace(&mut p.first_child, id);
        self.nodes[id as usize].next_sibling = old_first;
        id
    }

    fn add(&mut self, parent: NodeId, name: &str, is_dir: bool, size: Size) -> NodeId {
        let id = NodeId::try_from(self.nodes.len()).expect("too many entries for a u32 id");
        // File names are at most 255 bytes on every supported filesystem;
        // lossy UTF-8 conversion can triple that, which still fits a u16.
        let name = &name[..floor_char_boundary(name, u16::MAX as usize)];
        let name_start = self.names.len() as u64;
        self.names.push_str(name);
        self.nodes.push(Node {
            size,
            name_start,
            file_count: u32::from(!is_dir),
            parent,
            modified: 0,
            created: 0,
            first_child: NONE,
            next_sibling: NONE,
            name_len: name.len() as u16,
            is_dir,
            is_link: false,
            unread: false,
        });
        id
    }

    /// Detaches `id` (and everything below it) from the tree and subtracts its
    /// size and file count from every ancestor. Call after `finalize`. The
    /// node's storage is not reclaimed.
    pub fn remove(&mut self, id: NodeId) {
        let Some(parent) = self.parent(id) else {
            return;
        };
        let next = self.node(id).next_sibling;
        if self.node(parent).first_child == id {
            self.nodes[parent as usize].first_child = next;
        } else {
            let mut cur = self.node(parent).first_child;
            while cur != NONE && self.node(cur).next_sibling != id {
                cur = self.node(cur).next_sibling;
            }
            if cur == NONE {
                return; // already detached
            }
            self.nodes[cur as usize].next_sibling = next;
        }
        self.nodes[id as usize].next_sibling = NONE;

        let (size, count) = (self.node(id).size, self.node(id).file_count);
        let mut ancestor = Some(parent);
        while let Some(a) = ancestor {
            let n = &mut self.nodes[a as usize];
            n.size -= size;
            n.file_count = n.file_count.saturating_sub(count);
            ancestor = self.parent(a);
        }
    }

    pub fn set_clone(&mut self, id: NodeId, clone_id: u64, private: u64) {
        self.clones.insert(id, (clone_id, private));
    }

    /// `(clone id, private size)` when the file is an APFS clone.
    pub fn clone_of(&self, id: NodeId) -> Option<(u64, u64)> {
        self.clones.get(&id).copied()
    }

    /// Replaces everything below `dir` with the entries of `sub`, a fresh
    /// (finalized) scan of the same directory. Old entries are detached as
    /// with `remove`; sizes and file counts of `dir` and its ancestors are
    /// updated. Newest-change times only move forward, as with `remove`.
    pub fn replace_children(&mut self, dir: NodeId, sub: &Tree) {
        let old: Vec<NodeId> = self.children(dir).collect();
        let mut below = old.clone();
        while let Some(id) = below.pop() {
            self.clones.remove(&id);
            below.extend(self.children(id));
        }
        for child in old {
            self.remove(child);
        }
        // Clone sets whose shared blocks a file outside `dir` already counts
        // (the one holding more than its private bytes).
        let outside: HashSet<u64> = self
            .clones
            .iter()
            .filter(|&(&id, &(_, private))| self.node(id).size.disk > private)
            .map(|(_, c)| c.0)
            .collect();
        // `sub` lists parents before children, so one pass maps every id.
        let mut map = vec![NONE; sub.nodes.len()];
        map[ROOT as usize] = dir;
        for sid in 1..sub.nodes.len() {
            let n = &sub.nodes[sid];
            let parent = map[n.parent as usize];
            let id = self.push(parent, sub.name(sid as NodeId), n.is_dir, n.size);
            let new = &mut self.nodes[id as usize];
            new.file_count = n.file_count;
            new.modified = n.modified;
            new.created = n.created;
            new.is_link = n.is_link;
            new.unread = n.unread;
            map[sid] = id;
            if let Some(&c) = sub.clones.get(&(sid as NodeId)) {
                self.clones.insert(id, c);
            }
        }
        let total = sub.node(ROOT);
        let (size, count, modified) = (total.size, total.file_count, total.modified);
        let mut ancestor = Some(dir);
        while let Some(a) = ancestor {
            let n = &mut self.nodes[a as usize];
            n.size += size;
            n.file_count += count;
            n.modified = n.modified.max(modified);
            ancestor = self.parent(a);
        }
        // `sub` gave one of each clone set its full size; if a clone outside
        // `dir` already holds it, count only the private bytes here.
        for (&sid, &(clone_id, private)) in &sub.clones {
            let id = map[sid as usize];
            let extra = self.node(id).size.disk.saturating_sub(private);
            if extra == 0 || !outside.contains(&clone_id) {
                continue;
            }
            let mut cur = Some(id);
            while let Some(a) = cur {
                self.nodes[a as usize].size.disk -= extra;
                cur = self.parent(a);
            }
        }
    }

    pub fn set_link(&mut self, id: NodeId) {
        self.nodes[id as usize].is_link = true;
    }

    pub fn set_unread(&mut self, id: NodeId) {
        self.nodes[id as usize].unread = true;
    }

    pub fn set_times(&mut self, id: NodeId, modified: u32, created: u32) {
        let n = &mut self.nodes[id as usize];
        n.modified = modified;
        n.created = created;
    }

    /// Rolls file sizes, counts and newest modification times up into their
    /// parent directories.
    pub fn finalize(&mut self) {
        for id in (1..self.nodes.len()).rev() {
            let n = &self.nodes[id];
            let (size, count, modified, parent) =
                (n.size, n.file_count, n.modified, n.parent as usize);
            let p = &mut self.nodes[parent];
            p.size += size;
            p.file_count += count;
            p.modified = p.modified.max(modified);
        }
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    pub fn name(&self, id: NodeId) -> &str {
        let n = self.node(id);
        let start = n.name_start as usize;
        &self.names[start..start + n.name_len as usize]
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        Some(self.node(id).parent).filter(|&p| p != NONE)
    }

    /// Children of `id`, in no particular order.
    pub fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        let mut next = self.node(id).first_child;
        std::iter::from_fn(move || {
            let cur = next;
            (cur != NONE).then(|| {
                next = self.node(cur).next_sibling;
                cur
            })
        })
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    /// Full filesystem path of a node.
    pub fn path_of(&self, id: NodeId) -> PathBuf {
        let mut names = Vec::new();
        let mut cur = id;
        while let Some(parent) = self.parent(cur) {
            names.push(self.name(cur));
            cur = parent;
        }
        let mut path = self.root_path.clone();
        path.extend(names.iter().rev());
        path
    }
}

/// Largest index `<= max` that falls on a char boundary of `s`.
fn floor_char_boundary(s: &str, max: usize) -> usize {
    if s.len() <= max {
        return s.len();
    }
    (0..=max)
        .rev()
        .find(|&i| s.is_char_boundary(i))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sz(apparent: u64, disk: u64) -> Size {
        Size { apparent, disk }
    }

    #[test]
    fn finalize_aggregates_sizes_and_counts() {
        let mut t = Tree::new(Path::new("/r"));
        let a = t.push(ROOT, "a", true, sz(0, 1));
        t.push(a, "f1", false, sz(10, 100));
        let b = t.push(a, "b", true, sz(0, 1));
        t.push(b, "f2", false, sz(5, 50));
        t.push(ROOT, "f3", false, sz(1, 10));
        t.finalize();

        assert_eq!(t.node(ROOT).size, sz(16, 162));
        assert_eq!(t.node(ROOT).file_count, 3);
        assert_eq!(t.node(a).size, sz(15, 152));
        assert_eq!(t.node(b).size.get(SizeMode::Disk), 51);
        assert_eq!(t.path_of(b), PathBuf::from("/r/a/b"));
        assert_eq!(t.path_of(ROOT), PathBuf::from("/r"));
    }

    #[test]
    fn finalize_propagates_newest_modification() {
        let mut t = Tree::new(Path::new("/r"));
        let a = t.push(ROOT, "a", true, Size::default());
        t.set_times(a, 100, 50);
        let old = t.push(a, "old", false, Size::default());
        t.set_times(old, 200, 200);
        let new = t.push(a, "new", false, Size::default());
        t.set_times(new, 900, 300);
        let b = t.push(ROOT, "b", true, Size::default());
        t.set_times(b, 400, 40);
        t.finalize();

        assert_eq!(t.node(a).modified, 900);
        assert_eq!(t.node(a).created, 50); // creation is not propagated
        assert_eq!(t.node(b).modified, 400);
        assert_eq!(t.node(ROOT).modified, 900);
    }

    #[test]
    fn names_and_children() {
        let mut t = Tree::new(Path::new("/r"));
        let a = t.push(ROOT, "alpha", true, Size::default());
        let b = t.push(ROOT, "beta.txt", false, Size::default());
        let c = t.push(a, "ğüşçö", false, Size::default());

        assert_eq!(t.name(a), "alpha");
        assert_eq!(t.name(b), "beta.txt");
        assert_eq!(t.name(c), "ğüşçö");
        let mut kids: Vec<_> = t.children(ROOT).collect();
        kids.sort();
        assert_eq!(kids, vec![a, b]);
        assert_eq!(t.children(b).count(), 0);
        assert_eq!(t.parent(c), Some(a));
        assert_eq!(t.parent(ROOT), None);
    }

    #[test]
    fn remove_updates_ancestors() {
        let mut t = Tree::new(Path::new("/r"));
        let a = t.push(ROOT, "a", true, sz(0, 0));
        let f1 = t.push(a, "f1", false, sz(10, 100));
        let b = t.push(a, "b", true, sz(0, 0));
        t.push(b, "f2", false, sz(5, 50));
        let f3 = t.push(a, "f3", false, sz(1, 10));
        t.finalize();

        // Middle of the sibling list (children are prepended: f3, b, f1).
        t.remove(b);
        assert_eq!(t.node(a).size, sz(11, 110));
        assert_eq!(t.node(ROOT).size, sz(11, 110));
        assert_eq!(t.node(ROOT).file_count, 2);
        let mut kids: Vec<_> = t.children(a).collect();
        kids.sort();
        assert_eq!(kids, vec![f1, f3]);

        // Head of the list, then removing twice is a no-op.
        t.remove(f3);
        t.remove(f3);
        assert_eq!(t.children(a).collect::<Vec<_>>(), vec![f1]);
        assert_eq!(t.node(ROOT).size, sz(10, 100));

        t.remove(ROOT);
        assert_eq!(t.node(ROOT).size, sz(10, 100));
    }

    #[test]
    fn replace_children_matches_a_fresh_tree() {
        // Before: /r/a/{old: 100, keep/ (f: 5)}, /r/b (10).
        let mut t = Tree::new(Path::new("/r"));
        let a = t.push(ROOT, "a", true, sz(0, 1));
        t.push(a, "old", false, sz(100, 100));
        let keep = t.push(a, "keep", true, sz(0, 1));
        t.push(keep, "f", false, sz(5, 5));
        t.push(ROOT, "b", false, sz(10, 10));
        t.finalize();

        // A fresh scan of /r/a: "old" is gone, "keep" grew, "new" appeared.
        let mut sub = Tree::new(Path::new("/r/a"));
        let keep2 = sub.push(ROOT, "keep", true, sz(0, 1));
        let f = sub.push(keep2, "f", false, sz(7, 7));
        sub.set_times(f, 500, 400);
        sub.push(ROOT, "new", false, sz(20, 20));
        sub.finalize();

        t.replace_children(a, &sub);

        // a: own block 1 + keep (1 + 7) + new 20.
        assert_eq!(t.node(a).size, sz(27, 29));
        assert_eq!(t.node(a).file_count, 2);
        assert_eq!(t.node(ROOT).size, sz(37, 39));
        assert_eq!(t.node(ROOT).file_count, 3);
        assert_eq!(t.node(ROOT).modified, 500);
        let mut names: Vec<&str> = t.children(a).map(|c| t.name(c)).collect();
        names.sort();
        assert_eq!(names, vec!["keep", "new"]);
        let keep = t.children(a).find(|&c| t.name(c) == "keep").unwrap();
        let f = t.children(keep).next().unwrap();
        assert_eq!(t.node(f).size, sz(7, 7));
        assert_eq!(t.node(f).created, 400);
        assert_eq!(t.path_of(f), PathBuf::from("/r/a/keep/f"));
    }

    #[test]
    fn node_stays_small() {
        assert!(size_of::<Node>() <= 56);
    }
}
