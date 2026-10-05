//! The Android SDK's emulators (AVDs) and system images.
//!
//! An AVD is a `<name>.ini` file in the AVD folder that points to a
//! `<name>.avd` folder; that folder's `config.ini` names the system image it
//! boots (`image.sysdir.1`). Images no AVD uses can go; when the image of
//! any AVD cannot be read, none is offered.

use std::path::{Path, PathBuf};

use crate::i18n::count;
use crate::ui::fmt_size;

use super::{
    action, command, dir_size, home, missing, ready, which, CleanAction, Risk, Status, Step,
};

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// The SDK folder: `ANDROID_HOME`, `ANDROID_SDK_ROOT` (deprecated), then
/// Android Studio's default for the platform.
fn sdk_dir() -> Option<PathBuf> {
    let default = if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library/Android/sdk"))
    } else if cfg!(windows) {
        dirs::data_local_dir().map(|d| d.join("Android/Sdk"))
    } else {
        home().map(|h| h.join("Android/Sdk"))
    };
    [
        env_dir("ANDROID_HOME"),
        env_dir("ANDROID_SDK_ROOT"),
        default,
    ]
    .into_iter()
    .flatten()
    .find(|d| d.is_dir())
}

/// The AVD folder, as the emulator looks for it.
fn avd_home() -> Option<PathBuf> {
    env_dir("ANDROID_AVD_HOME")
        .or_else(|| env_dir("ANDROID_USER_HOME").map(|d| d.join("avd")))
        .or_else(|| env_dir("ANDROID_EMULATOR_HOME").map(|d| d.join("avd")))
        .or_else(|| home().map(|h| h.join(".android/avd")))
}

/// The value of `key` in a `key=value` file.
pub fn ini_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim())
}

/// "system-images/android-36/google_apis/arm64-v8a/" (or with `\` on
/// Windows) → "android-36/google_apis/arm64-v8a".
pub fn image_id(sysdir: &str) -> String {
    let s = sysdir.replace('\\', "/");
    let s = s.trim_matches('/');
    s.strip_prefix("system-images/").unwrap_or(s).to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Avd {
    /// The name `avdmanager` and the emulator use (the `.ini` file's).
    pub name: String,
    pub display: String,
    pub dir: PathBuf,
    pub ini: PathBuf,
    pub size: u64,
    /// The system image it boots, as `image_id` gives it; `None` when its
    /// `config.ini` cannot be read.
    pub image: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct AvdHome {
    pub avds: Vec<Avd>,
    /// `.avd` folders no `.ini` file points to, with their size.
    pub orphans: Vec<(PathBuf, u64)>,
}

pub fn read_avd_home(dir: &Path) -> AvdHome {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return AvdHome::default();
    };
    let entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    let mut home = AvdHome::default();
    for ini in entries
        .iter()
        .filter(|p| p.extension().is_some_and(|x| x == "ini") && p.is_file())
    {
        let Some(name) = ini.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(ini) else {
            continue;
        };
        // `path` is absolute; `path.rel` is relative to the folder above the
        // AVD folder (the user's `.android`), and survives moving it.
        let avd_dir = ini_value(&text, "path")
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .or_else(|| {
                let rel = ini_value(&text, "path.rel")?;
                Some(dir.parent()?.join(rel)).filter(|p| p.is_dir())
            })
            .or_else(|| Some(dir.join(format!("{name}.avd"))).filter(|p| p.is_dir()));
        let Some(avd_dir) = avd_dir else {
            continue;
        };
        let config = std::fs::read_to_string(avd_dir.join("config.ini")).ok();
        let config = config.as_deref();
        home.avds.push(Avd {
            display: config
                .and_then(|c| ini_value(c, "avd.ini.displayname"))
                .unwrap_or(&name)
                .to_string(),
            image: config
                .and_then(|c| ini_value(c, "image.sysdir.1"))
                .map(image_id),
            size: dir_size(&avd_dir).unwrap_or(0),
            name,
            dir: avd_dir,
            ini: ini.clone(),
        });
    }
    let used: Vec<PathBuf> = home.avds.iter().map(|a| canonical(&a.dir)).collect();
    for d in entries
        .iter()
        .filter(|p| p.extension().is_some_and(|x| x == "avd") && p.is_dir())
    {
        if !used.contains(&canonical(d)) {
            home.orphans.push((d.clone(), dir_size(d).unwrap_or(0)));
        }
    }
    home.avds.sort_by_key(|a| std::cmp::Reverse(a.size));
    home.orphans.sort();
    home
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemImage {
    /// "android-34/google_apis_playstore/arm64-v8a"
    pub id: String,
    pub path: PathBuf,
    pub size: u64,
}

/// The installed system images: `system-images/<api>/<tag>/<abi>`.
pub fn list_system_images(sdk: &Path) -> Vec<SystemImage> {
    let subdirs = |p: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(p)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        v.sort();
        v
    };
    let name = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    for api in subdirs(&sdk.join("system-images")) {
        for tag in subdirs(&api) {
            for abi in subdirs(&tag) {
                out.push(SystemImage {
                    id: format!("{}/{}/{}", name(&api), name(&tag), name(&abi)),
                    size: dir_size(&abi).unwrap_or(0),
                    path: abi,
                });
            }
        }
    }
    out
}

/// Images no AVD uses, or `None` when an AVD's image is unknown.
pub fn unused_images<'a>(images: &'a [SystemImage], avds: &[Avd]) -> Option<Vec<&'a SystemImage>> {
    let used: Vec<&str> = avds
        .iter()
        .map(|a| a.image.as_deref())
        .collect::<Option<_>>()?;
    Some(
        images
            .iter()
            .filter(|i| !used.contains(&i.id.as_str()))
            .collect(),
    )
}

/// `avdmanager` from the SDK's command-line tools, or from PATH. The old
/// `tools/bin/avdmanager` is skipped: it does not start on current Java.
fn avdmanager(sdk: Option<&Path>) -> Option<PathBuf> {
    let in_sdk = sdk.and_then(|sdk| {
        let tools = sdk.join("cmdline-tools");
        let latest = tools.join("latest/bin/avdmanager");
        if latest.is_file() {
            return Some(latest);
        }
        std::fs::read_dir(&tools)
            .ok()?
            .flatten()
            .map(|e| e.path().join("bin/avdmanager"))
            .find(|p| p.is_file())
    });
    in_sdk.or_else(|| {
        which("avdmanager").filter(|p| {
            p.parent()
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .is_none_or(|n| n != "tools")
        })
    })
}

/// The steps that delete an AVD: `avdmanager delete avd -n <name>`, or
/// moving its folder and `.ini` file to the trash.
pub fn delete_avd_steps(avdmanager: Option<&Path>, avd: &Avd) -> Vec<Step> {
    match avdmanager {
        Some(exe) => vec![command(exe, &["delete", "avd", "-n", &avd.name])],
        None => vec![Step::Trash(avd.dir.clone()), Step::Trash(avd.ini.clone())],
    }
}

pub fn measure() -> (Status, Vec<CleanAction>) {
    let sdk = sdk_dir();
    let avd_dir = avd_home().filter(|d| d.is_dir());
    if sdk.is_none() && avd_dir.is_none() {
        return missing(t!("kurulu değil", "not installed"));
    }
    let home = avd_dir.as_deref().map(read_avd_home).unwrap_or_default();
    let images = sdk.as_deref().map(list_system_images).unwrap_or_default();
    let unused = unused_images(&images, &home.avds);
    let avdmanager = avdmanager(sdk.as_deref());

    let mut actions = Vec::new();
    let mut reclaimable = 0;
    for avd in &home.avds {
        actions.push(action(
            tf!(
                "{} emülatörünü uygulama ve verileriyle sil ({})",
                "Delete the {} emulator with its apps and data ({})",
                avd.display,
                fmt_size(avd.size)
            ),
            Risk::DataLoss,
            delete_avd_steps(avdmanager.as_deref(), avd),
        ));
    }
    for (dir, size) in &home.orphans {
        reclaimable += size;
        let name = dir.file_name().unwrap_or_default().to_string_lossy();
        actions.push(action(
            tf!(
                "Hiçbir .ini'nin göstermediği {name} klasörü ({})",
                "Leftover {name} folder, no .ini points to it ({})",
                fmt_size(*size)
            ),
            Risk::DataLoss,
            vec![Step::Trash(dir.clone())],
        ));
    }
    for image in unused.iter().flatten().filter(|i| i.size > 0) {
        reclaimable += image.size;
        actions.push(action(
            tf!(
                "Kullanılmayan imaj {} ({})",
                "Unused image {} ({})",
                image.id,
                fmt_size(image.size)
            ),
            Risk::Redownload,
            vec![Step::Trash(image.path.clone())],
        ));
    }

    let avd_size: u64 = home.avds.iter().map(|a| a.size).sum();
    let image_size: u64 = images.iter().map(|i| i.size).sum();
    let mut detail = tf!(
        "{} ({}) · {} ({})",
        "{} ({}) · {} ({})",
        count(home.avds.len() as u64, "emülatör", "emulator", "emulators"),
        fmt_size(avd_size),
        count(
            images.len() as u64,
            "sistem imajı",
            "system image",
            "system images"
        ),
        fmt_size(image_size)
    );
    match &unused {
        Some(list) => detail.push_str(&tf!(", kullanılmayan {}", ", {} unused", list.len())),
        None => detail.push_str(t!(
            ", bir emülatörün imajı okunamadı",
            ", an emulator's image could not be read"
        )),
    }
    ready(reclaimable, detail, actions)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INI: &str = include_str!("../../tests/fixtures/android/Pixel_9_Pro_XL.ini");
    const CONFIG: &str = include_str!("../../tests/fixtures/android/config.ini");

    #[test]
    fn reads_the_ini_files() {
        assert_eq!(ini_value(INI, "path.rel"), Some("avd/Pixel_9_Pro_XL_2.avd"));
        assert_eq!(
            ini_value(CONFIG, "avd.ini.displayname"),
            Some("Pixel 9 Pro XL")
        );
        assert_eq!(
            ini_value(CONFIG, "image.sysdir.1").map(image_id).as_deref(),
            Some("android-36/google_apis_playstore/arm64-v8a")
        );
        assert_eq!(ini_value(CONFIG, "missing"), None);
        assert_eq!(
            image_id(r"system-images\android-34\google_apis\x86_64\"),
            "android-34/google_apis/x86_64"
        );
    }

    /// An SDK and an `.android` folder laid out as on the machine the
    /// fixtures come from: the `.ini` points to a renamed `_2.avd` folder,
    /// and the old folder is left over.
    fn layout() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let avd = tmp.path().join("user/avd");
        std::fs::create_dir_all(avd.join("Pixel_9_Pro_XL_2.avd")).unwrap();
        std::fs::create_dir_all(avd.join("Pixel_9_Pro_XL.avd/data")).unwrap();
        std::fs::write(avd.join("Pixel_9_Pro_XL.avd/cache.img"), vec![1; 9000]).unwrap();
        std::fs::write(avd.join("Pixel_9_Pro_XL.ini"), INI).unwrap();
        std::fs::write(avd.join("Pixel_9_Pro_XL_2.avd/config.ini"), CONFIG).unwrap();
        std::fs::write(
            avd.join("Pixel_9_Pro_XL_2.avd/userdata-qemu.img"),
            vec![1; 20000],
        )
        .unwrap();
        for (image, size) in [
            ("android-36/google_apis_playstore/arm64-v8a", 7000),
            ("android-34/google_apis_playstore/arm64-v8a", 5000),
            ("android-Baklava/google_apis_ps16k/arm64-v8a", 0),
        ] {
            let dir = tmp.path().join("sdk/system-images").join(image);
            std::fs::create_dir_all(&dir).unwrap();
            if size > 0 {
                std::fs::write(dir.join("system.img"), vec![1; size]).unwrap();
            }
        }
        tmp
    }

    #[test]
    fn finds_avds_orphans_and_unused_images() {
        let tmp = layout();
        let home = read_avd_home(&tmp.path().join("user/avd"));
        assert_eq!(home.avds.len(), 1);
        let avd = &home.avds[0];
        assert_eq!(avd.name, "Pixel_9_Pro_XL");
        assert_eq!(avd.display, "Pixel 9 Pro XL");
        assert!(
            avd.dir.ends_with("Pixel_9_Pro_XL_2.avd"),
            "path.rel is used"
        );
        assert!(avd.size >= 20000);
        assert_eq!(home.orphans.len(), 1);
        assert!(home.orphans[0].0.ends_with("Pixel_9_Pro_XL.avd"));

        let images = list_system_images(&tmp.path().join("sdk"));
        assert_eq!(images.len(), 3);
        let unused: Vec<&str> = unused_images(&images, &home.avds)
            .unwrap()
            .iter()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(
            unused,
            [
                "android-34/google_apis_playstore/arm64-v8a",
                "android-Baklava/google_apis_ps16k/arm64-v8a"
            ]
        );
    }

    #[test]
    fn keeps_every_image_when_an_avd_is_unreadable() {
        let tmp = layout();
        let avd = tmp.path().join("user/avd");
        std::fs::remove_file(avd.join("Pixel_9_Pro_XL_2.avd/config.ini")).unwrap();
        let home = read_avd_home(&avd);
        assert_eq!(home.avds[0].image, None);
        let images = list_system_images(&tmp.path().join("sdk"));
        assert_eq!(unused_images(&images, &home.avds), None);
    }

    #[test]
    fn deletes_with_avdmanager_or_moves_both_files_to_the_trash() {
        let tmp = layout();
        let home = read_avd_home(&tmp.path().join("user/avd"));
        let avd = &home.avds[0];
        let steps = delete_avd_steps(
            Some(Path::new("/sdk/cmdline-tools/latest/bin/avdmanager")),
            avd,
        );
        assert_eq!(steps.len(), 1);
        assert_eq!(
            steps[0].describe(),
            "avdmanager delete avd -n Pixel_9_Pro_XL"
        );
        let steps = delete_avd_steps(None, avd);
        assert_eq!(
            steps,
            [Step::Trash(avd.dir.clone()), Step::Trash(avd.ini.clone())]
        );
    }
}
