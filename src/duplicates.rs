//! Finding files with identical content.
//!
//! Files are compared in three narrowing steps so that most of them are
//! never read in full: same size, then same hash of their first and last
//! 64 KiB, then same hash of the whole content. Reading happens on a
//! background thread (files in parallel); the tree is not touched there.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;

use rayon::prelude::*;
use xxhash_rust::xxh3::{xxh3_128, Xxh3};

use crate::tree::{NodeId, Tree};

/// Smaller files are ignored: plenty of them, little space to win. This is
/// the default; `duplicates_min_mib` in the configuration changes it.
pub const MIN_SIZE: u64 = 1024 * 1024;
const EDGE: u64 = 64 * 1024;

pub struct Candidate {
    pub id: NodeId,
    pub path: PathBuf,
    /// Length of the file (apparent size).
    pub size: u64,
}

/// Version control folders: their files belong to the repository.
const VCS: [&str; 3] = [".git", ".hg", ".svn"];

/// Whether files in folder `name` must not be offered one by one: a
/// package (an app, a Photos library…) or a version control folder.
fn kept_whole(name: &str) -> bool {
    VCS.contains(&name) || crate::reports::is_bundle(name)
}

/// Whether `id` is such a folder or inside one.
pub fn inside_kept_whole(tree: &Tree, id: NodeId) -> bool {
    let mut at = Some(id);
    while let Some(n) = at {
        if tree.node(n).is_dir && kept_whole(tree.name(n)) {
            return true;
        }
        at = tree.parent(n);
    }
    false
}

/// Files below `base` of at least the configured size that share their
/// size with at least one other file. Packages and version control folders
/// are not searched: removing one of their files breaks them, even when
/// the same bytes exist elsewhere.
pub fn candidates(tree: &Tree, base: NodeId) -> Vec<Candidate> {
    if inside_kept_whole(tree, base) {
        return Vec::new();
    }
    // Never 0: extra hard links have size 0 and must not become "copies".
    let min_size = crate::config::get().duplicates_min_size().max(1);
    let mut by_size: HashMap<u64, Vec<NodeId>> = HashMap::new();
    let mut stack = vec![base];
    while let Some(dir) = stack.pop() {
        for id in tree.children(dir) {
            let n = tree.node(id);
            if n.is_dir {
                if !kept_whole(tree.name(id)) {
                    stack.push(id);
                }
            } else if n.size.apparent >= min_size {
                // Extra hard links were recorded with size 0, so each file
                // is only considered once.
                by_size.entry(n.size.apparent).or_default().push(id);
            }
        }
    }
    by_size
        .into_iter()
        .filter(|(_, ids)| ids.len() > 1)
        .flat_map(|(size, ids)| {
            ids.into_iter().map(move |id| Candidate {
                id,
                path: tree.path_of(id),
                size,
            })
        })
        .collect()
}

/// Progress of a running search, readable from the UI thread.
#[derive(Default)]
pub struct Progress {
    /// 1: sampling file edges, 2: hashing whole files.
    pub stage: AtomicU8,
    pub done: AtomicU64,
    pub total: AtomicU64,
}

/// Groups of identical files (each with two or more members).
pub fn find_groups(
    cands: &[Candidate],
    cancel: &AtomicBool,
    progress: &Progress,
) -> Vec<Vec<NodeId>> {
    // Step 1: same size.
    let groups = group_by(cands.iter().map(|c| (c, u128::from(c.size))));

    // Step 2: first and last 64 KiB.
    progress.stage.store(1, Ordering::Relaxed);
    let sampled: Vec<&Candidate> = groups.into_iter().flatten().collect();
    progress
        .total
        .store(sampled.len() as u64, Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);
    let hashed: Vec<(&Candidate, u128)> = sampled
        .into_par_iter()
        .filter_map(|c| {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            let h = edge_hash(c);
            progress.done.fetch_add(1, Ordering::Relaxed);
            h.map(|h| (c, h ^ u128::from(c.size).rotate_left(64)))
        })
        .collect();
    let groups = group_by(hashed.into_iter());

    // Step 3: whole content (the edges already cover small files).
    progress.stage.store(2, Ordering::Relaxed);
    let to_read: Vec<&Candidate> = groups.into_iter().flatten().collect();
    let total: u64 = to_read
        .iter()
        .filter(|c| c.size > 2 * EDGE)
        .map(|c| c.size)
        .sum();
    progress.total.store(total, Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);
    let hashed: Vec<(&Candidate, u128)> = to_read
        .into_par_iter()
        .filter_map(|c| {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            let h = if c.size > 2 * EDGE {
                full_hash(c, cancel, progress)?
            } else {
                edge_hash(c)?
            };
            Some((c, h ^ u128::from(c.size).rotate_left(64)))
        })
        .collect();
    if cancel.load(Ordering::Relaxed) {
        return Vec::new();
    }
    group_by(hashed.into_iter())
        .into_iter()
        .map(|g| g.into_iter().map(|c| c.id).collect())
        .collect()
}

/// Groups items by key, keeping groups with more than one member.
fn group_by<'a>(items: impl Iterator<Item = (&'a Candidate, u128)>) -> Vec<Vec<&'a Candidate>> {
    let mut map: HashMap<u128, Vec<&Candidate>> = HashMap::new();
    for (c, key) in items {
        map.entry(key).or_default().push(c);
    }
    map.into_values().filter(|g| g.len() > 1).collect()
}

/// Hash of the first and last `EDGE` bytes (the whole file if shorter).
fn edge_hash(c: &Candidate) -> Option<u128> {
    let mut f = File::open(&c.path).ok()?;
    if c.size <= 2 * EDGE {
        let mut buf = Vec::with_capacity(c.size as usize);
        f.read_to_end(&mut buf).ok()?;
        return Some(xxh3_128(&buf));
    }
    let mut buf = vec![0u8; 2 * EDGE as usize];
    f.read_exact(&mut buf[..EDGE as usize]).ok()?;
    f.seek(SeekFrom::End(-(EDGE as i64))).ok()?;
    f.read_exact(&mut buf[EDGE as usize..]).ok()?;
    Some(xxh3_128(&buf))
}

fn full_hash(c: &Candidate, cancel: &AtomicBool, progress: &Progress) -> Option<u128> {
    let mut f = File::open(&c.path).ok()?;
    let mut hasher = Xxh3::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        progress.done.fetch_add(n as u64, Ordering::Relaxed);
    }
    Some(hasher.digest128())
}

/// A duplicate search on a background thread. Dropping it cancels the
/// search.
pub struct DupJob {
    pub base: NodeId,
    pub progress: Arc<Progress>,
    rx: Receiver<Vec<Vec<NodeId>>>,
    cancel: Arc<AtomicBool>,
}

impl DupJob {
    pub fn start(base: NodeId, cands: Vec<Candidate>) -> Self {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Progress::default());
        let (c, p) = (Arc::clone(&cancel), Arc::clone(&progress));
        thread::spawn(move || {
            let groups = find_groups(&cands, &c, &p);
            let _ = tx.send(groups);
        });
        Self {
            base,
            progress,
            rx,
            cancel,
        }
    }

    /// The groups, once the search has finished.
    pub fn poll(&self) -> Option<Vec<Vec<NodeId>>> {
        match self.rx.try_recv() {
            Ok(groups) => Some(groups),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Vec::new()),
        }
    }
}

impl Drop for DupJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner;
    use crate::tree::ROOT;
    use std::fs;
    use std::path::Path;

    /// Deterministic, non-repeating content.
    fn content(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|i| (i as u32).wrapping_mul(2654435761).to_le_bytes()[seed as usize % 4] ^ seed)
            .collect()
    }

    fn find(root: &Path) -> (Vec<Vec<String>>, Tree) {
        let res = scanner::scan(root, Vec::new(), &Arc::default(), |_| {}).unwrap();
        let tree = res.tree;
        let cands = candidates(&tree, ROOT);
        let groups = find_groups(&cands, &AtomicBool::new(false), &Progress::default());
        let mut named: Vec<Vec<String>> = groups
            .into_iter()
            .map(|g| {
                let mut names: Vec<String> =
                    g.iter().map(|&id| tree.name(id).to_string()).collect();
                names.sort();
                names
            })
            .collect();
        named.sort();
        (named, tree)
    }

    #[test]
    fn minimum_size_follows_the_configuration() {
        let mut t = Tree::new(Path::new("/r"));
        let size = crate::tree::Size {
            apparent: 2 * MIN_SIZE,
            disk: 2 * MIN_SIZE,
        };
        t.push(ROOT, "a", false, size);
        t.push(ROOT, "b", false, size);
        t.finalize();
        assert_eq!(candidates(&t, ROOT).len(), 2);
        let config = crate::config::Config {
            duplicates_min_mib: 3,
            ..Default::default()
        };
        assert!(crate::config::with(config, || candidates(&t, ROOT)).is_empty());
    }

    #[test]
    fn bundles_and_repositories_are_not_searched() {
        let mut t = Tree::new(Path::new("/r"));
        let size = crate::tree::Size {
            apparent: 2 * MIN_SIZE,
            disk: 2 * MIN_SIZE,
        };
        let dir = |t: &mut Tree, name| t.push(ROOT, name, true, crate::tree::Size::default());
        let photos = dir(&mut t, "Photos Library.photoslibrary");
        let originals = t.push(photos, "originals", true, crate::tree::Size::default());
        t.push(originals, "IMG_1.jpg", false, size);
        let app = dir(&mut t, "Editor.app");
        t.push(app, "IMG_1.jpg", false, size);
        let git = dir(&mut t, ".git");
        t.push(git, "pack.bin", false, size);
        let pics = dir(&mut t, "Pictures");
        t.push(pics, "IMG_1.jpg", false, size);
        t.push(pics, "pack.bin", false, size);
        t.finalize();
        // The files inside the library, the app and the repository are never
        // offered; the two in Pictures stay (they share a size).
        let mut names: Vec<String> = candidates(&t, ROOT)
            .iter()
            .map(|c| t.path_of(c.id).display().to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["/r/Pictures/IMG_1.jpg", "/r/Pictures/pack.bin"]);
        // Run from inside a package, nothing is offered.
        assert!(candidates(&t, originals).is_empty());
        assert!(inside_kept_whole(&t, originals));
        assert!(!inside_kept_whole(&t, pics));
    }

    #[test]
    fn finds_identical_content_only() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        let size = 3 * 1024 * 1024;
        let a = content(size, 1);
        fs::write(r.join("a.bin"), &a).unwrap();
        fs::create_dir(r.join("sub")).unwrap();
        fs::write(r.join("sub/copy-of-a.dat"), &a).unwrap();
        // Same size and same first/last 64 KiB, different middle.
        let mut c = a.clone();
        c[size / 2] ^= 0xff;
        fs::write(r.join("c.bin"), &c).unwrap();
        // Identical but below the size threshold.
        let small = content(1000, 2);
        fs::write(r.join("s1"), &small).unwrap();
        fs::write(r.join("s2"), &small).unwrap();
        // Same size as the small pair would not matter; a lone big file.
        fs::write(r.join("lonely.bin"), content(2 * 1024 * 1024, 3)).unwrap();

        let (groups, _) = find(r);
        assert_eq!(
            groups,
            vec![vec!["a.bin".to_string(), "copy-of-a.dat".to_string()]]
        );
    }

    #[cfg(unix)]
    #[test]
    fn hard_links_are_not_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::write(r.join("a.bin"), content(2 * 1024 * 1024, 4)).unwrap();
        fs::hard_link(r.join("a.bin"), r.join("link.bin")).unwrap();
        let (groups, _) = find(r);
        assert!(groups.is_empty());
    }
}
