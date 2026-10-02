//! Arena-based file tree produced by a scan.
//!
//! Nodes live in a flat `Vec`; children always have a larger id than their
//! parent, which lets size aggregation run in a single reverse pass.

use std::path::{Path, PathBuf};

pub type NodeId = usize;
pub const ROOT: NodeId = 0;

#[derive(Debug)]
pub struct Node {
    pub name: Box<str>,
    /// Own size for files; total size of all descendants for directories
    /// (after `finalize`).
    pub size: u64,
    /// 1 for files; number of files below for directories (after `finalize`).
    pub file_count: u64,
    pub is_dir: bool,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
}

#[derive(Debug)]
pub struct Tree {
    nodes: Vec<Node>,
    root_path: PathBuf,
}

impl Tree {
    pub fn new(root_path: &Path) -> Self {
        let root = Node {
            name: root_path.display().to_string().into(),
            size: 0,
            file_count: 0,
            is_dir: true,
            parent: None,
            children: Vec::new(),
        };
        Self {
            nodes: vec![root],
            root_path: root_path.to_path_buf(),
        }
    }

    pub fn push(&mut self, parent: NodeId, name: &str, is_dir: bool, size: u64) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node {
            name: name.into(),
            size,
            file_count: u64::from(!is_dir),
            is_dir,
            parent: Some(parent),
            children: Vec::new(),
        });
        self.nodes[parent].children.push(id);
        id
    }

    /// Rolls file sizes and counts up into their parent directories.
    pub fn finalize(&mut self) {
        for id in (1..self.nodes.len()).rev() {
            let (size, count, parent) = {
                let n = &self.nodes[id];
                (
                    n.size,
                    n.file_count,
                    n.parent.expect("non-root node has a parent"),
                )
            };
            let p = &mut self.nodes[parent];
            p.size += size;
            p.file_count += count;
        }
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
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
        while let Some(parent) = self.nodes[cur].parent {
            names.push(&*self.nodes[cur].name);
            cur = parent;
        }
        let mut path = self.root_path.clone();
        path.extend(names.iter().rev());
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalize_aggregates_sizes_and_counts() {
        let mut t = Tree::new(Path::new("/r"));
        let a = t.push(ROOT, "a", true, 0);
        t.push(a, "f1", false, 10);
        let b = t.push(a, "b", true, 0);
        t.push(b, "f2", false, 5);
        t.push(ROOT, "f3", false, 1);
        t.finalize();

        assert_eq!(t.node(ROOT).size, 16);
        assert_eq!(t.node(ROOT).file_count, 3);
        assert_eq!(t.node(a).size, 15);
        assert_eq!(t.node(b).size, 5);
        assert_eq!(t.path_of(b), PathBuf::from("/r/a/b"));
        assert_eq!(t.path_of(ROOT), PathBuf::from("/r"));
    }
}
