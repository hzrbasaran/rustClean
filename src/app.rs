//! Application state and key handling.

use std::path::PathBuf;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::TableState;

use crate::delete::{self, Deletion};
use crate::disks::{self, DiskInfo};
use crate::scanner::{self, ScanHandle, ScanMsg, ScanProgress, ScanResult};
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
    /// Entry awaiting a yes/no answer before being moved to the trash.
    pub confirm: Option<NodeId>,
    /// Entry currently being moved to the trash.
    pub deleting: Option<(NodeId, Deletion)>,
    pub status: Option<Status>,
    /// Total moved to the trash during this session.
    pub trashed: Size,
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
            confirm: None,
            deleting: None,
            status: None,
            trashed: Size::default(),
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

    fn cycle_sort(&mut self) {
        self.sort = self.sort.next();
        self.load(self.current, 0);
    }

    fn set_status(&mut self, text: impl Into<String>, error: bool) {
        self.status = Some(Status {
            text: text.into(),
            error,
        });
    }

    fn request_delete(&mut self) {
        let Some(id) = self.selected() else { return };
        match delete::check(&self.tree.path_of(id), &disks::all_mount_points()) {
            Ok(()) => self.confirm = Some(id),
            Err(msg) => self.set_status(msg, true),
        }
    }

    fn confirm_delete(&mut self) {
        if let Some(id) = self.confirm.take() {
            let path = self.tree.path_of(id);
            self.set_status(format!("Çöp kutusuna taşınıyor: {}", path.display()), false);
            self.deleting = Some((id, Deletion::start(path)));
        }
    }

    /// Applies the result of a finished trash operation.
    fn poll_delete(&mut self) {
        let Some((id, deletion)) = &self.deleting else {
            return;
        };
        let Some(result) = deletion.poll() else {
            return;
        };
        let id = *id;
        self.deleting = None;
        let name = self.tree.name(id).to_string();
        match result {
            Ok(()) => {
                let size = self.tree.node(id).size;
                self.tree.remove(id);
                self.trashed += size;
                let row = self.table.selected().unwrap_or(0);
                self.load(self.current, row);
                self.set_status(
                    format!(
                        "✓ {name} çöp kutusuna taşındı ({}). Yer, çöp kutusu boşaltılınca açılır.",
                        crate::ui::fmt_size(size.get(self.size_mode))
                    ),
                    false,
                );
            }
            Err(err) => self.set_status(format!("✗ {name} taşınamadı: {err}"), true),
        }
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
        if b.deleting.is_some() {
            if code == KeyCode::Char('q') {
                self.should_quit = true;
            }
            return;
        }
        if b.confirm.is_some() {
            match code {
                KeyCode::Char('e' | 'E' | 'y' | 'Y') => b.confirm_delete(),
                _ => b.confirm = None,
            }
            return;
        }
        b.status = None;
        match code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Up | KeyCode::Char('k') => b.table.select_previous(),
            KeyCode::Down | KeyCode::Char('j') => b.table.select_next(),
            KeyCode::PageUp => b.table.scroll_up_by(20),
            KeyCode::PageDown => b.table.scroll_down_by(20),
            KeyCode::Home | KeyCode::Char('g') => b.table.select_first(),
            KeyCode::End | KeyCode::Char('G') => b.table.select_last(),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => b.enter(),
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') | KeyCode::Esc => b.back(),
            KeyCode::Char('s') => b.cycle_sort(),
            KeyCode::Char('a') => b.toggle_size_mode(),
            KeyCode::Char('x') | KeyCode::Delete => b.request_delete(),
            KeyCode::Char('r') => {
                let root = b.tree.root_path().to_path_buf();
                self.browser = None;
                self.start_scan(root);
            }
            KeyCode::Char('d') => {
                self.browser = None;
                self.refresh_disks();
                self.screen = Screen::DiskSelect;
            }
            _ => {}
        }
    }
}
