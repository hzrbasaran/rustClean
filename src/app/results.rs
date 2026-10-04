//! Search, reports, the duplicate search and result lists.

use std::collections::HashMap;

use crossterm::event::KeyCode;

use crate::duplicates::{self, DupJob};
use crate::lists::{ResultList, Row, RowSize};
use crate::reports::{self, MenuItem, ReportKind};
use crate::system;
use crate::toolsview::ToolsView;
use crate::tree::{NodeId, SizeMode, Tree};
use crate::{apps, search};

use super::{Action, Browser};

/// Index of the entry that has existed longest (by creation date, else
/// modification date): the copy kept when removing duplicates.
pub(super) fn oldest(tree: &Tree, ids: &[NodeId]) -> Option<usize> {
    let age = |id: NodeId| {
        let n = tree.node(id);
        if n.created != 0 {
            n.created
        } else {
            n.modified
        }
    };
    (0..ids.len()).min_by_key(|&i| (age(ids[i]), i))
}

/// Sizes of a duplicate group's members, and how many are APFS clones of
/// another member. Of each set of clones only one keeps its full size; the
/// others count their private bytes, which is all deleting them would free.
/// (In disk mode the scan already gave the full size to whichever clone it
/// met first, which need not be the group's first member.)
fn member_sizes(tree: &Tree, ids: &[NodeId], mode: SizeMode) -> (Vec<(NodeId, u64)>, usize) {
    let mut members: Vec<(NodeId, u64)> = ids
        .iter()
        .map(|&id| (id, tree.node(id).size.get(mode)))
        .collect();
    let mut first_of: HashMap<u64, usize> = HashMap::new();
    let mut clones = 0;
    for i in 0..members.len() {
        let Some((clone_id, private)) = tree.clone_of(members[i].0) else {
            continue;
        };
        match first_of.get(&clone_id) {
            None => {
                first_of.insert(clone_id, i);
            }
            Some(&f) => {
                clones += 1;
                members[f].1 = members[f].1.max(members[i].1);
                members[i].1 = members[i].1.min(private);
            }
        }
    }
    (members, clones)
}

impl Browser {
    pub(super) fn run_search(&mut self, pattern: String) {
        let pat = search::Pattern::new(&pattern);
        if pat.is_empty() {
            return;
        }
        let (tree, base, mode) = (&self.tree, self.current, self.size_mode);
        let mut items = search::find(tree, base, &pat);
        items.sort_by_key(|&id| std::cmp::Reverse(tree.node(id).size.get(mode)));
        let rows = items
            .into_iter()
            .map(|id| Row::single(tree, base, id, mode, String::new()))
            .collect();
        let mut list = ResultList::new(tf!("Arama „{pattern}”", "Search “{pattern}”"), base, rows);
        list.note = t!(
            "Eşleşen klasörlerin içi ayrıca listelenmez; klasörle birlikte taşınır.",
            "Contents of matching folders are not listed separately; they go with the folder.",
        )
        .into();
        list.pattern = Some(pattern);
        self.results = Some(list);
    }

    pub(super) fn open_menu_item(&mut self, item: MenuItem) {
        match item {
            MenuItem::Report(kind) => self.request_report(kind),
            MenuItem::Leftovers => self.request_report(ReportKind::Orphans),
            MenuItem::Changes => self.open_snapshot_picker(),
            MenuItem::System => {
                self.dashboard = None;
                self.tools = None;
                self.system = Some(system::collect());
            }
            MenuItem::Tools => {
                self.dashboard = None;
                self.system = None;
                self.tools = Some(ToolsView::open());
            }
        }
    }

    fn request_report(&mut self, kind: ReportKind) {
        self.report_menu = None;
        self.dashboard = None;
        self.set_status(
            tf!(
                "Rapor hazırlanıyor: {}…",
                "Preparing report: {}…",
                kind.label()
            ),
            false,
        );
        // Skip one frame so the message is drawn before the work starts.
        self.pending_report = Some((kind, 1));
    }

    pub(super) fn poll_report(&mut self) {
        let Some((kind, frames)) = &mut self.pending_report else {
            return;
        };
        if *frames > 0 {
            *frames -= 1;
            return;
        }
        let kind = *kind;
        self.pending_report = None;
        self.status = None;
        match kind {
            ReportKind::Apps => {
                self.results = Some(apps::run(&self.tree, self.size_mode));
            }
            ReportKind::Orphans => {
                let now = crate::ui::now_secs();
                self.results = Some(apps::orphans(&self.tree, self.size_mode, now, 0));
            }
            ReportKind::Duplicates => {
                let cands = duplicates::candidates(&self.tree, self.current);
                if cands.is_empty() {
                    let mut list =
                        ResultList::new(kind.label().to_string(), self.current, Vec::new());
                    list.note = t!(
                        "1 MiB üzerinde aynı boyutta iki dosya yok.",
                        "No two files of 1 MiB or more have the same size.",
                    )
                    .into();
                    self.results = Some(list);
                } else {
                    self.dup_job = Some(DupJob::start(self.current, cands));
                }
            }
            _ => {
                let now = crate::ui::now_secs();
                let list = reports::run(&self.tree, self.current, self.size_mode, now, kind, 0);
                self.results = Some(list);
            }
        }
    }

    pub(super) fn poll_dups(&mut self) {
        let Some(job) = &self.dup_job else {
            return;
        };
        let Some(groups) = job.poll() else {
            return;
        };
        let base = job.base;
        self.dup_job = None;
        let (tree, mode) = (&self.tree, self.size_mode);
        let mut rows: Vec<Row> = groups
            .into_iter()
            .map(|ids| {
                let first = ids[0];
                let (members, clones) = member_sizes(tree, &ids, mode);
                let each = members.iter().map(|m| m.1).max().unwrap_or(0);
                let mut detail = tf!(
                    "{} kopya · her biri {}",
                    "{} copies · {} each",
                    ids.len(),
                    crate::ui::fmt_size(each)
                );
                if clones > 0 {
                    detail += &tf!(" · {} APFS klonu", " · {} APFS clones", clones);
                }
                Row::group(
                    tree.name(first).to_string(),
                    detail,
                    members,
                    RowSize::Wasted,
                    2,
                )
            })
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.size()));
        let truncated = rows.len() > reports::LIMIT;
        rows.truncate(reports::LIMIT);
        let mut list = ResultList::new(ReportKind::Duplicates.label().to_string(), base, rows);
        list.truncated = truncated;
        list.keep_one = true;
        list.note = t!(
            "Boyut: kopyalar silinince açılacak yer. Space: en eski kopya hariç sepete ekle · \
             Enter: kopyaları gör. APFS klonları blok paylaşır: disk boyutunda bir kez sayılır, \
             silmek yer açmaz.",
            "Size: space freed by deleting the copies. Space: add all but the oldest copy to the \
             basket · Enter: see the copies. APFS clones share blocks: on disk they count once, and \
             deleting them frees nothing.",
        )
        .into();
        self.results = Some(list);
    }

    /// `f` in a report: re-runs it with the next minimum age.
    fn cycle_age_filter(&mut self) {
        let Some(r) = &self.results else {
            return;
        };
        let Some(kind) = r.report.filter(|k| k.supports_age()) else {
            self.set_status(
                t!(
                    "Bu listede yaş filtresi yok.",
                    "This list has no age filter."
                ),
                true,
            );
            return;
        };
        let steps = reports::AGE_STEPS;
        let next = steps
            .iter()
            .position(|&d| d == r.min_age_days)
            .map_or(0, |i| (i + 1) % steps.len());
        let base = r.base;
        let now = crate::ui::now_secs();
        let list = if kind == ReportKind::Orphans {
            apps::orphans(&self.tree, self.size_mode, now, steps[next])
        } else {
            reports::run(&self.tree, base, self.size_mode, now, kind, steps[next])
        };
        self.results = Some(list);
    }

    /// Opens the members of the selected group row.
    pub(super) fn drill_down(&mut self) {
        let Some(r) = &self.results else {
            return;
        };
        let Some(row) = r.selected_row() else {
            return;
        };
        let (tree, mode) = (&self.tree, self.size_mode);
        let mut ids = row.nodes.clone();
        ids.sort_by_key(|&id| std::cmp::Reverse(tree.node(id).size.get(mode)));
        let rows: Vec<Row> = ids
            .iter()
            .map(|&id| Row::single(tree, r.base, id, mode, String::new()))
            .collect();
        let mut list = ResultList::new(format!("{} › {}", r.title, row.label), r.base, rows);
        if r.keep_one {
            if let Some(keep) = oldest(tree, &ids) {
                list.keep = Some(ids[keep]);
                list.note = tf!(
                    "En eski kopya: {} · t: diğerlerini sepete ekle",
                    "Oldest copy: {} · t: add the others to the basket",
                    list.rows[keep].label
                );
            }
        }
        if let Some(r) = &mut self.results {
            r.drill_into(list);
        }
    }

    pub(super) fn on_key_results(&mut self, code: KeyCode) -> Action {
        let Some(r) = &mut self.results else {
            return Action::None;
        };
        match code {
            KeyCode::Char('q') => return Action::Quit,
            KeyCode::Up | KeyCode::Char('k') => r.table.select_previous(),
            KeyCode::Down | KeyCode::Char('j') => r.table.select_next(),
            KeyCode::PageUp => r.table.scroll_up_by(20),
            KeyCode::PageDown => r.table.scroll_down_by(20),
            KeyCode::Home | KeyCode::Char('g') => r.table.select_first(),
            KeyCode::End | KeyCode::Char('G') => r.table.select_last(),
            KeyCode::Char(' ') => self.toggle_row(),
            KeyCode::Char('t') => self.toggle_all_rows(),
            KeyCode::Char('x') | KeyCode::Delete => self.delete_key(),
            KeyCode::Char('S') => self.open_basket(),
            KeyCode::Char('f') => self.cycle_age_filter(),
            KeyCode::Char('u') => self.start_uninstall(),
            KeyCode::Char('c') if r.basket_view => {
                self.basket.clear();
                self.refresh_basket_view();
            }
            KeyCode::Char('/') => self.input = Some(r.pattern.clone().unwrap_or_default()),
            KeyCode::Char('m') => self.report_menu = Some(0),
            KeyCode::Char('a') => self.toggle_size_mode(),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => match r.selected_row() {
                Some(row) if row.group => self.drill_down(),
                Some(row) => {
                    let id = row.nodes[0];
                    self.results = None;
                    self.reveal(id);
                }
                None => {}
            },
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                if r.back() {
                    return Action::None;
                }
                self.results = None;
                let row = self.table.selected().unwrap_or(0);
                self.load(self.current, row);
            }
            _ => {}
        }
        Action::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_util::{browser_with, group};
    use crate::tree::{Size, ROOT};
    use std::path::Path;

    #[test]
    fn clones_count_once_in_duplicate_groups() {
        let mut t = Tree::new(Path::new("/r"));
        let ids: Vec<NodeId> = [1, 100, 1, 100, 100]
            .iter()
            .enumerate()
            .map(|(i, &disk)| {
                t.push(
                    ROOT,
                    &format!("f{i}"),
                    false,
                    Size {
                        apparent: 100,
                        disk,
                    },
                )
            })
            .collect();
        // f0..f2 are clones; the scan gave the full disk size to f1 and only
        // the private bytes to the others.
        for &id in &ids[..3] {
            t.set_clone(id, 7, 1);
        }
        t.set_clone(ids[3], 9, 2); // a clone of a file outside the group
        for mode in [SizeMode::Disk, SizeMode::Apparent] {
            let (members, clones) = member_sizes(&t, &ids, mode);
            assert_eq!(clones, 2);
            let sizes: Vec<u64> = members.iter().map(|m| m.1).collect();
            assert_eq!(sizes, [100, 1, 1, 100, 100], "{mode:?}");
        }
    }

    #[test]
    fn duplicate_groups_keep_the_oldest_copy() {
        // Under the cursor…
        let (mut br, [a, b, c]) = browser_with(vec![group(&[0, 1, 2])], true);
        assert_eq!(br.delete_targets(), Ok(vec![a, c]));
        // …and through the basket: Space adds all but the oldest copy.
        br.toggle_row();
        assert_eq!(br.basket.items(), &[a, c]);
        assert!(!br.basket.covers(&br.tree, b));
        let list = br.results.as_ref().unwrap();
        assert!(br.row_in_basket(list, &list.rows[0]));
        assert_eq!(br.delete_targets(), Ok(vec![a, c]));
    }
}
