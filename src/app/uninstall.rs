//! Uninstalling an app together with its data.

use crossterm::event::KeyCode;

use crate::apps;
use crate::lists::ResultList;
use crate::reports::ReportKind;
use crate::tree::NodeId;

use super::Browser;

/// An app and its data about to be moved to the trash; every entry can be
/// unchecked, since matching data to apps is a guess.
#[derive(Debug)]
pub struct UninstallDialog {
    pub label: String,
    /// The app bundle first, then its data, each with whether it is checked.
    pub items: Vec<(NodeId, bool)>,
    pub cursor: usize,
    /// A process was started from inside the bundle.
    pub running: bool,
}

impl UninstallDialog {
    fn toggle(&mut self) {
        if let Some(item) = self.items.get_mut(self.cursor) {
            item.1 = !item.1;
        }
    }

    /// Checks everything, or unchecks everything when all are checked.
    fn toggle_all(&mut self) {
        let all = self.items.iter().all(|i| i.1);
        for item in &mut self.items {
            item.1 = !all;
        }
    }

    pub fn chosen(&self) -> Vec<NodeId> {
        self.items.iter().filter(|i| i.1).map(|i| i.0).collect()
    }
}

impl Browser {
    /// The open apps report: the current result list, or the one a group
    /// was opened from.
    pub fn apps_report(&self) -> Option<&ResultList> {
        let r = self.results.as_ref()?;
        let is_apps = |l: &ResultList| l.report() == Some(ReportKind::Apps);
        if is_apps(r) {
            Some(r)
        } else {
            r.parent.as_deref().filter(|p| is_apps(p))
        }
    }

    /// `u`: the uninstall dialog for the app under the cursor in the apps
    /// report (or the app whose group is open).
    pub(super) fn start_uninstall(&mut self) {
        let Some(row) = self.apps_report().and_then(ResultList::selected_row) else {
            self.set_status(
                t!(
                    "u: uygulamalar raporunda bir uygulamayı kaldırır (m → Uygulamalar ve verileri).",
                    "u: removes an app from the apps report (m → Applications and their data).",
                ),
                true,
            );
            return;
        };
        if !cfg!(target_os = "macos") {
            self.set_status(
                t!(
                    "Kaldırma yalnız macOS'ta; burada uygulamanın kendi kaldırıcısını kullanın.",
                    "Uninstalling is macOS only; use the app's own uninstaller here.",
                ),
                true,
            );
            return;
        }
        let app = row.nodes[0];
        let path = self.tree.path_of(app);
        if let Err(msg) = apps::uninstall_check(&path) {
            self.set_status(msg, true);
            return;
        }
        let mode = self.size_mode;
        let mut data = row.nodes[1..].to_vec();
        data.sort_by_key(|&id| std::cmp::Reverse(self.tree.node(id).size.get(mode)));
        let label = row.label.clone();
        self.uninstall = Some(UninstallDialog {
            label,
            items: std::iter::once(app)
                .chain(data)
                .map(|id| (id, true))
                .collect(),
            cursor: 0,
            running: apps::is_running(&path),
        });
    }

    pub(super) fn on_key_uninstall(&mut self, code: KeyCode) {
        let Some(d) = &mut self.uninstall else {
            return;
        };
        match code {
            KeyCode::Up | KeyCode::Char('k') => d.cursor = d.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                d.cursor = (d.cursor + 1).min(d.items.len().saturating_sub(1));
            }
            KeyCode::Char(' ') => d.toggle(),
            KeyCode::Char('t') => d.toggle_all(),
            KeyCode::Char('e' | 'E' | 'y' | 'Y') => {
                let ids = d.chosen();
                if ids.is_empty() {
                    self.set_status(t!("Seçili öğe yok.", "Nothing selected."), true);
                    return;
                }
                self.uninstall = None;
                let ok = self.deletable(ids);
                if !ok.is_empty() {
                    self.start_deletion(&ok);
                }
            }
            KeyCode::Esc | KeyCode::Char('h' | 'H' | 'n' | 'N' | 'q') => self.uninstall = None,
            _ => {}
        }
    }

    /// After an uninstall, lists the apps again: removed apps drop out and
    /// the data sizes of the others are current. Keeps the cursor on the
    /// same app.
    pub(super) fn refresh_apps_report(&mut self) {
        if self.apps_report().is_some() {
            self.rebuild_results();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_util::{browser_with, group};
    use crate::lists::{Row, RowSize};
    use crate::scanner::ScanResult;
    use crate::tree::{Size, Tree, ROOT};
    use std::path::Path;
    use std::time::Duration;

    #[test]
    fn uninstall_dialog_lists_the_app_first_and_trashes_only_checked() {
        let mut t = Tree::new(Path::new("/r"));
        let apps_dir = t.push(ROOT, "Applications", true, Size::default());
        let app = t.push(apps_dir, "Foo.app", true, Size::default());
        t.push(
            app,
            "bin",
            false,
            Size {
                apparent: 5,
                disk: 5,
            },
        );
        let small = t.push(
            ROOT,
            "small",
            false,
            Size {
                apparent: 1,
                disk: 1,
            },
        );
        let big = t.push(
            ROOT,
            "big",
            false,
            Size {
                apparent: 9,
                disk: 9,
            },
        );
        t.finalize();
        let mut br = Browser::new(ScanResult {
            tree: t,
            errors: 0,
            elapsed: Duration::ZERO,
        });
        let row = Row::group(
            "Foo".into(),
            String::new(),
            vec![(app, 5), (small, 1), (big, 9)],
            RowSize::Sum,
            1,
        );
        let mut list = ResultList::new("apps".into(), ROOT, vec![row]);
        list.source = crate::lists::Source::Report {
            kind: ReportKind::Apps,
            min_age_days: 0,
        };
        br.results = Some(list);
        // From inside the group, too.
        br.drill_down();

        br.start_uninstall();
        if !cfg!(target_os = "macos") {
            assert!(br.uninstall.is_none());
            return;
        }
        let d = br.uninstall.as_mut().unwrap();
        assert_eq!(d.label, "Foo");
        assert_eq!(d.items, vec![(app, true), (big, true), (small, true)]);
        d.cursor = 1;
        d.toggle();
        assert_eq!(d.chosen(), vec![app, small]);
        d.toggle_all();
        assert_eq!(d.chosen(), vec![app, big, small]);
        d.toggle_all();
        assert!(d.chosen().is_empty());

        // Nothing checked: `e` keeps the dialog open.
        br.on_key(KeyCode::Char('e'));
        assert!(br.uninstall.is_some());
        br.on_key(KeyCode::Esc);
        assert!(br.uninstall.is_none());
    }

    #[test]
    fn u_outside_the_apps_report_explains_itself() {
        let (mut br, _) = browser_with(vec![group(&[0, 1])], false);
        br.start_uninstall();
        assert!(br.uninstall.is_none());
        assert!(br.status.is_some());
    }
}
