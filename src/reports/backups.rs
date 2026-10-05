//! iPhone and iPad backups made by Finder or iTunes / Apple Devices: one row
//! per backup folder in `MobileSync/Backup`, with the device, its model and
//! system version and the date of the backup, read from the backup's
//! `Info.plist` (and `Manifest.plist` for encryption).
//!
//! The folders live in `~/Library/Application Support/MobileSync/Backup` on
//! macOS, and in `%APPDATA%\Apple Computer\MobileSync\Backup` or
//! `%USERPROFILE%\Apple\MobileSync\Backup` on Windows. On macOS the folder
//! needs Full Disk Access: without it the scan sees it empty, and the report
//! says how to grant it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::lists::Row;
use crate::tree::{NodeId, SizeMode, Tree};

use super::{AgeFilter, Report, LIMIT};

/// What a backup's `Info.plist` and `Manifest.plist` tell about it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackupInfo {
    /// "Ali's iPhone".
    pub device: Option<String>,
    /// "iPhone 13 Pro", or the product type ("iPhone14,2") when the name is
    /// not recorded.
    pub model: Option<String>,
    /// "iPhone14,2": tells iPads from iPhones.
    pub product_type: Option<String>,
    /// The system version, "17.5.1".
    pub version: Option<String>,
    /// The last backup, seconds since the Unix epoch.
    pub date: Option<u32>,
    pub encrypted: bool,
    /// Identifies the device across backups (its UDID).
    pub device_id: Option<String>,
}

impl BackupInfo {
    /// Reads `Info.plist` and `Manifest.plist` in the backup folder `dir`;
    /// `None` when there is no readable `Info.plist`.
    pub fn read(dir: &Path) -> Option<Self> {
        let info = plist::Value::from_file(dir.join("Info.plist"))
            .ok()?
            .into_dictionary()?;
        let text = |key: &str| {
            info.get(key)
                .and_then(plist::Value::as_string)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let date = info
            .get("Last Backup Date")
            .and_then(plist::Value::as_date)
            .and_then(|d| SystemTime::from(d).duration_since(UNIX_EPOCH).ok())
            .and_then(|d| u32::try_from(d.as_secs()).ok());
        let encrypted = plist::Value::from_file(dir.join("Manifest.plist"))
            .ok()
            .and_then(plist::Value::into_dictionary)
            .and_then(|m| m.get("IsEncrypted").and_then(plist::Value::as_boolean))
            .unwrap_or(false);
        Some(Self {
            device: text("Device Name").or_else(|| text("Display Name")),
            model: text("Product Name").or_else(|| text("Product Type")),
            product_type: text("Product Type"),
            version: text("Product Version"),
            date,
            encrypted,
            device_id: text("Target Identifier").or_else(|| text("Unique Identifier")),
        })
    }
}

// The screen snapshots (Unix only) draw a hand-built tree whose folders are
// not on disk: they give the backup details here instead.
#[cfg(all(test, unix))]
thread_local! {
    static FAKE: std::cell::RefCell<Option<HashMap<String, BackupInfo>>> =
        const { std::cell::RefCell::new(None) };
}

/// Runs `f` with the details of backup folders taken from `infos` (by
/// folder name) instead of their files (test builds only).
#[cfg(all(test, unix))]
pub fn with_fake_backups<R>(infos: HashMap<String, BackupInfo>, f: impl FnOnce() -> R) -> R {
    let before = FAKE.replace(Some(infos));
    let out = f();
    FAKE.set(before);
    out
}

fn read_info(tree: &Tree, id: NodeId) -> Option<BackupInfo> {
    #[cfg(all(test, unix))]
    if let Some(info) =
        FAKE.with_borrow(|fake| fake.as_ref().map(|infos| infos.get(tree.name(id)).cloned()))
    {
        return info;
    }
    BackupInfo::read(&tree.path_of(id))
}

/// Where Finder and iTunes keep backups on this system.
fn known_locations() -> Vec<PathBuf> {
    // Tests never look at the real folders.
    if cfg!(test) {
        return Vec::new();
    }
    let mut out = Vec::new();
    if cfg!(target_os = "macos") {
        out.extend(dirs::data_dir().map(|d| d.join("MobileSync").join("Backup")));
    } else if cfg!(windows) {
        // iTunes from Apple's site, then iTunes / Apple Devices from the
        // Microsoft Store.
        out.extend(
            dirs::data_dir().map(|d| d.join("Apple Computer").join("MobileSync").join("Backup")),
        );
        out.extend(dirs::home_dir().map(|h| h.join("Apple").join("MobileSync").join("Backup")));
    }
    out
}

/// A `Backup` folder inside `MobileSync`.
fn is_backup_root(tree: &Tree, id: NodeId) -> bool {
    tree.node(id).is_dir
        && tree.name(id) == "Backup"
        && tree
            .parent(id)
            .is_some_and(|p| tree.name(p) == "MobileSync")
}

pub(super) fn backups(tree: &Tree, base: NodeId, mode: SizeMode, age: AgeFilter) -> Report {
    backups_in(tree, base, mode, age, &known_locations())
}

/// What the folders in `known` (that the tree has no backups of) look like
/// on disk.
#[derive(Debug, Default)]
struct Seen {
    /// A backup folder exists but cannot be read.
    unreadable: bool,
    /// A backup folder exists outside the scanned folder.
    elsewhere: bool,
}

/// `backups` with the system's backup folders given, for tests.
fn backups_in(
    tree: &Tree,
    base: NodeId,
    mode: SizeMode,
    age: AgeFilter,
    known: &[PathBuf],
) -> Report {
    let backups = find_backups(tree, base);
    if backups.is_empty() {
        return (Vec::new(), false, empty_note(&look(tree, base, known)));
    }

    let infos: Vec<(NodeId, Option<BackupInfo>)> = backups
        .into_iter()
        .map(|id| (id, read_info(tree, id)))
        .collect();
    // The date of a backup: as recorded, or the newest change in its folder.
    let date = |id: NodeId, info: &Option<BackupInfo>| {
        info.as_ref()
            .and_then(|i| i.date)
            .unwrap_or(tree.node(id).modified)
    };
    // The newest backup of each device, before the age filter.
    let device_key = |id: NodeId, info: &Option<BackupInfo>| -> String {
        info.as_ref()
            .and_then(|i| i.device_id.clone().or_else(|| i.device.clone()))
            .unwrap_or_else(|| tree.name(id).to_string())
    };
    let mut newest: HashMap<String, (u32, NodeId)> = HashMap::new();
    for (id, info) in &infos {
        let entry = newest.entry(device_key(*id, info)).or_insert((0, *id));
        if date(*id, info) >= entry.0 {
            *entry = (date(*id, info), *id);
        }
    }

    let mut rows: Vec<Row> = infos
        .iter()
        .filter(|(id, info)| age.ok(date(*id, info)))
        .map(|(id, info)| {
            let is_newest = newest
                .get(&device_key(*id, info))
                .is_some_and(|&(_, n)| n == *id);
            let detail = detail(info.as_ref(), date(*id, info), is_newest);
            let mut row = Row::single(tree, base, *id, mode, detail);
            row.label = label(tree, *id, info.as_ref());
            row
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.size()));
    let truncated = rows.len() > LIMIT;
    rows.truncate(LIMIT);
    (
        rows,
        truncated,
        t!(
            "Silinen yedekle cihaz geri yüklenemez: her cihazın en yeni yedeğini saklayın.",
            "A deleted backup cannot restore the device: keep each device's newest backup."
        ),
    )
}

/// The backup folders below `base`, or the one `base` is in.
fn find_backups(tree: &Tree, base: NodeId) -> Vec<NodeId> {
    // Browsing inside a backup: that backup.
    let mut id = base;
    while let Some(parent) = tree.parent(id) {
        if is_backup_root(tree, parent) {
            return vec![id];
        }
        id = parent;
    }
    let mut roots = Vec::new();
    if is_backup_root(tree, base) {
        roots.push(base);
    } else {
        super::walk(tree, base, |id| {
            if is_backup_root(tree, id) {
                roots.push(id);
                return false;
            }
            tree.node(id).is_dir
        });
    }
    roots
        .into_iter()
        .flat_map(|r| tree.children(r).filter(|&c| tree.node(c).is_dir))
        .collect()
}

/// How the system's backup folders look on disk, for the note of an empty
/// report.
fn look(tree: &Tree, base: NodeId, known: &[PathBuf]) -> Seen {
    let scanned = tree.path_of(base);
    let mut seen = Seen::default();
    for path in known {
        let inside = path.starts_with(&scanned);
        match std::fs::read_dir(path) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                seen.unreadable = true;
                seen.elsewhere |= !inside;
            }
            Ok(mut entries) => {
                // Backups that the scan did not include.
                if !inside && entries.next().is_some() {
                    seen.elsewhere = true;
                }
            }
            Err(_) => {}
        }
    }
    seen
}

fn empty_note(seen: &Seen) -> &'static str {
    match (seen.unreadable, seen.elsewhere) {
        (true, false) => {
            if cfg!(target_os = "macos") {
                t!(
                    "Yedek klasörü (MobileSync/Backup) okunamadı: Tam Disk Erişimi gerekiyor. Sistem Ayarları → Gizlilik ve Güvenlik → Tam Disk Erişimi'nde terminal uygulamanızı açın, terminali yeniden başlatıp yeniden tarayın.",
                    "The backup folder (MobileSync/Backup) could not be read: it needs Full Disk Access. Turn on your terminal app in System Settings → Privacy & Security → Full Disk Access, restart the terminal and scan again."
                )
            } else {
                t!(
                    "Yedek klasörü (MobileSync/Backup) okunamadı: erişim izni yok.",
                    "The backup folder (MobileSync/Backup) could not be read: access denied."
                )
            }
        }
        (true, true) => {
            if cfg!(target_os = "macos") {
                t!(
                    "Bu tarama cihaz yedeklerini içermiyor: ev klasörünü tarayın. Yedek klasörü için Tam Disk Erişimi de gerekiyor: Sistem Ayarları → Gizlilik ve Güvenlik → Tam Disk Erişimi'nde terminal uygulamanızı açın ve terminali yeniden başlatın.",
                    "This scan does not include the device backups: scan the home folder. The backup folder also needs Full Disk Access: turn on your terminal app in System Settings → Privacy & Security → Full Disk Access and restart the terminal."
                )
            } else {
                t!(
                    "Bu tarama cihaz yedeklerini içermiyor: ev klasörünü tarayın. Yedek klasörü şu an okunamıyor: erişim izni yok.",
                    "This scan does not include the device backups: scan the home folder. The backup folder cannot be read right now: access denied."
                )
            }
        }
        (false, true) => t!(
            "Bu tarama cihaz yedeklerini (MobileSync/Backup) içermiyor: ev klasörünü tarayın.",
            "This scan does not include the device backups (MobileSync/Backup): scan the home folder."
        ),
        (false, false) => t!(
            "Bu klasörün altında iPhone / iPad yedeği yok. Finder ve iTunes yedekleri MobileSync/Backup içinde tutar.",
            "No iPhone or iPad backups below this folder. Finder and iTunes keep them in MobileSync/Backup."
        ),
    }
}

/// "Ali's iPhone · iPhone 13 Pro", or the folder's name when the backup
/// has no readable `Info.plist`.
fn label(tree: &Tree, id: NodeId, info: Option<&BackupInfo>) -> String {
    let Some(device) = info.and_then(|i| i.device.as_deref()) else {
        return tree.name(id).to_string();
    };
    match info.and_then(|i| i.model.as_deref()) {
        Some(model) => format!("{device} · {model}"),
        None => device.to_string(),
    }
}

/// "14.03.2026 · en yenisi · şifreli · iOS 17.5.1": the date and the marker
/// first, as the column is narrow.
fn detail(info: Option<&BackupInfo>, date: u32, newest: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    if date != 0 {
        let when = crate::ui::fmt_date(date);
        // The day is enough.
        parts.push(when.split(' ').next().unwrap_or_default().to_string());
    }
    if newest {
        parts.push(t!("en yenisi", "newest").into());
    }
    match info {
        Some(info) => {
            if info.encrypted {
                parts.push(t!("şifreli", "encrypted").into());
            }
            if let Some(v) = &info.version {
                let ipad = info
                    .product_type
                    .as_deref()
                    .or(info.model.as_deref())
                    .is_some_and(|p| p.starts_with("iPad"));
                parts.push(format!("{} {v}", if ipad { "iPadOS" } else { "iOS" }));
            }
        }
        None => parts.push(t!("Info.plist okunamadı", "Info.plist unreadable").into()),
    }
    parts.join(" · ")
}

// Unix only: the expected texts need a fixed language and UTC dates. The
// integration test covers the report on Windows.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::i18n::{with_lang, Lang};
    use crate::reports::test_util::sz;
    use crate::stats::DAY;
    use crate::tree::ROOT;
    use crate::ui::with_fixed_now;

    const SAMPLE: &str = include_str!("../../tests/fixtures/backup_info.plist");
    /// 2026-10-01 12:00 UTC.
    const NOW: u64 = 1_790_856_000;
    /// The date in the sample, 2026-03-14 09:30 UTC.
    const SAMPLE_DATE: u32 = 1_773_480_600;

    fn info_plist(device: &str, id: &str, product: &str, version: &str, date: u64) -> String {
        let date = chrono::DateTime::from_timestamp(i64::try_from(date).unwrap(), 0)
            .unwrap()
            .format("%Y-%m-%dT%H:%M:%SZ");
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>Device Name</key><string>{device}</string>
<key>Product Type</key><string>{product}</string>
<key>Product Version</key><string>{version}</string>
<key>Target Identifier</key><string>{id}</string>
<key>Last Backup Date</key><date>{date}</date>
</dict></plist>"#
        )
    }

    fn manifest(dir: &Path, encrypted: bool) {
        let mut d = plist::Dictionary::new();
        d.insert("IsEncrypted".into(), plist::Value::Boolean(encrypted));
        plist::Value::Dictionary(d)
            .to_file_binary(dir.join("Manifest.plist"))
            .unwrap();
    }

    /// A home folder in a temp dir with three backups of two devices, and
    /// the tree of it built by hand (the files are on disk, the sizes are
    /// made up):
    ///
    /// - `AAAA`: the sample (Demo iPhone, 2026-03-14), encrypted, 900
    /// - `BBBB`: Work iPad, 300 days ago, 500
    /// - `BBBB-20240101-000000`: Work iPad, 700 days ago, 400
    /// - `CCCC`: no Info.plist, 100
    struct Home {
        dir: tempfile::TempDir,
        tree: Tree,
    }

    fn home() -> Home {
        let dir = tempfile::tempdir().unwrap();
        let backup = dir
            .path()
            .join("Library/Application Support/MobileSync/Backup");
        let write = |name: &str, info: Option<&str>, encrypted: bool| {
            let d = backup.join(name);
            std::fs::create_dir_all(&d).unwrap();
            if let Some(info) = info {
                std::fs::write(d.join("Info.plist"), info).unwrap();
            }
            manifest(&d, encrypted);
        };
        write("AAAA", Some(SAMPLE), true);
        let ipad =
            |days: u64| info_plist("Work iPad", "BBBB", "iPad13,4", "18.0", NOW - days * DAY);
        write("BBBB", Some(&ipad(300)), false);
        write("BBBB-20240101-000000", Some(&ipad(700)), false);
        write("CCCC", None, false);

        let mut t = Tree::new(dir.path());
        let lib = t.push(ROOT, "Library", true, sz(0));
        let support = t.push(lib, "Application Support", true, sz(0));
        let ms = t.push(support, "MobileSync", true, sz(0));
        let root = t.push(ms, "Backup", true, sz(0));
        for (name, size, days) in [
            ("AAAA", 900, 10),
            ("BBBB", 500, 300),
            ("BBBB-20240101-000000", 400, 700),
            ("CCCC", 100, 50),
        ] {
            let b = t.push(root, name, true, sz(0));
            let f = t.push(b, "Manifest.db", false, sz(size));
            t.set_times(f, (NOW - days * DAY) as u32, 0);
        }
        let docs = t.push(ROOT, "Documents", true, sz(0));
        t.push(docs, "notes.txt", false, sz(5));
        t.finalize();
        Home { dir, tree: t }
    }

    fn run(t: &Tree, base: NodeId, min_days: u32, known: &[PathBuf]) -> Report {
        with_fixed_now(NOW, || {
            with_lang(Lang::En, || {
                backups_in(
                    t,
                    base,
                    SizeMode::Disk,
                    AgeFilter::new(NOW, min_days),
                    known,
                )
            })
        })
    }

    fn table(r: &Report) -> Vec<(String, String)> {
        r.0.iter()
            .map(|row| (row.label.clone(), row.detail.clone()))
            .collect()
    }

    /// The backup folder of each row.
    fn folders(t: &Tree, r: &Report) -> Vec<String> {
        r.0.iter()
            .map(|row| t.name(row.nodes[0]).to_string())
            .collect()
    }

    #[test]
    fn reads_the_sample_info_plist() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Info.plist"), SAMPLE).unwrap();
        manifest(dir.path(), true);
        let info = BackupInfo::read(dir.path()).unwrap();
        assert_eq!(
            info,
            BackupInfo {
                device: Some("Demo iPhone".into()),
                model: Some("iPhone 13 Pro".into()),
                product_type: Some("iPhone14,2".into()),
                version: Some("17.5.1".into()),
                date: Some(SAMPLE_DATE),
                encrypted: true,
                device_id: Some("00000000-0000000000000000".into()),
            }
        );
        // Without a manifest the backup counts as unencrypted.
        std::fs::remove_file(dir.path().join("Manifest.plist")).unwrap();
        assert!(!BackupInfo::read(dir.path()).unwrap().encrypted);
        // No Info.plist: nothing known.
        std::fs::remove_file(dir.path().join("Info.plist")).unwrap();
        assert_eq!(BackupInfo::read(dir.path()), None);
    }

    #[test]
    fn one_row_per_backup_with_the_newest_of_each_device_marked() {
        let h = home();
        let r = run(&h.tree, ROOT, 0, &[]);
        assert_eq!(
            table(&r),
            [
                (
                    "Demo iPhone · iPhone 13 Pro".into(),
                    "2026-03-14 · newest · encrypted · iOS 17.5.1".into()
                ),
                (
                    "Work iPad · iPad13,4".into(),
                    "2025-12-05 · newest · iPadOS 18.0".into()
                ),
                (
                    "Work iPad · iPad13,4".into(),
                    "2024-10-31 · iPadOS 18.0".into()
                ),
                // Without Info.plist: the folder's name and newest change.
                (
                    "CCCC".into(),
                    "2026-08-12 · newest · Info.plist unreadable".into()
                ),
            ]
        );
        assert!(r.2.contains("cannot restore"), "{}", r.2);
        // A row is the backup folder.
        let id = r.0[0].nodes[0];
        assert_eq!(
            h.tree.path_of(id),
            h.dir
                .path()
                .join("Library/Application Support/MobileSync/Backup/AAAA")
        );
    }

    #[test]
    fn the_age_filter_uses_the_backup_date() {
        let h = home();
        // The folder of AAAA changed 10 days ago; its backup is from March.
        let r = run(&h.tree, ROOT, 180, &[]);
        assert_eq!(
            folders(&h.tree, &r),
            ["AAAA", "BBBB", "BBBB-20240101-000000"]
        );
        // The marker follows the device, not the filter: the older iPad backup
        // is not the newest one even when it is listed alone.
        let r = run(&h.tree, ROOT, 365, &[]);
        assert_eq!(folders(&h.tree, &r), ["BBBB-20240101-000000"]);
        assert!(!r.0[0].detail.contains("newest"), "{}", r.0[0].detail);
    }

    #[test]
    fn works_from_inside_the_backup_folders() {
        let h = home();
        let t = &h.tree;
        let child = |dir: NodeId, name: &str| t.children(dir).find(|&c| t.name(c) == name).unwrap();
        let lib = child(ROOT, "Library");
        let ms = child(child(lib, "Application Support"), "MobileSync");
        let root = child(ms, "Backup");
        assert_eq!(run(t, ms, 0, &[]).0.len(), 4);
        assert_eq!(run(t, root, 0, &[]).0.len(), 4);
        let one = run(t, child(root, "AAAA"), 0, &[]);
        assert_eq!(folders(t, &one), ["AAAA"]);
    }

    #[test]
    fn an_empty_report_says_why() {
        let h = home();
        let t = &h.tree;
        let docs = t
            .children(ROOT)
            .find(|&c| t.name(c) == "Documents")
            .unwrap();
        let backup = h
            .dir
            .path()
            .join("Library/Application Support/MobileSync/Backup");

        let none = run(t, docs, 0, &[h.dir.path().join("nowhere")]);
        assert!(none.0.is_empty());
        assert!(
            none.2.starts_with("No iPhone or iPad backups"),
            "{}",
            none.2
        );

        // The backups exist outside the scanned folder.
        let elsewhere = run(t, docs, 0, std::slice::from_ref(&backup));
        assert!(
            elsewhere.2.contains("scan the home folder"),
            "{}",
            elsewhere.2
        );
        assert!(!elsewhere.2.contains("Full Disk Access"), "{}", elsewhere.2);
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_backup_folder_asks_for_full_disk_access() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let ms = dir.path().join("Library/Application Support/MobileSync");
        std::fs::create_dir_all(ms.join("Backup/AAAA")).unwrap();
        std::fs::set_permissions(&ms, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root reads everything: nothing to check then.
        let blocked = std::fs::read_dir(ms.join("Backup")).is_err();

        // What the scanner sees: an empty MobileSync.
        let mut t = Tree::new(dir.path());
        let lib = t.push(ROOT, "Library", true, sz(0));
        let support = t.push(lib, "Application Support", true, sz(0));
        t.push(support, "MobileSync", true, sz(0));
        t.finalize();
        let r = run(&t, ROOT, 0, &[ms.join("Backup")]);
        let outside = run(&t, lib, 0, &[ms.join("Backup")]);
        let other = Tree::new(&dir.path().join("Documents"));
        let other = run(&other, ROOT, 0, &[ms.join("Backup")]);
        std::fs::set_permissions(&ms, std::fs::Permissions::from_mode(0o755)).unwrap();
        if !blocked {
            return;
        }

        assert!(r.0.is_empty());
        let expected = if cfg!(target_os = "macos") {
            "Full Disk Access"
        } else {
            "access denied"
        };
        assert!(r.2.contains(expected), "{}", r.2);
        assert!(!r.2.contains("scan the home folder"), "{}", r.2);
        assert!(outside.2.contains(expected), "{}", outside.2);
        assert!(other.2.contains("scan the home folder"), "{}", other.2);
        assert!(other.2.contains(expected), "{}", other.2);
    }
}
