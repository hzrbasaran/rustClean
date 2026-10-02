//! Discovery of the disks / volumes mounted on this machine.

use std::path::{Path, PathBuf};
use sysinfo::Disks;

#[derive(Debug, Clone)]
pub struct DiskInfo {
    pub name: String,
    pub mount_point: PathBuf,
    pub fs_type: String,
    pub total: u64,
    pub available: u64,
    pub removable: bool,
}

impl DiskInfo {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    pub fn usage_ratio(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.used() as f64 / self.total as f64
        }
    }
}

/// Disks worth showing to the user: virtual/system mounts are hidden and
/// duplicate mount points collapsed.
pub fn list_disks() -> Vec<DiskInfo> {
    let disks = Disks::new_with_refreshed_list();
    let mut out: Vec<DiskInfo> = disks
        .list()
        .iter()
        .map(|d| {
            let mount_point = d.mount_point().to_path_buf();
            let mut name = d.name().to_string_lossy().into_owned();
            if name.is_empty() {
                name = mount_point.display().to_string();
            }
            DiskInfo {
                name,
                mount_point,
                fs_type: d.file_system().to_string_lossy().into_owned(),
                total: d.total_space(),
                available: d.available_space(),
                removable: d.is_removable(),
            }
        })
        .filter(|d| is_user_visible(&d.mount_point, &d.fs_type))
        .collect();
    out.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
    out.dedup_by(|a, b| a.mount_point == b.mount_point);
    out
}

/// Every mount point on the system, including hidden ones. The scanner skips
/// these (other than its own root) so other volumes are never counted twice.
pub fn all_mount_points() -> Vec<PathBuf> {
    Disks::new_with_refreshed_list()
        .list()
        .iter()
        .map(|d| d.mount_point().to_path_buf())
        .collect()
}

#[cfg(target_os = "macos")]
fn is_user_visible(mount: &Path, _fs: &str) -> bool {
    // "/" is the main disk; external and extra volumes live under /Volumes.
    // Everything else (/System/Volumes/*, simulator and cryptex images) is
    // an implementation detail of macOS.
    mount == Path::new("/") || mount.starts_with("/Volumes")
}

#[cfg(target_os = "linux")]
fn is_user_visible(mount: &Path, fs: &str) -> bool {
    const PSEUDO_FS: &[&str] = &[
        "autofs",
        "bpf",
        "cgroup",
        "cgroup2",
        "configfs",
        "debugfs",
        "devpts",
        "devtmpfs",
        "efivarfs",
        "fusectl",
        "hugetlbfs",
        "mqueue",
        "nsfs",
        "overlay",
        "proc",
        "pstore",
        "ramfs",
        "securityfs",
        "squashfs",
        "sysfs",
        "tmpfs",
        "tracefs",
    ];
    !PSEUDO_FS.contains(&fs)
        && !mount.starts_with("/proc")
        && !mount.starts_with("/sys")
        && !mount.starts_with("/run")
        && !mount.starts_with("/snap")
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn is_user_visible(_mount: &Path, _fs: &str) -> bool {
    true
}
