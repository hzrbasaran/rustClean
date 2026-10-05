//! The summary dashboard of a folder.

use crossterm::event::KeyCode;
use ratatui::widgets::TableState;

use crate::disks::{self, DiskInfo};
use crate::stats::{self, Stats};
use crate::tree::NodeId;

use super::{Action, Browser};

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
    pub(super) fn selected(&self) -> Option<NodeId> {
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

impl Browser {
    pub(super) fn open_dashboard(&mut self) {
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
    pub(super) fn refresh_dashboard(&mut self) {
        if let Some(d) = &mut self.dashboard {
            d.stats = stats::compute(&self.tree, d.base, self.size_mode, crate::ui::now_secs());
            d.clamp_selection();
        }
    }

    pub(super) fn on_key_dashboard(&mut self, code: KeyCode) -> Action {
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
            KeyCode::Char('o') => self.start_export(),
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
}
