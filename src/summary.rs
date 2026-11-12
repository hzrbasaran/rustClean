//! The numbers `--summary` prints, as a value that serializes to JSON.

use std::path::PathBuf;

use serde::Serialize;

use crate::scanner::ScanResult;
use crate::tree::ROOT;

/// What a scan found, in raw units (bytes, seconds) so scripts can read it.
#[derive(Debug, Serialize)]
pub struct ScanSummary {
    pub root: PathBuf,
    pub disk_size: u64,
    pub apparent_size: u64,
    pub files: u64,
    pub entries: u64,
    pub inaccessible: u64,
    pub elapsed_secs: f64,
}

impl ScanSummary {
    pub fn of(res: &ScanResult) -> Self {
        let t = &res.tree;
        let root = t.node(ROOT);
        Self {
            root: t.root_path().to_path_buf(),
            disk_size: root.size.disk,
            apparent_size: root.size.apparent,
            files: u64::from(root.file_count),
            entries: (t.len() - 1) as u64,
            inaccessible: res.errors,
            elapsed_secs: res.elapsed.as_secs_f64(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use super::*;
    use crate::tree::{Size, Tree};

    #[test]
    fn serializes_raw_numbers() {
        let mut tree = Tree::new(Path::new("/r"));
        let dir = tree.push(ROOT, "d", true, Size::default());
        tree.push(
            dir,
            "a",
            false,
            Size {
                apparent: 5000,
                disk: 8192,
            },
        );
        tree.push(
            ROOT,
            "b",
            false,
            Size {
                apparent: 3000,
                disk: 4096,
            },
        );
        tree.finalize();
        let res = ScanResult {
            tree,
            errors: 2,
            elapsed: Duration::from_millis(1500),
        };
        let v = serde_json::to_value(ScanSummary::of(&res)).unwrap();
        assert_eq!(v["root"], "/r");
        assert_eq!(v["apparent_size"], 8000);
        assert_eq!(v["disk_size"], 12288);
        assert_eq!(v["files"], 2);
        assert_eq!(v["entries"], 3);
        assert_eq!(v["inaccessible"], 2);
        assert_eq!(v["elapsed_secs"], 1.5);
    }
}
