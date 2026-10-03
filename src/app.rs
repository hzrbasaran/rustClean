//! Application state and key handling.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::widgets::TableState;

use crate::apps;
use crate::basket::{Added, Basket};
use crate::delete::{self, Deletion};
use crate::disks::{self, DiskInfo};
use crate::duplicates::{self, DupJob};
use crate::history;
use crate::lists::{ResultList, Row, RowSize};
use crate::reports::{self, MenuItem, ReportKind};
use crate::scanner::{self, ScanHandle, ScanMsg, ScanProgress, ScanResult};
use crate::search;
use crate::stats::{self, Stats};
use crate::system::{self, SystemInfo};
use crate::toolsview::{ToolsKey, ToolsView};
use crate::tree::{NodeId, Size, SizeMode, Tree, ROOT};
use crate::treemap::{self, Slot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    Size,
    Name,
    Count,
    Modified,
}

impl SortMode {
    fn next(self) -> Self {
        match self {
            SortMode::Size => SortMode::Name,
            SortMode::Name => SortMode::Count,
            SortMode::Count => SortMode::Modified,
            SortMode::Modified => SortMode::Size,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortMode::Size => t!("boyut", "size"),
            SortMode::Name => t!("ad", "name"),
            SortMode::Count => t!("dosya sayısı", "file count"),
            SortMode::Modified => t!(
                "son değişiklik (en eski önce)",
                "last modified (oldest first)"
            ),
        }
    }
}

pub enum Screen {
    DiskSelect,
    Scanning,
    Browser,
}

/// One-line feedback shown under the file list.
pub struct Status {
    pub text: String,
    pub error: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Files,
    Dirs,
}

/// Summary of a directory, shown in place of the listing.
pub struct Dashboard {
    pub base: NodeId,
    pub stats: Stats,
    /// The disk the directory lives on.
    pub disk: Option<DiskInfo>,
    pub focus: Pane,
    pub files: TableState,
    pub dirs: TableState,
}

impl Dashboard {
    fn selected(&self) -> Option<NodeId> {
        match self.focus {
            Pane::Files => self
                .files
                .selected()
                .and_then(|i| self.stats.top_files.get(i).copied()),
            Pane::Dirs => self
                .dirs
                .selected()
                .and_then(|i| self.stats.top_dirs.get(i).map(|&(id, _)| id)),
        }
    }

    fn table_mut(&mut self) -> &mut TableState {
        match self.focus {
            Pane::Files => &mut self.files,
            Pane::Dirs => &mut self.dirs,
        }
    }

    /// Keeps selections in range after the lists changed.
    fn clamp_selection(&mut self) {
        let lens = [self.stats.top_files.len(), self.stats.top_dirs.len()];
        for (table, len) in [&mut self.files, &mut self.dirs].into_iter().zip(lens) {
            let sel = table.selected().unwrap_or(0);
            table.select((len > 0).then(|| sel.min(len - 1)));
        }
    }
}

/// Index of the entry that has existed longest (by creation date, else
/// modification date): the copy kept when removing duplicates.
fn oldest(tree: &Tree, ids: &[NodeId]) -> Option<usize> {
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

/// How the current directory is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    List,
    Map,
}

/// What the treemap's colors mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapColor {
    Kind,
    Age,
}

/// Entries a deletion could not move to the trash, with the full errors.
pub struct FailureDialog {
    pub items: Vec<delete::Failure>,
    /// How many entries were moved, and their size, for the summary line.
    pub moved: usize,
    pub size: String,
    /// First visible line.
    pub scroll: u16,
}

/// What the browser asks the app to do after a key press.
pub enum Action {
    None,
    Quit,
    Rescan,
    Disks,
}

pub struct Browser {
    pub tree: Tree,
    pub errors: u64,
    pub elapsed: Duration,
    pub current: NodeId,
    /// Children of `current`, in display order.
    pub entries: Vec<NodeId>,
    pub table: TableState,
    pub sort: SortMode,
    pub size_mode: SizeMode,
    /// Search pattern being typed.
    pub input: Option<String>,
    pub results: Option<ResultList>,
    pub dashboard: Option<Dashboard>,
    /// Selected row of the open report menu.
    pub report_menu: Option<usize>,
    /// Saved scans to compare with, and the selected one.
    pub snapshot_picker: Option<(Vec<history::Saved>, usize)>,
    /// Open "system data" panel.
    pub system: Option<SystemInfo>,
    /// Open developer tools cleanup screen.
    pub tools: Option<ToolsView>,
    /// Report to run once the "preparing" message has been drawn.
    pending_report: Option<(ReportKind, u8)>,
    /// Running duplicate search.
    pub dup_job: Option<DupJob>,
    /// History time stamp of this scan, so it is not compared with itself.
    snapshot_time: Option<u64>,
    /// Entries awaiting a yes/no answer before being moved to the trash.
    pub confirm: Option<Vec<NodeId>>,
    pub deleting: Option<Deletion>,
    pub status: Option<Status>,
    /// Entries collected for deletion.
    pub basket: Basket,
    pub view: View,
    pub map_color: MapColor,
    /// Where the treemap was last drawn; key handling lays it out again.
    pub map_area: Rect,
    /// Total moved to the trash during this session.
    pub trashed: Size,
    /// Outcome of the running deletion batch.
    batch_trashed: Size,
    batch_failures: Vec<delete::Failure>,
    /// Entries that could not be trashed, shown in a dialog until dismissed.
    pub failures: Option<FailureDialog>,
    /// Directories we came from, with the row that was selected there.
    history: Vec<(NodeId, usize)>,
}

impl Browser {
    fn new(res: ScanResult) -> Self {
        let mut b = Self {
            tree: res.tree,
            errors: res.errors,
            elapsed: res.elapsed,
            current: ROOT,
            entries: Vec::new(),
            table: TableState::default(),
            sort: SortMode::Size,
            size_mode: SizeMode::Disk,
            input: None,
            results: None,
            dashboard: None,
            report_menu: None,
            snapshot_picker: None,
            system: None,
            tools: None,
            pending_report: None,
            dup_job: None,
            snapshot_time: None,
            confirm: None,
            deleting: None,
            status: None,
            basket: Basket::default(),
            view: View::List,
            map_color: MapColor::Kind,
            map_area: Rect::default(),
            trashed: Size::default(),
            batch_trashed: Size::default(),
            batch_failures: Vec::new(),
            failures: None,
            history: Vec::new(),
        };
        b.load(ROOT, 0);
        b
    }

    fn load(&mut self, dir: NodeId, selected: usize) {
        self.current = dir;
        let tree = &self.tree;
        let mode = self.size_mode;
        let mut entries: Vec<NodeId> = tree.children(dir).collect();
        match self.sort {
            SortMode::Size => {
                entries.sort_by_key(|&a| std::cmp::Reverse(tree.node(a).size.get(mode)))
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

    fn selected(&self) -> Option<NodeId> {
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
    fn reveal(&mut self, id: NodeId) {
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
    fn open_dir(&mut self, dir: NodeId) {
        self.reveal(dir);
        self.history
            .push((self.current, self.table.selected().unwrap_or(0)));
        self.load(dir, 0);
    }

    fn open_dashboard(&mut self) {
        let base = self.current;
        let path = self.tree.path_of(base);
        let disks = disks::list_disks();
        let mut d = Dashboard {
            base,
            stats: stats::compute(&self.tree, base, self.size_mode, crate::ui::now_secs()),
            disk: disks::disk_for(&path, &disks).cloned(),
            focus: Pane::Files,
            files: TableState::default(),
            dirs: TableState::default(),
        };
        d.clamp_selection();
        self.dashboard = Some(d);
    }

    /// Recomputes the dashboard after the tree or size mode changed.
    fn refresh_dashboard(&mut self) {
        if let Some(d) = &mut self.dashboard {
            d.stats = stats::compute(&self.tree, d.base, self.size_mode, crate::ui::now_secs());
            d.clamp_selection();
        }
    }

    fn toggle_size_mode(&mut self) {
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

    fn set_status(&mut self, text: impl Into<String>, error: bool) {
        self.status = Some(Status {
            text: text.into(),
            error,
        });
    }

    fn run_search(&mut self, pattern: String) {
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

    fn open_menu_item(&mut self, item: MenuItem) {
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

    fn open_snapshot_picker(&mut self) {
        let saved: Vec<history::Saved> = history::dir_for(self.tree.root_path())
            .map(|d| history::list(&d))
            .unwrap_or_default()
            .into_iter()
            .filter(|s| Some(s.header.time) != self.snapshot_time)
            .collect();
        if saved.is_empty() {
            self.set_status(
                t!(
                    "Bu klasörün kayıtlı eski bir taraması yok; bir sonraki taramadan sonra karşılaştırabilirsiniz.",
                    "No earlier scan of this folder is saved; you can compare after the next scan.",
                ),
                true,
            );
        } else {
            self.snapshot_picker = Some((saved, 0));
        }
    }

    fn compare_with(&mut self, file: &std::path::Path) {
        let snap = match history::load(file) {
            Ok(s) => s,
            Err(e) => {
                self.set_status(
                    tf!("Kayıt okunamadı: {e}", "Could not read the saved scan: {e}"),
                    true,
                );
                return;
            }
        };
        self.dashboard = None;
        let (rows, truncated, note) =
            history::changes(&self.tree, self.current, &snap, self.size_mode);
        let title = tf!(
            "Değişenler ({} taramasına göre)",
            "Changes (since the scan of {})",
            crate::ui::fmt_date(snap.header.time.min(u64::from(u32::MAX)) as u32)
        );
        let mut list = ResultList::new(title, self.current, rows);
        list.truncated = truncated;
        list.note = note;
        self.results = Some(list);
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

    fn poll_report(&mut self) {
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

    /// Saves this scan to the history (in the background) and tells how much
    /// changed since the previous one.
    fn record_history(&mut self) {
        let Some(dir) = history::dir_for(self.tree.root_path()) else {
            return;
        };
        let now = crate::ui::now_secs();
        if let Some(prev) = history::list(&dir).first() {
            let total = self.tree.node(ROOT).size.disk;
            self.set_status(
                tf!(
                    "Son taramadan beri ({}): {} · m → Değişenler",
                    "Since the last scan ({}): {} · m → Changes",
                    crate::ui::fmt_ago(now.saturating_sub(prev.header.time)),
                    crate::ui::fmt_delta(total, prev.header.total.disk)
                ),
                false,
            );
        }
        self.snapshot_time = Some(now);
        let snap = history::capture(&self.tree, now);
        std::thread::spawn(move || {
            let _ = history::save(&dir, &snap);
        });
    }

    /// Turns a finished duplicate search into a report.
    fn poll_dups(&mut self) {
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

    /// The entries a result row stands for: its members, except the oldest
    /// copy in a group of duplicates.
    fn row_targets(&self, list: &ResultList, row: &Row) -> Vec<NodeId> {
        let keep = if list.keep_one && row.group {
            oldest(&self.tree, &row.nodes).map(|i| row.nodes[i])
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
    fn toggle_basket(&mut self, ids: Vec<NodeId>) {
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
                    tf!(
                        "{covered} öğenin üst klasörü zaten sepette.",
                        "The parent folder of {covered} items is already in the basket.",
                    ),
                    false,
                );
            }
        }
        self.refresh_basket_view();
    }

    /// Space in a result list.
    fn toggle_row(&mut self) {
        let Some(r) = &self.results else {
            return;
        };
        let Some(row) = r.selected_row() else {
            return;
        };
        let ids = self.row_targets(r, row);
        let basket_view = r.basket_view;
        if basket_view {
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
    fn toggle_all_rows(&mut self) {
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

    /// What `x` deletes: the basket when it has anything, otherwise the
    /// entry under the cursor.
    fn delete_targets(&self) -> Result<Vec<NodeId>, &'static str> {
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

    fn delete_key(&mut self) {
        match self.delete_targets() {
            Ok(ids) => self.request_delete(ids),
            Err(msg) => self.set_status(msg, true),
        }
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
            Some(Slot::Other { .. }) => {
                // The first entry too small for its own block.
                let shown = blocks
                    .iter()
                    .filter(|(s, _)| matches!(s, Slot::Item(_)))
                    .count();
                self.table
                    .select(Some(shown.min(self.entries.len().saturating_sub(1))));
            }
            None => {}
        }
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

    /// Shows the basket as a result list.
    fn open_basket(&mut self) {
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

    fn basket_list(&self) -> ResultList {
        let (tree, mode) = (&self.tree, self.size_mode);
        let mut ids = self.basket.items().to_vec();
        ids.sort_by_key(|&id| std::cmp::Reverse(tree.node(id).size.get(mode)));
        let rows = ids
            .into_iter()
            .map(|id| Row::single(tree, ROOT, id, mode, String::new()))
            .collect();
        let mut list = ResultList::new(t!("Sepet", "Basket").into(), ROOT, rows);
        list.basket_view = true;
        list.note = t!(
            "Space: sepetten çıkar · c: sepeti boşalt · x: hepsini çöpe taşı",
            "Space: remove from basket · c: empty the basket · x: move all to the trash",
        )
        .into();
        list
    }

    /// Rebuilds the basket view after the basket changed, keeping the cursor.
    fn refresh_basket_view(&mut self) {
        if !self.results.as_ref().is_some_and(|r| r.basket_view) {
            return;
        }
        let row = self
            .results
            .as_ref()
            .and_then(|r| r.table.selected())
            .unwrap_or(0);
        let mut list = self.basket_list();
        if !list.rows.is_empty() {
            list.table.select(Some(row.min(list.rows.len() - 1)));
        }
        self.results = Some(list);
    }

    /// Opens the members of the selected group row.
    fn drill_down(&mut self) {
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

    fn request_delete(&mut self, ids: Vec<NodeId>) {
        if ids.is_empty() {
            self.set_status(t!("Seçili öğe yok.", "Nothing selected."), true);
            return;
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
        if !ok.is_empty() {
            self.confirm = Some(ok);
        }
    }

    fn confirm_delete(&mut self) {
        let Some(ids) = self.confirm.take() else {
            return;
        };
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
    fn poll_delete(&mut self) {
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

    fn on_key(&mut self, code: KeyCode) -> Action {
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
                    dialog.scroll = dialog.scroll.saturating_add(1)
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
        if self.pending_report.is_some() {
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
                    self.run_search(pattern);
                }
                KeyCode::Esc => self.input = None,
                _ => {}
            }
            return Action::None;
        }
        self.status = None;
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
                    let shown = self
                        .map_blocks()
                        .iter()
                        .filter(|(s, _)| matches!(s, Slot::Item(_)))
                        .count();
                    self.table
                        .select(Some(shown.min(self.entries.len().saturating_sub(1))));
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
            KeyCode::Char('r') => return Action::Rescan,
            KeyCode::Char('d') => return Action::Disks,
            _ => {}
        }
        Action::None
    }

    fn on_key_dashboard(&mut self, code: KeyCode) -> Action {
        let Some(d) = &mut self.dashboard else {
            return Action::None;
        };
        match code {
            KeyCode::Char('q') => return Action::Quit,
            KeyCode::Tab | KeyCode::BackTab => {
                d.focus = match d.focus {
                    Pane::Files => Pane::Dirs,
                    Pane::Dirs => Pane::Files,
                };
            }
            KeyCode::Up | KeyCode::Char('k') => d.table_mut().select_previous(),
            KeyCode::Down | KeyCode::Char('j') => d.table_mut().select_next(),
            KeyCode::Home | KeyCode::Char('g') => d.table_mut().select_first(),
            KeyCode::End | KeyCode::Char('G') => d.table_mut().select_last(),
            KeyCode::Char('a') => self.toggle_size_mode(),
            KeyCode::Char('x') | KeyCode::Delete => self.delete_key(),
            KeyCode::Char(' ') => {
                if let Some(id) = d.selected().filter(|&id| id != d.base) {
                    d.table_mut().select_next();
                    self.toggle_basket(vec![id]);
                }
            }
            KeyCode::Char('S') => self.open_basket(),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                let target = d.selected();
                let base = d.base;
                self.dashboard = None;
                match target {
                    Some(id) if id == base => {}
                    Some(id) if self.tree.node(id).is_dir => self.open_dir(id),
                    Some(id) => self.reveal(id),
                    None => {}
                }
            }
            KeyCode::Esc | KeyCode::Char('i') | KeyCode::Backspace | KeyCode::Left => {
                self.dashboard = None;
            }
            _ => {}
        }
        Action::None
    }

    fn on_key_results(&mut self, code: KeyCode) -> Action {
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

pub struct App {
    pub screen: Screen,
    pub disks: Vec<DiskInfo>,
    pub disk_table: TableState,
    pub scan: Option<ScanHandle>,
    pub scan_root: PathBuf,
    pub progress: ScanProgress,
    pub browser: Option<Browser>,
    pub message: Option<String>,
    pub tick: usize,
    pub should_quit: bool,
}

impl App {
    pub fn new(start_path: Option<PathBuf>) -> Self {
        let mut app = Self {
            screen: Screen::DiskSelect,
            disks: Vec::new(),
            disk_table: TableState::default(),
            scan: None,
            scan_root: PathBuf::new(),
            progress: ScanProgress::default(),
            browser: None,
            message: None,
            tick: 0,
            should_quit: false,
        };
        app.refresh_disks();
        if let Some(path) = start_path {
            app.start_scan(path);
        }
        app
    }

    fn refresh_disks(&mut self) {
        self.disks = disks::list_disks();
        self.disk_table
            .select((!self.disks.is_empty()).then_some(0));
    }

    fn start_scan(&mut self, root: PathBuf) {
        self.message = None;
        self.progress = ScanProgress::default();
        self.scan = Some(scanner::start(root.clone(), disks::all_mount_points()));
        self.scan_root = root;
        self.screen = Screen::Scanning;
    }

    /// Drains messages from a running scan.
    pub fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if let Some(b) = &mut self.browser {
            b.poll_delete();
            b.poll_report();
            b.poll_dups();
            if let Some(view) = &mut b.tools {
                view.poll();
            }
        }
        let Some(handle) = &self.scan else { return };
        loop {
            match handle.rx.try_recv() {
                Ok(ScanMsg::Progress(p)) => self.progress = p,
                Ok(ScanMsg::Done(res)) => {
                    self.scan = None;
                    let mut browser = Browser::new(res);
                    browser.record_history();
                    self.browser = Some(browser);
                    self.screen = Screen::Browser;
                    return;
                }
                Ok(ScanMsg::Failed(err)) => {
                    self.scan = None;
                    self.message = Some(tf!("Tarama başarısız: {err}", "Scan failed: {err}"));
                    self.screen = Screen::DiskSelect;
                    return;
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.scan = None;
                    self.screen = Screen::DiskSelect;
                    return;
                }
            }
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }
        if key.code == KeyCode::Char('L') && !self.typing() {
            self.switch_language();
            return;
        }
        match self.screen {
            Screen::DiskSelect => self.on_key_disks(key.code),
            Screen::Scanning => self.on_key_scanning(key.code),
            Screen::Browser => self.on_key_browser(key.code),
        }
    }

    /// Whether a text field has the keyboard, so letters are text.
    fn typing(&self) -> bool {
        self.browser.as_ref().is_some_and(|b| {
            b.input.is_some()
                || b.tools
                    .as_ref()
                    .and_then(|t| t.confirm.as_ref())
                    .is_some_and(|c| c.typed.is_some())
        })
    }

    /// `L`: Turkish ↔ English, remembered for the next start. Texts built
    /// before (an open report) keep their language until rebuilt.
    fn switch_language(&mut self) {
        let lang = crate::i18n::lang().other();
        crate::i18n::set_lang(lang);
        crate::i18n::save(lang);
        let msg = t!("Dil: Türkçe (L: English)", "Language: English (L: Türkçe)");
        match &mut self.browser {
            Some(b) if matches!(self.screen, Screen::Browser) => b.set_status(msg, false),
            _ => self.message = Some(msg.to_string()),
        }
    }

    fn on_key_disks(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Up | KeyCode::Char('k') => self.disk_table.select_previous(),
            KeyCode::Down | KeyCode::Char('j') => self.disk_table.select_next(),
            KeyCode::Char('r') => self.refresh_disks(),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                let disk = self.disk_table.selected().and_then(|i| self.disks.get(i));
                if let Some(path) = disk.map(|d| d.mount_point.clone()) {
                    self.start_scan(path);
                }
            }
            _ => {}
        }
    }

    fn on_key_scanning(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc => {
                self.scan = None; // dropping the handle cancels the scan
                self.message = Some(t!("Tarama iptal edildi.", "Scan cancelled.").into());
                self.screen = Screen::DiskSelect;
            }
            _ => {}
        }
    }

    fn on_key_browser(&mut self, code: KeyCode) {
        let Some(b) = &mut self.browser else { return };
        match b.on_key(code) {
            Action::None => {}
            Action::Quit => self.should_quit = true,
            Action::Rescan => {
                let root = b.tree.root_path().to_path_buf();
                self.browser = None;
                self.start_scan(root);
            }
            Action::Disks => {
                self.browser = None;
                self.refresh_disks();
                self.screen = Screen::DiskSelect;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lists::RowSize;
    use crate::tree::Size;
    use std::path::Path;

    fn browser_with(rows: Vec<Row>, keep_one: bool) -> (Browser, [NodeId; 3]) {
        let mut t = Tree::new(Path::new("/r"));
        let s = Size::default();
        let a = t.push(ROOT, "a", false, s);
        t.set_times(a, 300, 300);
        let b = t.push(ROOT, "b", false, s);
        t.set_times(b, 100, 100); // oldest
        let c = t.push(ROOT, "c", false, s);
        t.set_times(c, 200, 200);
        t.finalize();
        let mut br = Browser::new(ScanResult {
            tree: t,
            errors: 0,
            elapsed: Duration::ZERO,
        });
        let rows = rows
            .into_iter()
            .map(|mut r| {
                r.nodes = r.nodes.iter().map(|&i| [a, b, c][i as usize]).collect();
                r
            })
            .collect();
        let mut list = ResultList::new("t".into(), ROOT, rows);
        list.keep_one = keep_one;
        br.results = Some(list);
        (br, [a, b, c])
    }

    fn group(members: &[NodeId]) -> Row {
        let members = members.iter().map(|&i| (i, 1)).collect();
        Row::group("g".into(), String::new(), members, RowSize::Sum, 2)
    }

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
        assert!(br.results.as_ref().unwrap().basket_view);
        assert_eq!(br.results.as_ref().unwrap().rows.len(), 2);
        br.toggle_row();
        assert_eq!(br.basket.len(), 1);
        assert_eq!(br.results.as_ref().unwrap().rows.len(), 1);
    }
}
