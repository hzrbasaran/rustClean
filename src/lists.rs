//! Result lists shown in place of the directory listing: search results and
//! reports. Rows are single entries or groups of entries that can be
//! drilled into.

use std::collections::HashSet;
use std::path::PathBuf;

use ratatui::widgets::TableState;

use crate::reports::ReportKind;
use crate::tree::{NodeId, SizeMode, Tree};

/// How a group row's size is derived from its members.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowSize {
    /// Total of all members.
    Sum,
    /// Space freed by keeping one copy: total minus the largest member.
    Wasted,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub label: String,
    /// Extra information shown in the last column.
    pub detail: String,
    pub nodes: Vec<NodeId>,
    sizes: Vec<u64>,
    kind: RowSize,
    /// Group rows can be drilled into.
    pub group: bool,
    /// The row disappears when fewer members remain.
    min_members: usize,
}

impl Row {
    /// A group of entries. It is dropped once fewer than `min_members`
    /// remain (2 for "same name" or "same content" groups).
    pub fn group(
        label: String,
        detail: String,
        members: Vec<(NodeId, u64)>,
        kind: RowSize,
        min_members: usize,
    ) -> Self {
        let (nodes, sizes) = members.into_iter().unzip();
        Self {
            label,
            detail,
            nodes,
            sizes,
            kind,
            group: true,
            min_members,
        }
    }

    /// A row for one entry, labelled with its path relative to `base`.
    pub fn single(tree: &Tree, base: NodeId, id: NodeId, mode: SizeMode, detail: String) -> Self {
        Self {
            label: relative_label(tree, base, id),
            detail,
            nodes: vec![id],
            sizes: vec![tree.node(id).size.get(mode)],
            kind: RowSize::Sum,
            group: false,
            min_members: 1,
        }
    }

    pub fn size(&self) -> u64 {
        let total: u64 = self.sizes.iter().sum();
        match self.kind {
            RowSize::Sum => total,
            RowSize::Wasted => total - self.sizes.iter().max().copied().unwrap_or(0),
        }
    }

    fn retain_nodes(&mut self, removed: &HashSet<NodeId>) {
        let keep: Vec<bool> = self.nodes.iter().map(|id| !removed.contains(id)).collect();
        let mut k = keep.iter();
        self.nodes.retain(|_| *k.next().unwrap());
        let mut k = keep.iter();
        self.sizes.retain(|_| *k.next().unwrap());
    }
}

/// Path of `id` relative to `base`, with a trailing `/` for directories.
pub fn relative_label(tree: &Tree, base: NodeId, id: NodeId) -> String {
    let base_path = tree.path_of(base);
    let path = tree.path_of(id);
    let mut label = path
        .strip_prefix(&base_path)
        .unwrap_or(&path)
        .display()
        .to_string();
    if tree.node(id).is_dir {
        label.push('/');
    }
    label
}

/// What a result list was built from, so it can be built again: after the
/// language changed, with another age filter, or after the tree changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The members of a group row of the parent list; rebuilt by opening
    /// that group again.
    Members,
    /// A report, with its minimum age filter in days.
    Report { kind: ReportKind, min_age_days: u32 },
    /// Search results for a pattern.
    Search(String),
    /// Changes since the saved scan in this file.
    Changes(PathBuf),
    /// Groups of identical files. The groups are the list's own rows, so
    /// rebuilding needs no new search.
    Duplicates,
    /// Groups of similar images, like `Duplicates`.
    Similar,
    /// The basket.
    Basket,
}

pub struct ResultList {
    pub title: String,
    /// Hint shown under the list.
    pub note: String,
    /// Directory the list was computed for.
    pub base: NodeId,
    pub rows: Vec<Row>,
    pub table: TableState,
    /// More results existed than are shown.
    pub truncated: bool,
    pub source: Source,
    /// Groups of copies: selecting a group selects all but its oldest copy.
    pub keep_one: bool,
    /// In a list of copies, the one to keep when selecting all.
    pub keep: Option<NodeId>,
    /// The group list this list was drilled down from.
    pub parent: Option<Box<ResultList>>,
}

impl ResultList {
    pub fn new(title: String, base: NodeId, rows: Vec<Row>) -> Self {
        let mut table = TableState::default();
        table.select((!rows.is_empty()).then_some(0));
        Self {
            title,
            note: String::new(),
            base,
            rows,
            table,
            truncated: false,
            source: Source::Members,
            keep_one: false,
            keep: None,
            parent: None,
        }
    }

    /// The report this list came from.
    pub fn report(&self) -> Option<ReportKind> {
        match self.source {
            Source::Report { kind, .. } => Some(kind),
            _ => None,
        }
    }

    /// Minimum age filter of the report, in days (0: none).
    pub fn min_age_days(&self) -> u32 {
        match self.source {
            Source::Report { min_age_days, .. } => min_age_days,
            _ => 0,
        }
    }

    /// This list shows the basket itself.
    pub fn is_basket(&self) -> bool {
        self.source == Source::Basket
    }

    pub fn selected_row(&self) -> Option<&Row> {
        self.table.selected().and_then(|i| self.rows.get(i))
    }

    pub fn total_size(&self) -> u64 {
        self.rows.iter().map(Row::size).sum()
    }

    /// Replaces this list with `child`, remembering it for `back`.
    pub fn drill_into(&mut self, mut child: ResultList) {
        std::mem::swap(self, &mut child);
        self.parent = Some(Box::new(child));
    }

    /// Returns to the parent list; false when this is the top level.
    pub fn back(&mut self) -> bool {
        match self.parent.take() {
            Some(parent) => {
                *self = *parent;
                true
            }
            None => false,
        }
    }

    /// Re-reads member sizes, e.g. after switching between apparent and
    /// on-disk sizes.
    pub fn resize(&mut self, size_of: &impl Fn(NodeId) -> u64) {
        for row in &mut self.rows {
            for (size, &id) in row.sizes.iter_mut().zip(&row.nodes) {
                *size = size_of(id);
            }
        }
        if let Some(parent) = &mut self.parent {
            parent.resize(size_of);
        }
    }

    /// Drops deleted entries from this list and its parents. Rows left empty
    /// disappear, and so do groups that no longer have anything to compare.
    pub fn remove_nodes(&mut self, removed: &HashSet<NodeId>) {
        let mut i = 0;
        while i < self.rows.len() {
            let row = &mut self.rows[i];
            row.retain_nodes(removed);
            if row.nodes.len() < row.min_members {
                self.rows.remove(i);
            } else {
                i += 1;
            }
        }
        match self.table.selected() {
            _ if self.rows.is_empty() => self.table.select(None),
            Some(s) if s >= self.rows.len() => self.table.select(Some(self.rows.len() - 1)),
            None => self.table.select(Some(0)),
            _ => {}
        }
        if let Some(parent) = &mut self.parent {
            parent.remove_nodes(removed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(label: &str, members: &[(NodeId, u64)], kind: RowSize) -> Row {
        Row::group(label.into(), String::new(), members.to_vec(), kind, 2)
    }

    fn one(label: &str, id: NodeId, size: u64) -> Row {
        Row::group(
            label.into(),
            String::new(),
            vec![(id, size)],
            RowSize::Sum,
            1,
        )
    }

    #[test]
    fn row_sizes() {
        assert_eq!(group("a", &[(1, 10), (2, 30)], RowSize::Sum).size(), 40);
        assert_eq!(
            group("d", &[(1, 30), (2, 30), (3, 30)], RowSize::Wasted).size(),
            60
        );
    }

    #[test]
    fn drill_and_remove() {
        let groups = vec![
            group("dup", &[(1, 30), (2, 30)], RowSize::Wasted),
            group("other", &[(3, 5), (4, 5), (5, 5)], RowSize::Wasted),
        ];
        let mut l = ResultList::new("groups".into(), 0, groups);
        let members = vec![one("1", 3, 5), one("2", 4, 5), one("3", 5, 5)];
        l.drill_into(ResultList::new("members".into(), 0, members));
        assert_eq!(l.title, "members");

        l.remove_nodes(&HashSet::from([4, 1]));
        assert_eq!(l.rows.len(), 2);
        assert!(l.back());
        assert!(!l.back());
        // "dup" lost a copy and has nothing left to compare; "other" shrank.
        assert_eq!(l.rows.len(), 1);
        assert_eq!(l.rows[0].label, "other");
        assert_eq!(l.rows[0].size(), 5);
    }
}
