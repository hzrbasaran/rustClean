//! Scanning a real folder tree.

use super::{find, scan, Fixture, KIB, MIB};
use crate::tree::{SizeMode, ROOT};

#[test]
fn sizes_and_counts_add_up() {
    let f = Fixture::standard();
    let res = scan(f.root());
    let t = &res.tree;
    assert_eq!(res.errors, 0);
    assert_eq!(t.node(ROOT).size.apparent, f.bytes);
    assert_eq!(u64::from(t.node(ROOT).file_count), f.files);
    assert_eq!(t.node(find(t, "Media")).size.apparent, 2 * 1536 * KIB);
    let downloads = find(t, "Downloads");
    assert_eq!(t.node(downloads).size.apparent, 550 * KIB);
    assert_eq!(t.node(downloads).file_count, 3);
    assert_eq!(
        t.path_of(find(t, "Projects/web/node_modules/pkg/index.js")),
        std::path::absolute(f.path("Projects/web/node_modules/pkg/index.js")).unwrap()
    );
}

#[test]
fn sparse_files_take_little_disk_space() {
    let f = Fixture::standard();
    let t = scan(f.root()).tree;
    let iso = t.node(find(&t, "Archive/old.iso"));
    assert_eq!(iso.size.get(SizeMode::Apparent), 100 * MIB + 1);
    // Windows may allocate the file; elsewhere it is sparse.
    if cfg!(unix) {
        assert!(iso.size.get(SizeMode::Disk) < MIB, "{:?}", iso.size);
    }
}

#[test]
fn modification_times_are_read() {
    let f = Fixture::standard();
    let t = scan(f.root()).tree;
    let age = |rel: &str| super::now() - u64::from(t.node(find(&t, rel)).modified);
    let day = 86_400;
    assert!((399 * day..=401 * day).contains(&age("Downloads/setup.dmg")));
    assert!(age("Downloads/notes.txt") < day);
}

#[cfg(unix)]
#[test]
fn symlinks_are_not_followed() {
    let f = Fixture::standard();
    std::os::unix::fs::symlink(f.path("Media"), f.path("Docs/media-link")).unwrap();
    let t = scan(f.root()).tree;
    let link = find(&t, "Docs/media-link");
    assert!(!t.node(link).is_dir);
    assert_eq!(t.children(link).count(), 0);
    // The linked folder is counted once.
    assert_eq!(u64::from(t.node(ROOT).file_count), f.files + 1);
    assert!(t.node(ROOT).size.apparent < f.bytes + KIB);
}

#[cfg(unix)]
#[test]
fn unreadable_folders_are_counted_and_skipped() {
    use std::os::unix::fs::PermissionsExt;

    let mut f = Fixture::standard();
    f.file("Private/secret.txt", 10 * KIB, b's', 0);
    let private = f.path("Private");
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o000)).unwrap();
    // root can read anything; the check is meaningless then.
    let readable = std::fs::read_dir(&private).is_ok();
    let res = scan(f.root());
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755)).unwrap();
    if readable {
        return;
    }
    assert_eq!(res.errors, 1);
    let t = &res.tree;
    assert_eq!(t.children(find(t, "Private")).count(), 0);
    // Everything else is still there.
    assert_eq!(t.node(ROOT).size.apparent, f.bytes - 10 * KIB);
}
