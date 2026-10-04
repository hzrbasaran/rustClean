//! Moving entries to the trash: targets, safety checks, confirmation, and
//! the background deletion.

use std::collections::HashSet;

use crate::delete::{self, Deletion};
use crate::disks;
use crate::tree::{NodeId, Size};

use super::Browser;

/// Entries a deletion could not move to the trash, with the full errors.
pub struct FailureDialog {
    pub items: Vec<delete::Failure>,
    /// How many entries were moved, and their size, for the summary line.
    pub moved: usize,
    pub size: String,
    /// First visible line.
    pub scroll: u16,
}

impl Browser {
    /// What `x` deletes: the basket when it has anything, otherwise the
    /// entry under the cursor.
    pub(super) fn delete_targets(&self) -> Result<Vec<NodeId>, &'static str> {
        if !self.basket.is_empty() {
            return Ok(self.basket.items().to_vec());
        }
        if let Some(r) = &self.results {
            return match r.selected_row() {
                Some(row) if row.group && !r.keep_one => Err(t!(
                    "Bu bir grup: içine girmek için Enter, sepete eklemek için Space.",
                    "This is a group: Enter to open it, Space to add it to the basket.",
                )),
                Some(row) => Ok(self.row_targets(r, row)),
                None => Ok(Vec::new()),
            };
        }
        if let Some(d) = &self.dashboard {
            return Ok(d
                .selected()
                .filter(|&id| id != d.base)
                .into_iter()
                .collect());
        }
        Ok(self.selected().into_iter().collect())
    }

    pub(super) fn delete_key(&mut self) {
        match self.delete_targets() {
            Ok(ids) => self.request_delete(ids),
            Err(msg) => self.set_status(msg, true),
        }
    }

    fn request_delete(&mut self, ids: Vec<NodeId>) {
        let ok = self.deletable(ids);
        if !ok.is_empty() {
            self.confirm = Some(ok);
        }
    }

    /// The entries that may be moved to the trash; the status line names
    /// the others.
    pub(super) fn deletable(&mut self, ids: Vec<NodeId>) -> Vec<NodeId> {
        if ids.is_empty() {
            self.set_status(t!("Seçili öğe yok.", "Nothing selected."), true);
            return ids;
        }
        let mounts = disks::all_mount_points();
        let mut ok = Vec::new();
        let mut refused = Vec::new();
        for id in ids {
            match delete::check(&self.tree.path_of(id), &mounts) {
                Ok(()) => ok.push(id),
                Err(msg) => refused.push(format!("{}: {msg}", self.tree.name(id))),
            }
        }
        if !refused.is_empty() {
            let more = if refused.len() > 1 {
                tf!(" (+{} öğe daha)", " (+{} more)", refused.len() - 1)
            } else {
                String::new()
            };
            self.set_status(
                tf!("Atlandı — {}{more}", "Skipped — {}{more}", refused[0]),
                true,
            );
        }
        ok
    }

    pub(super) fn confirm_delete(&mut self) {
        if let Some(ids) = self.confirm.take() {
            self.start_deletion(ids);
        }
    }

    pub(super) fn start_deletion(&mut self, ids: Vec<NodeId>) {
        let items: Vec<_> = ids.iter().map(|&id| (id, self.tree.path_of(id))).collect();
        self.batch_trashed = Size::default();
        self.batch_failures.clear();
        self.set_status(
            tf!(
                "Çöp kutusuna taşınıyor… 0/{}",
                "Moving to the trash… 0/{}",
                items.len()
            ),
            false,
        );
        self.deleting = Some(Deletion::start(items));
    }

    /// Applies results of a running trash operation.
    pub(super) fn poll_delete(&mut self) {
        let Some(deletion) = &mut self.deleting else {
            return;
        };
        let outcomes = deletion.poll();
        let (done, total, finished) = (deletion.done, deletion.total, deletion.finished());
        for (id, res) in outcomes {
            match res {
                Ok(()) => {
                    let size = self.tree.node(id).size;
                    self.tree.remove(id);
                    self.basket.remove(id);
                    self.trashed += size;
                    self.batch_trashed += size;
                    if let Some(r) = &mut self.results {
                        r.remove_nodes(&HashSet::from([id]));
                    }
                }
                Err(err) => self.batch_failures.push(delete::Failure::new(
                    self.tree.path_of(id).display().to_string(),
                    &err,
                )),
            }
        }
        if !finished {
            self.set_status(
                tf!(
                    "Çöp kutusuna taşınıyor… {done}/{total}",
                    "Moving to the trash… {done}/{total}"
                ),
                false,
            );
            return;
        }

        self.deleting = None;
        let row = self.table.selected().unwrap_or(0);
        self.load(self.current, row);
        self.refresh_dashboard();
        self.refresh_basket_view();
        self.refresh_apps_report();
        let moved = total - self.batch_failures.len();
        let size = crate::ui::fmt_size(self.batch_trashed.get(self.size_mode));
        if self.batch_failures.is_empty() {
            self.set_status(
                tf!(
                    "✓ {moved} öğe çöp kutusuna taşındı ({size}). Yer, çöp kutusu boşaltılınca açılır.",
                    "✓ {moved} items moved to the trash ({size}). The space is freed when the trash is emptied.",
                ),
                false,
            );
        } else {
            self.set_status(
                tf!(
                    "✗ {} öğe taşınamadı, {moved} öğe taşındı ({size}).",
                    "✗ {} items could not be moved, {moved} moved ({size}).",
                    self.batch_failures.len()
                ),
                true,
            );
            self.failures = Some(FailureDialog {
                items: std::mem::take(&mut self.batch_failures),
                moved,
                size,
                scroll: 0,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::test_util::{browser_with, group};
    use crate::lists::{Row, RowSize};

    #[test]
    fn x_without_basket_uses_the_cursor_row() {
        let single = Row::group("s".into(), String::new(), vec![(2, 1)], RowSize::Sum, 1);
        let (mut br, [_, _, c]) = browser_with(vec![single], false);
        br.results.as_mut().unwrap().rows[0].group = false;
        assert_eq!(br.delete_targets(), Ok(vec![c]));
    }

    #[test]
    fn unchecked_name_group_is_not_deleted_whole() {
        let (br, _) = browser_with(vec![group(&[0, 1, 2])], false);
        assert!(br.delete_targets().is_err());
    }
}
