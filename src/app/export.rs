//! `o`: saving the list on screen as CSV or JSON.

use crossterm::event::KeyCode;

use crate::export::{self, Format, Meta, Record};
use crate::lists::{ResultList, Source};

use super::{Browser, Pane};

/// What `o` would save: a name for the file, the header, the entries.
pub struct ExportSource {
    pub what: String,
    pub meta: Meta,
    pub records: Vec<Record>,
}

/// The short name of a result list for the file name: the report's
/// command-line name, `search`, `changes`, `basket`. A group's members are
/// named after the list they were opened from.
fn list_name(list: &ResultList) -> String {
    let mut top = list;
    while let (Source::Members, Some(parent)) = (&top.source, &top.parent) {
        top = parent;
    }
    match &top.source {
        Source::Report { kind, .. } => kind.slug().to_string(),
        Source::Duplicates => crate::reports::ReportKind::Duplicates.slug().to_string(),
        Source::Search(_) => "search".into(),
        Source::Changes(_) => "changes".into(),
        Source::Basket => "basket".into(),
        Source::Members => "list".into(),
    }
}

impl Browser {
    /// The list `o` saves on the current screen; `None` on screens without
    /// one (tools, system data, the deletion log).
    pub fn export_source(&self) -> Option<ExportSource> {
        if self.tools.is_some() || self.system.is_some() || self.deletion_log.is_some() {
            return None;
        }
        let tree = &self.tree;
        if let Some(d) = &self.dashboard {
            let root = tree.path_of(d.base);
            let (what, title, records) = match d.focus {
                Pane::Files => (
                    "summary-files",
                    t!("En büyük dosyalar", "Largest files"),
                    export::entries(tree, &d.stats.top_files),
                ),
                Pane::Dirs => (
                    "summary-folders",
                    t!(
                        "En dolu klasörler (doğrudan içerik)",
                        "Fullest folders (direct contents)"
                    ),
                    d.stats
                        .top_dirs
                        .iter()
                        .map(|&(id, direct)| {
                            let detail = tf!(
                                "doğrudan içerik: {}",
                                "direct contents: {}",
                                crate::ui::fmt_size(direct)
                            );
                            Record::of(tree, id, "", &detail)
                        })
                        .collect(),
                ),
            };
            return Some(ExportSource {
                what: what.into(),
                meta: Meta {
                    title: format!("{title} — {}", root.display()),
                    root,
                    truncated: false,
                },
                records,
            });
        }
        if let Some(list) = &self.results {
            return Some(ExportSource {
                what: list_name(list),
                meta: Meta {
                    title: list.title.clone(),
                    root: tree.path_of(list.base),
                    truncated: list.truncated,
                },
                records: export::list_records(tree, list),
            });
        }
        let root = tree.path_of(self.current);
        Some(ExportSource {
            what: "folder".into(),
            meta: Meta {
                title: root.display().to_string(),
                root,
                truncated: false,
            },
            records: export::entries(tree, &self.entries),
        })
    }

    /// `o`: asks for the format, when there is something to save.
    pub(super) fn start_export(&mut self) {
        match self.export_source() {
            Some(s) if !s.records.is_empty() => self.export_prompt = Some(s.records.len()),
            Some(_) => self.set_status(
                t!(
                    "Bu listede kaydedilecek bir şey yok.",
                    "There is nothing in this list to save."
                ),
                true,
            ),
            None => {}
        }
    }

    /// A key while the format is asked: `c` CSV, `j` JSON, anything else
    /// cancels.
    pub(super) fn on_key_export(&mut self, code: KeyCode) {
        self.export_prompt = None;
        let format = match code {
            KeyCode::Char('c' | 'C') => Format::Csv,
            KeyCode::Char('j' | 'J') => Format::Json,
            _ => return,
        };
        self.export(format);
    }

    fn export(&mut self, format: Format) {
        let Some(src) = self.export_source() else {
            return;
        };
        let dirs = match &self.export_dir {
            Some(dir) => vec![dir.clone()],
            None => export::default_dirs(),
        };
        match export::save(&dirs, &src.what, format, &src.meta, &src.records) {
            Ok(path) => self.set_status(
                tf!(
                    "{} kaydedildi: {}",
                    "{} saved: {}",
                    crate::i18n::count(src.records.len() as u64, "satır", "row", "rows"),
                    crate::ui::tilde(&path).display()
                ),
                false,
            ),
            Err(e) => self.set_status(tf!("Kaydedilemedi: {e}", "Could not save: {e}"), true),
        }
    }
}
