//! Linux system caches: the apt, dnf and pacman package caches, the systemd
//! journal and disabled snap revisions (#18).
//!
//! Measuring only reads (folder sizes, `journalctl --disk-usage`,
//! `snap list --all`). Every cleanup needs root, and rustClean never runs
//! `sudo`: the actions are [`Step::Manual`] commands the user runs
//! themselves. The code is plain std, so the parsers are tested on every
//! platform; [`ToolKind::available`] lists these tools only on Linux.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{
    action, dir_size, missing, output, ready, which, CleanAction, Risk, Status, Step, ToolKind,
};
use crate::ui::fmt_size;

/// The tools of this module, in the order they are listed.
pub(super) const KINDS: [ToolKind; 5] = [
    ToolKind::AptCache,
    ToolKind::DnfCache,
    ToolKind::PacmanCache,
    ToolKind::Journal,
    ToolKind::Snaps,
];

pub(super) fn label(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::AptCache => t!("apt paket önbelleği", "apt package cache"),
        ToolKind::DnfCache => t!("dnf paket önbelleği", "dnf package cache"),
        ToolKind::PacmanCache => t!("pacman paket önbelleği", "pacman package cache"),
        ToolKind::Journal => t!("systemd günlüğü (journal)", "systemd journal"),
        ToolKind::Snaps => t!("Devre dışı snap sürümleri", "Disabled snap revisions"),
        _ => unreachable!("not a Linux tool"),
    }
}

pub(super) fn measure(kind: ToolKind) -> (Status, Vec<CleanAction>) {
    match kind {
        ToolKind::AptCache => apt(),
        ToolKind::DnfCache => dnf(),
        ToolKind::PacmanCache => pacman(),
        ToolKind::Journal => journal(),
        ToolKind::Snaps => snaps(),
        _ => unreachable!("not a Linux tool"),
    }
}

/// An action whose commands the user runs with `sudo`.
fn root_action(label: &str, risk: Risk, commands: &[String]) -> CleanAction {
    action(
        label,
        risk,
        commands.iter().cloned().map(Step::Manual).collect(),
    )
}

fn not_installed() -> (Status, Vec<CleanAction>) {
    missing(t!("kurulu değil", "not installed"))
}

/// Sizes the folders that exist; `None` when none does.
fn folders_size(dirs: &[PathBuf]) -> Option<(u64, String)> {
    let found: Vec<(&PathBuf, u64)> = dirs
        .iter()
        .filter_map(|d| dir_size(d).map(|s| (d, s)))
        .collect();
    if found.is_empty() {
        return None;
    }
    let total = found.iter().map(|f| f.1).sum();
    let detail = found
        .iter()
        .map(|(d, _)| d.display().to_string())
        .collect::<Vec<_>>()
        .join(" · ");
    Some((total, detail))
}

// -------------------------------------------------------------------- apt

fn apt() -> (Status, Vec<CleanAction>) {
    if which("apt-get").is_none() {
        return not_installed();
    }
    // `apt-get clean` empties archives/ (downloaded .deb files) and the
    // package lists cache (*.bin); both live here.
    let Some((size, detail)) = folders_size(&[PathBuf::from("/var/cache/apt")]) else {
        return missing(t!("önbellek klasörü yok", "no cache folder"));
    };
    ready(size, detail, vec![apt_action()])
}

pub(crate) fn apt_action() -> CleanAction {
    root_action(
        t!(
            "İndirilmiş paketleri sil (root gerekir)",
            "Delete downloaded packages (needs root)",
        ),
        Risk::Redownload,
        &["sudo apt-get clean".into()],
    )
}

// -------------------------------------------------------------------- dnf

fn dnf() -> (Status, Vec<CleanAction>) {
    if which("dnf").is_none() && which("dnf5").is_none() {
        return not_installed();
    }
    let dirs = match std::fs::read_to_string("/etc/dnf/dnf.conf")
        .ok()
        .and_then(|c| parse_dnf_cachedir(&c))
    {
        Some(dir) => vec![dir],
        // dnf5 and dnf4 defaults.
        None => vec![
            PathBuf::from("/var/cache/libdnf5"),
            PathBuf::from("/var/cache/dnf"),
        ],
    };
    let Some((size, detail)) = folders_size(&dirs) else {
        return missing(t!("önbellek klasörü yok", "no cache folder"));
    };
    ready(size, detail, vec![dnf_action()])
}

fn dnf_action() -> CleanAction {
    root_action(
        t!(
            "Paket ve depo önbelleğini sil (root gerekir)",
            "Delete the package and metadata cache (needs root)",
        ),
        Risk::Redownload,
        &["sudo dnf clean all".into()],
    )
}

/// The `cachedir` option of the `[main]` section of `dnf.conf`.
pub fn parse_dnf_cachedir(conf: &str) -> Option<PathBuf> {
    ini_values(conf, "main", "cachedir")
        .into_iter()
        .next_back()
        .map(PathBuf::from)
}

// ----------------------------------------------------------------- pacman

fn pacman() -> (Status, Vec<CleanAction>) {
    if which("pacman").is_none() {
        return not_installed();
    }
    let conf = std::fs::read_to_string("/etc/pacman.conf").unwrap_or_default();
    let Some((size, detail)) = folders_size(&parse_pacman_cache_dirs(&conf)) else {
        return missing(t!("önbellek klasörü yok", "no cache folder"));
    };
    // paccache comes with pacman-contrib.
    ready(size, detail, pacman_actions(which("paccache").is_some()))
}

fn pacman_actions(paccache: bool) -> Vec<CleanAction> {
    let mut actions = Vec::new();
    if paccache {
        actions.push(root_action(
            t!(
                "Her paketin yalnızca en yeni sürümünü tut (root gerekir)",
                "Keep only the newest version of each package (needs root)",
            ),
            Risk::Safe,
            &["sudo paccache -rk1".into()],
        ));
    }
    actions.push(root_action(
        t!(
            "Önbelleği tamamen boşalt (root gerekir)",
            "Empty the whole cache (needs root)",
        ),
        Risk::Redownload,
        &["sudo pacman -Scc".into()],
    ));
    actions
}

/// The `CacheDir` folders of the `[options]` section of `pacman.conf`
/// (several per line and several lines are allowed), or pacman's default.
pub fn parse_pacman_cache_dirs(conf: &str) -> Vec<PathBuf> {
    let dirs: Vec<PathBuf> = ini_values(conf, "options", "CacheDir")
        .iter()
        .flat_map(|v| v.split_whitespace())
        .map(PathBuf::from)
        .collect();
    if dirs.is_empty() {
        vec![PathBuf::from("/var/cache/pacman/pkg/")]
    } else {
        dirs
    }
}

/// Values of `key` in `[section]` of an ini-style file. Comments start with
/// `#` (pacman and dnf also accept them after a value) or `;`.
fn ini_values(conf: &str, section: &str, key: &str) -> Vec<String> {
    let mut current = String::new();
    let mut out = Vec::new();
    for line in conf.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = name.trim().to_string();
            continue;
        }
        if current != section {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim().eq_ignore_ascii_case(key) && !v.trim().is_empty() {
                out.push(v.trim().to_string());
            }
        }
    }
    out
}

// ---------------------------------------------------------------- journal

/// How long `journalctl --vacuum-time` keeps logs.
const KEEP_JOURNAL: &str = "2weeks";

fn journal() -> (Status, Vec<CleanAction>) {
    let Some(exe) = which("journalctl") else {
        return missing(t!("systemd yok", "no systemd"));
    };
    let Some(size) = output(&exe, &["--disk-usage"], Duration::from_secs(20))
        .and_then(|o| parse_journal_usage(&o))
    else {
        return (
            Status::Unavailable(
                t!(
                    "journalctl --disk-usage başarısız",
                    "journalctl --disk-usage failed"
                )
                .into(),
            ),
            Vec::new(),
        );
    };
    ready(
        size,
        t!(
            "günlüklerin toplamı (görebildikleriniz)",
            "all journals (the ones you can read)",
        ),
        vec![journal_action()],
    )
}

pub(crate) fn journal_action() -> CleanAction {
    root_action(
        t!(
            "İki haftadan eski günlükleri sil (root gerekir)",
            "Delete logs older than two weeks (needs root)",
        ),
        Risk::Safe,
        &[format!("sudo journalctl --vacuum-time={KEEP_JOURNAL}")],
    )
}

/// The size in "Archived and active journals take up 1.2G in the file
/// system." (older versions: "Journals take up 1.2G on disk.").
pub fn parse_journal_usage(out: &str) -> Option<u64> {
    let rest = out.split("take up ").nth(1)?;
    parse_journal_size(rest.split_whitespace().next()?)
}

/// systemd's sizes: "512B", "8.0K", "56.0M", "1.2G" (powers of 1024).
fn parse_journal_size(s: &str) -> Option<u64> {
    let split = s.find(|c: char| c.is_ascii_alphabetic())?;
    let (num, unit) = s.split_at(split);
    let num: f64 = num.parse().ok()?;
    let power = match unit {
        "B" => 0,
        "K" => 1,
        "M" => 2,
        "G" => 3,
        "T" => 4,
        "P" => 5,
        _ => return None,
    };
    Some((num * 1024f64.powi(power)) as u64)
}

// ------------------------------------------------------------------ snaps

/// Where snapd keeps the squashfs image of each revision.
const SNAPS_DIR: &str = "/var/lib/snapd/snaps";

/// A snap revision that is installed but not active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapRevision {
    pub name: String,
    pub rev: String,
}

fn snaps() -> (Status, Vec<CleanAction>) {
    let Some(exe) = which("snap") else {
        return not_installed();
    };
    let Some(out) = output(&exe, &["list", "--all"], Duration::from_secs(20)) else {
        return (
            Status::Unavailable(t!("snap list başarısız", "snap list failed").into()),
            Vec::new(),
        );
    };
    let disabled = parse_snap_list(&out);
    if disabled.is_empty() {
        return missing(t!("devre dışı sürüm yok", "no disabled revisions"));
    }
    let size: u64 = disabled
        .iter()
        .filter_map(|s| std::fs::metadata(snap_file(Path::new(SNAPS_DIR), s)).ok())
        .map(|m| m.len())
        .sum();
    let detail = disabled
        .iter()
        .map(|s| format!("{} ({})", s.name, s.rev))
        .collect::<Vec<_>>()
        .join(" · ");
    ready(size, detail, vec![snap_action(&disabled, size)])
}

/// Removing the disabled revisions, one command each.
pub(crate) fn snap_action(disabled: &[SnapRevision], size: u64) -> CleanAction {
    let n = crate::i18n::count(disabled.len() as u64, "sürüm", "revision", "revisions");
    root_action(
        &tf!(
            "Devre dışı sürümleri kaldır: {n}, {} (root gerekir)",
            "Remove the disabled revisions: {n}, {} (needs root)",
            fmt_size(size)
        ),
        Risk::Safe,
        &disabled
            .iter()
            .map(|s| format!("sudo snap remove {} --revision={}", s.name, s.rev))
            .collect::<Vec<_>>(),
    )
}

fn snap_file(dir: &Path, s: &SnapRevision) -> PathBuf {
    dir.join(format!("{}_{}.snap", s.name, s.rev))
}

/// Disabled revisions in the output of `snap list --all`: the last column
/// (Notes) holds a comma-separated list such as `base,disabled`.
pub fn parse_snap_list(out: &str) -> Vec<SnapRevision> {
    out.lines()
        .skip_while(|l| !l.starts_with("Name"))
        .skip(1)
        .filter_map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() < 4 || !cols.last()?.split(',').any(|n| n == "disabled") {
                return None;
            }
            let (name, rev) = (cols[0], cols[2]);
            // Names and revisions go into a command line; accept only what
            // snapd itself allows.
            let name_ok = name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            let rev_ok = rev.chars().all(|c| c.is_ascii_alphanumeric());
            (name_ok && rev_ok && !name.is_empty() && !rev.is_empty()).then(|| SnapRevision {
                name: name.to_string(),
                rev: rev.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_list_fixture() {
        let revs = parse_snap_list(include_str!("../tests/fixtures/snap_list_all.txt"));
        let got: Vec<(&str, &str)> = revs
            .iter()
            .map(|r| (r.name.as_str(), r.rev.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("core20", "2015"),
                ("firefox", "4173"),
                ("hello-local", "x1"),
                ("snapd", "21184"),
            ]
        );
        assert!(parse_snap_list("No snaps are installed yet.").is_empty());
        // Something that would break the command line is skipped.
        let odd = "Name Version Rev Tracking Publisher Notes\n\
                   a;rm 1 2 latest/stable me disabled\n";
        assert!(parse_snap_list(odd).is_empty());
    }

    #[test]
    fn snap_file_names() {
        let rev = SnapRevision {
            name: "core20".into(),
            rev: "2015".into(),
        };
        let path = snap_file(Path::new(SNAPS_DIR), &rev);
        assert!(path.ends_with("core20_2015.snap"));
    }

    #[test]
    fn journal_usage() {
        let out = include_str!("../tests/fixtures/journalctl_disk_usage.txt");
        assert_eq!(
            parse_journal_usage(out),
            Some((1.2 * 1024f64.powi(3)) as u64)
        );
        assert_eq!(
            parse_journal_usage("Journals take up 56.0M on disk."),
            Some(56 << 20)
        );
        assert_eq!(
            parse_journal_usage("Archived and active journals take up 512B in the file system."),
            Some(512)
        );
        assert_eq!(parse_journal_usage("No journal files were found."), None);
        assert_eq!(parse_journal_usage("take up 3X in"), None);
    }

    #[test]
    fn pacman_cache_dirs() {
        let dirs = parse_pacman_cache_dirs(include_str!("../tests/fixtures/pacman.conf"));
        assert_eq!(
            dirs,
            [
                PathBuf::from("/var/cache/pacman/pkg/"),
                PathBuf::from("/mnt/shared/pkg/"),
                PathBuf::from("/home/me/.cache/pkg"),
            ]
        );
        assert_eq!(
            parse_pacman_cache_dirs("[options]\n#CacheDir = /x\n"),
            [PathBuf::from("/var/cache/pacman/pkg/")]
        );
    }

    #[test]
    fn dnf_cachedir() {
        assert_eq!(
            parse_dnf_cachedir(include_str!("../tests/fixtures/dnf.conf")),
            Some(PathBuf::from("/srv/dnf-cache"))
        );
        assert_eq!(parse_dnf_cachedir("[main]\ngpgcheck=1\n"), None);
    }

    #[test]
    fn every_action_is_shown_not_run() {
        let revs = parse_snap_list(include_str!("../tests/fixtures/snap_list_all.txt"));
        let mut actions = vec![apt_action(), dnf_action(), journal_action()];
        actions.extend(pacman_actions(true));
        actions.push(snap_action(&revs, 1 << 30));
        for a in &actions {
            assert!(a.manual(), "{}", a.label);
            assert!(a.steps.iter().all(|s| s.describe().starts_with("sudo ")));
        }
        assert_eq!(actions[0].steps[0].describe(), "sudo apt-get clean");
        assert_eq!(
            actions[2].steps[0].describe(),
            "sudo journalctl --vacuum-time=2weeks"
        );
        assert_eq!(pacman_actions(false).len(), 1);
        let snap = actions.last().unwrap();
        assert_eq!(snap.steps.len(), 4);
        assert_eq!(
            snap.steps[0].describe(),
            "sudo snap remove core20 --revision=2015"
        );
    }
}
