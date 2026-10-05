//! Xcode archives older than a year, and simulators that were never used or
//! not used for over a year.

use std::path::{Path, PathBuf};

use crate::i18n::count;
use crate::ui::{fmt_date, fmt_size, now_secs};

use super::{action, command, dir_size, home, missing, ready, CleanAction, Risk, Status, Step};

/// Archives and simulators unused for this long are offered for removal.
pub const STALE_SECS: u64 = 365 * 24 * 3600;

/// Parses "2026-01-08T09:00:29Z" to epoch seconds.
fn parse_time(s: &str) -> Option<u64> {
    let t = chrono::DateTime::parse_from_rfc3339(s).ok()?;
    u64::try_from(t.timestamp()).ok()
}

// --------------------------------------------------------------- archives

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    pub path: PathBuf,
    /// When it was made, in epoch seconds.
    pub time: u64,
    pub size: u64,
}

/// The `.xcarchive` bundles in Xcode's archives folder. Xcode keeps them in
/// one folder per day ("2026-05-07"); the folder name gives the date, the
/// bundle's modification time is the fallback.
pub fn list_archives(dir: &Path) -> Vec<Archive> {
    let Ok(days) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for day in days.flatten() {
        let day_path = day.path();
        if !day_path.is_dir() {
            continue;
        }
        let day_time =
            chrono::NaiveDate::parse_from_str(&day.file_name().to_string_lossy(), "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .and_then(|d| u64::try_from(d.and_utc().timestamp()).ok());
        let Ok(entries) = std::fs::read_dir(&day_path) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "xcarchive") || !path.is_dir() {
                continue;
            }
            let modified = e
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            let Some(time) = day_time.or(modified) else {
                continue;
            };
            out.push(Archive {
                size: dir_size(&path).unwrap_or(0),
                path,
                time,
            });
        }
    }
    out.sort_by_key(|a| a.time);
    out
}

pub fn archives() -> (Status, Vec<CleanAction>) {
    let Some(dir) = home()
        .map(|h| h.join("Library/Developer/Xcode/Archives"))
        .filter(|d| d.is_dir())
    else {
        return missing(t!("klasör yok", "no folder"));
    };
    let all = list_archives(&dir);
    let total: u64 = all.iter().map(|a| a.size).sum();
    let now = now_secs();
    let old: Vec<&Archive> = all.iter().filter(|a| a.time + STALE_SECS < now).collect();
    let old_size: u64 = old.iter().map(|a| a.size).sum();
    let detail = tf!(
        "{} ({}) · 12 aydan eski {} ({})",
        "{} ({}) · {} older than 12 months ({})",
        count(all.len() as u64, "arşiv", "archive", "archives"),
        fmt_size(total),
        old.len(),
        fmt_size(old_size)
    );
    let mut actions = Vec::new();
    if !old.is_empty() {
        actions.push(action(
            tf!(
                "12 aydan eski {} çöp kutusuna taşı ({})",
                "Move {} older than 12 months to the trash ({})",
                count(old.len() as u64, "arşivi", "archive", "archives"),
                fmt_size(old_size)
            ),
            Risk::DataLoss,
            old.iter().map(|a| Step::Trash(a.path.clone())).collect(),
        ));
    }
    ready(old_size, detail, actions)
}

// ------------------------------------------------------------- simulators

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimDevice {
    pub udid: String,
    pub name: String,
    /// "iOS 18.2"
    pub runtime: String,
    pub size: u64,
    /// When it was last used, in epoch seconds; `None` if never.
    pub last_used: Option<u64>,
}

/// "com.apple.CoreSimulator.SimRuntime.iOS-18-2" → "iOS 18.2".
fn runtime_name(key: &str) -> String {
    let short = key.rsplit('.').next().unwrap_or(key);
    match short.split_once('-') {
        Some((platform, version)) => format!("{platform} {}", version.replace('-', ".")),
        None => short.to_string(),
    }
}

/// Available, shut down simulators that were never used or not used within
/// `STALE_SECS` of `now`, largest first. Unavailable ones (their runtime is
/// gone) are left to `simctl delete unavailable`. Xcode 15 and later write
/// `lastUsedAt`; older versions wrote `lastBootedAt`.
pub fn parse_stale_devices(json: &str, now: u64) -> Vec<SimDevice> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(by_runtime) = v.get("devices").and_then(|d| d.as_object()) else {
        return Vec::new();
    };
    let str_of =
        |d: &serde_json::Value, k: &str| d.get(k).and_then(|s| s.as_str()).map(str::to_string);
    let mut out: Vec<SimDevice> = by_runtime
        .iter()
        .flat_map(|(rt, list)| list.as_array().into_iter().flatten().map(move |d| (rt, d)))
        .filter(|(_, d)| d.get("isAvailable").and_then(serde_json::Value::as_bool) == Some(true))
        .filter(|(_, d)| d.get("state").and_then(|s| s.as_str()) == Some("Shutdown"))
        .filter_map(|(rt, d)| {
            let used = d
                .get("lastUsedAt")
                .or_else(|| d.get("lastBootedAt"))
                .and_then(|s| s.as_str());
            let last_used = match used {
                // A date that cannot be read: keep the device.
                Some(s) => Some(parse_time(s)?),
                None => None,
            };
            if last_used.is_some_and(|t| t + STALE_SECS >= now) {
                return None;
            }
            Some(SimDevice {
                udid: str_of(d, "udid")?,
                name: str_of(d, "name")?,
                runtime: runtime_name(rt),
                size: d
                    .get("dataPathSize")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                last_used,
            })
        })
        .collect();
    out.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    out
}

/// Actions for stale simulators: all never-used ones, all unused for a
/// year, then each device. Steps are one `simctl delete` per device, so a
/// device chosen twice is deleted once (the screen drops repeated steps).
pub fn stale_device_actions(xcrun: &Path, devices: &[SimDevice]) -> Vec<CleanAction> {
    let delete = |d: &SimDevice| command(xcrun, &["simctl", "delete", &d.udid]);
    let (never, old): (Vec<&SimDevice>, Vec<&SimDevice>) =
        devices.iter().partition(|d| d.last_used.is_none());
    let size = |list: &[&SimDevice]| fmt_size(list.iter().map(|d| d.size).sum());
    let mut actions = Vec::new();
    if never.len() > 1 {
        actions.push(action(
            tf!(
                "Hiç kullanılmamış {} tümü ({})",
                "All {} never used ({})",
                count(never.len() as u64, "simülatörün", "simulator", "simulators"),
                size(&never)
            ),
            Risk::Safe,
            never.iter().map(|d| delete(d)).collect(),
        ));
    }
    if old.len() > 1 {
        actions.push(action(
            tf!(
                "Bir yıldır kullanılmayan {} tümü, uygulama verileriyle ({})",
                "All {} unused for a year, with their app data ({})",
                count(old.len() as u64, "simülatörün", "simulator", "simulators"),
                size(&old)
            ),
            Risk::DataLoss,
            old.iter().map(|d| delete(d)).collect(),
        ));
    }
    for d in devices {
        let (when, risk) = match d.last_used {
            Some(t) => (
                tf!("son kullanım {}", "last used {}", fmt_date(t as u32)),
                Risk::DataLoss,
            ),
            None => (t!("hiç kullanılmadı", "never used").to_string(), Risk::Safe),
        };
        actions.push(action(
            tf!(
                "{} · {} · {when} · {}",
                "{} · {} · {when} · {}",
                d.name,
                d.runtime,
                fmt_size(d.size)
            ),
            risk,
            vec![delete(d)],
        ));
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 5 October 2026, when the fixture was extended.
    const NOW: u64 = 1_791_158_400;

    fn fixture() -> Vec<SimDevice> {
        parse_stale_devices(
            include_str!("../../tests/fixtures/simctl_devices.json"),
            NOW,
        )
    }

    #[test]
    fn stale_simulators_from_the_fixture() {
        let devices = fixture();
        let names: Vec<(&str, Option<u64>)> = devices
            .iter()
            .filter(|d| d.last_used.is_some())
            .map(|d| (d.name.as_str(), d.last_used))
            .collect();
        // Used in 2024 (the newer and the older key); the booted and the
        // recently used devices are not listed.
        assert_eq!(
            names,
            [
                ("iPhone 15 Pro", parse_time("2024-06-01T10:00:00Z")),
                (
                    "iPad Air (5th generation)",
                    parse_time("2024-03-12T08:30:00Z")
                ),
            ]
        );
        let never = devices.iter().filter(|d| d.last_used.is_none()).count();
        assert!(never > 10, "{never}");
        assert!(devices.iter().all(|d| d.udid.len() == 36));
        assert!(devices.windows(2).all(|w| w[0].size >= w[1].size));
        assert!(!devices.iter().any(|d| d.name == "iPhone 16 (booted)"));
        let ios = devices.iter().find(|d| d.name == "iPhone 15 Pro").unwrap();
        assert_eq!(ios.runtime, "iOS 18.2");
        assert_eq!(parse_stale_devices("not json", NOW), []);
    }

    #[test]
    fn unreadable_dates_keep_the_device() {
        let json = r#"{"devices": {"com.apple.CoreSimulator.SimRuntime.iOS-18-2": [
            {"udid": "A", "name": "a", "isAvailable": true, "state": "Shutdown", "lastUsedAt": "yesterday"},
            {"udid": "B", "name": "b", "isAvailable": true, "state": "Shutdown"}
        ]}}"#;
        let names: Vec<String> = parse_stale_devices(json, NOW)
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(names, ["b"]);
    }

    #[test]
    fn simulator_actions_share_their_steps() {
        let devices = fixture();
        let actions = stale_device_actions(Path::new("/usr/bin/xcrun"), &devices);
        // Both groups, then one per device.
        assert_eq!(actions.len(), devices.len() + 2);
        assert_eq!(actions[0].risk, Risk::Safe);
        assert_eq!(actions[1].risk, Risk::DataLoss);
        assert_eq!(actions[1].steps.len(), 2);
        let single = actions
            .iter()
            .find(|a| a.label.contains("iPhone 15 Pro"))
            .unwrap();
        assert_eq!(single.risk, Risk::DataLoss);
        assert!(actions[1].steps.contains(&single.steps[0]));
        assert!(single.steps[0]
            .describe()
            .starts_with("xcrun simctl delete "));
    }

    #[test]
    fn archives_by_day_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let make = |rel: &str| {
            let p = tmp.path().join(rel);
            std::fs::create_dir_all(p.join("Products")).unwrap();
            std::fs::write(p.join("Info.plist"), vec![b'x'; 5000]).unwrap();
        };
        make("2024-09-07/App 07.09.2024, 10.00.xcarchive");
        make("2026-06-19/App 19.06.2026, 11.43.xcarchive");
        std::fs::create_dir_all(tmp.path().join("2025-01-01")).unwrap(); // empty day
        std::fs::create_dir_all(tmp.path().join("2026-01-01/Notes")).unwrap(); // not an archive
        std::fs::write(tmp.path().join(".DS_Store"), b"x").unwrap();
        let list = list_archives(tmp.path());
        assert_eq!(list.len(), 2);
        assert!(list[0].path.ends_with("App 07.09.2024, 10.00.xcarchive"));
        assert_eq!(list[0].time, parse_time("2024-09-07T00:00:00Z").unwrap());
        assert!(list[0].size >= 5000);
        let old: Vec<_> = list.iter().filter(|a| a.time + STALE_SECS < NOW).collect();
        assert_eq!(old.len(), 1);
    }
}
