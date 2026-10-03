//! Predefined reports over the directory being browsed.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::hash::{BuildHasherDefault, Hasher};

use xxhash_rust::xxh3::xxh3_64;

use crate::lists::{ResultList, Row, RowSize};
use crate::stats::DAY;
use crate::tree::{NodeId, SizeMode, Tree};

/// Maximum number of rows a report shows.
pub const LIMIT: usize = 200;
const MIB: u64 = 1024 * 1024;
pub const OLD_BIG_SIZE: u64 = 100 * MIB;
pub const OLD_BIG_AGE: u64 = 365 * DAY;
/// A directory is skipped in "largest folders" when one subdirectory holds
/// more than this share of it: it only wraps that subdirectory.
const WRAPPER_SHARE: f64 = 0.9;

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
    Duplicates,
}

impl ReportKind {
    pub fn label(self) -> &'static str {
        match self {
            ReportKind::LargestFiles => "En büyük dosyalar",
            ReportKind::LargestDirs => "En büyük klasörler",
            ReportKind::RepeatedNames => "En çok tekrar eden dosya adları",
            ReportKind::Apps => "Uygulamalar ve verileri",
            ReportKind::Orphans => "Sahipsiz uygulama artıkları",
            ReportKind::DevJunk => "Geliştirici çöpleri",
            ReportKind::Caches => "Önbellek klasörleri",
            ReportKind::OldBig => "Eski ve büyük dosyalar",
            ReportKind::Duplicates => "Kopya dosyalar (içeriği aynı)",
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
        )
    }

    pub fn description(self) -> &'static str {
        match self {
            ReportKind::LargestFiles => "Bu klasörün altındaki en büyük 200 dosya",
            ReportKind::LargestDirs => {
                "Toplam boyuta göre; yalnızca tek bir alt klasörü saran klasörler elenir"
            }
            ReportKind::RepeatedNames => "Aynı adı taşıyan dosyalar, adet sırasıyla",
            ReportKind::Apps => "Uygulama paketi + Library'deki verileri (önbellek, destek…)",
            ReportKind::Orphans => {
                "Silinmiş uygulamalardan kalan veriler (Library: destek, önbellek, kapsayıcılar)"
            }
            ReportKind::DevJunk => "node_modules, target, build, Pods, DerivedData, .venv…",
            ReportKind::Caches => "Caches ve .cache içindeki uygulama önbellekleri",
            ReportKind::OldBig => "100 MiB'tan büyük, 1 yıldır değişmemiş dosyalar",
            ReportKind::Duplicates => {
                "İçeriği birebir aynı dosyalar (≥ 1 MiB); dosyalar okunur, sürebilir"
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
    pub const ALL: [MenuItem; 13] = [
        MenuItem::Report(ReportKind::LargestFiles),
        MenuItem::Report(ReportKind::LargestDirs),
        MenuItem::Report(ReportKind::RepeatedNames),
        MenuItem::Report(ReportKind::Apps),
        MenuItem::Report(ReportKind::Orphans),
        MenuItem::Report(ReportKind::DevJunk),
        MenuItem::Report(ReportKind::Caches),
        MenuItem::Report(ReportKind::OldBig),
        MenuItem::Report(ReportKind::Duplicates),
        MenuItem::Changes,
        MenuItem::Leftovers,
        MenuItem::Tools,
        MenuItem::System,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MenuItem::Report(k) => k.label(),
            MenuItem::Changes => "Son taramadan bu yana değişenler",
            MenuItem::Leftovers => "Silinmiş uygulama artıkları",
            MenuItem::Tools => "Geliştirici araçları temizliği",
            MenuItem::System => "Sistem verileri",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            MenuItem::Report(k) => k.description(),
            MenuItem::Changes => {
                "Kayıtlı bir taramayla karşılaştırır: büyüyen, yeni ve silinen klasörler"
            }
            MenuItem::Leftovers => {
                "Kaldırılmış uygulamalardan kalan veriler; ev klasörü taraması yeterli"
            }
            MenuItem::Tools => {
                "Docker, Xcode, npm, Gradle… önbelleklerini aracın kendi komutuyla temizler"
            }
            MenuItem::System => "APFS bölümleri, Time Machine anlık görüntüleri, takas dosyası",
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
        ReportKind::Apps | ReportKind::Orphans | ReportKind::Duplicates => {
            unreachable!("{kind:?} is computed elsewhere")
        }
    };
    let mut title = kind.label().to_string();
    if age.min_days > 0 {
        title.push_str(&format!(" · ≥ {} gündür dokunulmamış", age.min_days));
    }
    let mut list = ResultList::new(title, base, rows);
    list.truncated = truncated;
    list.note = note.to_string();
    list.report = Some(kind);
    list.min_age_days = age.min_days;
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

fn largest_files(tree: &Tree, base: NodeId, mode: SizeMode, age: AgeFilter) -> Report {
    let mut top = Top::default();
    walk(tree, base, |id| {
        let n = tree.node(id);
        if !n.is_dir && age.ok(n.modified) {
            top.push(n.size.get(mode), id);
        }
        true
    });
    let (ids, truncated) = top.finish();
    let rows = singles(
        tree,
        base,
        mode,
        ids.into_iter().map(|id| (id, String::new())),
    );
    (rows, truncated, "Enter: dosyanın bulunduğu klasörü aç")
}

fn largest_dirs(tree: &Tree, base: NodeId, mode: SizeMode, age: AgeFilter) -> Report {
    let mut top = Top::default();
    walk(tree, base, |id| {
        let n = tree.node(id);
        if !n.is_dir {
            return false;
        }
        let total = n.size.get(mode);
        let biggest_child = tree
            .children(id)
            .filter(|&c| tree.node(c).is_dir)
            .map(|c| tree.node(c).size.get(mode))
            .max()
            .unwrap_or(0);
        let wrapper = total > 0 && biggest_child as f64 > total as f64 * WRAPPER_SHARE;
        if total > 0 && !wrapper && age.ok(n.modified) {
            top.push(total, id);
        }
        true
    });
    let (ids, truncated) = top.finish();
    let rows = singles(
        tree,
        base,
        mode,
        ids.into_iter().map(|id| (id, String::new())),
    );
    let note = "İç içe klasörler ayrıca listelenebilir; boyutları birbirini içerir.";
    (rows, truncated, note)
}

fn old_big(tree: &Tree, base: NodeId, mode: SizeMode, now: u64) -> Report {
    let mut found = Vec::new();
    walk(tree, base, |id| {
        let n = tree.node(id);
        let old = n.modified != 0 && now.saturating_sub(u64::from(n.modified)) > OLD_BIG_AGE;
        if !n.is_dir && old && n.size.get(mode) >= OLD_BIG_SIZE {
            found.push((id, String::new()));
        }
        true
    });
    let (items, truncated) = by_size(tree, mode, found);
    let rows = singles(tree, base, mode, items);
    (
        rows,
        truncated,
        "≥ 100 MiB ve 1 yıldan uzun süredir değişmemiş dosyalar.",
    )
}

/// What kind of regenerable directory `id` is, if any.
fn dev_junk_kind(tree: &Tree, id: NodeId) -> Option<&'static str> {
    let name = tree.name(id);
    let parent = tree.parent(id)?;
    let sibling = |pred: &dyn Fn(&str) -> bool| tree.children(parent).any(|c| pred(tree.name(c)));
    let has = |file: &str| sibling(&|n| n == file);
    let kind = match name {
        "node_modules" => "Node.js bağımlılıkları",
        "__pycache__" => "Python önbelleği",
        ".gradle" => "Gradle önbelleği",
        "DerivedData" => "Xcode derleme verisi",
        ".next" | ".nuxt" | ".turbo" | ".parcel-cache" | ".angular" => "web derleme önbelleği",
        ".dart_tool" => "Dart/Flutter araçları",
        "target" if has("Cargo.toml") => "Rust derleme çıktısı",
        "build"
            if has("build.gradle")
                || has("build.gradle.kts")
                || has("CMakeLists.txt")
                || has("package.json")
                || has("setup.py") =>
        {
            "derleme çıktısı"
        }
        "dist" if has("package.json") || has("setup.py") => "paket çıktısı",
        "Pods" if has("Podfile") => "CocoaPods bağımlılıkları",
        ".build" if has("Package.swift") => "Swift derleme çıktısı",
        ".venv" | "venv" if tree.children(id).any(|c| tree.name(c) == "pyvenv.cfg") => {
            "Python sanal ortamı"
        }
        "vendor" if has("composer.json") => "PHP bağımlılıkları",
        "bin" | "obj" if sibling(&|n| n.ends_with(".csproj")) => ".NET derleme çıktısı",
        _ => return None,
    };
    Some(kind)
}

/// When the project around a junk folder was last touched: the newest
/// change among its other entries. Reinstalling dependencies refreshes
/// `node_modules` but says nothing about whether the project is in use.
fn project_modified(tree: &Tree, junk: NodeId) -> u32 {
    tree.parent(junk)
        .map(|p| {
            tree.children(p)
                .filter(|&c| c != junk)
                .map(|c| tree.node(c).modified)
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0)
}

fn dev_junk(tree: &Tree, base: NodeId, mode: SizeMode, age: AgeFilter) -> Report {
    let mut found = Vec::new();
    walk(tree, base, |id| {
        if !tree.node(id).is_dir {
            return false;
        }
        match dev_junk_kind(tree, id) {
            Some(kind) => {
                let project = project_modified(tree, id);
                if age.ok(project) {
                    let detail = if project == 0 {
                        kind.to_string()
                    } else {
                        format!("{kind} · proje: {}", crate::ui::fmt_date(project))
                    };
                    found.push((id, detail));
                }
                false
            }
            None => true,
        }
    });
    let (items, truncated) = by_size(tree, mode, found);
    let rows = singles(tree, base, mode, items);
    (
        rows,
        truncated,
        "Bu klasörler projeyi yeniden derleyince / kurunca geri gelir.",
    )
}

fn caches(tree: &Tree, base: NodeId, mode: SizeMode, age: AgeFilter) -> Report {
    const CONTAINERS: [&str; 2] = ["caches", ".cache"];
    const SELF: [&str; 6] = [
        "cache",
        "code cache",
        "gpucache",
        "cacheddata",
        "shadercache",
        "dawncache",
    ];
    let mut found = Vec::new();
    let mut visit_dir = |id: NodeId| -> bool {
        if !tree.node(id).is_dir {
            return false;
        }
        let name = tree.name(id).to_lowercase();
        if CONTAINERS.contains(&name.as_str()) {
            found.extend(
                tree.children(id)
                    .map(|c| (c, format!("{} içinde", tree.name(id)))),
            );
            false
        } else if SELF.contains(&name.as_str()) {
            found.push((id, "önbellek".to_string()));
            false
        } else {
            true
        }
    };
    // The browsed directory itself may be a cache container.
    if visit_dir(base) {
        walk(tree, base, &mut visit_dir);
    }
    found.retain(|(id, _)| tree.node(*id).size.get(mode) > 0 && age.ok(tree.node(*id).modified));
    let (items, truncated) = by_size(tree, mode, found);
    let rows = singles(tree, base, mode, items);
    (
        rows,
        truncated,
        "Önbellekler silinince uygulamalar gerektiğinde yeniden oluşturur.",
    )
}

/// Case-insensitive hash of a file name; ASCII names (nearly all) are
/// lowercased on the stack.
fn name_hash(name: &str) -> u64 {
    let mut buf = [0u8; 255];
    if name.is_ascii() && name.len() <= buf.len() {
        let lower = &mut buf[..name.len()];
        lower.copy_from_slice(name.as_bytes());
        lower.make_ascii_lowercase();
        xxh3_64(lower)
    } else {
        xxh3_64(name.to_lowercase().as_bytes())
    }
}

/// The keys are already hashes; hashing them again would only cost time.
#[derive(Default)]
struct IdentityHasher(u64);

impl Hasher for IdentityHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 << 8) | u64::from(b);
        }
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}

type HashKeyed<V> = HashMap<u64, V, BuildHasherDefault<IdentityHasher>>;

fn repeated_names(tree: &Tree, base: NodeId, mode: SizeMode, age: AgeFilter) -> Report {
    // Pass 1: count per name (by hash, to keep memory small on big disks).
    let mut counts: HashKeyed<(u32, u64)> = HashKeyed::default();
    walk(tree, base, |id| {
        let n = tree.node(id);
        if !n.is_dir && age.ok(n.modified) {
            let e = counts.entry(name_hash(tree.name(id))).or_default();
            e.0 += 1;
            e.1 += n.size.get(mode);
        }
        true
    });
    let mut repeated: Vec<(u64, u32, u64)> = counts
        .into_iter()
        .filter(|(_, (count, _))| *count >= 2)
        .map(|(h, (count, size))| (h, count, size))
        .collect();
    repeated.sort_by_key(|&(h, count, size)| (Reverse(count), Reverse(size), h));
    let truncated = repeated.len() > LIMIT;
    repeated.truncate(LIMIT);
    let order: HashKeyed<usize> = repeated
        .iter()
        .enumerate()
        .map(|(i, &(h, ..))| (h, i))
        .collect();

    // Pass 2: collect the members of the shown names.
    let mut members: Vec<Vec<(NodeId, u64)>> = vec![Vec::new(); repeated.len()];
    walk(tree, base, |id| {
        let n = tree.node(id);
        if !n.is_dir && age.ok(n.modified) {
            if let Some(&i) = order.get(&name_hash(tree.name(id))) {
                members[i].push((id, n.size.get(mode)));
            }
        }
        true
    });
    let rows = members
        .into_iter()
        .filter(|m| m.len() >= 2)
        .map(|mut m| {
            m.sort_by_key(|&(id, size)| (Reverse(size), id));
            let label = tree.name(m[0].0).to_string();
            Row::group(label, String::new(), m, RowSize::Sum, 2)
        })
        .collect();
    (rows, truncated, "Enter: bu adı taşıyan dosyaları listele")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{Size, ROOT};
    use std::path::Path;

    fn sz(n: u64) -> Size {
        Size {
            apparent: n,
            disk: n,
        }
    }

    fn labels(list: &ResultList) -> Vec<&str> {
        list.rows.iter().map(|r| r.label.as_str()).collect()
    }

    fn report(t: &Tree, kind: ReportKind) -> ResultList {
        run(t, ROOT, SizeMode::Disk, 1000 * DAY, kind, 0)
    }

    #[test]
    fn largest_files_in_order() {
        let mut t = Tree::new(Path::new("/r"));
        let d = t.push(ROOT, "d", true, sz(0));
        t.push(d, "small", false, sz(1));
        t.push(d, "big", false, sz(30));
        t.push(ROOT, "mid", false, sz(20));
        t.finalize();
        let l = report(&t, ReportKind::LargestFiles);
        assert_eq!(labels(&l), vec!["d/big", "mid", "d/small"]);
        assert!(!l.truncated);
    }

    #[test]
    fn largest_dirs_skip_wrappers() {
        let mut t = Tree::new(Path::new("/r"));
        // lib -> dev -> sim(1000): lib and dev only wrap sim.
        let lib = t.push(ROOT, "lib", true, sz(0));
        let dev = t.push(lib, "dev", true, sz(0));
        let sim = t.push(dev, "sim", true, sz(0));
        t.push(sim, "img", false, sz(1000));
        t.push(lib, "pref", false, sz(5));
        // proj holds two comparable subdirectories: a real aggregate.
        let proj = t.push(ROOT, "proj", true, sz(0));
        let a = t.push(proj, "a", true, sz(0));
        t.push(a, "f", false, sz(300));
        let b = t.push(proj, "b", true, sz(0));
        t.push(b, "f", false, sz(200));
        t.finalize();
        let l = report(&t, ReportKind::LargestDirs);
        assert_eq!(
            labels(&l),
            vec!["lib/dev/sim/", "proj/", "proj/a/", "proj/b/"]
        );
    }

    #[test]
    fn old_big_thresholds() {
        let now = 1000 * DAY;
        let mut t = Tree::new(Path::new("/r"));
        let old_big = t.push(ROOT, "old_big", false, sz(OLD_BIG_SIZE));
        t.set_times(old_big, (now - 400 * DAY) as u32, 0);
        let new_big = t.push(ROOT, "new_big", false, sz(OLD_BIG_SIZE * 2));
        t.set_times(new_big, (now - 10 * DAY) as u32, 0);
        let old_small = t.push(ROOT, "old_small", false, sz(OLD_BIG_SIZE - 1));
        t.set_times(old_small, (now - 400 * DAY) as u32, 0);
        t.push(ROOT, "unknown_date", false, sz(OLD_BIG_SIZE * 3));
        t.finalize();
        let l = run(&t, ROOT, SizeMode::Disk, now, ReportKind::OldBig, 0);
        assert_eq!(labels(&l), vec!["old_big"]);
    }

    #[test]
    fn dev_junk_needs_marker_files() {
        let mut t = Tree::new(Path::new("/r"));
        let rust = t.push(ROOT, "rust", true, sz(0));
        t.push(rust, "Cargo.toml", false, sz(1));
        let target = t.push(rust, "target", true, sz(0));
        t.push(target, "debug.bin", false, sz(500));
        // A "target" folder without Cargo.toml is not build output.
        let other = t.push(ROOT, "photos", true, sz(0));
        let not_junk = t.push(other, "target", true, sz(0));
        t.push(not_junk, "x.jpg", false, sz(900));
        let web = t.push(ROOT, "web", true, sz(0));
        let nm = t.push(web, "node_modules", true, sz(0));
        let nested = t.push(nm, "node_modules", true, sz(0));
        t.push(nested, "lib.js", false, sz(200));
        let py = t.push(ROOT, "py", true, sz(0));
        let venv = t.push(py, ".venv", true, sz(0));
        t.push(venv, "pyvenv.cfg", false, sz(1));
        let dotnet = t.push(ROOT, "app", true, sz(0));
        t.push(dotnet, "App.csproj", false, sz(1));
        let bin = t.push(dotnet, "bin", true, sz(0));
        t.push(bin, "app.dll", false, sz(50));
        t.finalize();

        let l = report(&t, ReportKind::DevJunk);
        assert_eq!(
            labels(&l),
            vec!["rust/target/", "web/node_modules/", "app/bin/", "py/.venv/"]
        );
        assert!(l.rows[0].detail.starts_with("Rust derleme çıktısı"));
    }

    #[test]
    fn caches_list_per_app_entries() {
        let mut t = Tree::new(Path::new("/r"));
        let lib = t.push(ROOT, "Library", true, sz(0));
        let caches = t.push(lib, "Caches", true, sz(0));
        let a = t.push(caches, "com.a", true, sz(0));
        t.push(a, "blob", false, sz(70));
        let b = t.push(caches, "com.b", true, sz(0));
        t.push(b, "blob", false, sz(30));
        let app = t.push(ROOT, "Chrome", true, sz(0));
        let gpu = t.push(app, "GPUCache", true, sz(0));
        t.push(gpu, "data", false, sz(40));
        t.finalize();

        let l = report(&t, ReportKind::Caches);
        assert_eq!(
            labels(&l),
            vec![
                "Library/Caches/com.a/",
                "Chrome/GPUCache/",
                "Library/Caches/com.b/"
            ]
        );

        // Opened directly on a cache container, its entries are listed.
        let l = run(&t, caches, SizeMode::Disk, 0, ReportKind::Caches, 0);
        assert_eq!(labels(&l), vec!["com.a/", "com.b/"]);
    }

    #[test]
    fn age_filter_uses_project_age_for_dev_junk() {
        let now = 1000 * DAY;
        let ago = |d: u64| (now - d * DAY) as u32;
        let mut t = Tree::new(Path::new("/r"));
        // Old project whose node_modules was reinstalled yesterday.
        let old = t.push(ROOT, "eski", true, sz(0));
        let pkg = t.push(old, "package.json", false, sz(1));
        t.set_times(pkg, ago(400), 0);
        let nm = t.push(old, "node_modules", true, sz(0));
        let lib = t.push(nm, "lib.js", false, sz(500));
        t.set_times(lib, ago(1), 0);
        // Project in active use.
        let new = t.push(ROOT, "yeni", true, sz(0));
        let pkg = t.push(new, "package.json", false, sz(1));
        t.set_times(pkg, ago(3), 0);
        let nm = t.push(new, "node_modules", true, sz(0));
        let lib = t.push(nm, "lib.js", false, sz(900));
        t.set_times(lib, ago(3), 0);
        t.finalize();

        let all = run(&t, ROOT, SizeMode::Disk, now, ReportKind::DevJunk, 0);
        assert_eq!(
            labels(&all),
            vec!["yeni/node_modules/", "eski/node_modules/"]
        );
        let stale = run(&t, ROOT, SizeMode::Disk, now, ReportKind::DevJunk, 90);
        assert_eq!(labels(&stale), vec!["eski/node_modules/"]);
        assert!(stale.title.contains("≥ 90 gün"));
        assert_eq!(stale.min_age_days, 90);

        // Files: the date of the file itself.
        let files = run(&t, ROOT, SizeMode::Disk, now, ReportKind::LargestFiles, 180);
        assert_eq!(labels(&files), vec!["eski/package.json"]);
        // Not supported: the filter is ignored.
        let old_big = run(&t, ROOT, SizeMode::Disk, now, ReportKind::OldBig, 90);
        assert_eq!(old_big.min_age_days, 0);
    }

    #[test]
    fn repeated_names_group_case_insensitively() {
        let mut t = Tree::new(Path::new("/r"));
        let a = t.push(ROOT, "a", true, sz(0));
        let b = t.push(ROOT, "b", true, sz(0));
        let c = t.push(ROOT, "c", true, sz(0));
        for dir in [a, b, c] {
            t.push(dir, "package.json", false, sz(10));
        }
        t.push(a, "README.md", false, sz(5));
        t.push(b, "readme.MD", false, sz(7));
        t.push(c, "unique.txt", false, sz(100));
        t.finalize();

        let l = report(&t, ReportKind::RepeatedNames);
        assert_eq!(labels(&l), vec!["package.json", "readme.MD"]);
        assert!(l.rows.iter().all(|r| r.group));
        assert_eq!(l.rows[0].nodes.len(), 3);
        assert_eq!(l.rows[0].size(), 30);
        assert_eq!(l.rows[1].size(), 12);
    }
}
