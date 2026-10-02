//! Arena-based file tree produced by a scan.
//!
//! Nodes live in a flat `Vec`; children always have a larger id than their
//! parent, which lets size aggregation run in a single reverse pass. A full
//! disk can hold tens of millions of entries, so nodes are kept small: ids
//! are `u32`, children form a sibling-linked list and all names share one
//! byte buffer.

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
    parent: NodeId,
    first_child: NodeId,
    next_sibling: NodeId,
    name_len: u16,
    pub is_dir: bool,
}

#[derive(Debug)]
pub struct Tree {
    nodes: Vec<Node>,
    names: String,
    root_path: PathBuf,
}

impl Tree {
    pub fn new(root_path: &Path) -> Self {
        let mut tree = Self {
            nodes: Vec::new(),
            names: String::new(),
            root_path: root_path.to_path_buf(),
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
            first_child: NONE,
            next_sibling: NONE,
            name_len: name.len() as u16,
            is_dir,
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

    /// Rolls file sizes and counts up into their parent directories.
    pub fn finalize(&mut self) {
        for id in (1..self.nodes.len()).rev() {
            let n = &self.nodes[id];
            let (size, count, parent) = (n.size, n.file_count, n.parent as usize);
            let p = &mut self.nodes[parent];
            p.size += size;
            p.file_count += count;
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
    fn node_stays_small() {
        assert!(std::mem::size_of::<Node>() <= 48);
    }
}
