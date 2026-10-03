//! Summary statistics for a directory, shown on the dashboard.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::tree::{NodeId, SizeMode, Tree};

pub const DAY: u64 = 24 * 60 * 60;
/// Upper bounds (inclusive) of the first three age groups; older entries
/// fall in the fourth.
pub const AGE_LIMITS: [u64; 3] = [7 * DAY, 30 * DAY, 365 * DAY];
/// Label of age group `i` (as returned by `age_group`, or `AGE_UNKNOWN`).
pub fn age_label(i: usize) -> &'static str {
    match i {
        0 => t!("≤ 7 gün", "≤ 7 days"),
        1 => t!("≤ 30 gün", "≤ 30 days"),
        2 => t!("≤ 1 yıl", "≤ 1 year"),
        3 => t!("> 1 yıl", "> 1 year"),
        _ => t!("bilinmiyor", "unknown"),
    }
}
/// Index of the age group for unknown dates.
pub const AGE_UNKNOWN: usize = 4;

/// Age group (0..=3) for something last touched `age` seconds ago.
pub fn age_group(age: u64) -> usize {
    AGE_LIMITS.iter().position(|&l| age <= l).unwrap_or(3)
}

const TOP: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Video,
    Audio,
    Image,
    Archive,
    Document,
    Code,
    Build,
    Other,
}

impl Category {
    pub const ALL: [Category; 8] = [
        Category::Video,
        Category::Audio,
        Category::Image,
        Category::Archive,
        Category::Document,
        Category::Code,
        Category::Build,
        Category::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Video => t!("Video", "Video"),
            Category::Audio => t!("Ses", "Audio"),
            Category::Image => t!("Resim", "Images"),
            Category::Archive => t!("Arşiv / disk imajı", "Archives / disk images"),
            Category::Document => t!("Belge", "Documents"),
            Category::Code => t!("Kod", "Code"),
            Category::Build => t!("Derleme çıktısı", "Build output"),
            Category::Other => t!("Diğer", "Other"),
        }
    }

    /// Category of a file, from its extension.
    pub fn of(name: &str) -> Self {
        // ".bashrc" has no extension; "a.tar.gz" has "gz".
        let Some((stem, ext)) = name.rsplit_once('.') else {
            return Category::Other;
        };
        if stem.is_empty() || ext.is_empty() || ext.len() > 8 {
            return Category::Other;
        }
        let mut buf = [0u8; 8];
        let lower = &mut buf[..ext.len()];
        lower.copy_from_slice(ext.as_bytes());
        lower.make_ascii_lowercase();
        let Ok(ext) = std::str::from_utf8(lower) else {
            return Category::Other;
        };
        match ext {
            "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" | "wmv" | "flv" | "mpg" | "mpeg"
            | "3gp" => Category::Video,
            "mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg" | "aiff" | "aif" | "wma" | "opus" => {
                Category::Audio
            }
            "jpg" | "jpeg" | "png" | "gif" | "heic" | "heif" | "webp" | "tif" | "tiff" | "bmp"
            | "raw" | "cr2" | "cr3" | "nef" | "arw" | "dng" | "svg" | "psd" | "ico" | "icns" => {
                Category::Image
            }
            "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "dmg" | "iso"
            | "img" | "pkg" | "xip" | "vmdk" | "qcow2" => Category::Archive,
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "key" | "pages"
            | "numbers" | "txt" | "md" | "rtf" | "csv" | "odt" | "ods" | "epub" => {
                Category::Document
            }
            "rs" | "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "py" | "java" | "kt" | "swift"
            | "c" | "h" | "cc" | "cpp" | "hpp" | "m" | "mm" | "cs" | "go" | "rb" | "php"
            | "html" | "css" | "scss" | "json" | "yaml" | "yml" | "toml" | "xml" | "sh" | "sql"
            | "lock" | "dart" | "vue" => Category::Code,
            "o" | "a" | "so" | "dylib" | "dll" | "exe" | "lib" | "rlib" | "rmeta" | "d" | "pdb"
            | "class" | "jar" | "wasm" | "pyc" | "dex" | "apk" | "aab" | "ipa" | "obj" | "bin"
            | "pch" | "incremental" => Category::Build,
            _ => Category::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bucket {
    pub size: u64,
    pub files: u64,
}

impl Bucket {
    fn add(&mut self, size: u64) {
        self.size += size;
        self.files += 1;
    }
}

#[derive(Debug)]
pub struct Stats {
    /// Total size of the directory (including directory blocks in disk mode).
    pub size: u64,
    pub files: u64,
    pub dirs: u64,
    /// Non-empty categories, largest first.
    pub categories: Vec<(Category, Bucket)>,
    /// Files by last modification; indexed like `age_label`.
    pub ages: [Bucket; 5],
    /// Largest files, largest first.
    pub top_files: Vec<NodeId>,
    /// Directories holding the most data directly (not counting
    /// subdirectories), with that size; largest first. Ranking by total size
    /// would just list chains of nested parents.
    pub top_dirs: Vec<(NodeId, u64)>,
}

/// Statistics for everything below `base`. `now` is in seconds since the
/// Unix epoch.
pub fn compute(tree: &Tree, base: NodeId, mode: SizeMode, now: u64) -> Stats {
    let mut categories = [Bucket::default(); Category::ALL.len()];
    let mut ages = [Bucket::default(); 5];
    let mut top_files: BinaryHeap<Reverse<(u64, NodeId)>> = BinaryHeap::new();
    let mut top_dirs: BinaryHeap<Reverse<(u64, NodeId)>> = BinaryHeap::new();
    let (mut files, mut dirs) = (0u64, 0u64);

    let mut stack = vec![base];
    while let Some(dir) = stack.pop() {
        let mut direct = 0u64;
        for id in tree.children(dir) {
            let n = tree.node(id);
            if n.is_dir {
                dirs += 1;
                stack.push(id);
                continue;
            }
            let size = n.size.get(mode);
            files += 1;
            direct += size;
            let cat = Category::of(tree.name(id));
            categories[cat as usize].add(size);
            let group = if n.modified == 0 {
                AGE_UNKNOWN
            } else {
                age_group(now.saturating_sub(u64::from(n.modified)))
            };
            ages[group].add(size);
            push_top(&mut top_files, (size, id));
        }
        if direct > 0 {
            push_top(&mut top_dirs, (direct, dir));
        }
    }

    let mut categories: Vec<(Category, Bucket)> = Category::ALL
        .into_iter()
        .zip(categories)
        .filter(|(_, b)| b.files > 0)
        .collect();
    categories.sort_by_key(|(_, b)| Reverse(b.size));

    Stats {
        size: tree.node(base).size.get(mode),
        files,
        dirs,
        categories,
        ages,
        top_files: into_sorted(top_files)
            .into_iter()
            .map(|(_, id)| id)
            .collect(),
        top_dirs: into_sorted(top_dirs)
            .into_iter()
            .map(|(size, id)| (id, size))
            .collect(),
    }
}

/// Keeps the `TOP` largest items in a min-heap.
fn push_top(heap: &mut BinaryHeap<Reverse<(u64, NodeId)>>, item: (u64, NodeId)) {
    if heap.len() < TOP {
        heap.push(Reverse(item));
    } else if heap.peek().is_some_and(|Reverse(min)| item > *min) {
        heap.pop();
        heap.push(Reverse(item));
    }
}

fn into_sorted(heap: BinaryHeap<Reverse<(u64, NodeId)>>) -> Vec<(u64, NodeId)> {
    // Ascending order of `Reverse` is descending order of the items.
    heap.into_sorted_vec()
        .into_iter()
        .map(|Reverse(x)| x)
        .collect()
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

    #[test]
    fn categorizes_by_extension() {
        assert_eq!(Category::of("film.MP4"), Category::Video);
        assert_eq!(Category::of("yedek.tar.gz"), Category::Archive);
        assert_eq!(Category::of("libfoo.rlib"), Category::Build);
        assert_eq!(Category::of("main.rs"), Category::Code);
        assert_eq!(Category::of("Rapor.PDF"), Category::Document);
        assert_eq!(Category::of(".bashrc"), Category::Other);
        assert_eq!(Category::of("Makefile"), Category::Other);
        assert_eq!(Category::of("trailing."), Category::Other);
        assert_eq!(Category::of("x.verylongextension"), Category::Other);
        assert_eq!(Category::of("fotoğraf.jpeg"), Category::Image);
    }

    #[test]
    fn age_groups() {
        assert_eq!(age_group(0), 0);
        assert_eq!(age_group(7 * DAY), 0);
        assert_eq!(age_group(7 * DAY + 1), 1);
        assert_eq!(age_group(30 * DAY), 1);
        assert_eq!(age_group(200 * DAY), 2);
        assert_eq!(age_group(366 * DAY), 3);
    }

    #[test]
    fn computes_subtree_stats() {
        let now = 1000 * DAY;
        let days_ago = |d: u64| (now - d * DAY) as u32;

        let mut t = Tree::new(Path::new("/r"));
        let outside = t.push(ROOT, "outside.mp4", false, sz(10_000));
        t.set_times(outside, days_ago(1), 0);
        let base = t.push(ROOT, "base", true, sz(0));
        let video = t.push(base, "a.mov", false, sz(500));
        t.set_times(video, days_ago(2), 0);
        let deep = t.push(base, "deep", true, sz(0));
        let deeper = t.push(deep, "deeper", true, sz(0));
        let big = t.push(deeper, "big.zip", false, sz(900));
        t.set_times(big, days_ago(400), 0);
        let code = t.push(deeper, "x.rs", false, sz(100));
        t.set_times(code, days_ago(20), 0);
        let unknown = t.push(deep, "notes", false, sz(50));
        t.finalize();

        let s = compute(&t, base, SizeMode::Disk, now);
        assert_eq!(s.size, 1550);
        assert_eq!((s.files, s.dirs), (4, 2));
        // Only the subtree counts: outside.mp4 is not a video here.
        assert_eq!(
            s.categories,
            vec![
                (
                    Category::Archive,
                    Bucket {
                        size: 900,
                        files: 1
                    }
                ),
                (
                    Category::Video,
                    Bucket {
                        size: 500,
                        files: 1
                    }
                ),
                (
                    Category::Code,
                    Bucket {
                        size: 100,
                        files: 1
                    }
                ),
                (Category::Other, Bucket { size: 50, files: 1 }),
            ]
        );
        assert_eq!(
            s.ages[0],
            Bucket {
                size: 500,
                files: 1
            }
        );
        assert_eq!(
            s.ages[1],
            Bucket {
                size: 100,
                files: 1
            }
        );
        assert_eq!(s.ages[2], Bucket::default());
        assert_eq!(
            s.ages[3],
            Bucket {
                size: 900,
                files: 1
            }
        );
        assert_eq!(s.ages[AGE_UNKNOWN], Bucket { size: 50, files: 1 });
        assert_eq!(s.top_files, vec![big, video, code, unknown]);
        // By direct content: deeper (1000) > base (500) > deep (50).
        assert_eq!(s.top_dirs, vec![(deeper, 1000), (base, 500), (deep, 50)]);
    }

    #[test]
    fn keeps_only_the_largest() {
        let mut t = Tree::new(Path::new("/r"));
        let ids: Vec<NodeId> = (1..=25)
            .map(|i| t.push(ROOT, &format!("f{i}"), false, sz(i)))
            .collect();
        t.finalize();

        let s = compute(&t, ROOT, SizeMode::Apparent, 0);
        assert_eq!(s.top_files.len(), TOP);
        assert_eq!(s.top_files[0], ids[24]);
        assert_eq!(s.top_files[9], ids[15]);
    }
}
