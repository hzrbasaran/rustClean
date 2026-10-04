//! Predefined reports over the directory being browsed: the report kinds,
//! the menu, the age filter, and the helpers the reports share. Each group
//! of reports lives in its own submodule.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::lists::{ResultList, Row, Source};
use crate::stats::DAY;
use crate::tree::{NodeId, SizeMode, Tree};

mod caches;
mod dev_junk;
mod downloads;
mod names;
mod size;

use caches::caches;
use dev_junk::dev_junk;
use downloads::downloads;
use names::repeated_names;
use size::{largest_dirs, largest_files, old_big};

/// Maximum number of rows a report shows.
pub const LIMIT: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    LargestFiles,
    LargestDirs,
    RepeatedNames,
    Apps,
    Orphans,
    DevJunk,
    Caches,
    OldBig,
    Downloads,
    Duplicates,
}

impl ReportKind {
    pub fn label(self) -> &'static str {
        match self {
            ReportKind::LargestFiles => t!("En büyük dosyalar", "Largest files"),
            ReportKind::LargestDirs => t!("En büyük klasörler", "Largest folders"),
            ReportKind::RepeatedNames => t!(
                "En çok tekrar eden dosya adları",
                "Most repeated file names"
            ),
            ReportKind::Apps => t!("Uygulamalar ve verileri", "Applications and their data"),
            ReportKind::Orphans => t!("Sahipsiz uygulama artıkları", "Orphaned app leftovers"),
            ReportKind::DevJunk => t!("Geliştirici çöpleri", "Developer junk"),
            ReportKind::Caches => t!("Önbellek klasörleri", "Cache folders"),
            ReportKind::OldBig => t!("Eski ve büyük dosyalar", "Old and large files"),
            ReportKind::Downloads => t!(
                "İndirilen kurulum dosyaları ve arşivler",
                "Installers and archives in Downloads"
            ),
            ReportKind::Duplicates => t!(
                "Kopya dosyalar (içeriği aynı)",
                "Duplicate files (same content)"
            ),
        }
    }

    /// Whether `f` (minimum age) applies to this report.
    pub fn supports_age(self) -> bool {
        matches!(
            self,
            ReportKind::LargestFiles
                | ReportKind::LargestDirs
                | ReportKind::RepeatedNames
                | ReportKind::DevJunk
                | ReportKind::Caches
                | ReportKind::Orphans
                | ReportKind::Downloads
        )
    }

    pub fn description(self) -> &'static str {
        match self {
            ReportKind::LargestFiles => t!(
                "Bu klasörün altındaki en büyük 200 dosya",
                "The 200 largest files below this folder"
            ),
            ReportKind::LargestDirs => {
                t!(
                    "Toplam boyuta göre; yalnızca tek bir alt klasörü saran klasörler elenir",
                    "By total size; folders that only wrap one subfolder are skipped"
                )
            }
            ReportKind::RepeatedNames => t!(
                "Aynı adı taşıyan dosyalar, adet sırasıyla",
                "Files sharing a name, by count"
            ),
            ReportKind::Apps => t!(
                "Uygulama paketi + Library'deki verileri (önbellek, destek…)",
                "App bundle + its data in Library (caches, support…)"
            ),
            ReportKind::Orphans => {
                t!("Silinmiş uygulamalardan kalan veriler (Library: destek, önbellek, kapsayıcılar)", "Data left behind by removed apps (Library: support, caches, containers)")
            }
            ReportKind::DevJunk => "node_modules, target, build, Pods, DerivedData, .venv…",
            ReportKind::Caches => t!(
                "Caches ve .cache içindeki uygulama önbellekleri",
                "App caches inside Caches and .cache"
            ),
            ReportKind::OldBig => t!(
                "100 MiB'tan büyük, 1 yıldır değişmemiş dosyalar",
                "Files over 100 MiB, unchanged for a year"
            ),
            ReportKind::Downloads => t!(
                "Downloads klasörlerindeki .dmg, .pkg, .iso, .zip… dosyaları",
                ".dmg, .pkg, .iso, .zip… files in Downloads folders"
            ),
            ReportKind::Duplicates => {
                t!(
                    "İçeriği birebir aynı dosyalar (≥ 1 MiB); dosyalar okunur, sürebilir",
                    "Files with identical content (≥ 1 MiB); files are read, may take a while"
                )
            }
        }
    }
}

/// An entry of the `m` menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    Report(ReportKind),
    Changes,
    /// The orphaned leftovers report, listed again among the tools.
    Leftovers,
    Tools,
    System,
}

impl MenuItem {
    pub const ALL: [MenuItem; 14] = [
        MenuItem::Report(ReportKind::LargestFiles),
        MenuItem::Report(ReportKind::LargestDirs),
        MenuItem::Report(ReportKind::RepeatedNames),
        MenuItem::Report(ReportKind::Apps),
        MenuItem::Report(ReportKind::Orphans),
        MenuItem::Report(ReportKind::DevJunk),
        MenuItem::Report(ReportKind::Caches),
        MenuItem::Report(ReportKind::OldBig),
        MenuItem::Report(ReportKind::Downloads),
        MenuItem::Report(ReportKind::Duplicates),
        MenuItem::Changes,
        MenuItem::Leftovers,
        MenuItem::Tools,
        MenuItem::System,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MenuItem::Report(k) => k.label(),
            MenuItem::Changes => t!(
                "Son taramadan bu yana değişenler",
                "Changes since the last scan"
            ),
            MenuItem::Leftovers => t!("Silinmiş uygulama artıkları", "Removed app leftovers"),
            MenuItem::Tools => t!("Geliştirici araçları temizliği", "Developer tools cleanup"),
            MenuItem::System => t!("Sistem verileri", "System data"),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            MenuItem::Report(k) => k.description(),
            MenuItem::Changes => {
                t!(
                    "Kayıtlı bir taramayla karşılaştırır: büyüyen, yeni ve silinen klasörler",
                    "Compares with a saved scan: grown, new and removed folders"
                )
            }
            MenuItem::Leftovers => {
                t!(
                    "Kaldırılmış uygulamalardan kalan veriler; ev klasörü taraması yeterli",
                    "Data left by uninstalled apps; scanning the home folder is enough"
                )
            }
            MenuItem::Tools => {
                t!(
                    "Docker, Xcode, npm, Gradle… önbelleklerini aracın kendi komutuyla temizler",
                    "Cleans Docker, Xcode, npm, Gradle… caches with the tool's own commands"
                )
            }
            MenuItem::System => t!(
                "APFS bölümleri, Time Machine anlık görüntüleri, takas dosyası",
                "APFS volumes, Time Machine snapshots, swap file"
            ),
        }
    }

    /// Items after the reports form the "Araçlar" section.
    pub fn is_tool(self) -> bool {
        !matches!(self, MenuItem::Report(_))
    }
}

/// Steps of the `f` key, in days (0 = no filter).
pub const AGE_STEPS: [u32; 5] = [0, 30, 90, 180, 365];

/// Keeps entries untouched for at least `min_days`.
#[derive(Debug, Clone, Copy)]
pub struct AgeFilter {
    now: u64,
    min_days: u32,
}

impl AgeFilter {
    pub fn new(now: u64, min_days: u32) -> Self {
        Self { now, min_days }
    }

    /// Entries with an unknown date only pass when there is no filter.
    pub fn ok(&self, modified: u32) -> bool {
        self.min_days == 0
            || (modified != 0
                && self.now.saturating_sub(u64::from(modified)) >= u64::from(self.min_days) * DAY)
    }
}

/// Runs one of the reports that only need the tree. Apps and duplicates
/// have their own modules. `min_age_days` applies where `supports_age`.
pub fn run(
    tree: &Tree,
    base: NodeId,
    mode: SizeMode,
    now: u64,
    kind: ReportKind,
    min_age_days: u32,
) -> ResultList {
    let age = AgeFilter::new(now, if kind.supports_age() { min_age_days } else { 0 });
    let (rows, truncated, note) = match kind {
        ReportKind::LargestFiles => largest_files(tree, base, mode, age),
        ReportKind::LargestDirs => largest_dirs(tree, base, mode, age),
        ReportKind::RepeatedNames => repeated_names(tree, base, mode, age),
        ReportKind::DevJunk => dev_junk(tree, base, mode, age),
        ReportKind::Caches => caches(tree, base, mode, age),
        ReportKind::OldBig => old_big(tree, base, mode, now),
        ReportKind::Downloads => downloads(tree, base, mode, age),
        ReportKind::Apps | ReportKind::Orphans | ReportKind::Duplicates => {
            unreachable!("{kind:?} is computed elsewhere")
        }
    };
    let mut title = kind.label().to_string();
    if age.min_days > 0 {
        title.push_str(&tf!(
            " · ≥ {} gündür dokunulmamış",
            " · untouched for ≥ {} days",
            age.min_days
        ));
    }
    let mut list = ResultList::new(title, base, rows);
    list.truncated = truncated;
    list.note = note.to_string();
    list.source = Source::Report {
        kind,
        min_age_days: age.min_days,
    };
    list
}

type Report = (Vec<Row>, bool, &'static str);

/// Depth-first walk below `base`. `visit` returns whether to descend into a
/// directory.
fn walk(tree: &Tree, base: NodeId, mut visit: impl FnMut(NodeId) -> bool) {
    let mut stack: Vec<NodeId> = tree.children(base).collect();
    while let Some(id) = stack.pop() {
        if visit(id) && tree.node(id).is_dir {
            stack.extend(tree.children(id));
        }
    }
}

/// Keeps the `LIMIT` largest `(size, id)` pairs; returns them largest first
/// and whether anything was left out.
#[derive(Default)]
struct Top {
    heap: BinaryHeap<Reverse<(u64, NodeId)>>,
    seen: usize,
}

impl Top {
    fn push(&mut self, size: u64, id: NodeId) {
        self.seen += 1;
        if self.heap.len() < LIMIT {
            self.heap.push(Reverse((size, id)));
        } else if self
            .heap
            .peek()
            .is_some_and(|Reverse(min)| (size, id) > *min)
        {
            self.heap.pop();
            self.heap.push(Reverse((size, id)));
        }
    }

    fn finish(self) -> (Vec<NodeId>, bool) {
        let truncated = self.seen > self.heap.len();
        let ids = self
            .heap
            .into_sorted_vec()
            .into_iter()
            .map(|Reverse((_, id))| id)
            .collect();
        (ids, truncated)
    }
}

fn singles(
    tree: &Tree,
    base: NodeId,
    mode: SizeMode,
    ids: impl IntoIterator<Item = (NodeId, String)>,
) -> Vec<Row> {
    ids.into_iter()
        .map(|(id, detail)| Row::single(tree, base, id, mode, detail))
        .collect()
}

/// Sorts `(id, detail)` pairs by size, largest first, and caps them.
fn by_size(
    tree: &Tree,
    mode: SizeMode,
    mut items: Vec<(NodeId, String)>,
) -> (Vec<(NodeId, String)>, bool) {
    items.sort_by_key(|(id, _)| Reverse(tree.node(*id).size.get(mode)));
    let truncated = items.len() > LIMIT;
    items.truncate(LIMIT);
    (items, truncated)
}

#[cfg(test)]
mod test_util {
    use super::*;
    use crate::tree::{Size, ROOT};

    pub(super) fn sz(n: u64) -> Size {
        Size {
            apparent: n,
            disk: n,
        }
    }

    /// Row labels with '/' separators on every platform.
    pub(super) fn labels(list: &ResultList) -> Vec<String> {
        list.rows
            .iter()
            .map(|r| r.label.replace('\\', "/"))
            .collect()
    }

    pub(super) fn report(t: &Tree, kind: ReportKind) -> ResultList {
        run(t, ROOT, SizeMode::Disk, 1000 * DAY, kind, 0)
    }
}
