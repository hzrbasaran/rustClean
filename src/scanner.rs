//! Parallel directory scanning.
//!
//! `jwalk` reads directories on a rayon pool; per-entry `stat` calls happen in
//! the `process_read_dir` callback so they run in parallel too. The consuming
//! thread only assembles the `Tree`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use jwalk::WalkDirGeneric;

use crate::tree::{Tree, ROOT};

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

/// Metadata gathered in parallel for each entry.
#[derive(Debug, Default)]
struct Meta {
    size: u64,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(unix)]
    nlink: u64,
    error: bool,
}

/// Scans `root` synchronously. Symlinks are not followed, other filesystems
/// (and any path in `skip`) are not descended into, and hard-linked files are
/// counted once.
pub fn scan(
    root: &Path,
    skip: Vec<PathBuf>,
    cancel: &Arc<AtomicBool>,
    mut on_progress: impl FnMut(&ScanProgress),
) -> Result<ScanResult> {
    let started = Instant::now();
    let root = std::path::absolute(root).context("geçersiz yol")?;
    let root_md = fs::metadata(&root).with_context(|| format!("{} okunamadı", root.display()))?;
    anyhow::ensure!(root_md.is_dir(), "{} bir klasör değil", root.display());
    #[cfg(unix)]
    let root_dev = std::os::unix::fs::MetadataExt::dev(&root_md);

    let skip: HashSet<PathBuf> = skip.into_iter().filter(|p| *p != root).collect();
    let cancel_flag = Arc::clone(cancel);

    let walk = WalkDirGeneric::<((), Meta)>::new(&root)
        .follow_links(false)
        .skip_hidden(false)
        .process_read_dir(move |_depth, dir, _state, children| {
            if cancel_flag.load(Ordering::Relaxed) {
                children.clear();
                return;
            }
            for child in children.iter_mut().flatten() {
                let path = dir.join(&child.file_name);
                let md = match fs::symlink_metadata(&path) {
                    Ok(md) => md,
                    Err(_) => {
                        child.client_state.error = true;
                        child.read_children = None;
                        continue;
                    }
                };
                let meta = &mut child.client_state;
                meta.size = md.len();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    meta.dev = md.dev();
                    meta.ino = md.ino();
                    meta.nlink = md.nlink();
                }
                if child.file_type.is_dir() {
                    #[cfg(unix)]
                    let other_fs = meta.dev != root_dev;
                    #[cfg(not(unix))]
                    let other_fs = false;
                    if other_fs || skip.contains(&path) {
                        child.read_children = None;
                    }
                }
            }
        });

    let mut tree = Tree::new(&root);
    let mut dirs: HashMap<PathBuf, usize> = HashMap::new();
    dirs.insert(root.clone(), ROOT);
    #[cfg(unix)]
    let mut seen_links: HashSet<(u64, u64)> = HashSet::new();
    let mut progress = ScanProgress::default();
    let mut last_report = Instant::now();

    for entry in walk {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                progress.errors += 1;
                continue;
            }
        };
        if entry.depth == 0 {
            continue;
        }
        let Some(&parent) = dirs.get(&*entry.parent_path) else {
            continue;
        };
        let meta = &entry.client_state;
        if meta.error {
            progress.errors += 1;
            continue;
        }
        let is_dir = entry.file_type.is_dir();
        #[allow(unused_mut)]
        let mut size = if is_dir { 0 } else { meta.size };
        #[cfg(unix)]
        if !is_dir && meta.nlink > 1 && !seen_links.insert((meta.dev, meta.ino)) {
            size = 0;
        }

        let name = entry.file_name.to_string_lossy();
        let id = tree.push(parent, &name, is_dir, size);
        if is_dir {
            dirs.insert(entry.path(), id);
            progress.dirs += 1;
        } else {
            progress.files += 1;
            progress.bytes += size;
        }

        if last_report.elapsed() >= PROGRESS_INTERVAL {
            progress.current = entry.parent_path.to_path_buf();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::NodeId;

    fn write(path: &Path, len: usize) {
        fs::write(path, vec![0u8; len]).unwrap();
    }

    fn run(root: &Path) -> ScanResult {
        scan(root, Vec::new(), &Arc::default(), |_| {}).unwrap()
    }

    fn child(tree: &Tree, parent: NodeId, name: &str) -> NodeId {
        *tree
            .node(parent)
            .children
            .iter()
            .find(|&&c| &*tree.node(c).name == name)
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
        assert_eq!(t.node(ROOT).size, 3100);
        assert_eq!(t.node(ROOT).file_count, 3);
        let a = child(t, ROOT, "a");
        assert_eq!(t.node(a).size, 3000);
        assert_eq!(t.node(child(t, a, "b")).size, 2000);
        assert_eq!(t.node(child(t, ROOT, "empty")).size, 0);
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
        assert_eq!(res.tree.node(ROOT).size, 10);
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
        assert!(t.node(link).size < 4000);
        assert_eq!(t.node(child(t, ROOT, "real")).size, 4000);
    }

    #[cfg(unix)]
    #[test]
    fn counts_hardlinks_once() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(&r.join("orig.bin"), 3000);
        fs::hard_link(r.join("orig.bin"), r.join("copy.bin")).unwrap();

        let res = run(r);
        assert_eq!(res.tree.node(ROOT).size, 3000);
    }

    #[test]
    fn rejects_missing_root() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        assert!(scan(&missing, Vec::new(), &Arc::default(), |_| {}).is_err());
    }
}
