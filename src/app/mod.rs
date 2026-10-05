//! Application state and key handling. `App` moves between the disk list,
//! the scan and the browser; `Browser` holds everything about one scan. Its
//! methods are split by job over the submodules; the struct stays here so
//! they can all use its private fields.

use std::path::PathBuf;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::widgets::TableState;

use crate::basket::Basket;
use crate::delete::{self, Deletion};
use crate::disks::{self, DiskInfo};
use crate::duplicates::DupJob;
use crate::history;
use crate::lists::ResultList;
use crate::reports::ReportKind;
use crate::scanner::{self, ScanHandle, ScanMsg, ScanProgress, ScanResult};
use crate::system::SystemInfo;
use crate::toolsview::ToolsView;
use crate::tree::{NodeId, Size, SizeMode, Tree, ROOT};

use rescan::Rescan;

mod basket;
mod browser;
mod dashboard;
mod export;
mod htmlmap;
mod mouse;
mod rescan;
mod results;
mod snapshots;
mod trash;
mod uninstall;

pub use dashboard::{Dashboard, Pane};
pub use mouse::{Hit, Mouse};
pub use trash::FailureDialog;
pub use uninstall::UninstallDialog;

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

/// The deletion log screen.
pub struct LogView {
    /// Newest first.
    pub entries: Vec<crate::trashlog::Entry>,
    /// First line shown.
    pub scroll: u16,
}

impl LogView {
    pub fn open() -> Self {
        Self {
            entries: crate::trashlog::read(),
            scroll: 0,
        }
    }
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
    /// The first line of the report menu's list that is shown, when the
    /// list does not fit (kept by the drawing, so the view only moves when
    /// the selection reaches an edge).
    pub menu_scroll: usize,
    /// Saved scans to compare with, and the selected one.
    pub snapshot_picker: Option<(Vec<history::Saved>, usize)>,
    /// Open "system data" panel.
    pub system: Option<SystemInfo>,
    /// Open deletion log.
    pub deletion_log: Option<LogView>,
    /// Open developer tools cleanup screen.
    pub tools: Option<ToolsView>,
    /// Report to run once the "preparing" message has been drawn.
    pending_report: Option<(ReportKind, u8)>,
    /// Running duplicate search.
    pub dup_job: Option<DupJob>,
    /// Folder being rescanned (`R`), with its progress.
    pub rescan: Option<Rescan>,
    /// History time stamp of this scan, so it is not compared with itself.
    snapshot_time: Option<u64>,
    /// Entries awaiting a yes/no answer before being moved to the trash.
    pub confirm: Option<Vec<NodeId>>,
    /// Open "uninstall app" dialog.
    pub uninstall: Option<UninstallDialog>,
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
    /// How the running deletion batch was started, for the deletion log.
    batch_via: crate::trashlog::Via,
    /// Entries that could not be trashed, shown in a dialog until dismissed.
    pub failures: Option<FailureDialog>,
    /// `o` asks whether to save the list as CSV or JSON; the number of
    /// entries it would save.
    pub export_prompt: Option<usize>,
    /// Where `o` saves instead of the working or home folder (tests).
    pub export_dir: Option<PathBuf>,
    /// Directories we came from, with the row that was selected there.
    history: Vec<(NodeId, usize)>,
    /// Where `w` writes the treemap page. `None`: the working directory,
    /// else the home folder (tests set a temp directory).
    pub html_dir: Option<PathBuf>,
}

impl Browser {
    pub(crate) fn new(res: ScanResult) -> Self {
        let config = crate::config::get();
        let mut b = Self {
            tree: res.tree,
            errors: res.errors,
            elapsed: res.elapsed,
            current: ROOT,
            entries: Vec::new(),
            table: TableState::default(),
            sort: config.sort,
            size_mode: config.size,
            input: None,
            results: None,
            dashboard: None,
            report_menu: None,
            menu_scroll: 0,
            snapshot_picker: None,
            system: None,
            deletion_log: None,
            tools: None,
            pending_report: None,
            dup_job: None,
            rescan: None,
            snapshot_time: None,
            confirm: None,
            uninstall: None,
            deleting: None,
            status: None,
            basket: Basket::default(),
            view: View::List,
            map_color: MapColor::Kind,
            map_area: Rect::default(),
            trashed: Size::default(),
            batch_trashed: Size::default(),
            batch_failures: Vec::new(),
            batch_via: crate::trashlog::Via::List,
            failures: None,
            export_prompt: None,
            export_dir: None,
            history: Vec::new(),
            html_dir: None,
        };
        b.load(ROOT, 0);
        b
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
    /// The help screen (`?`) is open, scrolled this far.
    pub help: Option<u16>,
    /// A problem with the configuration file, shown on the disk list and
    /// on the status line of the first scan opened.
    config_notice: Option<String>,
    /// Mouse support (`M`) and where the last frame drew its clickable rows.
    pub mouse: Mouse,
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
            help: None,
            config_notice: None,
            mouse: Mouse::default(),
        };
        app.refresh_disks();
        if let Some(path) = start_path {
            app.start_scan(path);
        }
        app
    }

    /// Shows a problem with the configuration file: on the disk list now,
    /// and on the status line when the scan opens.
    pub fn config_notice(&mut self, text: String) {
        if !matches!(self.screen, Screen::Scanning) {
            self.message = Some(text.clone());
        }
        self.config_notice = Some(text);
    }

    fn refresh_disks(&mut self) {
        self.disks = disks::list_disks();
        self.disk_table
            .select((!self.disks.is_empty()).then_some(0));
    }

    fn start_scan(&mut self, root: PathBuf) {
        self.message = None;
        self.progress = ScanProgress::default();
        self.scan = Some(scanner::start(root.clone(), crate::config::scan_skip()));
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
            b.poll_rescan();
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
                    if let Some(text) = self.config_notice.take() {
                        browser.set_status(text, true);
                    }
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
        if let Some(scroll) = &mut self.help {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => *scroll = scroll.saturating_add(1),
                KeyCode::PageUp => *scroll = scroll.saturating_sub(10),
                KeyCode::PageDown => *scroll = scroll.saturating_add(10),
                KeyCode::Home => *scroll = 0,
                KeyCode::Char('L') => self.switch_language(),
                KeyCode::Char('T') => self.switch_theme(),
                KeyCode::Char('M') => self.toggle_mouse(),
                KeyCode::Char('q') => self.should_quit = true,
                _ => self.help = None,
            }
            return;
        }
        if key.code == KeyCode::Char('?') && !self.typing() {
            self.help = Some(0);
            return;
        }
        if key.code == KeyCode::Char('L') && !self.typing() {
            self.switch_language();
            return;
        }
        if key.code == KeyCode::Char('T') && !self.typing() {
            self.switch_theme();
            return;
        }
        if key.code == KeyCode::Char('M') && !self.typing() {
            self.toggle_mouse();
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

    /// `L`: Turkish ↔ English, remembered for the next start. An open result
    /// list is built again, so its title, notes and details follow.
    fn switch_language(&mut self) {
        let lang = crate::i18n::lang().other();
        crate::i18n::set_lang(lang);
        crate::i18n::save(lang);
        if let Some(b) = &mut self.browser {
            b.rebuild_results();
        }
        let msg = t!("Dil: Türkçe (L: English)", "Language: English (L: Türkçe)");
        match &mut self.browser {
            Some(b) if matches!(self.screen, Screen::Browser) => b.set_status(msg, false),
            _ => self.message = Some(msg.to_string()),
        }
    }

    /// `T`: dark → light → color-blind, remembered for the next start. With
    /// `--no-color` or `NO_COLOR` the colors stay off.
    fn switch_theme(&mut self) {
        use crate::ui::theme::{self, ThemeKind};
        let current = theme::current();
        let msg = if current == ThemeKind::Mono {
            t!(
                "Renkler kapalı (--no-color / NO_COLOR).",
                "Colors are off (--no-color / NO_COLOR)."
            )
            .to_string()
        } else {
            let next = current.next();
            theme::set(next);
            theme::save(next);
            tf!(
                "Tema: {} (T: değiştir)",
                "Theme: {} (T: change)",
                next.label()
            )
        };
        match &mut self.browser {
            Some(b) if matches!(self.screen, Screen::Browser) => b.set_status(msg, false),
            _ => self.message = Some(msg),
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
pub(super) mod test_util {
    use std::path::Path;
    use std::time::Duration;

    use super::Browser;
    use crate::lists::{ResultList, Row, RowSize};
    use crate::scanner::ScanResult;
    use crate::tree::{NodeId, Size, Tree, ROOT};

    pub(crate) fn browser_with(rows: Vec<Row>, keep_one: bool) -> (Browser, [NodeId; 3]) {
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

    pub(crate) fn group(members: &[NodeId]) -> Row {
        let members = members.iter().map(|&i| (i, 1)).collect();
        Row::group("g".into(), String::new(), members, RowSize::Sum, 2)
    }
}
