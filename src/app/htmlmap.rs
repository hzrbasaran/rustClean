//! `w`: the treemap of the current folder as an HTML page (`crate::htmlmap`).

use crate::htmlmap;

use super::Browser;

impl Browser {
    /// Writes the page and says where it went.
    pub(super) fn export_html(&mut self) {
        let now = crate::ui::now_secs();
        let root = htmlmap::extract(
            &self.tree,
            self.current,
            self.size_mode,
            now,
            &htmlmap::LIMITS,
        );
        let path = crate::ui::tilde(&self.tree.path_of(self.current));
        let date = crate::ui::fmt_date(u32::try_from(now).unwrap_or(u32::MAX));
        let page = htmlmap::render(
            &root,
            &htmlmap::PageInfo {
                path: &path.display().to_string(),
                mode: self.size_mode,
                date: &date,
                limits: htmlmap::LIMITS,
            },
        );
        let stamp = crate::newfile::stamp();
        let dirs = self
            .html_dir
            .clone()
            .map_or_else(crate::newfile::default_dirs, |d| vec![d]);
        match htmlmap::save(&page, &stamp, &dirs) {
            Ok(file) => self.set_status(
                tf!(
                    "Harita sayfası kaydedildi: {}",
                    "Treemap page saved: {}",
                    crate::ui::tilde(&file).display()
                ),
                false,
            ),
            Err(e) => self.set_status(
                tf!(
                    "Harita sayfası yazılamadı: {e}",
                    "Could not write the treemap page: {e}"
                ),
                true,
            ),
        }
    }
}
