//! Application state and key handling.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::TableState;

use crate::apps;
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
            SortMode::Size => "boyut",
            SortMode::Name => "ad",
            SortMode::Count => "dosya sayısı",
            SortMode::Modified => "son değişiklik (en eski önce)",
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
    /// Total moved to the trash during this session.
    pub trashed: Size,
    /// Outcome of the running deletion batch.
    batch_trashed: Size,
    batch_failures: Vec<String>,
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
            trashed: Size::default(),
            batch_trashed: Size::default(),
            batch_failures: Vec::new(),
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
        // Search results start checked: the user typed exactly what to find.
        let mut list = ResultList::new(format!("Arama „{pattern}”"), base, rows, true);
        list.note = "Eşleşen klasörlerin içi ayrıca listelenmez; klasörle birlikte taşınır.".into();
        list.pattern = Some(pattern);
        self.results = Some(list);
    }

    fn open_menu_item(&mut self, item: MenuItem) {
        match item {
            MenuItem::Report(kind) => self.request_report(kind),
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
                "Bu klasörün kayıtlı eski bir taraması yok; bir sonraki taramadan sonra karşılaştırabilirsiniz.",
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
                self.set_status(format!("Kayıt okunamadı: {e}"), true);
                return;
            }
        };
        self.dashboard = None;
        let (rows, truncated, note) =
            history::changes(&self.tree, self.current, &snap, self.size_mode);
        let title = format!(
            "Değişenler ({} taramasına göre)",
            crate::ui::fmt_date(snap.header.time.min(u64::from(u32::MAX)) as u32)
        );
        let mut list = ResultList::new(title, self.current, rows, false);
        list.truncated = truncated;
        list.note = note;
        self.results = Some(list);
    }

    fn request_report(&mut self, kind: ReportKind) {
        self.report_menu = None;
        self.dashboard = None;
        self.set_status(format!("Rapor hazırlanıyor: {}…", kind.label()), false);
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
            ReportKind::Duplicates => {
                let cands = duplicates::candidates(&self.tree, self.current);
                if cands.is_empty() {
                    let mut list =
                        ResultList::new(kind.label().to_string(), self.current, Vec::new(), false);
                    list.note = "1 MiB üzerinde aynı boyutta iki dosya yok.".into();
                    self.results = Some(list);
                } else {
                    self.dup_job = Some(DupJob::start(self.current, cands));
                }
            }
            _ => {
                let now = crate::ui::now_secs();
                let list = reports::run(&self.tree, self.current, self.size_mode, now, kind);
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
                format!(
                    "Son taramadan beri ({}): {} · m → Değişenler",
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
                let detail = format!(
                    "{} kopya · her biri {}",
                    ids.len(),
                    crate::ui::fmt_size(tree.node(first).size.get(mode))
                );
                let members = ids
                    .iter()
                    .map(|&id| (id, tree.node(id).size.get(mode)))
                    .collect();
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
        let mut list = ResultList::new(
            ReportKind::Duplicates.label().to_string(),
            base,
            rows,
            false,
        );
        list.truncated = truncated;
        list.keep_one = true;
        list.note =
            "Boyut: kopyalar silinince açılacak yer. Enter: kopyaları gör (en eskisi korunur). \
                     APFS klonları blok paylaştığından silmek yer açmayabilir."
                .into();
        self.results = Some(list);
    }

    /// What `x` removes in a result list: the checked rows, or the row under
    /// the cursor when nothing is checked. Duplicate groups always keep their
    /// oldest copy.
    fn result_targets(&self) -> Result<Vec<NodeId>, &'static str> {
        let Some(r) = &self.results else {
            return Ok(Vec::new());
        };
        let rows: Vec<&Row> = if r.checked.iter().any(|&c| c) {
            r.rows
                .iter()
                .zip(&r.checked)
                .filter(|(_, &c)| c)
                .map(|(row, _)| row)
                .collect()
        } else {
            match r.selected_row() {
                Some(row) if row.group && !r.keep_one => {
                    return Err(
                        "Bu bir grup: içine girmek için Enter, tamamını seçmek için Space.",
                    );
                }
                Some(row) => vec![row],
                None => return Ok(Vec::new()),
            }
        };
        let mut ids = Vec::new();
        for row in rows {
            let keep = if r.keep_one && row.group {
                oldest(&self.tree, &row.nodes)
            } else {
                None
            };
            ids.extend(
                row.nodes
                    .iter()
                    .enumerate()
                    .filter(|&(i, _)| Some(i) != keep)
                    .map(|(_, &id)| id),
            );
        }
        Ok(ids)
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
        let mut list = ResultList::new(format!("{} › {}", r.title, row.label), r.base, rows, false);
        if r.keep_one {
            // Keep the copy that has existed longest; check the rest.
            if let Some(keep) = oldest(tree, &ids) {
                list.checked = (0..ids.len()).map(|i| i != keep).collect();
                list.note = format!(
                    "En eski kopya ({}) korunuyor; diğerleri seçili.",
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
            self.set_status("Seçili öğe yok.", true);
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
                format!(" (+{} öğe daha)", refused.len() - 1)
            } else {
                String::new()
            };
            self.set_status(format!("Atlandı — {}{more}", refused[0]), true);
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
        self.set_status(format!("Çöp kutusuna taşınıyor… 0/{}", items.len()), false);
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
                    self.trashed += size;
                    self.batch_trashed += size;
                    if let Some(r) = &mut self.results {
                        r.remove_nodes(&HashSet::from([id]));
                    }
                }
                Err(err) => self
                    .batch_failures
                    .push(format!("{}: {err}", self.tree.name(id))),
            }
        }
        if !finished {
            self.set_status(format!("Çöp kutusuna taşınıyor… {done}/{total}"), false);
            return;
        }

        self.deleting = None;
        let row = self.table.selected().unwrap_or(0);
        self.load(self.current, row);
        self.refresh_dashboard();
        let moved = total - self.batch_failures.len();
        let size = crate::ui::fmt_size(self.batch_trashed.get(self.size_mode));
        if self.batch_failures.is_empty() {
            self.set_status(
                format!("✓ {moved} öğe çöp kutusuna taşındı ({size}). Yer, çöp kutusu boşaltılınca açılır."),
                false,
            );
        } else {
            self.set_status(
                format!(
                    "✗ {} öğe taşınamadı, {moved} öğe taşındı ({size}). İlk hata — {}",
                    self.batch_failures.len(),
                    self.batch_failures[0]
                ),
                true,
            );
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
                    self.set_status("Kopya araması iptal edildi.", true);
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
        match code {
            KeyCode::Char('q') => return Action::Quit,
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
            KeyCode::Char('x') | KeyCode::Delete => {
                if let Some(id) = self.selected() {
                    self.request_delete(vec![id]);
                }
            }
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
            KeyCode::Char('x') | KeyCode::Delete => {
                if let Some(id) = d.selected().filter(|&id| id != d.base) {
                    self.request_delete(vec![id]);
                }
            }
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
            KeyCode::Char(' ') => r.toggle_selected(),
            KeyCode::Char('t') => r.toggle_all(),
            KeyCode::Char('x') | KeyCode::Delete => match self.result_targets() {
                Ok(ids) => self.request_delete(ids),
                Err(msg) => self.set_status(msg, true),
            },
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
                    self.message = Some(format!("Tarama başarısız: {err}"));
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
        match self.screen {
            Screen::DiskSelect => self.on_key_disks(key.code),
            Screen::Scanning => self.on_key_scanning(key.code),
            Screen::Browser => self.on_key_browser(key.code),
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
                self.message = Some("Tarama iptal edildi.".into());
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
        let mut list = ResultList::new("t".into(), ROOT, rows, false);
        list.keep_one = keep_one;
        br.results = Some(list);
        (br, [a, b, c])
    }

    fn group(members: &[NodeId]) -> Row {
        let members = members.iter().map(|&i| (i, 1)).collect();
        Row::group("g".into(), String::new(), members, RowSize::Sum, 2)
    }

    #[test]
    fn x_without_checks_uses_the_cursor_row() {
        let single = Row::group("s".into(), String::new(), vec![(2, 1)], RowSize::Sum, 1);
        let (mut br, [_, _, c]) = browser_with(vec![single], false);
        br.results.as_mut().unwrap().rows[0].group = false;
        assert_eq!(br.result_targets(), Ok(vec![c]));
    }

    #[test]
    fn unchecked_name_group_is_not_deleted_whole() {
        let (br, _) = browser_with(vec![group(&[0, 1, 2])], false);
        assert!(br.result_targets().is_err());
    }

    #[test]
    fn duplicate_groups_keep_the_oldest_copy() {
        // Under the cursor…
        let (mut br, [a, _, c]) = browser_with(vec![group(&[0, 1, 2])], true);
        assert_eq!(br.result_targets(), Ok(vec![a, c]));
        // …and when checked.
        br.results.as_mut().unwrap().checked = vec![true];
        assert_eq!(br.result_targets(), Ok(vec![a, c]));
    }
}
