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

use crate::tree::{NodeId, Size, Tree, ROOT};

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

/// Where snapshots of `root` are kept. `RUSTCLEAN_DATA_DIR` overrides the
/// platform data directory (used by tests).
pub fn dir_for(root: &Path) -> Option<PathBuf> {
    let base = match std::env::var_os("RUSTCLEAN_DATA_DIR") {
        Some(d) => PathBuf::from(d),
        None => dirs::data_dir()?.join("rustClean"),
    };
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
    fn dir_depends_on_root() {
        let a = dir_for(Path::new("/a")).unwrap();
        let b = dir_for(Path::new("/b")).unwrap();
        assert_ne!(a, b);
        assert_eq!(a, dir_for(Path::new("/a")).unwrap());
    }
}
