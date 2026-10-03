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
