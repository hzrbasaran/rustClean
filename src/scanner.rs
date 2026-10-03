//! Parallel directory scanning.
//!
//! Directories are read and their entries stat'ed on a thread pool; a single
//! thread assembles the `Tree` from the results.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::clones::{self, CloneInfo};
use crate::tree::{NodeId, Size, Tree, ROOT};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Default)]
pub struct ScanProgress {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub errors: u64,
    pub current: PathBuf,
}

#[derive(Debug)]
pub struct ScanResult {
    pub tree: Tree,
    /// Entries that could not be read (permissions, vanished files, ...).
    pub errors: u64,
    pub elapsed: Duration,
}

pub enum ScanMsg {
    Progress(ScanProgress),
    Done(ScanResult),
    Failed(String),
}

/// A scan running on a background thread. Dropping the handle cancels it.
pub struct ScanHandle {
    pub rx: Receiver<ScanMsg>,
    pub started: Instant,
    cancel: Arc<AtomicBool>,
}

impl Drop for ScanHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub fn start(root: PathBuf, skip: Vec<PathBuf>) -> ScanHandle {
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_worker = Arc::clone(&cancel);
    thread::spawn(move || {
        let progress_tx = tx.clone();
        let res = scan(&root, skip, &cancel_worker, |p| {
            let _ = progress_tx.send(ScanMsg::Progress(p.clone()));
        });
        if cancel_worker.load(Ordering::Relaxed) {
            return;
        }
        let _ = tx.send(match res {
            Ok(r) => ScanMsg::Done(r),
            Err(e) => ScanMsg::Failed(format!("{e:#}")),
        });
    });
    ScanHandle {
        rx,
        started: Instant::now(),
        cancel,
    }
}

/// Settings shared by all worker threads of one scan.
struct Ctx {
    cancel: Arc<AtomicBool>,
    skip: HashSet<PathBuf>,
    #[cfg(unix)]
    root_dev: u64,
}

/// A directory waiting to be read; `node` is its id in the tree.
struct Job {
    dir: PathBuf,
    node: NodeId,
}

/// The contents of one directory, sent from a worker to the tree builder.
struct Listing {
    dir: PathBuf,
    node: NodeId,
    entries: Vec<Entry>,
    /// Entries (or the directory itself) that could not be read.
    errors: u64,
}

struct Entry {
    name: Box<str>,
    is_dir: bool,
    size: Size,
    modified: u32,
    created: u32,
    /// `(dev, inode)` of files with more than one hard link.
    #[cfg(unix)]
    hard_link: Option<(u64, u64)>,
    /// Set for APFS clones (files sharing blocks with another file).
    clone: Option<CloneInfo>,
    /// Set for directories that should be descended into.
    descend: Option<PathBuf>,
}

/// Scans `root` synchronously. Symlinks are not followed, other filesystems
/// (and any path in `skip`) are not descended into, and hard-linked files are
/// counted once.
///
/// Worker threads read directories in no particular order; this thread owns
/// the tree and hands out a new job for every subdirectory it inserts, so a
/// parent is always in the tree before its children.
pub fn scan(
    root: &Path,
    skip: Vec<PathBuf>,
    cancel: &Arc<AtomicBool>,
    mut on_progress: impl FnMut(&ScanProgress),
) -> Result<ScanResult> {
    let started = Instant::now();
    let root = std::path::absolute(root).context(t!("geçersiz yol", "invalid path"))?;
    let root_md = fs::metadata(&root)
        .with_context(|| tf!("{} okunamadı", "could not read {}", root.display()))?;
    anyhow::ensure!(
        root_md.is_dir(),
        "{}",
        tf!("{} bir klasör değil", "{} is not a folder", root.display())
    );

    let ctx = Arc::new(Ctx {
        cancel: Arc::clone(cancel),
        skip: skip.into_iter().filter(|p| *p != root).collect(),
        #[cfg(unix)]
        root_dev: std::os::unix::fs::MetadataExt::dev(&root_md),
    });
    // Scanning is dominated by I/O latency, so use more threads than cores.
    let threads = thread::available_parallelism().map_or(4, |n| n.get() * 2);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("scan-{i}"))
        .build()
        .context(t!(
            "iş parçacığı havuzu oluşturulamadı",
            "could not create the thread pool"
        ))?;
    let (tx, rx) = mpsc::channel::<Listing>();
    let spawn = |job: Job| {
        let ctx = Arc::clone(&ctx);
        let tx = tx.clone();
        pool.spawn(move || {
            let _ = tx.send(read_dir(job, &ctx));
        });
    };

    let mut tree = Tree::new(&root);
    #[cfg(unix)]
    let mut seen_links: HashSet<(u64, u64)> = HashSet::new();
    let mut seen_clones: HashSet<u64> = HashSet::new();
    let mut progress = ScanProgress::default();
    let mut last_report = Instant::now();

    spawn(Job {
        dir: root.clone(),
        node: ROOT,
    });
    let mut pending = 1usize;
    while pending > 0 && !cancel.load(Ordering::Relaxed) {
        let listing = match rx.recv_timeout(PROGRESS_INTERVAL) {
            Ok(l) => l,
            Err(RecvTimeoutError::Timeout) => {
                on_progress(&progress);
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        pending -= 1;
        progress.errors += listing.errors;

        for entry in listing.entries {
            #[allow(unused_mut)]
            let mut size = entry.size;
            #[cfg(unix)]
            if entry.hard_link.is_some_and(|key| !seen_links.insert(key)) {
                size = Size::default();
            }
            // Clones share blocks: the first one seen carries them, the
            // others only what they hold on their own.
            if let Some(c) = entry.clone {
                if !seen_clones.insert(c.id) {
                    size.disk = size.disk.min(c.private);
                }
            }
            let id = tree.push(listing.node, &entry.name, entry.is_dir, size);
            if let Some(c) = entry.clone {
                tree.set_clone(id, c.id, c.private);
            }
            tree.set_times(id, entry.modified, entry.created);
            if entry.is_dir {
                progress.dirs += 1;
            } else {
                progress.files += 1;
                progress.bytes += size.apparent;
            }
            if let Some(dir) = entry.descend {
                spawn(Job { dir, node: id });
                pending += 1;
            }
        }

        if last_report.elapsed() >= PROGRESS_INTERVAL {
            progress.current = listing.dir;
            on_progress(&progress);
            last_report = Instant::now();
        }
    }

    tree.finalize();
    Ok(ScanResult {
        tree,
        errors: progress.errors,
        elapsed: started.elapsed(),
    })
}

/// Reads one directory and stats its entries. Runs on a worker thread.
fn read_dir(job: Job, ctx: &Ctx) -> Listing {
    let mut listing = Listing {
        dir: job.dir,
        node: job.node,
        entries: Vec::new(),
        errors: 0,
    };
    if ctx.cancel.load(Ordering::Relaxed) {
        return listing;
    }
    let read = match fs::read_dir(&listing.dir) {
        Ok(r) => r,
        Err(_) => {
            listing.errors = 1;
            return listing;
        }
    };
    // Opened on the first file: clone lookups go through it.
    let mut clone_dir: Option<Option<clones::Dir>> = None;
    for dir_entry in read {
        // `DirEntry::metadata` does not follow symlinks.
        let Ok((dir_entry, md)) = dir_entry.and_then(|e| e.metadata().map(|md| (e, md))) else {
            listing.errors += 1;
            continue;
        };
        let is_dir = md.is_dir();
        #[allow(unused_mut)]
        let mut size = Size {
            apparent: md.len(),
            disk: md.len(),
        };
        #[cfg(unix)]
        let hard_link = {
            use std::os::unix::fs::MetadataExt;
            // st_blocks is always in 512-byte units.
            size.disk = md.blocks() * 512;
            (!is_dir && md.nlink() > 1).then(|| (md.dev(), md.ino()))
        };
        // A directory's own length is filesystem bookkeeping, so only its
        // allocated blocks count (as with `du`).
        if is_dir {
            size.apparent = 0;
        }

        let mut descend = None;
        if is_dir {
            #[cfg(unix)]
            let other_fs = std::os::unix::fs::MetadataExt::dev(&md) != ctx.root_dev;
            #[cfg(not(unix))]
            let other_fs = false;
            let path = dir_entry.path();
            if !other_fs && !ctx.skip.contains(&path) {
                descend = Some(path);
            }
        }

        let clone = if md.is_file() && size.disk >= clones::MIN_SIZE {
            clone_dir
                .get_or_insert_with(|| clones::Dir::open(&listing.dir))
                .as_ref()
                .and_then(|d| d.info(&dir_entry.file_name()))
                .filter(CloneInfo::is_shared)
        } else {
            None
        };

        listing.entries.push(Entry {
            name: dir_entry.file_name().to_string_lossy().into(),
            is_dir,
            size,
            clone,
            modified: epoch_secs(md.modified()),
            created: epoch_secs(md.created()),
            #[cfg(unix)]
            hard_link,
            descend,
        });
    }
    listing
}

/// Seconds since the Unix epoch, or 0 when unknown (or before 1970).
fn epoch_secs(time: std::io::Result<std::time::SystemTime>) -> u32 {
    time.ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs().min(u64::from(u32::MAX)) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::tree::SizeMode;

    fn write(path: &Path, len: usize) {
        fs::write(path, vec![0u8; len]).unwrap();
    }

    fn run(root: &Path) -> ScanResult {
        scan(root, Vec::new(), &Arc::default(), |_| {}).unwrap()
    }

    fn child(tree: &Tree, parent: NodeId, name: &str) -> NodeId {
        tree.children(parent)
            .find(|&c| tree.name(c) == name)
            .unwrap_or_else(|| panic!("{name} not found"))
    }

    #[test]
    fn sums_nested_sizes() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join("a/b")).unwrap();
        write(&r.join("top.bin"), 100);
        write(&r.join("a/one.bin"), 1000);
        write(&r.join("a/b/two.bin"), 2000);
        fs::create_dir(r.join("empty")).unwrap();

        let res = run(r);
        let t = &res.tree;
        assert_eq!(res.errors, 0);
        assert_eq!(t.node(ROOT).size.apparent, 3100);
        assert_eq!(t.node(ROOT).file_count, 3);
        let a = child(t, ROOT, "a");
        assert_eq!(t.node(a).size.apparent, 3000);
        assert_eq!(t.node(child(t, a, "b")).size.apparent, 2000);
        assert_eq!(t.node(child(t, ROOT, "empty")).size.apparent, 0);
        assert_eq!(
            t.path_of(child(t, a, "b")),
            std::path::absolute(r.join("a/b")).unwrap()
        );
    }

    #[test]
    fn skips_listed_paths() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir(r.join("mnt")).unwrap();
        write(&r.join("mnt/big.bin"), 5000);
        write(&r.join("keep.bin"), 10);

        let skip = vec![std::path::absolute(r.join("mnt")).unwrap()];
        let res = scan(r, skip, &Arc::default(), |_| {}).unwrap();
        assert_eq!(res.tree.node(ROOT).size.apparent, 10);
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir(r.join("real")).unwrap();
        write(&r.join("real/data.bin"), 4000);
        std::os::unix::fs::symlink(r.join("real"), r.join("link")).unwrap();

        let res = run(r);
        let t = &res.tree;
        let link = child(t, ROOT, "link");
        assert!(!t.node(link).is_dir);
        assert!(t.node(link).size.apparent < 4000);
        assert_eq!(t.node(child(t, ROOT, "real")).size.apparent, 4000);
    }

    #[cfg(unix)]
    #[test]
    fn counts_hardlinks_once() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(&r.join("orig.bin"), 3000);
        fs::hard_link(r.join("orig.bin"), r.join("copy.bin")).unwrap();

        let res = run(r);
        assert_eq!(res.tree.node(ROOT).size.apparent, 3000);
    }

    #[cfg(unix)]
    #[test]
    fn tracks_disk_usage_separately() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(&r.join("tiny.bin"), 1);
        // A sparse file: large length, almost nothing allocated.
        fs::File::create(r.join("sparse.bin"))
            .unwrap()
            .set_len(64 * 1024 * 1024)
            .unwrap();

        let res = run(r);
        let t = &res.tree;
        let tiny = t.node(child(t, ROOT, "tiny.bin")).size;
        assert_eq!(tiny.apparent, 1);
        assert!(tiny.get(SizeMode::Disk) >= 512);
        let sparse = t.node(child(t, ROOT, "sparse.bin")).size;
        assert_eq!(sparse.apparent, 64 * 1024 * 1024);
        assert!(sparse.disk < 1024 * 1024);
    }

    #[test]
    fn records_times() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(&r.join("f.bin"), 1);
        write(&r.join("fresh.bin"), 1);
        let old = std::time::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        fs::File::options()
            .write(true)
            .open(r.join("f.bin"))
            .unwrap()
            .set_modified(old)
            .unwrap();

        let res = run(r);
        let t = &res.tree;
        let f = t.node(child(t, ROOT, "f.bin"));
        assert_eq!(f.modified, 1_000_000_000);
        // Creation time is available on macOS, Windows and Linux with statx.
        // (macOS moves it back when mtime is set earlier, so check a file
        // that was left alone.)
        let now = epoch_secs(Ok(std::time::SystemTime::now()));
        let fresh = t.node(child(t, ROOT, "fresh.bin"));
        assert!(now - fresh.created < 3600, "created: {}", fresh.created);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn counts_apfs_clones_once() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        let data: Vec<u8> = (0..3_000_000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 9) as u8)
            .collect();
        fs::write(r.join("a.bin"), &data).unwrap();
        fs::copy(r.join("a.bin"), r.join("clone.bin")).unwrap(); // clonefile on APFS
        fs::write(r.join("copy.bin"), &data).unwrap(); // a real second copy

        let res = run(r);
        let t = &res.tree;
        let a = child(t, ROOT, "a.bin");
        let clone = child(t, ROOT, "clone.bin");
        let copy = child(t, ROOT, "copy.bin");
        if t.clone_of(a).is_none() {
            eprintln!("not an APFS volume; skipping");
            return;
        }
        assert_eq!(t.clone_of(a).unwrap().0, t.clone_of(clone).unwrap().0);
        assert!(t.clone_of(copy).is_none());
        // Apparent sizes are file lengths: three files.
        assert_eq!(t.node(ROOT).size.apparent, 9_000_000);
        // On disk the clone's shared blocks count once: about two files.
        let one = t.node(copy).size.disk;
        let shared = t.node(a).size.disk + t.node(clone).size.disk;
        assert_eq!(shared, one, "a + clone hold one file's blocks");
        assert_eq!(t.node(ROOT).size.disk, 2 * one);
    }

    /// Sizes and file counts of every entry, by relative path.
    fn snapshot(tree: &Tree) -> Vec<(String, u64, u64, u32)> {
        let root = tree.path_of(ROOT);
        let mut out = Vec::new();
        let mut stack = vec![ROOT];
        while let Some(id) = stack.pop() {
            let n = tree.node(id);
            let rel = tree.path_of(id);
            let rel = rel
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, n.size.apparent, n.size.disk, n.file_count));
            stack.extend(tree.children(id));
        }
        out.sort();
        out
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn rescanning_a_folder_counts_outside_clones_once() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join("a")).unwrap();
        fs::create_dir_all(r.join("b")).unwrap();
        let data: Vec<u8> = (0..2_000_000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 11) as u8)
            .collect();
        fs::write(r.join("a/orig.bin"), &data).unwrap();
        fs::copy(r.join("a/orig.bin"), r.join("b/clone.bin")).unwrap();

        let mut res = run(r);
        if res
            .tree
            .clone_of(child(&res.tree, child(&res.tree, ROOT, "a"), "orig.bin"))
            .is_none()
        {
            eprintln!("not an APFS volume; skipping");
            return;
        }
        // Rescan each folder in turn: the other one holds the clone's twin.
        for name in ["a", "b"] {
            let d = child(&res.tree, ROOT, name);
            let sub = scan(&res.tree.path_of(d), Vec::new(), &Arc::default(), |_| {}).unwrap();
            res.tree.replace_children(d, &sub.tree);
            let fresh = run(r);
            assert_eq!(
                res.tree.node(ROOT).size.disk,
                fresh.tree.node(ROOT).size.disk,
                "after rescanning {name}"
            );
        }
    }

    #[test]
    fn rescanning_a_folder_matches_a_full_rescan() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join("proj/src")).unwrap();
        fs::create_dir_all(r.join("other")).unwrap();
        write(&r.join("proj/src/a.rs"), 1000);
        write(&r.join("proj/old.bin"), 5000);
        write(&r.join("other/x"), 300);

        let mut res = run(r);
        let proj = child(&res.tree, ROOT, "proj");

        // Change the folder: delete, grow, add a subfolder.
        fs::remove_file(r.join("proj/old.bin")).unwrap();
        write(&r.join("proj/src/a.rs"), 4000);
        fs::create_dir(r.join("proj/new")).unwrap();
        write(&r.join("proj/new/b.bin"), 2500);

        let sub = scan(&res.tree.path_of(proj), Vec::new(), &Arc::default(), |_| {}).unwrap();
        res.tree.replace_children(proj, &sub.tree);

        let fresh = run(r);
        assert_eq!(snapshot(&res.tree), snapshot(&fresh.tree));
    }

    #[test]
    fn rejects_missing_root() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        assert!(scan(&missing, Vec::new(), &Arc::default(), |_| {}).is_err());
    }
}
