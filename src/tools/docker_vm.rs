//! The Docker Desktop disk image (`Docker.raw`): its size on disk and its
//! apparent (maximum) size.
//!
//! Only measured. Docker Desktop gives the space freed by pruning back to
//! the host by itself (TRIM). The manual reclaim in its documentation,
//! `docker run --privileged --pid=host docker/desktop-reclaim-space`, runs a
//! privileged container with the host's process namespace from an image last
//! updated in 2019 and built for amd64 only, so rustClean shows a hint
//! instead of running it. Shrinking the disk limit in Docker Desktop's
//! settings deletes every image and container.

use std::path::PathBuf;

use crate::ui::fmt_size;

use super::home;

pub fn hint() -> &'static str {
    t!(
        "Docker Desktop, temizlikle boşalan alanı diske kendisi geri verir (TRIM, birkaç dakika sürebilir). Belgelerdeki elle geri kazanma komutu eski, yalnızca amd64 bir imajdan ayrıcalıklı container çalıştırdığı için sunulmuyor. Ayrıntı: docs.docker.com/desktop/troubleshoot-and-support/faqs/macfaqs",
        "Docker Desktop gives the space freed by pruning back to the disk by itself (TRIM, may take a few minutes). Its documented manual reclaim runs a privileged container from an old amd64-only image, so it is not offered. See docs.docker.com/desktop/troubleshoot-and-support/faqs/macfaqs",
    )
}

/// Docker Desktop's settings that matter here.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// The folder holding `Docker.raw`, when moved in the settings.
    pub data_folder: Option<PathBuf>,
    /// Whether the VM passes TRIM to the disk image.
    pub trim: Option<bool>,
}

/// Reads `settings-store.json` (Docker Desktop 4.34 and later) or the older
/// `settings.json`, whose keys start in lower case.
pub fn parse_settings(json: &str) -> Settings {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Settings::default();
    };
    let get = |upper: &str, lower: &str| v.get(upper).or_else(|| v.get(lower)).cloned();
    Settings {
        data_folder: get("DataFolder", "dataFolder")
            .and_then(|f| f.as_str().map(PathBuf::from))
            .filter(|f| !f.as_os_str().is_empty()),
        trim: get("DiskTRIM", "diskTRIM").and_then(|t| t.as_bool()),
    }
}

fn settings() -> Settings {
    let Some(dir) = home().map(|h| h.join("Library/Group Containers/group.com.docker")) else {
        return Settings::default();
    };
    ["settings-store.json", "settings.json"]
        .iter()
        .find_map(|f| std::fs::read_to_string(dir.join(f)).ok())
        .map(|s| parse_settings(&s))
        .unwrap_or_default()
}

/// `(on disk, apparent)` size of `Docker.raw`, and whether TRIM is on.
fn measure() -> Option<(u64, u64, Option<bool>)> {
    let home = home()?;
    let settings = if cfg!(target_os = "macos") {
        settings()
    } else {
        Settings::default()
    };
    let default = if cfg!(target_os = "macos") {
        home.join("Library/Containers/com.docker.docker/Data/vms/0/data")
    } else {
        home.join(".docker/desktop/vms/0/data")
    };
    let file = settings.data_folder.unwrap_or(default).join("Docker.raw");
    let md = std::fs::metadata(file).ok()?;
    #[cfg(unix)]
    let disk = {
        use std::os::unix::fs::MetadataExt;
        md.blocks() * 512
    };
    #[cfg(not(unix))]
    let disk = md.len();
    Some((disk, md.len(), settings.trim))
}

/// "Docker.raw 111 GB on disk of 119 GB" for the tool's details, or `None`
/// without Docker Desktop.
pub fn describe() -> Option<String> {
    let (disk, apparent, trim) = measure()?;
    Some(format_sizes(disk, apparent, trim))
}

fn format_sizes(disk: u64, apparent: u64, trim: Option<bool>) -> String {
    let mut text = tf!(
        "Docker.raw diskte {} / en çok {}",
        "Docker.raw {} on disk of {}",
        fmt_size(disk),
        fmt_size(apparent)
    );
    if trim == Some(false) {
        text.push_str(t!(" (TRIM kapalı)", " (TRIM off)"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_settings_store() {
        let s = parse_settings(include_str!(
            "../../tests/fixtures/docker_settings_store.json"
        ));
        assert_eq!(
            s.data_folder,
            Some(PathBuf::from(
                "/Users/demo/Library/Containers/com.docker.docker/Data/vms/0/data"
            ))
        );
        assert_eq!(s.trim, Some(true));
        // The older settings.json, and nothing usable.
        let old = parse_settings(r#"{"dataFolder": "/x", "diskTRIM": false}"#);
        assert_eq!(old.data_folder, Some(PathBuf::from("/x")));
        assert_eq!(old.trim, Some(false));
        assert_eq!(parse_settings("{}"), Settings::default());
        assert_eq!(parse_settings("not json"), Settings::default());
    }

    #[test]
    fn describes_both_sizes() {
        let gib = 1 << 30;
        let text = format_sizes(2 * gib, 64 * gib, Some(true));
        assert!(text.starts_with("Docker.raw diskte"), "{text}");
        assert!(!text.contains("TRIM"));
        assert!(format_sizes(gib, gib, Some(false)).contains("TRIM kapalı"));
    }
}
