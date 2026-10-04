//! Saved scans: recording this one and comparing with an earlier one.

use crate::history;
use crate::lists::ResultList;
use crate::tree::ROOT;

use super::Browser;

impl Browser {
    pub(super) fn open_snapshot_picker(&mut self) {
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

    pub(super) fn compare_with(&mut self, file: &std::path::Path) {
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

    /// Saves this scan to the history (in the background) and tells how much
    /// changed since the previous one.
    pub(super) fn record_history(&mut self) {
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
}
