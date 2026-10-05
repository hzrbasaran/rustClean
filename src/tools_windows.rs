//! Windows system folders: the user's `%TEMP%`, `C:\Windows\Temp`, the
//! Windows Update download cache and the Recycle Bin (#19).
//!
//! Only `%TEMP%` is cleaned by rustClean, through the normal confirmed flow:
//! its entries go to the Recycle Bin one by one, and files in use are
//! skipped. The other folders need administrator rights: they are measured
//! when readable, and their actions are [`Step::Manual`] commands for an
//! administrator PowerShell. The Recycle Bin is only measured; emptying it
//! deletes for good, so that is left to the user.
//!
//! The code is plain std, so it builds and is tested on every platform;
//! [`ToolKind::available`] lists these tools only on Windows.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use super::{
    action, dir_size, missing, ready, size_on_disk, CleanAction, Risk, RunEvent, Status, Step,
    ToolKind,
};
use crate::ui::fmt_size;

/// The tools of this module, in the order they are listed.
pub(super) const KINDS: [ToolKind; 4] = [
    ToolKind::WinTemp,
    ToolKind::WinSystemTemp,
    ToolKind::WinUpdate,
    ToolKind::RecycleBin,
];

pub(super) fn label(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::WinTemp => t!("Geçici dosyalar (%TEMP%)", "Temporary files (%TEMP%)"),
        ToolKind::WinSystemTemp => t!("Windows geçici klasörü", "Windows temp folder"),
        ToolKind::WinUpdate => t!("Windows Update indirmeleri", "Windows Update downloads"),
        ToolKind::RecycleBin => t!("Geri Dönüşüm Kutusu", "Recycle Bin"),
        _ => unreachable!("not a Windows tool"),
    }
}

pub(super) fn measure(kind: ToolKind) -> (Status, Vec<CleanAction>) {
    match kind {
        ToolKind::WinTemp => user_temp(),
        ToolKind::WinSystemTemp => system_temp(),
        ToolKind::WinUpdate => update_cache(),
        ToolKind::RecycleBin => recycle_bin(),
        _ => unreachable!("not a Windows tool"),
    }
}

/// `%SystemRoot%`, usually `C:\Windows`.
fn system_root() -> PathBuf {
    std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("windir"))
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
}

// ------------------------------------------------------------------ %TEMP%

fn user_temp() -> (Status, Vec<CleanAction>) {
    let dir = std::env::var_os("TEMP").map_or_else(std::env::temp_dir, PathBuf::from);
    if !dir.is_dir() {
        return missing(t!("klasör yok", "no folder"));
    }
    if !is_temp_folder(&dir, dirs::home_dir().as_deref()) {
        // A TEMP pointing at a drive or the home folder: never offer it.
        return (
            Status::Unavailable(tf!(
                "%TEMP% beklenmedik bir yeri gösteriyor: {}",
                "%TEMP% points to an unexpected place: {}",
                dir.display()
            )),
            Vec::new(),
        );
    }
    let size = dir_size(&dir).unwrap_or(0);
    ready(size, dir.display().to_string(), vec![temp_action(dir)])
}

pub(crate) fn temp_action(dir: PathBuf) -> CleanAction {
    action(
        t!(
            "Geçici dosyaları çöpe taşı (kullanımdaki dosyalar atlanır)",
            "Move temporary files to the trash (files in use are skipped)",
        ),
        Risk::Safe,
        vec![Step::TrashEach(dir)],
    )
}

/// Whether `dir` is a temporary folder that may be emptied: named exactly
/// `temp` or `tmp` (in any case; `Templates` or `MyTemp` are refused), not
/// a drive root, and neither the home folder nor one of its parents.
pub fn is_temp_folder(dir: &Path, home: Option<&Path>) -> bool {
    let named = dir.file_name().is_some_and(|n| {
        let n = n.to_string_lossy();
        n.eq_ignore_ascii_case("temp") || n.eq_ignore_ascii_case("tmp")
    });
    let below_root = dir.parent().is_some();
    let holds_home = home.is_some_and(|h| h.starts_with(dir));
    dir.is_absolute() && named && below_root && !holds_home
}

/// Moves the entries of `dir` to the trash one at a time, so a file in use
/// does not stop the rest. Fails only when the folder cannot be read.
pub(super) fn trash_each(kind: ToolKind, dir: &Path, tx: &Sender<RunEvent>) -> bool {
    let entries: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(r) => r.flatten().map(|e| e.path()).collect(),
        Err(e) => {
            let _ = tx.send(RunEvent::Output(tf!(
                "okunamadı: {e}",
                "could not read: {e}"
            )));
            return false;
        }
    };
    if entries.is_empty() {
        let _ = tx.send(RunEvent::Output(
            t!("klasör zaten boş", "folder is already empty").into(),
        ));
        return true;
    }
    let mut moved = crate::tree::Size::default();
    let (mut done, mut skipped) = (0u64, 0u64);
    for entry in &entries {
        // Measured before the move, for the deletion log.
        let size = size_on_disk(entry);
        if crate::delete::trash_all(std::slice::from_ref(entry)).is_ok() {
            moved += size;
            done += 1;
        } else {
            skipped += 1;
        }
    }
    let _ = tx.send(RunEvent::Output(tf!(
        "{} çöp kutusuna taşındı, {} atlandı (kullanımda)",
        "{} moved to the trash, {} skipped (in use)",
        crate::i18n::count(done, "öğe", "item", "items"),
        crate::i18n::count(skipped, "öğe", "item", "items"),
    )));
    if done > 0 {
        crate::trashlog::append(&[crate::trashlog::Entry {
            time: crate::ui::now_secs(),
            path: dir.display().to_string(),
            size: moved,
            via: crate::trashlog::Via::Tool(kind),
        }]);
    }
    true
}

// ------------------------------------------- folders that need an administrator

/// The PowerShell line that deletes the contents of `dir`. Items in use
/// are reported and skipped; the rest is deleted.
fn remove_contents(dir: &Path) -> String {
    let pattern = format!("{}\\*", dir.display());
    if pattern.contains(' ') {
        format!("Remove-Item '{pattern}' -Recurse -Force")
    } else {
        format!("Remove-Item {pattern} -Recurse -Force")
    }
}

/// Measures a folder that only administrators may change; `actions` are
/// shown either way, and the size only when the folder is readable.
fn admin_folder(dir: &Path, actions: Vec<CleanAction>) -> (Status, Vec<CleanAction>) {
    if !dir.is_dir() {
        return missing(t!("klasör yok", "no folder"));
    }
    if std::fs::read_dir(dir).is_err() {
        return (
            Status::Unavailable(
                t!(
                    "ölçmek için yönetici izni gerekir",
                    "needs administrator rights to measure",
                )
                .into(),
            ),
            actions,
        );
    }
    let size = dir_size(dir).unwrap_or(0);
    ready(size, dir.display().to_string(), actions)
}

fn system_temp() -> (Status, Vec<CleanAction>) {
    let dir = system_root().join("Temp");
    admin_folder(&dir, vec![system_temp_action(&dir)])
}

pub(crate) fn system_temp_action(dir: &Path) -> CleanAction {
    action(
        t!(
            "İçeriği sil (yönetici PowerShell'inde)",
            "Delete the contents (in an administrator PowerShell)",
        ),
        Risk::Safe,
        vec![Step::Manual(remove_contents(dir))],
    )
}

fn update_cache() -> (Status, Vec<CleanAction>) {
    let dir = system_root().join("SoftwareDistribution").join("Download");
    admin_folder(&dir, vec![update_action(&dir)])
}

pub(crate) fn update_action(dir: &Path) -> CleanAction {
    action(
        t!(
            "İndirilmiş güncellemeleri sil (yönetici PowerShell'inde)",
            "Delete downloaded updates (in an administrator PowerShell)",
        ),
        Risk::Redownload,
        update_commands(dir),
    )
}

/// Windows Update holds its downloads while it runs: stop it, empty the
/// folder, start it again.
fn update_commands(dir: &Path) -> Vec<Step> {
    vec![
        Step::Manual("Stop-Service -Name wuauserv, bits -Force".into()),
        Step::Manual(remove_contents(dir)),
        Step::Manual("Start-Service -Name wuauserv, bits".into()),
    ]
}

// ------------------------------------------------------------- Recycle Bin

fn recycle_bin() -> (Status, Vec<CleanAction>) {
    // Each drive has its own bin, with a folder per user (by SID); the
    // other users' folders are not readable and are left out of the size.
    let bins: Vec<(String, u64)> = ('C'..='Z')
        .filter_map(|letter| {
            let dir = recycle_bin_dir(letter);
            let size = dir_size(&dir)?;
            Some((format!("{letter}:"), size))
        })
        .collect();
    if bins.is_empty() {
        return missing(t!(
            "Geri Dönüşüm Kutusu okunamadı",
            "could not read the Recycle Bin"
        ));
    }
    let total = bins.iter().map(|b| b.1).sum();
    let detail = bins
        .iter()
        .map(|(drive, size)| format!("{drive} {}", fmt_size(*size)))
        .collect::<Vec<_>>()
        .join(" · ");
    ready(total, detail, vec![recycle_action()])
}

/// Emptying deletes for good, so rustClean leaves it to the user.
/// `Clear-RecycleBin` asks before it empties.
pub(crate) fn recycle_action() -> CleanAction {
    action(
        t!(
            "Geri Dönüşüm Kutusunu boşalt — KALICI SİLİNİR (PowerShell'de)",
            "Empty the Recycle Bin — DELETED FOR GOOD (in PowerShell)",
        ),
        Risk::DataLoss,
        vec![Step::Manual("Clear-RecycleBin".into())],
    )
}

fn recycle_bin_dir(letter: char) -> PathBuf {
    PathBuf::from(format!(r"{letter}:\$Recycle.Bin"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only a folder named exactly `temp` or `tmp`, in any case, is emptied.
    #[test]
    fn temp_folder_names_match_exactly() {
        // An absolute base on every platform.
        let base = std::env::temp_dir().join("base");
        for name in ["Temp", "TEMP", "tmp"] {
            assert!(is_temp_folder(&base.join(name), None), "{name}");
        }
        for name in ["Templates", "Contemporary", "MyTemp", "tmpfiles"] {
            assert!(!is_temp_folder(&base.join(name), None), "{name}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn temp_folder_guard() {
        let home = Path::new("/home/me");
        assert!(is_temp_folder(
            Path::new("/home/me/AppData/Local/Temp"),
            Some(home)
        ));
        assert!(is_temp_folder(Path::new("/data/tmp"), Some(home)));
        // Directly below a root is fine (C:\Temp); a root, the home folder
        // and its parents are never emptied.
        assert!(is_temp_folder(Path::new("/tmp"), None));
        assert!(!is_temp_folder(Path::new("/"), Some(home)));
        assert!(!is_temp_folder(Path::new("/home/me"), Some(home)));
        assert!(!is_temp_folder(
            Path::new("/home/temp"),
            Some(Path::new("/home/temp/me"))
        ));
        // Not named like a temporary folder.
        assert!(!is_temp_folder(Path::new("/home/me/Documents"), Some(home)));
        assert!(!is_temp_folder(Path::new("relative/Temp"), Some(home)));
    }

    #[cfg(windows)]
    #[test]
    fn temp_folder_guard_on_windows_paths() {
        let home = Path::new(r"C:\Users\me");
        assert!(is_temp_folder(
            Path::new(r"C:\Users\me\AppData\Local\Temp"),
            Some(home)
        ));
        assert!(is_temp_folder(Path::new(r"D:\Temp"), Some(home)));
        assert!(!is_temp_folder(Path::new(r"C:\"), Some(home)));
        assert!(!is_temp_folder(Path::new(r"C:\Users"), Some(home)));
        assert!(!is_temp_folder(Path::new(r"\Temp"), Some(home)));
    }

    #[test]
    fn admin_commands_are_shown_exactly() {
        let dir = PathBuf::from(r"C:\Windows\Temp");
        assert_eq!(
            remove_contents(&dir),
            r"Remove-Item C:\Windows\Temp\* -Recurse -Force"
        );
        assert_eq!(
            remove_contents(Path::new(r"D:\My Windows")),
            r"Remove-Item 'D:\My Windows\*' -Recurse -Force"
        );
        let steps = update_commands(&dir);
        assert_eq!(steps.len(), 3);
        assert!(steps.iter().all(|s| matches!(s, Step::Manual(_))));
        assert_eq!(
            steps[0].describe(),
            "Stop-Service -Name wuauserv, bits -Force"
        );
        assert_eq!(recycle_bin_dir('D'), PathBuf::from(r"D:\$Recycle.Bin"));
        for a in [
            system_temp_action(&dir),
            update_action(&dir),
            recycle_action(),
        ] {
            assert!(a.manual(), "{}", a.label);
        }
        // %TEMP% is the one rustClean cleans itself.
        assert!(!temp_action(dir).manual());
    }

    #[test]
    fn admin_folder_without_the_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let (status, actions) = admin_folder(&tmp.path().join("nope"), Vec::new());
        assert!(matches!(status, Status::Missing(_)));
        assert!(actions.is_empty());
        std::fs::write(tmp.path().join("a"), vec![1u8; 10_000]).unwrap();
        let (status, _) = admin_folder(tmp.path(), Vec::new());
        assert!(matches!(status, Status::Ready { reclaimable, .. } if reclaimable > 0));
    }

    #[test]
    fn trash_each_moves_every_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("Temp");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.tmp"), "x").unwrap();
        std::fs::write(dir.join("sub/b.tmp"), "y").unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(trash_each(ToolKind::WinTemp, &dir, &tx));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let said: Vec<RunEvent> = rx.try_iter().collect();
        assert!(said
            .iter()
            .any(|e| matches!(e, RunEvent::Output(l) if l.contains("2 öğe"))));
        // An empty folder is fine; a missing one fails.
        assert!(trash_each(ToolKind::WinTemp, &dir, &tx));
        assert!(!trash_each(
            ToolKind::WinTemp,
            &tmp.path().join("nope"),
            &tx
        ));
    }
}
