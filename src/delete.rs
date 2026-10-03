//! Moving entries to the trash.
//!
//! The move runs on its own thread so a slow volume cannot freeze the UI.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use crate::tree::NodeId;

/// Refuses paths that must not be trashed: entries that no longer exist and
/// mount points of other volumes.
pub fn check(path: &Path, mount_points: &[PathBuf]) -> Result<(), String> {
    let md = fs::symlink_metadata(path).map_err(|_| "Öğe artık mevcut değil.".to_string())?;
    if mount_points.iter().any(|m| m == path) {
        return Err("Bu bir disk bağlama noktası, silinemez.".into());
    }
    #[cfg(unix)]
    if md.is_dir() {
        use std::os::unix::fs::MetadataExt;
        let parent_dev = path
            .parent()
            .and_then(|p| fs::symlink_metadata(p).ok())
            .map(|p| p.dev());
        if parent_dev.is_some_and(|dev| dev != md.dev()) {
            return Err("Bu klasör başka bir diske ait, silinemez.".into());
        }
    }
    #[cfg(not(unix))]
    let _ = md;
    Ok(())
}

/// An entry that could not be moved to the trash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub path: String,
    pub error: String,
    /// What the user can do about it, when the error is recognized.
    pub hint: Option<&'static str>,
}

impl Failure {
    pub fn new(path: String, raw_error: &str) -> Self {
        Self {
            path,
            error: tidy(raw_error),
            hint: explain(raw_error),
        }
    }
}

/// Makes the trash library's error readable: unwraps `Unknown {
/// description: "..." }` and undoes its escaping.
pub fn tidy(raw: &str) -> String {
    let inner = raw
        .split_once("description: \"")
        .and_then(|(_, rest)| rest.rsplit_once('"').map(|(d, _)| d))
        .unwrap_or(raw);
    let text = inner
        .replace("\\\"", "\"")
        .replace("\\'", "'")
        .replace("\\n", " ")
        .replace("\\\\", "\\");
    // The path is shown separately; keep only the reason.
    let text = match text.strip_prefix("While deleting '") {
        // `While deleting '"<path>"', reason` (or `': reason`).
        Some(rest) => match rest.find("\"'") {
            Some(end) => rest[end + 2..].trim_start_matches([',', ':']).to_string(),
            None => rest.to_string(),
        },
        None => text,
    };
    text.trim()
        .trim_start_matches("`trashItemAtURL` failed: ")
        .trim()
        .to_string()
}

/// A hint for errors the user can fix.
pub fn explain(raw: &str) -> Option<&'static str> {
    let e = raw.to_lowercase();
    let permission = [
        "not permitted",
        "permission",
        "code=513",
        "code=257",
        "eperm",
        "eacces",
    ]
    .iter()
    .any(|k| e.contains(k));
    if permission {
        return Some(
            "macOS izin vermedi. Öğe kilitli olabilir (Finder → Bilgi Al → Kilitli) ya da \
             korumalı bir konumdadır (ör. Library/Containers). Korumalı konumlar için \
             Sistem Ayarları → Gizlilik ve Güvenlik → Tam Disk Erişimi'nden terminal \
             uygulamanızı ekleyip terminali yeniden açın.",
        );
    }
    if e.contains("no such file") || e.contains("code=4") || e.contains("couldn’t be found") {
        return Some("Öğe artık yerinde değil; başka bir program silmiş ya da taşımış olabilir.");
    }
    if e.contains("in use") || e.contains("busy") {
        return Some("Öğe kullanımda. İlgili uygulamayı kapatıp yeniden deneyin.");
    }
    None
}

/// Moves a batch of entries to the trash, one after another.
pub struct Deletion {
    rx: Receiver<(NodeId, Result<(), String>)>,
    pub total: usize,
    pub done: usize,
}

impl Deletion {
    pub fn start(items: Vec<(NodeId, PathBuf)>) -> Self {
        let total = items.len();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let ctx = trash_context();
            for (id, path) in items {
                let res = ctx.delete(&path).map_err(|e| e.to_string());
                if tx.send((id, res)).is_err() {
                    break;
                }
            }
        });
        Self { rx, total, done: 0 }
    }

    /// Outcomes that arrived since the last call.
    pub fn poll(&mut self) -> Vec<(NodeId, Result<(), String>)> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(r) => {
                    self.done += 1;
                    out.push(r);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.done = self.total; // worker died; nothing more will come
                    break;
                }
            }
        }
        out
    }

    pub fn finished(&self) -> bool {
        self.done >= self.total
    }
}

pub(crate) fn trash_context() -> trash::TrashContext {
    #[allow(unused_mut)]
    let mut ctx = trash::TrashContext::default();
    // The default Finder method needs an Automation permission prompt per
    // terminal app and blocks until it is answered. NSFileManager needs no
    // permission; the trade-off is that Finder may not offer "Put Back".
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        ctx.set_delete_method(DeleteMethod::NsFileManager);
    }
    ctx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidies_and_explains_errors() {
        let raw = r#"Unknown { description: "While deleting '\"/Users/me/Library/Containers/llc.x\"': Error Domain=NSCocoaErrorDomain Code=513 \"“llc.x” couldn’t be moved to the trash because you don’t have permission to access it.\"" }"#;
        let text = tidy(raw);
        assert!(
            text.starts_with("Error Domain=NSCocoaErrorDomain Code=513"),
            "{text}"
        );
        assert!(!text.contains("\\\""));
        // The real format: the path is dropped, the reason kept.
        let real = r#"Unknown { description: "While deleting '\"/tmp/a b/x.bin\"', `trashItemAtURL` failed: “x.bin” couldn’t be moved to the trash because you don’t have permission to access it." }"#;
        assert_eq!(
            tidy(real),
            "“x.bin” couldn’t be moved to the trash because you don’t have permission to access it."
        );
        assert!(explain(raw).unwrap().contains("Tam Disk Erişimi"));
        assert!(explain("Operation not permitted (os error 1)").is_some());
        assert!(explain("No such file or directory")
            .unwrap()
            .contains("yerinde değil"));
        assert_eq!(explain("something else"), None);
        assert_eq!(tidy("plain message"), "plain message");
    }

    #[test]
    fn allows_regular_entries() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        fs::write(&file, b"x").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();

        assert!(check(&file, &[]).is_ok());
        assert!(check(&dir.path().join("sub"), &[]).is_ok());
    }

    #[test]
    fn refuses_missing_and_mount_points() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("mnt");
        fs::create_dir(&sub).unwrap();

        assert!(check(&dir.path().join("nope"), &[]).is_err());
        assert!(check(&sub, std::slice::from_ref(&sub)).is_err());
    }
}
