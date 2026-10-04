//! Leftover data of apps that are no longer installed. These rules must err
//! on the side of keeping data: anything that might belong to the system or
//! to an installed app is not a leftover.

use std::path::PathBuf;

use crate::lists::{ResultList, Row, Source};
use crate::reports::{AgeFilter, ReportKind, LIMIT};
use crate::tree::{NodeId, SizeMode, Tree, ROOT};

use super::data::{container_core, data_folders, match_all, normalize, vendor_children};
use super::find::{find_apps, installed_on_disk};
use super::{App, Platform};

/// Folders smaller than this are not worth listing as leftovers.
const ORPHAN_MIN: u64 = 1024 * 1024;

/// Data folders of the system or shared by many apps, never leftovers.
fn is_system_data(key: &str) -> bool {
    const SYSTEM: &[&str] = &[
        "accessibility",
        "addressbook",
        "animoji",
        "appstore",
        "assistant",
        "cache",
        "caches",
        "callhistorydb",
        "callhistorytransactions",
        "cef",
        "clouddocs",
        "cloudkit",
        "coresimulator",
        "crashpad",
        "crashreporter",
        "desktop pictures",
        "diagnosticreports",
        "diskimages",
        "dock",
        "familycircle",
        "fileprovider",
        "geoservices",
        "homekit",
        "icdd",
        "icloud",
        "identityservices",
        "ilifemediabrowser",
        "knowledge",
        "logs",
        "metadata",
        "mobile documents",
        "mobilesync",
        "networkserviceproxy",
        "passkit",
        "privacypreservingmeasurement",
        "quick look",
        "siritts",
        "spotlight",
        "syncservices",
        "temp",
        "tmp",
    ];
    let core = container_core(key);
    // macOS services are named like "askpermissiond", "familycircled".
    let daemon =
        core.len() >= 6 && core.ends_with('d') && core.chars().all(|c| c.is_ascii_lowercase());
    core.starts_with("com.apple.") || core == "apple" || daemon || SYSTEM.contains(&core)
}

/// A reverse-DNS identifier ("com.vendor.app"), not a plain name.
fn is_bundle_like(core: &str) -> bool {
    core.matches('.').count() >= 2
}

/// Looser checks than `match_all`, used only before calling something a
/// leftover: when in doubt, it belongs to an installed app.
fn loosely_claimed(apps: &[App], key: &str) -> bool {
    let core = container_core(key);
    let norm = normalize(core);
    apps.iter().any(|app| {
        let by_name = std::iter::once(normalize(&app.name))
            .chain(app.aliases.iter().map(|a| normalize(a)))
            .any(|n| norm == n || (n.len() >= 5 && norm.starts_with(&n)));
        // "dev.warp" for "dev.warp.Warp-Stable", and helpers sharing a
        // product prefix: "com.microsoft.autoupdate.fba" for
        // "com.microsoft.autoupdate2" (at least three components in common).
        let product = core
            .rsplit_once('.')
            .map(|(p, _)| p)
            .filter(|p| p.matches('.').count() >= 2);
        let by_prefix = app.bundle.as_deref().is_some_and(|b| {
            b.starts_with(&format!("{core}.")) || product.is_some_and(|p| b.starts_with(p))
        });
        // The vendor folder of an installed app ("Microsoft" for Excel).
        let vendor_of_app = app
            .bundle
            .as_deref()
            .and_then(|b| b.split('.').nth(1))
            .is_some_and(|v| v == norm)
            || (app.name.contains(' ')
                && app
                    .name
                    .split_whitespace()
                    .next()
                    .is_some_and(|w| normalize(w) == norm));
        by_name || by_prefix || vendor_of_app
    })
}

/// Whether an unclaimed data folder should be reported as a leftover.
fn is_leftover_candidate(apps: &[App], place: &str, key: &str) -> bool {
    let core = container_core(key);
    if is_system_data(key) || loosely_claimed(apps, key) {
        return false;
    }
    match place {
        // Shared group containers ("TEAMID.Office") carry no app identity.
        "Group Containers" => is_bundle_like(core),
        // Plain names here are mostly command line tools' caches; the
        // caches report covers them.
        "Caches" | "Logs" | "HTTPStorages" | "WebKit" => is_bundle_like(core),
        // A command of the same name in PATH means the tool is installed.
        _ => is_bundle_like(core) || crate::tools::which(core).is_none(),
    }
}

/// Data folders that belong to no installed app: leftovers of removed apps.
pub fn orphans(tree: &Tree, mode: SizeMode, now: u64, min_age_days: u32) -> ResultList {
    let kind = ReportKind::Orphans;
    let age = AgeFilter::new(now, min_age_days);
    let platform = Platform::current();
    // Installed apps come from the scan and, on macOS, straight from the
    // usual application folders, so scanning just the home folder is enough.
    let mut apps = find_apps(tree, ROOT, platform);
    if platform == Platform::MacOs {
        let mut dirs = vec![
            PathBuf::from("/Applications"),
            PathBuf::from("/System/Applications"),
        ];
        dirs.extend(dirs::home_dir().map(|h| h.join("Applications")));
        apps.extend(installed_on_disk(&dirs));
    }
    let mut title = kind.label().to_string();
    if min_age_days > 0 {
        title.push_str(&tf!(
            " · ≥ {min_age_days} gündür dokunulmamış",
            " · untouched for ≥ {min_age_days} days"
        ));
    }
    let finish = |rows: Vec<Row>, truncated: bool, note: String| {
        let mut list = ResultList::new(title.clone(), ROOT, rows);
        list.truncated = truncated;
        list.note = note;
        list.source = Source::Report { kind, min_age_days };
        list
    };
    if apps.is_empty() {
        return finish(
            Vec::new(),
            false,
            t!(
                "Kurulu uygulama bulunamadı, bu yüzden sahipsiz veri belirlenemez.",
                "No installed applications found, so leftovers cannot be determined."
            )
            .into(),
        );
    }
    let (data, _) = data_folders(tree, platform);
    if data.is_empty() {
        return finish(
            Vec::new(),
            false,
            t!(
                "Bu taramada Library klasörü yok. Ev klasörünüzü (~) ya da diski tarayın.",
                "This scan has no Library folder. Scan your home folder (~) or the disk."
            )
            .into(),
        );
    }
    let (_, unmatched) = match_all(&apps, &data, &|id| vendor_children(tree, id));
    let mut found: Vec<(NodeId, String)> = unmatched
        .into_iter()
        .map(|i| &data[i])
        .filter(|d| {
            let place = tree.parent(d.id).map_or("", |p| tree.name(p));
            is_leftover_candidate(&apps, place, &d.key)
        })
        .filter(|d| {
            let n = tree.node(d.id);
            n.size.get(mode) >= ORPHAN_MIN && age.ok(n.modified)
        })
        .map(|d| {
            let place = tree.parent(d.id).map_or("", |p| tree.name(p));
            let reason = if is_bundle_like(container_core(&d.key)) {
                t!(
                    "paket kimliği: yüklü uygulama yok",
                    "bundle id: no installed app"
                )
            } else {
                t!(
                    "ad: yüklü uygulama yok (araç olabilir)",
                    "name: no installed app (may be a tool)"
                )
            };
            (d.id, format!("{place} · {reason}"))
        })
        .collect();
    found.sort_by_key(|(id, _)| std::cmp::Reverse(tree.node(*id).size.get(mode)));
    let truncated = found.len() > LIMIT;
    found.truncate(LIMIT);
    let rows = found
        .into_iter()
        .map(|(id, detail)| Row::single(tree, ROOT, id, mode, detail))
        .collect();
    finish(
        rows,
        truncated,
        t!(
            "Komut satırı araçları da veri tutabilir; silmeden önce Enter ile içine bakın.",
            "Command line tools keep data too; press Enter to look inside before deleting."
        )
        .into(),
    )
}

#[cfg(test)]
mod tests {

    use super::*;

    use crate::apps::test_util::app;

    #[test]
    fn leftover_candidates_err_on_the_safe_side() {
        let apps = [
            app(1, "Microsoft Excel", Some("com.microsoft.excel")),
            app(2, "Docker", Some("com.docker.docker")),
            app(3, "Warp", Some("dev.warp.warp-stable")),
            app(4, "HTTP Toolkit", None),
            app(5, "Microsoft AutoUpdate", Some("com.microsoft.autoupdate2")),
        ];
        let c = |place: &str, key: &str| is_leftover_candidate(&apps, place, &key.to_lowercase());
        // Shared or vendor folders of installed apps.
        assert!(!c("Group Containers", "UBF8T346G9.Office"));
        assert!(!c("Application Support", "Microsoft"));
        assert!(!c("Group Containers", "2BBY89MBSN.dev.warp"));
        assert!(!c("Application Support", "Docker Desktop"));
        assert!(!c("Application Support", "httptoolkit"));
        assert!(!c("HTTPStorages", "com.microsoft.autoupdate.fba"));
        // Tool caches with plain names and macOS services.
        assert!(!c("Caches", "typescript"));
        assert!(!c("Caches", "askpermissiond"));
        // Real leftovers.
        assert!(c("Containers", "com.tinyspeck.slackmacgap"));
        assert!(c("Group Containers", "S8EX82NJP6.com.macpaw.CleanMyMac5"));
        assert!(c("Application Support", "zz-removed-app-zz"));
    }

    #[test]
    fn system_folders_are_never_leftovers() {
        assert!(is_system_data("com.apple.notes"));
        assert!(is_system_data("abcde12345.com.apple.foo"));
        assert!(is_system_data("mobilesync"));
        assert!(is_system_data("crashreporter"));
        assert!(!is_system_data("com.spotify.client"));
        assert!(!is_system_data("oldtool"));
    }
}
