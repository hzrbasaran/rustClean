//! Rescanning one folder in the background (`R`).

use std::sync::mpsc::TryRecvError;

use crate::scanner::{self, ScanHandle, ScanMsg, ScanProgress, ScanResult};
use crate::tree::NodeId;

use super::Browser;

/// A rescan of one folder running in the background.
pub struct Rescan {
    pub dir: NodeId,
    pub handle: ScanHandle,
    pub progress: ScanProgress,
}

impl Browser {
    /// Turns a finished duplicate search into a report.
    /// `R`: rescans the folder being browsed in the background.
    pub(super) fn start_rescan(&mut self) {
        let dir = self.current;
        let path = self.tree.path_of(dir);
        self.set_status(
            tf!("Yeniden taranıyor: {}", "Rescanning: {}", path.display()),
            false,
        );
        self.rescan = Some(Rescan {
            dir,
            handle: scanner::start(path, crate::config::scan_skip()),
            progress: ScanProgress::default(),
        });
    }

    /// Puts a finished folder rescan into the tree.
    pub(super) fn poll_rescan(&mut self) {
        let Some(rescan) = &mut self.rescan else {
            return;
        };
        let mut finished = None;
        loop {
            match rescan.handle.rx.try_recv() {
                Ok(ScanMsg::Progress(p)) => rescan.progress = p,
                Ok(ScanMsg::Done(res)) => {
                    finished = Some(Ok(res));
                    break;
                }
                Ok(ScanMsg::Failed(err)) => {
                    finished = Some(Err(err));
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = Some(Err(t!("tarama durdu", "the scan stopped").to_string()));
                    break;
                }
            }
        }
        let Some(result) = finished else {
            return;
        };
        let dir = rescan.dir;
        self.rescan = None;
        match result {
            Ok(res) => self.apply_rescan(dir, &res),
            Err(err) => self.set_status(
                tf!("Yeniden tarama başarısız: {}", "Rescan failed: {}", err),
                true,
            ),
        }
    }

    fn apply_rescan(&mut self, dir: NodeId, res: &ScanResult) {
        let mode = self.size_mode;
        let before = self.tree.node(dir).size.get(mode);
        // Keep the cursor on the same name; ids below `dir` change.
        let selected_name = self.selected().map(|id| self.tree.name(id).to_string());
        self.basket.remove_below(&self.tree, dir);
        self.tree.replace_children(dir, &res.tree);
        let row = self.table.selected().unwrap_or(0);
        self.load(self.current, row);
        if let Some(name) = selected_name {
            if let Some(pos) = self.entries.iter().position(|&e| self.tree.name(e) == name) {
                self.table.select(Some(pos));
            }
        }
        self.refresh_dashboard();
        let after = self.tree.node(dir).size.get(mode);
        self.set_status(
            tf!(
                "✓ Yeniden tarandı ({:.1} sn): {} → {} ({})",
                "✓ Rescanned ({:.1} s): {} → {} ({})",
                res.elapsed.as_secs_f64(),
                crate::ui::fmt_size(before),
                crate::ui::fmt_size(after),
                crate::ui::fmt_delta(after, before)
            ),
            false,
        );
    }
}
