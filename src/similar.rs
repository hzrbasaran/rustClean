//! Finding images that look alike: resized, re-compressed or re-saved
//! copies that the duplicates report misses, as their bytes differ.
//!
//! Each image is decoded on a background thread (in parallel) and reduced
//! to a 64-bit gradient hash; images whose hashes differ in at most a few
//! bits are grouped. A BK-tree finds the close hashes without comparing
//! every pair. The same folders as the clutter report are searched: no
//! hidden folders, bundles (a Photos library holds its own files), Library
//! or system folders.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;

use rayon::prelude::*;

use crate::tree::{NodeId, Tree};

/// Smaller images (icons, thumbnails) are left out.
pub const MIN_SIZE: u64 = 100 * 1024;

/// The file types that are read. HEIC and camera RAW files cannot be
/// decoded and are not searched.
pub const EXTENSIONS: [&str; 8] = ["jpg", "jpeg", "png", "webp", "gif", "tif", "tiff", "bmp"];

/// The most bits two hashes may differ in to count as similar: resized and
/// re-compressed copies stay within it, different photos do not. This is
/// the default; `similar_distance` in the configuration changes it.
pub const MAX_DISTANCE: u32 = 4;

/// Decoding stops above this many bytes per image, so a huge image cannot
/// use up the memory.
#[cfg(feature = "similar-images")]
const MAX_ALLOC: u64 = 512 * 1024 * 1024;

/// An image file to compare.
pub struct Candidate {
    pub id: NodeId,
    pub path: PathBuf,
}

/// One image of a group: its width and height in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Member {
    pub id: NodeId,
    pub width: u32,
    pub height: u32,
}

/// Whether this build can read images (the `similar-images` feature).
pub const AVAILABLE: bool = cfg!(feature = "similar-images");

/// Image files below `base` of at least `min_size` bytes, in the folders the
/// search goes into.
pub fn candidates(tree: &Tree, base: NodeId, min_size: u64) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut stack = vec![(base, tree.path_of(base))];
    while let Some((dir, path)) = stack.pop() {
        for id in tree.children(dir) {
            let n = tree.node(id);
            let child = path.join(tree.name(id));
            if n.is_dir {
                if crate::reports::searched(tree, id, &child) {
                    stack.push((id, child));
                }
            } else if !n.is_link && n.size.apparent >= min_size && is_image(tree.name(id)) {
                out.push(Candidate { id, path: child });
            }
        }
    }
    out
}

fn is_image(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, e)| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Progress of a running search, readable from the interface thread.
#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
}

/// The hash and size of one image, or `None` when it cannot be decoded.
#[cfg(feature = "similar-images")]
fn hash_one(path: &std::path::Path) -> Option<(u64, u32, u32)> {
    use image_hasher::{HashAlg, HasherConfig};
    let mut reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    let hasher = HasherConfig::new()
        .hash_alg(HashAlg::Gradient)
        .hash_size(8, 8)
        .to_hasher();
    let hash = hasher.hash_image(&img);
    let bytes: [u8; 8] = hash.as_bytes().try_into().ok()?;
    Some((u64::from_le_bytes(bytes), img.width(), img.height()))
}

#[cfg(not(feature = "similar-images"))]
fn hash_one(_path: &std::path::Path) -> Option<(u64, u32, u32)> {
    None
}

/// Groups of similar images (each with two or more members).
pub fn find_groups(
    cands: &[Candidate],
    max_distance: u32,
    cancel: &AtomicBool,
    progress: &Progress,
) -> Vec<Vec<Member>> {
    progress.total.store(cands.len() as u64, Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);
    let hashed: Vec<(u64, Member)> = cands
        .par_iter()
        .filter_map(|c| {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            let h = hash_one(&c.path);
            progress.done.fetch_add(1, Ordering::Relaxed);
            let (hash, width, height) = h?;
            Some((
                hash,
                Member {
                    id: c.id,
                    width,
                    height,
                },
            ))
        })
        .collect();
    if cancel.load(Ordering::Relaxed) {
        return Vec::new();
    }
    let hashes: Vec<u64> = hashed.iter().map(|(h, _)| *h).collect();
    group_close(&hashes, max_distance)
        .into_iter()
        .map(|g| g.into_iter().map(|i| hashed[i].1).collect())
        .collect()
}

/// Indexes of `hashes` grouped so that each hash is within `max` bits of
/// another one in its group. Groups with one member are left out.
fn group_close(hashes: &[u64], max: u32) -> Vec<Vec<usize>> {
    let mut tree = BkTree::default();
    for (i, &h) in hashes.iter().enumerate() {
        tree.insert(h, i);
    }
    let mut sets = UnionFind::new(hashes.len());
    for (i, &h) in hashes.iter().enumerate() {
        for j in tree.within(h, max) {
            sets.join(i, j);
        }
    }
    let mut groups: std::collections::HashMap<usize, Vec<usize>> = Default::default();
    for i in 0..hashes.len() {
        groups.entry(sets.root(i)).or_default().push(i);
    }
    let mut out: Vec<Vec<usize>> = groups.into_values().filter(|g| g.len() > 1).collect();
    out.sort();
    out
}

/// A BK-tree over Hamming distance: each child is filed under its distance
/// to the parent, so a search only visits children whose distance can be in
/// range (the triangle inequality).
#[derive(Default)]
struct BkTree {
    /// Node 0 is the root.
    nodes: Vec<BkNode>,
}

struct BkNode {
    hash: u64,
    /// Index of the hash in the input.
    index: usize,
    /// `(distance to this node, child node)`.
    children: Vec<(u32, usize)>,
}

impl BkTree {
    fn insert(&mut self, hash: u64, index: usize) {
        let node = BkNode {
            hash,
            index,
            children: Vec::new(),
        };
        if self.nodes.is_empty() {
            self.nodes.push(node);
            return;
        }
        let mut at = 0;
        loop {
            let d = (self.nodes[at].hash ^ hash).count_ones();
            match self.nodes[at].children.iter().find(|(cd, _)| *cd == d) {
                Some(&(_, next)) => at = next,
                None => {
                    let new = self.nodes.len();
                    self.nodes.push(node);
                    self.nodes[at].children.push((d, new));
                    return;
                }
            }
        }
    }

    /// The indexes of the hashes at most `max` bits from `hash`.
    fn within(&self, hash: u64, max: u32) -> Vec<usize> {
        let mut out = Vec::new();
        if self.nodes.is_empty() {
            return out;
        }
        let mut stack = vec![0];
        while let Some(at) = stack.pop() {
            let node = &self.nodes[at];
            let d = (node.hash ^ hash).count_ones();
            if d <= max {
                out.push(node.index);
            }
            for &(cd, child) in &node.children {
                if cd + max >= d && cd <= d + max {
                    stack.push(child);
                }
            }
        }
        out
    }
}

struct UnionFind(Vec<usize>);

impl UnionFind {
    fn new(n: usize) -> Self {
        Self((0..n).collect())
    }

    fn root(&mut self, mut i: usize) -> usize {
        while self.0[i] != i {
            self.0[i] = self.0[self.0[i]];
            i = self.0[i];
        }
        i
    }

    fn join(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.root(a), self.root(b));
        if ra != rb {
            self.0[ra.max(rb)] = ra.min(rb);
        }
    }
}

/// A similar-image search on a background thread. Dropping it cancels the
/// search.
pub struct SimilarJob {
    pub base: NodeId,
    pub progress: Arc<Progress>,
    rx: Receiver<Vec<Vec<Member>>>,
    cancel: Arc<AtomicBool>,
}

impl SimilarJob {
    pub fn start(base: NodeId, cands: Vec<Candidate>, max_distance: u32) -> Self {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Progress::default());
        let (c, p) = (Arc::clone(&cancel), Arc::clone(&progress));
        thread::spawn(move || {
            let _ = tx.send(find_groups(&cands, max_distance, &c, &p));
        });
        Self {
            base,
            progress,
            rx,
            cancel,
        }
    }

    /// The groups, once the search has finished.
    pub fn poll(&self) -> Option<Vec<Vec<Member>>> {
        match self.rx.try_recv() {
            Ok(groups) => Some(groups),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Vec::new()),
        }
    }
}

impl Drop for SimilarJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_hashes_group_and_far_ones_do_not() {
        let a = 0b1010_1010u64 << 32 | 0xFFFF;
        let hashes = [
            a,
            a ^ 0b111,        // 3 bits off: similar to a
            a ^ (0b11 << 40), // 2 bits off
            !a,               // far from everything
            !a ^ 1,           // close to the one before
        ];
        assert_eq!(group_close(&hashes, 4), [vec![0, 1, 2], vec![3, 4]]);
        assert_eq!(group_close(&hashes, 0), Vec::<Vec<usize>>::new());
    }

    #[test]
    fn the_bk_tree_finds_what_a_full_scan_finds() {
        // A deterministic spread of hashes, some of them close.
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut hashes = Vec::new();
        for i in 0..400 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let h: u64 = if i % 5 == 4 {
                hashes[i - 1] ^ (x & 0b1011)
            } else {
                x
            };
            hashes.push(h);
        }
        let mut tree = BkTree::default();
        for (i, &h) in hashes.iter().enumerate() {
            tree.insert(h, i);
        }
        for (i, &h) in hashes.iter().enumerate().step_by(7) {
            let mut fast = tree.within(h, 6);
            fast.sort_unstable();
            let slow: Vec<usize> = (0..hashes.len())
                .filter(|&j| (hashes[j] ^ h).count_ones() <= 6)
                .collect();
            assert_eq!(fast, slow, "hash {i}");
        }
    }

    #[test]
    fn only_images_in_searched_folders_are_candidates() {
        use crate::tree::{Size, ROOT};
        let mut t = Tree::new(std::path::Path::new("/Users/demo"));
        let size = |n| Size {
            apparent: n,
            disk: n,
        };
        let pics = t.push(ROOT, "Pictures", true, Size::default());
        t.push(pics, "a.JPG", false, size(200_000));
        t.push(pics, "b.png", false, size(200_000));
        t.push(pics, "icon.png", false, size(2_000));
        t.push(pics, "notes.txt", false, size(200_000));
        t.push(pics, "c.heic", false, size(200_000));
        let lib = t.push(pics, "Photos Library.photoslibrary", true, Size::default());
        t.push(lib, "inside.jpg", false, size(200_000));
        let hidden = t.push(ROOT, ".cache", true, Size::default());
        t.push(hidden, "thumb.jpg", false, size(200_000));
        t.finalize();
        let mut names: Vec<String> = candidates(&t, ROOT, MIN_SIZE)
            .iter()
            .map(|c| t.name(c.id).to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["a.JPG", "b.png"]);
    }

    #[cfg(feature = "similar-images")]
    #[test]
    fn resized_and_recompressed_copies_are_similar() {
        let dir = tempfile::tempdir().unwrap();
        let photo = |w: u32, h: u32| {
            image::RgbImage::from_fn(w, h, |x, y| {
                // Broad shapes, like a photo, not noise.
                let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
                let r = (255.0 * fx) as u8;
                let g = (255.0 * (1.0 - fy)) as u8;
                let b = if (fx - 0.5).powi(2) + (fy - 0.4).powi(2) < 0.04 {
                    230
                } else {
                    40
                };
                image::Rgb([r, g, b])
            })
        };
        let other = image::RgbImage::from_fn(300, 200, |x, y| {
            let v = if (x / 30 + y / 30) % 2 == 0 { 250 } else { 10 };
            image::Rgb([v, 255 - v, v / 2])
        });
        let original = dir.path().join("original.png");
        photo(600, 400).save(&original).unwrap();
        let small = dir.path().join("small.jpg");
        image::DynamicImage::ImageRgb8(photo(600, 400))
            .resize(240, 160, image::imageops::FilterType::Triangle)
            .to_rgb8()
            .save(&small)
            .unwrap();
        let different = dir.path().join("different.png");
        other.save(&different).unwrap();
        let cands: Vec<Candidate> = [&original, &small, &different]
            .iter()
            .enumerate()
            .map(|(i, p)| Candidate {
                id: i as NodeId,
                path: p.to_path_buf(),
            })
            .collect();
        let groups = find_groups(
            &cands,
            MAX_DISTANCE,
            &AtomicBool::new(false),
            &Progress::default(),
        );
        assert_eq!(groups.len(), 1, "{groups:?}");
        let mut ids: Vec<NodeId> = groups[0].iter().map(|m| m.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, [0, 1]);
        let big = groups[0].iter().find(|m| m.id == 0).unwrap();
        assert_eq!((big.width, big.height), (600, 400));
    }
}
