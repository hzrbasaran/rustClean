//! Writing a new file without ever replacing one: what `o` (CSV / JSON) and
//! `w` (the treemap page) save. The file goes to the first folder of a list
//! that takes it, as `<base>.<ext>`, or `<base>-1.<ext>`, `<base>-2.<ext>`…
//! when that name is taken.

use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

/// Where files are saved: the working folder, else the home folder. Test
/// builds use a folder under the temp directory instead, so `cargo test`
/// never writes into the repository or the home folder.
pub fn default_dirs() -> Vec<PathBuf> {
    if cfg!(test) {
        let dir =
            std::env::temp_dir().join(format!("rustclean-test-export-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        return vec![dir];
    }
    std::env::current_dir()
        .ok()
        .into_iter()
        .chain(dirs::home_dir())
        .collect()
}

/// The local time for file names: `20261005-143012`.
pub fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// Creates a new file in the first of `dirs` that takes it, fills it with
/// `write` and returns its path. A folder that cannot be written, or a
/// write that fails, moves on to the next folder; a half-written file is
/// removed.
pub fn create(
    dirs: &[PathBuf],
    base: &str,
    ext: &str,
    write: impl Fn(&mut dyn Write) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let mut last = io::Error::new(io::ErrorKind::NotFound, "no folder to write to");
    for dir in dirs {
        match create_in(dir, base, ext, &write) {
            Ok(path) => return Ok(path),
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn create_in(
    dir: &Path,
    base: &str,
    ext: &str,
    write: &impl Fn(&mut dyn Write) -> io::Result<()>,
) -> io::Result<PathBuf> {
    for n in 0..1000 {
        let name = match n {
            0 => format!("{base}.{ext}"),
            n => format!("{base}-{n}.{ext}"),
        };
        let path = dir.join(name);
        let file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let mut w = BufWriter::new(file);
        let written = write(&mut w)
            .and_then(|()| w.flush())
            .and_then(|()| w.get_ref().sync_all());
        if let Err(e) = written {
            drop(w);
            let _ = fs::remove_file(&path);
            return Err(e);
        }
        return Ok(path);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many files with the same name",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &'static str) -> impl Fn(&mut dyn Write) -> io::Result<()> {
        move |w| w.write_all(s.as_bytes())
    }

    fn name(p: &Path) -> String {
        p.file_name().unwrap().to_string_lossy().into_owned()
    }

    #[test]
    fn never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = [dir.path().to_path_buf()];
        let a = create(&dirs, "rustclean-x", "csv", text("one")).unwrap();
        let b = create(&dirs, "rustclean-x", "csv", text("two")).unwrap();
        let c = create(&dirs, "rustclean-x", "csv", text("three")).unwrap();
        assert_eq!(name(&a), "rustclean-x.csv");
        assert_eq!(name(&b), "rustclean-x-1.csv");
        assert_eq!(name(&c), "rustclean-x-2.csv");
        assert_eq!(fs::read_to_string(&a).unwrap(), "one");
        assert_eq!(fs::read_to_string(&c).unwrap(), "three");
    }

    #[test]
    fn falls_back_to_the_next_folder() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        let path = create(
            &[missing, dir.path().to_path_buf()],
            "rustclean-x",
            "html",
            text("page"),
        )
        .unwrap();
        assert_eq!(path.parent(), Some(dir.path()));
        assert!(create(&[], "rustclean-x", "html", text("page")).is_err());
    }

    #[test]
    fn a_failed_write_leaves_no_file() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let fail_in_first = |w: &mut dyn Write| {
            w.write_all(b"half")?;
            w.flush()?;
            Err(io::Error::other("disk full"))
        };
        let dirs = [first.path().to_path_buf()];
        let err = create(&dirs, "rustclean-x", "json", fail_in_first).unwrap_err();
        assert_eq!(err.to_string(), "disk full");
        assert_eq!(fs::read_dir(first.path()).unwrap().count(), 0);

        // A write that fails in one folder is tried again in the next.
        let tries = std::cell::Cell::new(0);
        let once = |w: &mut dyn Write| {
            tries.set(tries.get() + 1);
            if tries.get() == 1 {
                return Err(io::Error::other("disk full"));
            }
            w.write_all(b"whole")
        };
        let dirs = [first.path().to_path_buf(), second.path().to_path_buf()];
        let path = create(&dirs, "rustclean-x", "json", once).unwrap();
        assert_eq!(path.parent(), Some(second.path()));
        assert_eq!(fs::read_to_string(path).unwrap(), "whole");
        assert_eq!(fs::read_dir(first.path()).unwrap().count(), 0);
    }
}
