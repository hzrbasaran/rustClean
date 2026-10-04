//! Scan history: a compact summary of each scan is saved so later scans
//! can show what changed.
//!
//! A snapshot holds every directory of at least `MIN_DIR` on disk and
//! every file of at least `MIN_FILE`, with paths relative to the scan root.
//! Files live in `<data dir>/rustClean/history/<hash of root>/<time>.rcs`,
//! deflate-compressed, newest `KEEP` per root.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use xxhash_rust::xxh3::xxh3_64;

use std::collections::{HashMap, HashSet};

use crate::lists::Row;
use crate::reports::LIMIT;
use crate::tree::{NodeId, Size, SizeMode, Tree, ROOT};
use crate::ui::{fmt_delta, fmt_size};

pub const MIN_DIR: u64 = 1024 * 1024;
pub const MIN_FILE: u64 = 100 * 1024 * 1024;
/// Snapshots kept per scan root.
pub const KEEP: usize = 10;

const MAGIC: &[u8; 4] = b"RCSH";
const VERSION: u16 = 1;
const EXT: &str = "rcs";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Path relative to the root, components joined with '/'.
    pub path: String,
    pub is_dir: bool,
    pub size: Size,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub root: PathBuf,
    /// Seconds since the Unix epoch.
    pub time: u64,
    pub total: Size,
    pub files: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub header: Header,
    pub entries: Vec<Entry>,
}

/// A saved snapshot, known by its header only.
#[derive(Debug, Clone)]
pub struct Saved {
    pub file: PathBuf,
    pub header: Header,
}

/// Where snapshots of `root` are kept, below the data directory.
pub fn dir_for(root: &Path) -> Option<PathBuf> {
    let base = crate::paths::data_dir()?;
    let key = xxh3_64(root.to_string_lossy().as_bytes());
    Some(base.join("history").join(format!("{key:016x}")))
}

/// Summarizes the tree. Only directories of at least `MIN_DIR` are entered:
/// nothing below a smaller one can qualify.
pub fn capture(tree: &Tree, time: u64) -> Snapshot {
    let root = tree.node(ROOT);
    let mut entries = Vec::new();
    let mut stack: Vec<(NodeId, String)> = vec![(ROOT, String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        for id in tree.children(dir) {
            let n = tree.node(id);
            let min = if n.is_dir { MIN_DIR } else { MIN_FILE };
            if n.size.disk < min {
                continue;
            }
            let path = if prefix.is_empty() {
                tree.name(id).to_string()
            } else {
                format!("{prefix}/{}", tree.name(id))
            };
            entries.push(Entry {
                path: path.clone(),
                is_dir: n.is_dir,
                size: n.size,
            });
            if n.is_dir {
                stack.push((id, path));
            }
        }
    }
    Snapshot {
        header: Header {
            root: tree.root_path().to_path_buf(),
            time,
            total: root.size,
            files: u64::from(root.file_count),
        },
        entries,
    }
}

/// Writes `snap` into `dir` and removes all but the newest `KEEP`.
pub fn save(dir: &Path, snap: &Snapshot) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let mut file = dir.join(format!("{}.{EXT}", snap.header.time));
    let mut n = 1;
    while file.exists() {
        file = dir.join(format!("{}-{n}.{EXT}", snap.header.time));
        n += 1;
    }
    let tmp = file.with_extension("tmp");
    {
        let mut w = DeflateEncoder::new(BufWriter::new(File::create(&tmp)?), Compression::fast());
        write_header(&mut w, &snap.header)?;
        w.write_all(&(snap.entries.len() as u64).to_le_bytes())?;
        for e in &snap.entries {
            w.write_all(&[u8::from(e.is_dir)])?;
            w.write_all(&e.size.apparent.to_le_bytes())?;
            w.write_all(&e.size.disk.to_le_bytes())?;
            write_str(&mut w, &e.path)?;
        }
        w.finish()?.flush()?;
    }
    fs::rename(&tmp, &file)?;
    prune(dir)?;
    Ok(file)
}

/// Saved snapshots in `dir`, newest first. Unreadable files are skipped.
pub fn list(dir: &Path) -> Vec<Saved> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<Saved> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == EXT))
        .filter_map(|file| {
            let mut r = open(&file).ok()?;
            let header = read_header(&mut r).ok()?;
            Some(Saved { file, header })
        })
        .collect();
    out.sort_by(|a, b| b.header.time.cmp(&a.header.time).then(b.file.cmp(&a.file)));
    out
}

pub fn load(file: &Path) -> io::Result<Snapshot> {
    let mut r = open(file)?;
    let header = read_header(&mut r)?;
    let count = read_u64(&mut r)?;
    let mut entries = Vec::with_capacity(count.min(10_000_000) as usize);
    for _ in 0..count {
        let mut kind = [0u8];
        r.read_exact(&mut kind)?;
        let apparent = read_u64(&mut r)?;
        let disk = read_u64(&mut r)?;
        entries.push(Entry {
            is_dir: kind[0] == 1,
            size: Size { apparent, disk },
            path: read_str(&mut r)?,
        });
    }
    Ok(Snapshot { header, entries })
}

/// A directory is skipped when one subdirectory carries more than this
/// share of its change: growth shows up in every parent otherwise.
const WRAPPER_SHARE: f64 = 0.9;

/// What changed below `base` since `snap`: rows for entries that grew,
/// shrank or appeared by at least `MIN_DIR`, largest growth first, plus a
/// note about what disappeared.
pub fn changes(
    tree: &Tree,
    base: NodeId,
    snap: &Snapshot,
    mode: SizeMode,
) -> (Vec<Row>, bool, String) {
    struct Change {
        id: NodeId,
        delta: i128,
        is_new: bool,
    }

    let base_rel = rel_path(tree, base);
    let under_base =
        |p: &str| base_rel.is_empty() || p == base_rel || p.starts_with(&format!("{base_rel}/"));
    let old: HashMap<&str, u64> = snap
        .entries
        .iter()
        .filter(|e| under_base(&e.path))
        .map(|e| (e.path.as_str(), e.size.get(mode)))
        .collect();

    // Walk the current tree where anything could have changed by MIN_DIR.
    let mut found: Vec<Change> = Vec::new();
    let mut delta_of: HashMap<NodeId, i128> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut stack: Vec<(NodeId, String)> = vec![(base, base_rel.clone())];
    while let Some((dir, prefix)) = stack.pop() {
        for id in tree.children(dir) {
            let n = tree.node(id);
            let path = if prefix.is_empty() {
                tree.name(id).to_string()
            } else {
                format!("{prefix}/{}", tree.name(id))
            };
            let now = n.size.get(mode);
            let then = old.get(path.as_str()).copied();
            let tracked = if n.is_dir { MIN_DIR } else { MIN_FILE };
            if now < tracked && then.is_none() {
                continue;
            }
            let delta = i128::from(now) - i128::from(then.unwrap_or(0));
            delta_of.insert(id, delta);
            if delta.unsigned_abs() >= u128::from(MIN_DIR) {
                found.push(Change {
                    id,
                    delta,
                    is_new: then.is_none(),
                });
            }
            if n.is_dir {
                stack.push((id, path.clone()));
            }
            seen.insert(path);
        }
    }

    // Drop directories whose change sits almost entirely in one child.
    found.retain(|c| {
        let biggest_child = tree
            .children(c.id)
            .filter_map(|ch| delta_of.get(&ch))
            .filter(|d| d.signum() == c.delta.signum())
            .map(|d| d.unsigned_abs())
            .max()
            .unwrap_or(0);
        biggest_child as f64 <= c.delta.unsigned_abs() as f64 * WRAPPER_SHARE
    });
    found.sort_by_key(|c| std::cmp::Reverse(c.delta));
    let truncated = found.len() > LIMIT;
    found.truncate(LIMIT);
    let rows = found
        .iter()
        .map(|c| {
            let now = tree.node(c.id).size.get(mode);
            let detail = if c.is_new {
                tf!("yeni (+{})", "new (+{})", fmt_size(now))
            } else {
                fmt_delta(now, (i128::from(now) - c.delta) as u64)
            };
            Row::single(tree, base, c.id, mode, detail)
        })
        .collect();

    // Entries that are gone; only the topmost of a removed subtree.
    let mut gone: Vec<(&str, u64)> = old
        .iter()
        .filter(|(p, _)| !seen.contains(**p) && **p != base_rel)
        .filter(|(p, _)| {
            p.rsplit_once('/')
                .is_none_or(|(parent, _)| seen.contains(parent) || parent == base_rel)
        })
        .map(|(p, s)| (*p, *s))
        .collect();
    gone.sort_by_key(|&(p, s)| (std::cmp::Reverse(s), p));
    let note = if gone.is_empty() {
        String::from(t!(
            "Silinen büyük öğe yok.",
            "No large entries were removed."
        ))
    } else {
        let total: u64 = gone.iter().map(|g| g.1).sum();
        let names: Vec<String> = gone
            .iter()
            .take(3)
            .map(|(p, s)| {
                let p = p.strip_prefix(&format!("{base_rel}/")).unwrap_or(p);
                format!("{p} ({})", fmt_size(*s))
            })
            .collect();
        tf!(
            "Silinenler: {}, {} — {}",
            "Removed: {}, {} — {}",
            crate::i18n::count(gone.len() as u64, "öğe", "item", "items"),
            fmt_size(total),
            names.join(", ")
        )
    };
    (rows, truncated, note)
}

/// Path of `id` relative to the root, '/'-joined (empty for the root).
fn rel_path(tree: &Tree, id: NodeId) -> String {
    let mut names = Vec::new();
    let mut cur = id;
    while let Some(parent) = tree.parent(cur) {
        names.push(tree.name(cur));
        cur = parent;
    }
    names.reverse();
    names.join("/")
}

fn prune(dir: &Path) -> io::Result<()> {
    for old in list(dir).into_iter().skip(KEEP) {
        fs::remove_file(old.file)?;
    }
    Ok(())
}

fn open(file: &Path) -> io::Result<impl Read> {
    Ok(DeflateDecoder::new(BufReader::new(File::open(file)?)))
}

fn write_header(w: &mut impl Write, h: &Header) -> io::Result<()> {
    w.write_all(MAGIC)?;
    w.write_all(&VERSION.to_le_bytes())?;
    write_str(w, &h.root.to_string_lossy())?;
    for n in [h.time, h.total.apparent, h.total.disk, h.files] {
        w.write_all(&n.to_le_bytes())?;
    }
    Ok(())
}

fn read_header(r: &mut impl Read) -> io::Result<Header> {
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic)?;
    let mut version = [0u8; 2];
    r.read_exact(&mut version)?;
    if &magic != MAGIC || u16::from_le_bytes(version) != VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a rustClean snapshot",
        ));
    }
    let root = PathBuf::from(read_str(r)?);
    let time = read_u64(r)?;
    let total = Size {
        apparent: read_u64(r)?,
        disk: read_u64(r)?,
    };
    let files = read_u64(r)?;
    Ok(Header {
        root,
        time,
        total,
        files,
    })
}

fn write_str(w: &mut impl Write, s: &str) -> io::Result<()> {
    w.write_all(&(s.len() as u32).to_le_bytes())?;
    w.write_all(s.as_bytes())
}

fn read_str(r: &mut impl Read) -> io::Result<String> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let mut buf = vec![0u8; u32::from_le_bytes(len) as usize];
    r.read_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn read_u64(r: &mut impl Read) -> io::Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    fn sz(n: u64) -> Size {
        Size {
            apparent: n,
            disk: n,
        }
    }

    fn sample_tree() -> Tree {
        let mut t = Tree::new(Path::new("/data"));
        let big = t.push(ROOT, "big", true, sz(0));
        let inner = t.push(big, "iç klasör", true, sz(0));
        t.push(inner, "film.mkv", false, sz(150 * MIB));
        t.push(big, "small.txt", false, sz(10));
        let tiny = t.push(ROOT, "tiny", true, sz(0));
        t.push(tiny, "a", false, sz(100));
        t.finalize();
        t
    }

    #[test]
    fn capture_keeps_large_entries_only() {
        let snap = capture(&sample_tree(), 1234);
        let mut paths: Vec<&str> = snap.entries.iter().map(|e| e.path.as_str()).collect();
        paths.sort();
        assert_eq!(
            paths,
            vec!["big", "big/iç klasör", "big/iç klasör/film.mkv"]
        );
        assert_eq!(snap.header.time, 1234);
        assert_eq!(snap.header.files, 3);
        assert_eq!(snap.header.total, sz(150 * MIB + 110));
    }

    #[test]
    fn save_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let snap = capture(&sample_tree(), 1000);
        let file = save(dir.path(), &snap).unwrap();
        assert_eq!(load(&file).unwrap(), snap);
        let saved = list(dir.path());
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].header, snap.header);
    }

    #[test]
    fn keeps_newest_ten() {
        let dir = tempfile::tempdir().unwrap();
        let base = capture(&sample_tree(), 0);
        for time in 1..=13 {
            let mut snap = base.clone();
            snap.header.time = time;
            save(dir.path(), &snap).unwrap();
        }
        // Same second twice: both kept, under different names.
        save(dir.path(), &base_at(&base, 13)).unwrap();
        let times: Vec<u64> = list(dir.path()).iter().map(|s| s.header.time).collect();
        assert_eq!(times, vec![13, 13, 12, 11, 10, 9, 8, 7, 6, 5]);
    }

    fn base_at(s: &Snapshot, time: u64) -> Snapshot {
        let mut s = s.clone();
        s.header.time = time;
        s
    }

    #[test]
    fn ignores_foreign_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("junk.rcs"), b"not deflate data").unwrap();
        fs::write(dir.path().join("notes.txt"), b"hello").unwrap();
        assert!(list(dir.path()).is_empty());
    }

    #[test]
    fn changes_since_snapshot() {
        // Then: big/ (150 MiB film), old/ (5 MiB), stable/ (2 MiB).
        let mut then = Tree::new(Path::new("/data"));
        let big = then.push(ROOT, "big", true, sz(0));
        let inner = then.push(big, "iç klasör", true, sz(0));
        then.push(inner, "film.mkv", false, sz(150 * MIB));
        let old = then.push(ROOT, "old", true, sz(0));
        let old_sub = then.push(old, "sub", true, sz(0));
        then.push(old_sub, "x", false, sz(5 * MIB));
        let stable = then.push(ROOT, "stable", true, sz(0));
        then.push(stable, "s", false, sz(2 * MIB));
        then.finalize();
        let snap = capture(&then, 100);

        // Now: the film's folder grew by 40 MiB, old/ is gone, new/ appeared.
        let mut now = Tree::new(Path::new("/data"));
        let big = now.push(ROOT, "big", true, sz(0));
        let inner = now.push(big, "iç klasör", true, sz(0));
        now.push(inner, "film.mkv", false, sz(150 * MIB));
        now.push(inner, "film2.mkv", false, sz(40 * MIB));
        let new = now.push(ROOT, "new", true, sz(0));
        now.push(new, "n", false, sz(3 * MIB));
        let stable = now.push(ROOT, "stable", true, sz(0));
        now.push(stable, "s", false, sz(2 * MIB));
        now.finalize();

        let (rows, truncated, note) = changes(&now, ROOT, &snap, SizeMode::Disk);
        let got: Vec<(String, &str)> = rows
            .iter()
            .map(|r| (r.label.replace('\\', "/"), r.detail.as_str()))
            .collect();
        // "big/" only wraps "big/iç klasör/", so it is not listed.
        assert_eq!(
            got,
            vec![
                ("big/iç klasör/".to_string(), "+40.0 MiB"),
                ("new/".to_string(), "yeni (+3.0 MiB)")
            ]
        );
        assert!(!truncated);
        assert_eq!(note, "Silinenler: 1 öğe, 5.0 MiB — old (5.0 MiB)");

        // Below "big" only.
        let (rows, _, note) = changes(&now, big, &snap, SizeMode::Disk);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label.replace('\\', "/"), "iç klasör/");
        assert_eq!(note, "Silinen büyük öğe yok.");
    }

    #[test]
    fn dir_depends_on_root() {
        let a = dir_for(Path::new("/a")).unwrap();
        let b = dir_for(Path::new("/b")).unwrap();
        assert_ne!(a, b);
        assert_eq!(a, dir_for(Path::new("/a")).unwrap());
    }
}
