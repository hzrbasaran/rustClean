//! Application state and key handling.

use std::path::PathBuf;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::TableState;

use crate::delete::{self, Deletion};
use crate::disks::{self, DiskInfo};
use crate::scanner::{self, ScanHandle, ScanMsg, ScanProgress, ScanResult};
use crate::search;
use crate::tree::{NodeId, Size, SizeMode, Tree, ROOT};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    Size,
    Name,
    Count,
}

impl SortMode {
    fn next(self) -> Self {
        match self {
            SortMode::Size => SortMode::Name,
            SortMode::Name => SortMode::Count,
            SortMode::Count => SortMode::Size,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortMode::Size => "boyut",
            SortMode::Name => "ad",
            SortMode::Count => "dosya sayısı",
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

/// Entries found by a name search, shown in place of the directory listing.
pub struct SearchResults {
    pub pattern: String,
    /// Directory the search ran in.
    pub base: NodeId,
    pub items: Vec<NodeId>,
    /// Path of each item relative to `base`.
    pub labels: Vec<String>,
    pub checked: Vec<bool>,
    pub table: TableState,
}

impl SearchResults {
    pub fn checked_ids(&self) -> Vec<NodeId> {
        self.items
            .iter()
            .zip(&self.checked)
            .filter_map(|(&id, &c)| c.then_some(id))
            .collect()
    }

    fn remove(&mut self, id: NodeId) {
        if let Some(i) = self.items.iter().position(|&x| x == id) {
            self.items.remove(i);
            self.labels.remove(i);
            self.checked.remove(i);
            if self.items.is_empty() {
                self.table.select(None);
            } else if self.table.selected().is_some_and(|s| s >= self.items.len()) {
                self.table.select(Some(self.items.len() - 1));
            }
        }
    }
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
    pub results: Option<SearchResults>,
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

    fn toggle_size_mode(&mut self) {
        self.size_mode = match self.size_mode {
            SizeMode::Apparent => SizeMode::Disk,
            SizeMode::Disk => SizeMode::Apparent,
        };
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
        let base = self.current;
        let tree = &self.tree;
        let mode = self.size_mode;
        let mut items = search::find(tree, base, &pat);
        items.sort_by_key(|&id| std::cmp::Reverse(tree.node(id).size.get(mode)));
        let base_path = tree.path_of(base);
        let labels = items
            .iter()
            .map(|&id| {
                let path = tree.path_of(id);
                let rel = path.strip_prefix(&base_path).unwrap_or(&path);
                let mut label = rel.display().to_string();
                if tree.node(id).is_dir {
                    label.push('/');
                }
                label
            })
            .collect();
        let mut table = TableState::default();
        table.select((!items.is_empty()).then_some(0));
        self.results = Some(SearchResults {
            pattern,
            base,
            checked: vec![true; items.len()],
            items,
            labels,
            table,
        });
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
                        r.remove(id);
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
            KeyCode::Char(' ') => {
                if let Some(i) = r.table.selected().filter(|&i| i < r.checked.len()) {
                    r.checked[i] = !r.checked[i];
                    r.table.select_next();
                }
            }
            KeyCode::Char('t') => {
                let all = r.checked.iter().all(|&c| c);
                r.checked.iter_mut().for_each(|c| *c = !all);
            }
            KeyCode::Char('x') | KeyCode::Delete => {
                let ids = r.checked_ids();
                self.request_delete(ids);
            }
            KeyCode::Char('/') => self.input = Some(r.pattern.clone()),
            KeyCode::Char('a') => self.toggle_size_mode(),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                let target = r.table.selected().and_then(|i| r.items.get(i).copied());
                self.results = None;
                if let Some(id) = target {
                    self.reveal(id);
                }
            }
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
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
        }
        let Some(handle) = &self.scan else { return };
        loop {
            match handle.rx.try_recv() {
                Ok(ScanMsg::Progress(p)) => self.progress = p,
                Ok(ScanMsg::Done(res)) => {
                    self.scan = None;
                    self.browser = Some(Browser::new(res));
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
