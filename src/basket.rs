//! The basket: entries the user collected for deletion, from anywhere in
//! the tree. A directory in the basket covers everything below it, so
//! nothing is counted (or trashed) twice.

use std::collections::HashSet;

use crate::tree::{NodeId, SizeMode, Tree};

#[derive(Debug, Default)]
pub struct Basket {
    /// In the order they were added.
    items: Vec<NodeId>,
    set: HashSet<NodeId>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Added {
    Yes,
    /// A parent directory is already in the basket.
    CoveredByParent,
}

impl Basket {
    pub fn items(&self) -> &[NodeId] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Whether `id` or one of its ancestors is in the basket.
    pub fn covers(&self, tree: &Tree, id: NodeId) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if self.set.contains(&c) {
                return true;
            }
            cur = tree.parent(c);
        }
        false
    }

    /// Adds `id`; entries below it leave the basket, as it now covers them.
    pub fn add(&mut self, tree: &Tree, id: NodeId) -> Added {
        if self.covers(tree, id) {
            return if self.set.contains(&id) {
                Added::Yes
            } else {
                Added::CoveredByParent
            };
        }
        let below: Vec<NodeId> = self
            .items
            .iter()
            .copied()
            .filter(|&item| is_below(tree, item, id))
            .collect();
        for item in below {
            self.remove(item);
        }
        self.items.push(id);
        self.set.insert(id);
        Added::Yes
    }

    pub fn remove(&mut self, id: NodeId) {
        if self.set.remove(&id) {
            self.items.retain(|&i| i != id);
        }
    }

    /// Drops entries strictly below `dir` (e.g. before `dir` is rescanned and
    /// its old entries stop existing).
    pub fn remove_below(&mut self, tree: &Tree, dir: NodeId) {
        let gone: Vec<NodeId> = self
            .items
            .iter()
            .copied()
            .filter(|&item| is_below(tree, item, dir))
            .collect();
        for item in gone {
            self.remove(item);
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.set.clear();
    }

    pub fn size(&self, tree: &Tree, mode: SizeMode) -> u64 {
        self.items
            .iter()
            .map(|&id| tree.node(id).size.get(mode))
            .sum()
    }
}

/// Whether `id` lies strictly below `dir`.
fn is_below(tree: &Tree, id: NodeId, dir: NodeId) -> bool {
    let mut cur = tree.parent(id);
    while let Some(c) = cur {
        if c == dir {
            return true;
        }
        cur = tree.parent(c);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{Size, ROOT};
    use std::path::Path;

    fn sz(n: u64) -> Size {
        Size {
            apparent: n,
            disk: n,
        }
    }

    #[test]
    fn nested_entries_are_kept_once() {
        let mut t = Tree::new(Path::new("/r"));
        let a = t.push(ROOT, "a", true, sz(0));
        let f1 = t.push(a, "f1", false, sz(10));
        let sub = t.push(a, "sub", true, sz(0));
        let f2 = t.push(sub, "f2", false, sz(5));
        let other = t.push(ROOT, "other", false, sz(100));
        t.finalize();

        let mut b = Basket::default();
        assert_eq!(b.add(&t, f1), Added::Yes);
        assert_eq!(b.add(&t, f2), Added::Yes);
        assert_eq!(b.add(&t, other), Added::Yes);
        assert_eq!(b.size(&t, SizeMode::Disk), 115);

        // Adding the parent replaces the entries below it.
        assert_eq!(b.add(&t, a), Added::Yes);
        assert_eq!(b.items(), &[other, a]);
        assert_eq!(b.size(&t, SizeMode::Disk), 115);
        assert!(b.covers(&t, f2));

        // Something below a basket entry cannot be added on its own.
        assert_eq!(b.add(&t, sub), Added::CoveredByParent);
        assert_eq!(b.len(), 2);

        b.remove(a);
        assert_eq!(b.items(), &[other]);
        b.add(&t, f2);
        b.remove_below(&t, a);
        assert_eq!(b.items(), &[other]);
        assert!(!b.covers(&t, f2));
        b.clear();
        assert!(b.is_empty());
    }
}
