//! The deletion log: one JSON line per entry moved to the trash, in
//! `deletions.jsonl` in the data directory, newest last. Only moves that
//! succeeded are written. The file keeps the newest `KEEP` entries.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use crate::reports::ReportKind;
use crate::tools::ToolKind;
use crate::tree::Size;

/// Entries kept; the file is cut back to this once it is 10 % over.
pub const KEEP: usize = 10_000;

/// How an entry came to be moved to the trash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Via {
    /// From the folder list or the treemap.
    List,
    Summary,
    Report(ReportKind),
    Search,
    /// The changes since an earlier scan.
    Changes,
    Basket,
    /// Uninstalling the app of this name.
    Uninstall(String),
    /// The contents of a folder, by a developer tool cleanup.
    Tool(ToolKind),
}

impl Via {
    /// What the log screen shows.
    pub fn label(&self) -> String {
        match self {
            Via::List => t!("liste", "list").into(),
            Via::Summary => t!("özet", "summary").into(),
            Via::Report(kind) => tf!("rapor: {}", "report: {}", kind.label()),
            Via::Search => t!("arama", "search").into(),
            Via::Changes => t!("değişenler", "changes").into(),
            Via::Basket => t!("sepet", "basket").into(),
            Via::Uninstall(app) => tf!("kaldırma: {app}", "uninstall: {app}"),
            Via::Tool(kind) => tf!("araç: {}", "tool: {}", kind.label()),
        }
    }

    /// `(via, detail)` as written to the file.
    fn codes(&self) -> (&'static str, String) {
        match self {
            Via::List => ("list", String::new()),
            Via::Summary => ("summary", String::new()),
            Via::Report(kind) => ("report", kind.code()),
            Via::Search => ("search", String::new()),
            Via::Changes => ("changes", String::new()),
            Via::Basket => ("basket", String::new()),
            Via::Uninstall(app) => ("uninstall", app.clone()),
            Via::Tool(kind) => ("tool", kind.code()),
        }
    }

    fn parse(via: &str, detail: &str) -> Option<Via> {
        Some(match via {
            "list" => Via::List,
            "summary" => Via::Summary,
            "report" => Via::Report(ReportKind::from_code(detail)?),
            "search" => Via::Search,
            "changes" => Via::Changes,
            "basket" => Via::Basket,
            "uninstall" => Via::Uninstall(detail.to_string()),
            "tool" => Via::Tool(ToolKind::from_code(detail)?),
            _ => return None,
        })
    }
}

/// One entry moved to the trash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Seconds since the Unix epoch.
    pub time: u64,
    pub path: String,
    pub size: Size,
    pub via: Via,
}

impl Entry {
    fn to_json(&self) -> String {
        let (via, detail) = self.via.codes();
        serde_json::json!({
            "time": self.time,
            "path": self.path,
            "apparent": self.size.apparent,
            "disk": self.size.disk,
            "via": via,
            "detail": detail,
        })
        .to_string()
    }

    fn from_json(line: &str) -> Option<Entry> {
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        let detail = v.get("detail").and_then(|d| d.as_str()).unwrap_or("");
        Some(Entry {
            time: v.get("time")?.as_u64()?,
            path: v.get("path")?.as_str()?.to_string(),
            size: Size {
                apparent: v.get("apparent")?.as_u64()?,
                disk: v.get("disk")?.as_u64()?,
            },
            via: Via::parse(v.get("via")?.as_str()?, detail)?,
        })
    }
}

fn file() -> Option<PathBuf> {
    Some(crate::paths::data_dir()?.join("deletions.jsonl"))
}

/// Appends `entries`. Failures are ignored: the deletion itself happened,
/// and a log that cannot be written is not worth an error dialog.
pub fn append(entries: &[Entry]) {
    if entries.is_empty() {
        return;
    }
    let Some(file) = file() else {
        return;
    };
    if let Some(dir) = file.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let text: String = entries.iter().map(|e| e.to_json() + "\n").collect();
    // One write in append mode, so a line from the tools' thread and one from
    // the interface never interleave.
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&file) {
        let _ = f.write_all(text.as_bytes());
    }
    trim(&file, KEEP);
}

/// Cuts the file back to the newest `keep` lines once it is 10 % over.
fn trim(file: &std::path::Path, keep: usize) {
    let Ok(text) = fs::read_to_string(file) else {
        return;
    };
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= keep + keep / 10 {
        return;
    }
    let kept: String = lines[lines.len() - keep..]
        .iter()
        .map(|l| format!("{l}\n"))
        .collect();
    let tmp = file.with_extension("jsonl.tmp");
    if fs::write(&tmp, kept).is_ok() {
        let _ = fs::rename(tmp, file);
    }
}

/// The log, newest first. Lines that cannot be read are skipped.
pub fn read() -> Vec<Entry> {
    let Some(text) = file().and_then(|f| fs::read_to_string(f).ok()) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry> = text.lines().filter_map(Entry::from_json).collect();
    entries.reverse();
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(time: u64, path: &str, via: Via) -> Entry {
        Entry {
            time,
            path: path.into(),
            size: Size {
                apparent: 100,
                disk: 4096,
            },
            via,
        }
    }

    #[test]
    fn round_trips_every_way() {
        for via in [
            Via::List,
            Via::Summary,
            Via::Report(ReportKind::Downloads),
            Via::Search,
            Via::Changes,
            Via::Basket,
            Via::Uninstall("Sketch \"Pro\"".into()),
            Via::Tool(ToolKind::Npm),
        ] {
            let e = entry(42, "/a/b \"c\".txt", via);
            assert_eq!(Entry::from_json(&e.to_json()), Some(e));
        }
        assert_eq!(Entry::from_json("not json"), None);
        assert_eq!(
            Entry::from_json(r#"{"time":1,"path":"/x","apparent":1,"disk":1,"via":"magic"}"#),
            None
        );
    }

    #[test]
    fn trims_to_the_newest() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("log.jsonl");
        let lines: String = (0..120).map(|i| format!("{i}\n")).collect();
        fs::write(&f, &lines).unwrap();
        // 120 lines with keep = 100: over by more than 10 %.
        trim(&f, 100);
        let text = fs::read_to_string(&f).unwrap();
        let kept: Vec<&str> = text.lines().collect();
        assert_eq!(kept.len(), 100);
        assert_eq!(kept[0], "20");
        assert_eq!(kept[99], "119");
        // Within 10 %: left as it is.
        let lines: String = (0..105).map(|i| format!("{i}\n")).collect();
        fs::write(&f, &lines).unwrap();
        trim(&f, 100);
        assert_eq!(fs::read_to_string(&f).unwrap().lines().count(), 105);
    }
}
