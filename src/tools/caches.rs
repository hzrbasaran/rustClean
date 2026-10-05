//! More package manager and SDK caches: Go, Maven, Bun, uv, conda, the
//! Flutter / Dart pub cache and Playwright's browsers.
//!
//! Each one is found the way the tool itself finds it (its own command or
//! environment variable first, then the default folder) and cleaned with
//! the tool's own command when there is one that works from anywhere.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::ui::fmt_size;

use super::{
    action, cache_command, command, dir_size, guarded, home, missing, output, ready, trash_folder,
    which, CleanAction, Risk, Status, Step,
};

/// A folder from an environment variable, when it is set and not empty.
fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Go: the module cache (downloaded sources) and the build cache.
pub fn go() -> (Status, Vec<CleanAction>) {
    let Some(go) = which("go") else {
        return missing(t!("kurulu değil", "not installed"));
    };
    let Some(out) = output(
        &go,
        &["env", "GOMODCACHE", "GOCACHE"],
        Duration::from_secs(20),
    ) else {
        return (
            Status::Unavailable(t!("go env başarısız", "go env failed").into()),
            Vec::new(),
        );
    };
    let (modcache, buildcache) = parse_go_env(&out);
    let mut total = 0;
    let mut details = Vec::new();
    let mut actions = Vec::new();
    // The module cache is read-only on purpose; only `go clean` removes it
    // properly.
    if let Some(size) = modcache.as_deref().and_then(dir_size) {
        total += size;
        details.push(tf!("modüller {}", "modules {}", fmt_size(size)));
        actions.push(action(
            t!(
                "Modül önbelleğini temizle (modüller yeniden indirilir)",
                "Clean the module cache (modules are downloaded again)",
            ),
            Risk::Redownload,
            vec![command(&go, &["clean", "-modcache"])],
        ));
    }
    if let Some(size) = buildcache.as_deref().and_then(dir_size) {
        total += size;
        details.push(tf!("derleme {}", "build {}", fmt_size(size)));
        actions.push(action(
            t!(
                "Derleme önbelleğini temizle (yeniden derlenir)",
                "Clean the build cache (rebuilt when needed)",
            ),
            Risk::Safe,
            vec![command(&go, &["clean", "-cache"])],
        ));
    }
    if actions.is_empty() {
        return missing(t!("önbellek klasörü yok", "no cache folder"));
    }
    ready(total, details.join(" · "), actions)
}

/// The module and build cache folders from `go env GOMODCACHE GOCACHE`
/// (one per line; an empty line when not set).
pub fn parse_go_env(out: &str) -> (Option<PathBuf>, Option<PathBuf>) {
    let mut lines = out
        .lines()
        .map(|l| Some(l.trim()).filter(|l| !l.is_empty()).map(PathBuf::from));
    (lines.next().flatten(), lines.next().flatten())
}

/// Maven: the local repository of downloaded artifacts.
pub fn maven() -> (Status, Vec<CleanAction>) {
    trash_folder(
        home().map(|h| h.join(".m2/repository")),
        Risk::Redownload,
        t!(
            "Yerel depoyu çöpe taşı (yeniden indirilir ya da derlenir)",
            "Move the local repository to the trash (downloaded or built again)",
        ),
    )
}

/// Bun's global install cache. `bun pm cache rm` refuses to run outside a
/// project (it needs a `package.json`), so the folder is emptied directly,
/// which is what that command does.
pub fn bun() -> (Status, Vec<CleanAction>) {
    let dir = env_dir("BUN_INSTALL_CACHE_DIR")
        .or_else(|| env_dir("BUN_INSTALL").map(|d| d.join("install/cache")))
        .or_else(|| home().map(|h| h.join(".bun/install/cache")));
    trash_folder(
        dir,
        Risk::Redownload,
        t!(
            "Bun önbelleğini çöpe taşı (paketler yeniden indirilir)",
            "Move the Bun cache to the trash (packages are downloaded again)",
        ),
    )
}

/// uv reports its cache folder (`UV_CACHE_DIR` included) and cleans it.
pub fn uv() -> (Status, Vec<CleanAction>) {
    cache_command(
        "uv",
        &["cache", "dir"],
        &["cache", "clean"],
        Risk::Redownload,
        t!(
            "uv önbelleğini temizle (paketler yeniden indirilir)",
            "Clean the uv cache (packages are downloaded again)",
        ),
    )
}

/// conda: its own dry run tells what `conda clean --all` would free.
pub fn conda() -> (Status, Vec<CleanAction>) {
    // `CONDA_EXE` is set by `conda init` even when only the shell function
    // is in PATH.
    let Some(conda) = which("conda").or_else(|| env_dir("CONDA_EXE").filter(|p| p.is_file()))
    else {
        return missing(t!("kurulu değil", "not installed"));
    };
    let Some(out) = output(
        &conda,
        &["clean", "--all", "--dry-run", "--json"],
        Duration::from_secs(120),
    ) else {
        return (
            Status::Unavailable(
                t!(
                    "conda clean --dry-run başarısız",
                    "conda clean --dry-run failed"
                )
                .into(),
            ),
            Vec::new(),
        );
    };
    let Some(dry) = parse_conda_dry_run(&out) else {
        return (
            Status::Unavailable(
                t!("conda çıktısı okunamadı", "could not read conda's output").into(),
            ),
            Vec::new(),
        );
    };
    let detail = tf!(
        "paketler {} · arşivler {} · {}",
        "packages {} · tarballs {} · {}",
        fmt_size(dry.packages),
        fmt_size(dry.tarballs),
        dry.dirs.join(", ")
    );
    ready(
        dry.packages + dry.tarballs,
        detail,
        vec![action(
            t!(
                "Kullanılmayan paketler, arşivler, dizin önbelleği (conda clean)",
                "Unused packages, tarballs, index cache (conda clean)",
            ),
            Risk::Redownload,
            // Without --yes conda asks, and its standard input is closed.
            vec![command(&conda, &["clean", "--all", "--yes"])],
        )],
    )
}

/// What `conda clean --all --dry-run --json` would free.
#[derive(Debug, PartialEq, Eq)]
pub struct CondaDryRun {
    /// Unused extracted packages, in bytes.
    pub packages: u64,
    /// Downloaded package archives, in bytes.
    pub tarballs: u64,
    /// The package cache folders.
    pub dirs: Vec<String>,
}

pub fn parse_conda_dry_run(json: &str) -> Option<CondaDryRun> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let total = |key: &str| {
        v.get(key)
            .and_then(|p| p.get("total_size"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    };
    let mut dirs: Vec<String> = ["packages", "tarballs"]
        .iter()
        .filter_map(|k| v.get(k)?.get("pkgs_dirs")?.as_object())
        .flat_map(|m| m.keys().cloned())
        .collect();
    dirs.sort();
    dirs.dedup();
    (v.get("success").and_then(serde_json::Value::as_bool) != Some(false)).then(|| CondaDryRun {
        packages: total("packages"),
        tarballs: total("tarballs"),
        dirs,
    })
}

/// The pub cache shared by Flutter and Dart (`PUB_CACHE`, or its default).
pub fn dart_pub() -> (Status, Vec<CleanAction>) {
    let dir = env_dir("PUB_CACHE").or_else(|| {
        if cfg!(windows) {
            dirs::data_local_dir().map(|d| d.join("Pub/Cache"))
        } else {
            home().map(|h| h.join(".pub-cache"))
        }
    });
    let Some(dir) = dir.filter(|d| d.is_dir()) else {
        return missing(t!("klasör yok", "no folder"));
    };
    let dir = match guarded(dir) {
        Ok(dir) => dir,
        Err(refused) => return refused,
    };
    let size = dir_size(&dir).unwrap_or(0);
    ready(
        size,
        dir.display().to_string(),
        vec![action(
            t!(
                "pub önbelleğini temizle (yeniden indirilir; global paketler de gider)",
                "Clean the pub cache (downloaded again; global packages go too)",
            ),
            Risk::Redownload,
            vec![pub_clean_step(&dir)],
        )],
    )
}

/// `flutter pub cache clean`, or Dart's, or emptying the folder. `--force`
/// skips the question the closed standard input could not answer.
fn pub_clean_step(dir: &Path) -> Step {
    match which("flutter").or_else(|| which("dart")) {
        Some(exe) => command(&exe, &["pub", "cache", "clean", "--force"]),
        None => Step::TrashContents(dir.to_path_buf()),
    }
}

/// Browsers downloaded by Playwright (`PLAYWRIGHT_BROWSERS_PATH`, or the
/// user's cache folder).
pub fn playwright() -> (Status, Vec<CleanAction>) {
    let dir = env_dir("PLAYWRIGHT_BROWSERS_PATH")
        // "0" means "inside node_modules", which is not one folder.
        .filter(|d| d.as_os_str() != "0")
        .or_else(|| dirs::cache_dir().map(|d| d.join("ms-playwright")));
    let Some(dir) = dir.filter(|d| d.is_dir()) else {
        return missing(t!("klasör yok", "no folder"));
    };
    let dir = match guarded(dir) {
        Ok(dir) => dir,
        Err(refused) => return refused,
    };
    let size = dir_size(&dir).unwrap_or(0);
    let step = match which("npx") {
        // --yes: npx would otherwise ask before fetching Playwright.
        Some(npx) => command(&npx, &["--yes", "playwright", "uninstall", "--all"]),
        None => Step::TrashContents(dir.clone()),
    };
    ready(
        size,
        dir.display().to_string(),
        vec![action(
            t!(
                "Tüm Playwright tarayıcılarını kaldır (install yeniden indirir)",
                "Remove every Playwright browser (install downloads them again)",
            ),
            Risk::Redownload,
            vec![step],
        )],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_env_lines() {
        let (m, b) = parse_go_env("/Users/demo/go/pkg/mod\n/Users/demo/Library/Caches/go-build\n");
        assert_eq!(m, Some(PathBuf::from("/Users/demo/go/pkg/mod")));
        assert_eq!(
            b,
            Some(PathBuf::from("/Users/demo/Library/Caches/go-build"))
        );
        assert_eq!(
            parse_go_env("\n/tmp/build\n"),
            (None, Some("/tmp/build".into()))
        );
        assert_eq!(parse_go_env(""), (None, None));
    }

    #[test]
    fn conda_dry_run_fixture() {
        let dry = parse_conda_dry_run(include_str!(
            "../../tests/fixtures/conda_clean_dry_run.json"
        ))
        .unwrap();
        assert_eq!(dry.packages, 1_921_024);
        assert_eq!(dry.tarballs, 4_716_353);
        assert_eq!(dry.dirs, ["/opt/conda/pkgs"]);
        assert_eq!(parse_conda_dry_run(r#"{"success": false}"#), None);
        assert_eq!(parse_conda_dry_run("not json"), None);
    }
}
