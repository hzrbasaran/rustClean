//! Moving around the tree, the treemap, and the browser's key dispatcher.

use crossterm::event::KeyCode;
use ratatui::layout::Rect;

use crate::reports::MenuItem;
use crate::system;
use crate::toolsview::ToolsKey;
use crate::tree::{NodeId, SizeMode, ROOT};
use crate::treemap::{self, Slot};

use super::{Action, Browser, LogView, MapColor, SortMode, Status, View};

impl Browser {
    pub(super) fn load(&mut self, dir: NodeId, selected: usize) {
        self.current = dir;
        let tree = &self.tree;
        let mode = self.size_mode;
        let mut entries: Vec<NodeId> = tree.children(dir).collect();
        match self.sort {
            SortMode::Size => {
                entries.sort_by_key(|&a| std::cmp::Reverse(tree.node(a).size.get(mode)));
            }
            SortMode::Name => entries.sort_by_cached_key(|&a| tree.name(a).to_lowercase()),
            SortMode::Count => entries.sort_by_key(|&a| std::cmp::Reverse(tree.node(a).file_count)),
            // Oldest first; unknown dates (0) go last.
            SortMode::Modified => entries.sort_by_key(|&a| {
                let m = tree.node(a).modified;
                (m == 0, m)
            }),
        }
        self.entries = entries;
        self.table
            .select((!self.entries.is_empty()).then(|| selected.min(self.entries.len() - 1)));
    }

    pub(super) fn selected(&self) -> Option<NodeId> {
        self.table
            .selected()
            .and_then(|i| self.entries.get(i).copied())
    }

    fn enter(&mut self) {
        if let Some(id) = self.selected().filter(|&id| self.tree.node(id).is_dir) {
            self.history
                .push((self.current, self.table.selected().unwrap_or(0)));
            self.load(id, 0);
        }
    }

    fn back(&mut self) {
        if let Some((dir, row)) = self.history.pop() {
            self.load(dir, row);
        }
    }

    /// Opens the directory containing `id` with `id` selected.
    pub(super) fn reveal(&mut self, id: NodeId) {
        let dir = self.tree.parent(id).unwrap_or(ROOT);
        let mut ancestors = Vec::new();
        let mut cur = dir;
        while let Some(p) = self.tree.parent(cur) {
            ancestors.push((p, 0));
            cur = p;
        }
        ancestors.reverse();
        self.history = ancestors;
        self.load(dir, 0);
        if let Some(pos) = self.entries.iter().position(|&e| e == id) {
            self.table.select(Some(pos));
        }
    }

    fn cycle_sort(&mut self) {
        self.sort = self.sort.next();
        self.load(self.current, 0);
    }

    /// Opens `dir` itself, with the history leading back to the root.
    pub(super) fn open_dir(&mut self, dir: NodeId) {
        self.reveal(dir);
        self.history
            .push((self.current, self.table.selected().unwrap_or(0)));
        self.load(dir, 0);
    }

    pub(super) fn toggle_size_mode(&mut self) {
        self.size_mode = match self.size_mode {
            SizeMode::Apparent => SizeMode::Disk,
            SizeMode::Disk => SizeMode::Apparent,
        };
        self.refresh_dashboard();
        if let Some(r) = &mut self.results {
            let (tree, mode) = (&self.tree, self.size_mode);
            r.resize(&|id| tree.node(id).size.get(mode));
        }
        // Re-sort, keeping the cursor on the same entry.
        let selected = self.selected();
        self.load(self.current, 0);
        if let Some(pos) = selected.and_then(|id| self.entries.iter().position(|&e| e == id)) {
            self.table.select(Some(pos));
        }
    }

    pub(super) fn set_status(&mut self, text: impl Into<String>, error: bool) {
        self.status = Some(Status {
            text: text.into(),
            error,
        });
    }

    /// Treemap blocks of the current directory; items index `entries`.
    pub fn map_blocks(&self) -> Vec<(Slot, Rect)> {
        let sizes: Vec<u64> = self
            .entries
            .iter()
            .map(|&id| self.tree.node(id).size.get(self.size_mode))
            .collect();
        treemap::layout(&sizes, self.map_area)
    }

    /// The block holding the selected entry ("other" for small ones).
    pub fn selected_block(&self) -> Option<Slot> {
        let sel = self.table.selected()?;
        let blocks = self.map_blocks();
        blocks
            .iter()
            .map(|(s, _)| *s)
            .find(|s| *s == Slot::Item(sel))
            .or_else(|| {
                blocks
                    .iter()
                    .map(|(s, _)| *s)
                    .find(|s| matches!(s, Slot::Other { .. }))
            })
    }

    fn map_move(&mut self, dir: treemap::Dir) {
        let blocks = self.map_blocks();
        let Some(current) = self.selected_block() else {
            return;
        };
        let Some(from) = blocks.iter().position(|(s, _)| *s == current) else {
            return;
        };
        let rects: Vec<Rect> = blocks.iter().map(|(_, r)| *r).collect();
        match treemap::neighbor(&rects, from, dir).map(|i| blocks[i].0) {
            Some(Slot::Item(i)) => self.table.select(Some(i)),
            Some(Slot::Other { .. }) => self.select_first_small(),
            None => {}
        }
    }

    /// Selects the first entry too small for its own block ("other").
    pub(super) fn select_first_small(&mut self) {
        let shown = self
            .map_blocks()
            .iter()
            .filter(|(s, _)| matches!(s, Slot::Item(_)))
            .count();
        self.table
            .select(Some(shown.min(self.entries.len().saturating_sub(1))));
    }

    pub(super) fn on_key(&mut self, code: KeyCode) -> Action {
        if self.deleting.is_some() {
            return if code == KeyCode::Char('q') {
                Action::Quit
            } else {
                Action::None
            };
        }
        if let Some(dialog) = &mut self.failures {
            match code {
                KeyCode::Up | KeyCode::Char('k') => dialog.scroll = dialog.scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    dialog.scroll = dialog.scroll.saturating_add(1);
                }
                KeyCode::PageUp => dialog.scroll = dialog.scroll.saturating_sub(10),
                KeyCode::PageDown => dialog.scroll = dialog.scroll.saturating_add(10),
                _ => self.failures = None,
            }
            return Action::None;
        }
        if self.confirm.is_some() {
            match code {
                KeyCode::Char('e' | 'E' | 'y' | 'Y') => self.confirm_delete(),
                _ => self.confirm = None,
            }
            return Action::None;
        }
        if self.uninstall.is_some() {
            self.on_key_uninstall(code);
            return Action::None;
        }
        if self.pending_report.is_some() {
            return Action::None;
        }
        if self.rescan.is_some() {
            match code {
                KeyCode::Char('q') => return Action::Quit,
                KeyCode::Esc => {
                    self.rescan = None; // dropping the handle cancels the scan
                    self.set_status(
                        t!("Yeniden tarama iptal edildi.", "Rescan cancelled."),
                        true,
                    );
                }
                _ => {}
            }
            return Action::None;
        }
        if self.dup_job.is_some() {
            match code {
                KeyCode::Char('q') => return Action::Quit,
                KeyCode::Esc => {
                    self.dup_job = None; // dropping the job cancels it
                    self.set_status(
                        t!("Kopya araması iptal edildi.", "Duplicate search cancelled."),
                        true,
                    );
                }
                _ => {}
            }
            return Action::None;
        }
        if let Some(input) = &mut self.input {
            match code {
                KeyCode::Char(c) => input.push(c),
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Enter => {
                    let pattern = std::mem::take(input);
                    self.input = None;
                    self.run_search(&pattern);
                }
                KeyCode::Esc => self.input = None,
                _ => {}
            }
            return Action::None;
        }
        self.status = None;
        if self.export_prompt.is_some() {
            self.on_key_export(code);
            return Action::None;
        }
        if let Some(sel) = &mut self.report_menu {
            let n = MenuItem::ALL.len();
            match code {
                KeyCode::Up | KeyCode::Char('k') => *sel = (*sel + n - 1) % n,
                KeyCode::Down | KeyCode::Char('j') => *sel = (*sel + 1) % n,
                KeyCode::Enter => {
                    let item = MenuItem::ALL[*sel];
                    self.report_menu = None;
                    self.open_menu_item(item);
                }
                KeyCode::Char('q') => return Action::Quit,
                KeyCode::Esc | KeyCode::Char('m') => self.report_menu = None,
                _ => {}
            }
            return Action::None;
        }
        if let Some((saved, sel)) = &mut self.snapshot_picker {
            let n = saved.len();
            match code {
                KeyCode::Up | KeyCode::Char('k') => *sel = (*sel + n - 1) % n,
                KeyCode::Down | KeyCode::Char('j') => *sel = (*sel + 1) % n,
                KeyCode::Enter => {
                    let file = saved[*sel].file.clone();
                    self.snapshot_picker = None;
                    self.compare_with(&file);
                }
                KeyCode::Char('q') => return Action::Quit,
                KeyCode::Esc => self.snapshot_picker = None,
                _ => {}
            }
            return Action::None;
        }
        if let Some(view) = &mut self.tools {
            match view.on_key(code) {
                ToolsKey::None => {}
                ToolsKey::Quit => return Action::Quit,
                ToolsKey::Close => self.tools = None,
                ToolsKey::Menu => self.report_menu = Some(0),
            }
            return Action::None;
        }
        if let Some(log) = &mut self.deletion_log {
            match code {
                KeyCode::Char('q') => return Action::Quit,
                KeyCode::Up | KeyCode::Char('k') => log.scroll = log.scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => log.scroll = log.scroll.saturating_add(1),
                KeyCode::PageUp => log.scroll = log.scroll.saturating_sub(20),
                KeyCode::PageDown => log.scroll = log.scroll.saturating_add(20),
                KeyCode::Home | KeyCode::Char('g') => log.scroll = 0,
                KeyCode::End | KeyCode::Char('G') => log.scroll = u16::MAX,
                KeyCode::Char('r') => *log = LogView::open(),
                KeyCode::Char('m') => self.report_menu = Some(0),
                KeyCode::Esc | KeyCode::Backspace | KeyCode::Left => self.deletion_log = None,
                _ => {}
            }
            return Action::None;
        }
        if self.system.is_some() {
            match code {
                KeyCode::Char('q') => return Action::Quit,
                KeyCode::Char('r') => self.system = Some(system::collect()),
                KeyCode::Char('m') => self.report_menu = Some(0),
                KeyCode::Esc | KeyCode::Backspace | KeyCode::Left => self.system = None,
                _ => {}
            }
            return Action::None;
        }
        if self.dashboard.is_some() {
            return self.on_key_dashboard(code);
        }
        if self.results.is_some() {
            return self.on_key_results(code);
        }
        if self.view == View::Map {
            let dir = match code {
                KeyCode::Left | KeyCode::Char('h') => Some(treemap::Dir::Left),
                KeyCode::Right | KeyCode::Char('l') => Some(treemap::Dir::Right),
                KeyCode::Up | KeyCode::Char('k') => Some(treemap::Dir::Up),
                KeyCode::Down | KeyCode::Char('j') => Some(treemap::Dir::Down),
                _ => None,
            };
            if let Some(dir) = dir {
                self.map_move(dir);
                return Action::None;
            }
            match code {
                // Esc leaves the map (Backspace still goes up a folder).
                KeyCode::Esc => {
                    self.view = View::List;
                    return Action::None;
                }
                KeyCode::Char('c') => {
                    self.map_color = match self.map_color {
                        MapColor::Kind => MapColor::Age,
                        MapColor::Age => MapColor::Kind,
                    };
                    return Action::None;
                }
                KeyCode::Enter
                    if self
                        .selected_block()
                        .is_some_and(|s| matches!(s, Slot::Other { .. })) =>
                {
                    // Small entries are only reachable from the list.
                    self.view = View::List;
                    self.select_first_small();
                    return Action::None;
                }
                _ => {}
            }
        }
        match code {
            KeyCode::Char('q') => return Action::Quit,
            KeyCode::Char('t') => {
                self.view = match self.view {
                    View::List => View::Map,
                    View::Map => View::List,
                };
            }
            KeyCode::Up | KeyCode::Char('k') => self.table.select_previous(),
            KeyCode::Down | KeyCode::Char('j') => self.table.select_next(),
            KeyCode::PageUp => self.table.scroll_up_by(20),
            KeyCode::PageDown => self.table.scroll_down_by(20),
            KeyCode::Home | KeyCode::Char('g') => self.table.select_first(),
            KeyCode::End | KeyCode::Char('G') => self.table.select_last(),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => self.enter(),
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') | KeyCode::Esc => self.back(),
            KeyCode::Char('s') => self.cycle_sort(),
            KeyCode::Char('a') => self.toggle_size_mode(),
            KeyCode::Char('/') => self.input = Some(String::new()),
            KeyCode::Char('i') => self.open_dashboard(),
            KeyCode::Char('m') => self.report_menu = Some(0),
            KeyCode::Char('x') | KeyCode::Delete => self.delete_key(),
            KeyCode::Char(' ') => {
                if let Some(id) = self.selected() {
                    self.toggle_basket(vec![id]);
                    self.table.select_next();
                }
            }
            KeyCode::Char('S') => self.open_basket(),
            KeyCode::Char('o') => self.start_export(),
            KeyCode::Char('r') => return Action::Rescan,
            KeyCode::Char('R') => self.start_rescan(),
            KeyCode::Char('d') => return Action::Disks,
            KeyCode::Char('w') => self.export_html(),
            _ => {}
        }
        Action::None
    }
}
