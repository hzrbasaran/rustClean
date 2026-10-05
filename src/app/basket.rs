//! Adding entries to the basket and showing it.

use crate::basket::Added;
use crate::lists::{ResultList, Row, Source};
use crate::tree::{NodeId, ROOT};

use super::results::keeper;
use super::Browser;

impl Browser {
    /// The entries a result row stands for: its members, except the copy to
    /// keep in a group of duplicates or similar images.
    pub(super) fn row_targets(&self, list: &ResultList, row: &Row) -> Vec<NodeId> {
        let keep = if list.keep_one && row.group {
            keeper(self, &list.source, &row.nodes).map(|i| row.nodes[i])
        } else {
            None
        };
        row.nodes
            .iter()
            .copied()
            .filter(|&id| Some(id) != keep)
            .collect()
    }

    /// Whether a result row is fully in the basket (shown as `[✓]`).
    pub fn row_in_basket(&self, list: &ResultList, row: &Row) -> bool {
        let targets = self.row_targets(list, row);
        !targets.is_empty() && targets.iter().all(|&id| self.basket.covers(&self.tree, id))
    }

    /// Adds the entries to the basket, or removes them when all are in it.
    pub(super) fn toggle_basket(&mut self, ids: Vec<NodeId>) {
        if ids.is_empty() {
            return;
        }
        if ids.iter().all(|&id| self.basket.covers(&self.tree, id)) {
            let mut by_parent = 0;
            for id in ids {
                if self.basket.items().contains(&id) {
                    self.basket.remove(id);
                } else {
                    by_parent += 1;
                }
            }
            if by_parent > 0 {
                self.set_status(
                    t!(
                        "Üst klasörü sepette olduğu için ayrıca çıkarılamaz; önce üst klasörü çıkarın.",
                        "Its parent folder is in the basket, so it cannot be removed on its own; remove the parent first.",
                    ),
                    true,
                );
            }
        } else {
            let covered = ids
                .into_iter()
                .filter(|&id| self.basket.add(&self.tree, id) == Added::CoveredByParent)
                .count();
            if covered > 0 {
                self.set_status(
                    if covered == 1 {
                        t!(
                            "1 öğenin üst klasörü zaten sepette.",
                            "The parent folder of 1 item is already in the basket.",
                        )
                        .to_string()
                    } else {
                        tf!(
                            "{covered} öğenin üst klasörü zaten sepette.",
                            "The parent folders of {covered} items are already in the basket.",
                        )
                    },
                    false,
                );
            }
        }
        self.refresh_basket_view();
    }

    /// Space in a result list.
    pub(super) fn toggle_row(&mut self) {
        let Some(r) = &self.results else {
            return;
        };
        let Some(row) = r.selected_row() else {
            return;
        };
        let ids = self.row_targets(r, row);
        if r.is_basket() {
            for id in ids {
                self.basket.remove(id);
            }
            self.refresh_basket_view();
        } else {
            self.toggle_basket(ids);
        }
        if let Some(r) = &mut self.results {
            r.table.select_next();
        }
    }

    /// `t` in a result list: every row (in a list of copies, all but the one
    /// to keep).
    pub(super) fn toggle_all_rows(&mut self) {
        let Some(r) = &self.results else {
            return;
        };
        let ids: Vec<NodeId> = r
            .rows
            .iter()
            .flat_map(|row| self.row_targets(r, row))
            .filter(|&id| Some(id) != r.keep)
            .collect();
        self.toggle_basket(ids);
    }

    /// Shows the basket as a result list.
    pub(super) fn open_basket(&mut self) {
        if self.basket.is_empty() {
            self.set_status(
                t!(
                    "Sepet boş. Space ile öğe ekleyin.",
                    "The basket is empty. Add entries with Space."
                ),
                false,
            );
            return;
        }
        self.dashboard = None;
        self.results = Some(self.basket_list());
    }

    pub(super) fn basket_list(&self) -> ResultList {
        let (tree, mode) = (&self.tree, self.size_mode);
        let mut ids = self.basket.items().to_vec();
        ids.sort_by_key(|&id| std::cmp::Reverse(tree.node(id).size.get(mode)));
        let rows = ids
            .into_iter()
            .map(|id| Row::single(tree, ROOT, id, mode, String::new()))
            .collect();
        let mut list = ResultList::new(t!("Sepet", "Basket").into(), ROOT, rows);
        list.source = Source::Basket;
        list.note = t!(
            "Space: sepetten çıkar · c: sepeti boşalt · x: hepsini çöpe taşı",
            "Space: remove from basket · c: empty the basket · x: move all to the trash",
        )
        .into();
        list
    }

    /// Rebuilds the basket view after the basket changed, keeping the cursor.
    pub(super) fn refresh_basket_view(&mut self) {
        if self.results.as_ref().is_some_and(ResultList::is_basket) {
            self.rebuild_results();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::test_util::{browser_with, group};

    #[test]
    fn space_toggles_and_t_selects_all() {
        let rows = vec![group(&[0, 1]), group(&[2])];
        let (mut br, [a, b, c]) = browser_with(rows, false);
        br.toggle_all_rows();
        assert_eq!(br.basket.items(), &[a, b, c]);
        br.toggle_all_rows();
        assert!(br.basket.is_empty());
        br.toggle_row(); // first row
        assert_eq!(br.basket.items(), &[a, b]);
        // The basket view lists the basket; Space there removes.
        br.open_basket();
        assert!(br.results.as_ref().unwrap().is_basket());
        assert_eq!(br.results.as_ref().unwrap().rows.len(), 2);
        br.toggle_row();
        assert_eq!(br.basket.len(), 1);
        assert_eq!(br.results.as_ref().unwrap().rows.len(), 1);
    }
}
