//! Cleaning up after developer tools with their own commands (Docker,
//! Xcode simulators, package manager caches).
//!
//! Measuring only reads: it runs listing commands (`docker system df`,
//! `simctl list`, `brew cleanup -n`, `npm config get cache`…) and sizes
//! cache folders. Cleaning runs only the actions the user confirmed.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::scanner;
use crate::ui::fmt_size;

// System caches of Linux and Windows (#18, #19). Plain std code, so they
// build and test everywhere; `ToolKind::available` shows them only on their
// platform.
#[path = "tools_linux.rs"]
pub(crate) mod linux;
#[path = "tools_windows.rs"]
pub(crate) mod windows;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    /// Nothing of value is lost.
    Safe,
    /// Comes back by downloading or building again.
    Redownload,
    /// Can delete user data.
    DataLoss,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// A program (by full path) and its arguments.
    Command(Vec<String>),
    /// Move everything inside this folder to the trash.
    TrashContents(PathBuf),
    /// A command that needs root or administrator rights. rustClean never
    /// runs it; it only shows it, exactly as the user should type it.
    Manual(String),
    /// Move the entries inside this folder to the trash one by one, skipping
    /// the ones that cannot be moved (files in use).
    TrashEach(PathBuf),
}

impl Step {
    /// How the step is shown to the user before it runs.
    pub fn describe(&self) -> String {
        match self {
            Step::Command(args) => {
                let mut args = args.clone();
                if let Some(first) = args.first_mut() {
                    // Show the program name, not its full path.
                    if let Some(name) = Path::new(first.as_str()).file_name() {
                        *first = name.to_string_lossy().into_owned();
                    }
                }
                args.join(" ")
            }
            Step::TrashContents(dir) | Step::TrashEach(dir) => {
                tf!("{}/* → çöp kutusu", "{}/* → trash", dir.display())
            }
            Step::Manual(line) => line.clone(),
        }
    }
}

impl CleanAction {
    /// Whether the user has to run this action themselves (it needs root or
    /// administrator rights).
    pub fn manual(&self) -> bool {
        self.steps.iter().any(|s| matches!(s, Step::Manual(_)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanAction {
    pub label: String,
    pub risk: Risk,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Measuring,
    /// Not installed / nothing to clean; the text says why.
    Missing(String),
    /// Installed but not usable right now (e.g. Docker not running).
    Unavailable(String),
    Ready {
        reclaimable: u64,
        detail: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Docker,
    Simulators,
    XcodeData,
    Npm,
    Pnpm,
    Yarn,
    Pip,
    Gradle,
    CocoaPods,
    Homebrew,
    Cargo,
    AptCache,
    DnfCache,
    PacmanCache,
    Journal,
    Snaps,
    WinTemp,
    WinSystemTemp,
    WinUpdate,
    RecycleBin,
}

impl ToolKind {
    /// Every tool, on any platform.
    pub const ALL: [ToolKind; 20] = [
        ToolKind::Docker,
        ToolKind::Simulators,
        ToolKind::XcodeData,
        ToolKind::Npm,
        ToolKind::Pnpm,
        ToolKind::Yarn,
        ToolKind::Pip,
        ToolKind::Gradle,
        ToolKind::CocoaPods,
        ToolKind::Homebrew,
        ToolKind::Cargo,
        ToolKind::AptCache,
        ToolKind::DnfCache,
        ToolKind::PacmanCache,
        ToolKind::Journal,
        ToolKind::Snaps,
        ToolKind::WinTemp,
        ToolKind::WinSystemTemp,
        ToolKind::WinUpdate,
        ToolKind::RecycleBin,
    ];

    /// A name that does not depend on the language (the variant's), for
    /// files.
    pub fn code(self) -> String {
        format!("{self:?}")
    }

    pub fn from_code(code: &str) -> Option<ToolKind> {
        Self::ALL.into_iter().find(|k| k.code() == code)
    }

    /// Tools that make sense on this platform.
    pub fn available() -> Vec<ToolKind> {
        use ToolKind::*;
        let mut all = vec![Docker, Npm, Pnpm, Yarn, Pip, Gradle, Cargo];
        if cfg!(target_os = "macos") {
            all.splice(1..1, [Simulators, XcodeData]);
            all.extend([CocoaPods, Homebrew]);
        } else if cfg!(target_os = "linux") {
            all.push(Homebrew);
        }
        if cfg!(target_os = "linux") {
            all.extend(linux::KINDS);
        } else if cfg!(windows) {
            all.extend(windows::KINDS);
        }
        all
    }

    pub fn label(self) -> &'static str {
        match self {
            ToolKind::Docker => "Docker",
            ToolKind::Simulators => t!("Xcode simülatörleri", "Xcode simulators"),
            ToolKind::XcodeData => "Xcode DerivedData / DeviceSupport",
            ToolKind::Npm => t!("npm önbelleği", "npm cache"),
            ToolKind::Pnpm => t!("pnpm deposu", "pnpm store"),
            ToolKind::Yarn => t!("Yarn önbelleği", "Yarn cache"),
            ToolKind::Pip => t!("pip önbelleği", "pip cache"),
            ToolKind::Gradle => t!("Gradle önbelleği", "Gradle cache"),
            ToolKind::CocoaPods => t!("CocoaPods önbelleği", "CocoaPods cache"),
            ToolKind::Homebrew => "Homebrew",
            ToolKind::Cargo => t!("Cargo indirme önbelleği", "Cargo download cache"),
            ToolKind::AptCache
            | ToolKind::DnfCache
            | ToolKind::PacmanCache
            | ToolKind::Journal
            | ToolKind::Snaps => linux::label(self),
            ToolKind::WinTemp
            | ToolKind::WinSystemTemp
            | ToolKind::WinUpdate
            | ToolKind::RecycleBin => windows::label(self),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Tool {
    pub kind: ToolKind,
    pub status: Status,
    pub actions: Vec<CleanAction>,
}

/// A finished measurement of one tool.
pub type Measurement = (ToolKind, Status, Vec<CleanAction>);

/// Measures every tool on its own thread; results arrive as they finish.
pub fn measure_all() -> (Vec<Tool>, Receiver<Measurement>) {
    let kinds = ToolKind::available();
    let (tx, rx) = mpsc::channel();
    for &kind in &kinds {
        let tx = tx.clone();
        thread::spawn(move || {
            let (status, actions) = measure(kind);
            let _ = tx.send((kind, status, actions));
        });
    }
    let tools = kinds
        .into_iter()
        .map(|kind| Tool {
            kind,
            status: Status::Measuring,
            actions: Vec::new(),
        })
        .collect();
    (tools, rx)
}

pub fn measure(kind: ToolKind) -> (Status, Vec<CleanAction>) {
    match kind {
        ToolKind::Docker => docker(),
        ToolKind::Simulators => simulators(),
        ToolKind::XcodeData => xcode_data(),
        ToolKind::Npm => cache_command(
            "npm",
            &["config", "get", "cache"],
            &["cache", "clean", "--force"],
            Risk::Redownload,
            t!("npm önbelleğini temizle", "Clean the npm cache"),
        ),
        ToolKind::Pnpm => cache_command(
            "pnpm",
            &["store", "path"],
            &["store", "prune"],
            Risk::Safe,
            t!(
                "Kullanılmayan paketleri depodan sil (store prune)",
                "Remove unused packages from the store (store prune)",
            ),
        ),
        ToolKind::Yarn => cache_command(
            "yarn",
            &["cache", "dir"],
            &["cache", "clean"],
            Risk::Redownload,
            t!("Yarn önbelleğini temizle", "Clean the Yarn cache"),
        ),
        ToolKind::Pip => {
            let pip = if which("pip3").is_some() {
                "pip3"
            } else {
                "pip"
            };
            cache_command(
                pip,
                &["cache", "dir"],
                &["cache", "purge"],
                Risk::Redownload,
                t!("pip önbelleğini boşalt", "Purge the pip cache"),
            )
        }
        ToolKind::Gradle => gradle(),
        ToolKind::CocoaPods => cocoapods(),
        ToolKind::Homebrew => homebrew(),
        ToolKind::Cargo => trash_folder(
            home().map(|h| h.join(".cargo/registry/cache")),
            Risk::Redownload,
            t!(
                "İndirilmiş .crate dosyalarını çöpe taşı",
                "Move downloaded .crate files to the trash",
            ),
        ),
        ToolKind::AptCache
        | ToolKind::DnfCache
        | ToolKind::PacmanCache
        | ToolKind::Journal
        | ToolKind::Snaps => linux::measure(kind),
        ToolKind::WinTemp
        | ToolKind::WinSystemTemp
        | ToolKind::WinUpdate
        | ToolKind::RecycleBin => windows::measure(kind),
    }
}

// ---------------------------------------------------------------- running

/// Progress of running cleanup steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEvent {
    Started(String),
    Output(String),
    Done { step: String, ok: bool },
    Finished,
}

/// Runs the steps one after another on a background thread. Programs are
/// started directly (never through a shell), so arguments are passed as is.
/// Runs the cleanup steps of `kind` on a thread. Folder contents moved to
/// the trash are written to the deletion log.
pub fn run(kind: ToolKind, steps: Vec<Step>) -> Receiver<RunEvent> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for step in steps {
            let desc = step.describe();
            let _ = tx.send(RunEvent::Started(desc.clone()));
            let ok = run_step(kind, &step, &tx);
            let _ = tx.send(RunEvent::Done { step: desc, ok });
        }
        let _ = tx.send(RunEvent::Finished);
    });
    rx
}

fn run_step(kind: ToolKind, step: &Step, tx: &mpsc::Sender<RunEvent>) -> bool {
    match step {
        Step::Command(args) => {
            let child = Command::new(&args[0])
                .args(&args[1..])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn();
            let mut child = match child {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(RunEvent::Output(tf!(
                        "başlatılamadı: {e}",
                        "could not start: {e}"
                    )));
                    return false;
                }
            };
            let forward = |pipe: Option<Box<dyn std::io::Read + Send>>| {
                let tx = tx.clone();
                thread::spawn(move || {
                    if let Some(pipe) = pipe {
                        use std::io::BufRead;
                        for line in std::io::BufReader::new(pipe).lines().map_while(Result::ok) {
                            let _ = tx.send(RunEvent::Output(line));
                        }
                    }
                })
            };
            let out = forward(
                child
                    .stdout
                    .take()
                    .map(|p| -> Box<dyn std::io::Read + Send> { Box::new(p) }),
            );
            let err = forward(
                child
                    .stderr
                    .take()
                    .map(|p| -> Box<dyn std::io::Read + Send> { Box::new(p) }),
            );
            let status = child.wait();
            let _ = out.join();
            let _ = err.join();
            status.is_ok_and(|s| s.success())
        }
        Step::Manual(_) => {
            // The picker never offers these; refuse anyway.
            let _ = tx.send(RunEvent::Output(
                t!(
                    "rustClean bu komutu çalıştırmaz; kendiniz çalıştırın",
                    "rustClean does not run this command; run it yourself",
                )
                .into(),
            ));
            false
        }
        Step::TrashEach(dir) => windows::trash_each(kind, dir, tx),
        Step::TrashContents(dir) => {
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
            let _ = tx.send(RunEvent::Output(tf!(
                "{} çöp kutusuna taşınıyor…",
                "moving {} to the trash…",
                crate::i18n::count(entries.len() as u64, "öğe", "item", "items")
            )));
            // Measured before the move, for the deletion log.
            let size = entries
                .iter()
                .fold(crate::tree::Size::default(), |mut total, e| {
                    total += size_on_disk(e);
                    total
                });
            match crate::delete::trash_all(&entries) {
                Ok(()) => {
                    crate::trashlog::append(&[crate::trashlog::Entry {
                        time: crate::ui::now_secs(),
                        path: dir.display().to_string(),
                        size,
                        via: crate::trashlog::Via::Tool(kind),
                    }]);
                    true
                }
                Err(e) => {
                    let _ = tx.send(RunEvent::Output(tf!("hata: {e}", "error: {e}")));
                    false
                }
            }
        }
    }
}

// ---------------------------------------------------------------- helpers

/// Total size of `path` and everything below it, without following links.
fn size_on_disk(path: &Path) -> crate::tree::Size {
    let mut total = crate::tree::Size::default();
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        let Ok(md) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        // As in the scan: a directory's own length is bookkeeping, only its
        // blocks count.
        if !md.is_dir() {
            total.apparent += md.len();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            total.disk += md.blocks() * 512;
        }
        #[cfg(not(unix))]
        if !md.is_dir() {
            total.disk += md.len();
        }
        if md.is_dir() {
            if let Ok(rd) = std::fs::read_dir(&p) {
                stack.extend(rd.flatten().map(|e| e.path()));
            }
        }
    }
    total
}

fn home() -> Option<PathBuf> {
    dirs::home_dir()
}

/// Full path of a program found in PATH.
pub fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|dir| {
            let base = dir.join(program);
            [base.with_extension("exe"), base]
        })
        .find(|p| p.is_file())
}

/// Runs a command and returns its standard output, or `None` on failure or
/// when it takes longer than `timeout`.
fn output(program: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = thread::spawn(move || {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut stdout, &mut s).ok();
        s
    });
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().ok()? {
            let text = reader.join().ok()?;
            return status.success().then_some(text);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Size on disk of a folder, using the same scanner as the main view.
fn dir_size(dir: &Path) -> Option<u64> {
    if !dir.is_dir() {
        return None;
    }
    scanner::scan(dir, Vec::new(), &Arc::default(), |_| {})
        .ok()
        .map(|r| r.tree.node(crate::tree::ROOT).size.disk)
}

fn command(program: &Path, args: &[&str]) -> Step {
    let mut v = vec![program.to_string_lossy().into_owned()];
    v.extend(args.iter().map(ToString::to_string));
    Step::Command(v)
}

fn action(label: impl Into<String>, risk: Risk, steps: Vec<Step>) -> CleanAction {
    CleanAction {
        label: label.into(),
        risk,
        steps,
    }
}

fn ready(
    reclaimable: u64,
    detail: impl Into<String>,
    actions: Vec<CleanAction>,
) -> (Status, Vec<CleanAction>) {
    (
        Status::Ready {
            reclaimable,
            detail: detail.into(),
        },
        actions,
    )
}

fn missing(why: &str) -> (Status, Vec<CleanAction>) {
    (Status::Missing(why.to_string()), Vec::new())
}

/// Tools whose cache folder is reported by a command and cleaned by another.
fn cache_command(
    program: &str,
    where_args: &[&str],
    clean_args: &[&str],
    risk: Risk,
    label: &str,
) -> (Status, Vec<CleanAction>) {
    let Some(exe) = which(program) else {
        return missing(t!("kurulu değil", "not installed"));
    };
    let Some(dir) = output(&exe, where_args, Duration::from_secs(20))
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| p.is_dir())
    else {
        return missing(t!("önbellek klasörü yok", "no cache folder"));
    };
    let size = dir_size(&dir).unwrap_or(0);
    ready(
        size,
        dir.display().to_string(),
        vec![action(label, risk, vec![command(&exe, clean_args)])],
    )
}

fn trash_folder(dir: Option<PathBuf>, risk: Risk, label: &str) -> (Status, Vec<CleanAction>) {
    let Some(dir) = dir.filter(|d| d.is_dir()) else {
        return missing(t!("klasör yok", "no folder"));
    };
    let size = dir_size(&dir).unwrap_or(0);
    ready(
        size,
        dir.display().to_string(),
        vec![action(label, risk, vec![Step::TrashContents(dir)])],
    )
}

// ------------------------------------------------------------------ tools

fn docker() -> (Status, Vec<CleanAction>) {
    let Some(exe) = which("docker") else {
        return missing(t!("kurulu değil", "not installed"));
    };
    let Some(out) = output(
        &exe,
        &["system", "df", "--format", "{{json .}}"],
        Duration::from_secs(15),
    ) else {
        return (
            Status::Unavailable(t!("Docker çalışmıyor", "Docker is not running").into()),
            Vec::new(),
        );
    };
    let parts = parse_docker_df(&out);
    let total = parts.iter().map(|p| p.1).sum();
    let detail = parts
        .iter()
        .map(|(kind, size)| format!("{kind} {}", fmt_size(*size)))
        .collect::<Vec<_>>()
        .join(" · ");
    let actions = vec![
        action(
            t!(
                "Durmuş container'lar, sahipsiz imajlar ve derleme önbelleği",
                "Stopped containers, dangling images and build cache",
            ),
            Risk::Safe,
            vec![
                command(&exe, &["container", "prune", "-f"]),
                command(&exe, &["image", "prune", "-f"]),
                command(&exe, &["builder", "prune", "-f"]),
            ],
        ),
        action(
            t!(
                "Kullanılmayan TÜM imajlar (gerekince yeniden indirilir)",
                "ALL unused images (downloaded again when needed)",
            ),
            Risk::Redownload,
            vec![command(&exe, &["image", "prune", "-a", "-f"])],
        ),
        action(
            t!(
                "Kullanılmayan volume'lar — İÇİNDEKİ VERİLER SİLİNİR",
                "Unused volumes — THE DATA IN THEM IS DELETED",
            ),
            Risk::DataLoss,
            vec![command(&exe, &["volume", "prune", "-a", "-f"])],
        ),
    ];
    ready(total, detail, actions)
}

/// `(type, reclaimable bytes)` per line of `docker system df --format '{{json .}}'`.
pub fn parse_docker_df(out: &str) -> Vec<(String, u64)> {
    out.lines()
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).ok()?;
            let kind = match v.get("Type")?.as_str()? {
                "Images" => t!("imaj", "images"),
                "Containers" => t!("container", "containers"),
                "Local Volumes" => t!("volume", "volumes"),
                "Build Cache" => "build cache",
                other => other,
            };
            let reclaimable = v.get("Reclaimable")?.as_str()?;
            let size = parse_human(reclaimable.split_whitespace().next()?)?;
            Some((kind.to_string(), size))
        })
        .collect()
}

/// Sizes like "18.38GB", "835.9MB", "0B", "1.2kB" (Docker uses powers of
/// 1000), or "1GiB".
pub fn parse_human(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s.find(|c: char| c.is_ascii_alphabetic())?;
    let (num, unit) = s.split_at(split);
    let num: f64 = num.trim().parse().ok()?;
    let mult: f64 = match unit.to_ascii_lowercase().as_str() {
        "b" => 1.0,
        "kb" => 1e3,
        "mb" => 1e6,
        "gb" => 1e9,
        "tb" => 1e12,
        "kib" => 1024.0,
        "mib" => 1024f64.powi(2),
        "gib" => 1024f64.powi(3),
        "tib" => 1024f64.powi(4),
        _ => return None,
    };
    Some((num * mult) as u64)
}

fn simulators() -> (Status, Vec<CleanAction>) {
    let Some(xcrun) = which("xcrun") else {
        return missing(t!("Xcode kurulu değil", "Xcode is not installed"));
    };
    let devices = output(
        &xcrun,
        &["simctl", "list", "devices", "-j"],
        Duration::from_secs(30),
    );
    let runtimes = output(
        &xcrun,
        &["simctl", "runtime", "list", "-j"],
        Duration::from_secs(30),
    );
    let Some(devices) = devices else {
        return missing(t!("simctl çalıştırılamadı", "could not run simctl"));
    };
    let (count, size) = parse_unavailable_devices(&devices);
    let runtimes = runtimes.map(|r| parse_runtimes(&r)).unwrap_or_default();
    let runtime_total: u64 = runtimes.iter().map(|r| r.size).sum();

    let mut actions = Vec::new();
    if count > 0 {
        actions.push(action(
            tf!(
                "Kullanılamayan {count} simülatörü sil ({})",
                "Delete {count} unavailable simulators ({})",
                fmt_size(size)
            ),
            Risk::Safe,
            vec![command(&xcrun, &["simctl", "delete", "unavailable"])],
        ));
    }
    for r in &runtimes {
        actions.push(action(
            tf!(
                "{} çalışma zamanını sil ({})",
                "Delete the {} runtime ({})",
                r.name,
                fmt_size(r.size)
            ),
            Risk::Redownload,
            vec![command(&xcrun, &["simctl", "runtime", "delete", &r.id])],
        ));
    }
    let detail = tf!(
        "kullanılamayan cihaz {count} ({}) · çalışma zamanı {} ({})",
        "unavailable devices {count} ({}) · runtimes {} ({})",
        fmt_size(size),
        runtimes.len(),
        fmt_size(runtime_total)
    );
    ready(size, detail, actions)
}

/// Count and data size of simulator devices whose runtime is gone.
pub fn parse_unavailable_devices(json: &str) -> (usize, u64) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return (0, 0);
    };
    let Some(by_runtime) = v.get("devices").and_then(|d| d.as_object()) else {
        return (0, 0);
    };
    by_runtime
        .values()
        .filter_map(|list| list.as_array())
        .flatten()
        .filter(|d| d.get("isAvailable").and_then(serde_json::Value::as_bool) == Some(false))
        .fold((0, 0), |(n, size), d| {
            let s = d
                .get("dataPathSize")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            (n + 1, size + s)
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runtime {
    pub id: String,
    pub name: String,
    pub size: u64,
}

/// Deletable simulator runtimes, largest first.
pub fn parse_runtimes(json: &str) -> Vec<Runtime> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(map) = v.as_object() else {
        return Vec::new();
    };
    let mut out: Vec<Runtime> = map
        .values()
        .filter(|r| r.get("deletable").and_then(serde_json::Value::as_bool) != Some(false))
        .filter_map(|r| {
            let id = r.get("identifier")?.as_str()?.to_string();
            let version = r.get("version").and_then(|v| v.as_str()).unwrap_or("?");
            let platform = r
                .get("runtimeIdentifier")
                .and_then(|p| p.as_str())
                .and_then(|p| p.rsplit('.').next())
                .and_then(|p| p.split('-').next())
                .unwrap_or("Simulator");
            Some(Runtime {
                id,
                name: format!("{platform} {version}"),
                size: r
                    .get("sizeBytes")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
            })
        })
        .collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.size));
    out
}

fn xcode_data() -> (Status, Vec<CleanAction>) {
    let Some(base) = home().map(|h| h.join("Library/Developer/Xcode")) else {
        return missing(t!("klasör yok", "no folder"));
    };
    let mut total = 0;
    let mut details = Vec::new();
    let mut actions = Vec::new();
    for (name, label) in [
        (
            "DerivedData",
            t!(
                "DerivedData içeriğini çöpe taşı (projeler yeniden derlenir)",
                "Move DerivedData contents to the trash (projects are rebuilt)",
            ),
        ),
        (
            "iOS DeviceSupport",
            t!(
                "iOS DeviceSupport içeriğini çöpe taşı (cihaz bağlanınca yeniden oluşur)",
                "Move iOS DeviceSupport contents to the trash (recreated when a device connects)",
            ),
        ),
    ] {
        let dir = base.join(name);
        if let Some(size) = dir_size(&dir) {
            total += size;
            details.push(format!("{name} {}", fmt_size(size)));
            actions.push(action(label, Risk::Safe, vec![Step::TrashContents(dir)]));
        }
    }
    if actions.is_empty() {
        return missing(t!("klasör yok", "no folder"));
    }
    ready(total, details.join(" · "), actions)
}

fn gradle() -> (Status, Vec<CleanAction>) {
    let (status, mut actions) = trash_folder(
        home().map(|h| h.join(".gradle/caches")),
        Risk::Redownload,
        t!(
            "Gradle önbelleğini çöpe taşı (bağımlılıklar yeniden indirilir)",
            "Move the Gradle cache to the trash (dependencies are downloaded again)",
        ),
    );
    // Running daemons hold files in the cache; stop them first.
    if let (Some(gradle), Some(a)) = (which("gradle"), actions.first_mut()) {
        a.steps.insert(0, command(&gradle, &["--stop"]));
    }
    (status, actions)
}

fn cocoapods() -> (Status, Vec<CleanAction>) {
    let Some(dir) = home()
        .map(|h| h.join("Library/Caches/CocoaPods"))
        .filter(|d| d.is_dir())
    else {
        return missing(t!("klasör yok", "no folder"));
    };
    let size = dir_size(&dir).unwrap_or(0);
    let step = match which("pod") {
        Some(pod) => command(&pod, &["cache", "clean", "--all"]),
        None => Step::TrashContents(dir.clone()),
    };
    ready(
        size,
        dir.display().to_string(),
        vec![action(
            t!("CocoaPods önbelleğini temizle", "Clean the CocoaPods cache"),
            Risk::Redownload,
            vec![step],
        )],
    )
}

fn homebrew() -> (Status, Vec<CleanAction>) {
    let Some(brew) = which("brew") else {
        return missing(t!("kurulu değil", "not installed"));
    };
    let Some(out) = output(&brew, &["cleanup", "-n"], Duration::from_secs(120)) else {
        return (
            Status::Unavailable(t!("brew cleanup -n başarısız", "brew cleanup -n failed").into()),
            Vec::new(),
        );
    };
    let size = parse_brew_cleanup(&out).unwrap_or(0);
    ready(
        size,
        t!(
            "eski sürümler, indirme önbelleği",
            "old versions, download cache"
        ),
        vec![action(
            t!(
                "Eski sürümleri ve önbelleği temizle (brew cleanup)",
                "Remove old versions and the cache (brew cleanup)",
            ),
            Risk::Safe,
            vec![command(&brew, &["cleanup", "--prune=all"])],
        )],
    )
}

/// "==> This operation would free approximately 1GB of disk space."
pub fn parse_brew_cleanup(out: &str) -> Option<u64> {
    let rest = out.split("free approximately ").nth(1)?;
    let size = rest.split_whitespace().next()?;
    // Homebrew prints binary sizes with decimal-looking units ("1GB").
    let bin = size
        .replace("KB", "KiB")
        .replace("MB", "MiB")
        .replace("GB", "GiB");
    parse_human(&bin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_sizes() {
        assert_eq!(parse_human("18.38GB"), Some(18_380_000_000));
        assert_eq!(parse_human("835.9MB"), Some(835_900_000));
        assert_eq!(parse_human("0B"), Some(0));
        assert_eq!(parse_human("1.5kB"), Some(1500));
        assert_eq!(parse_human("2GiB"), Some(2 << 30));
        assert_eq!(parse_human("abc"), None);
    }

    #[test]
    fn docker_df_fixture() {
        let out = include_str!("../tests/fixtures/docker_df.jsonl");
        let parts = parse_docker_df(out);
        let kinds: Vec<&str> = parts.iter().map(|p| p.0.as_str()).collect();
        assert_eq!(kinds, vec!["imaj", "container", "volume", "build cache"]);
        assert_eq!(parts[0].1, 18_380_000_000);
        assert_eq!(parts[3].1, 21_270_000_000);
    }

    #[test]
    fn brew_cleanup_line() {
        let out = "Would remove: /x (1 file)\n==> This operation would free approximately 1GB of disk space.\n";
        assert_eq!(parse_brew_cleanup(out), Some(1 << 30));
        assert_eq!(
            parse_brew_cleanup("==> This operation would free approximately 34.6MB of disk space."),
            Some((34.6 * (1u64 << 20) as f64) as u64)
        );
        assert_eq!(parse_brew_cleanup("nothing to do"), None);
    }

    #[test]
    fn simulator_fixtures() {
        let (count, size) =
            parse_unavailable_devices(include_str!("../tests/fixtures/simctl_devices.json"));
        assert_eq!(count, 80);
        assert!(size > 0);
        let runtimes = parse_runtimes(include_str!("../tests/fixtures/simctl_runtimes.json"));
        assert_eq!(runtimes.len(), 4);
        assert!(runtimes.windows(2).all(|w| w[0].size >= w[1].size));
        assert!(runtimes.iter().all(|r| r.name.starts_with("iOS ")));
        assert_eq!(parse_unavailable_devices("not json"), (0, 0));
    }

    #[cfg(unix)]
    fn collect(rx: &Receiver<RunEvent>) -> Vec<RunEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.recv_timeout(Duration::from_secs(10)) {
            let end = ev == RunEvent::Finished;
            out.push(ev);
            if end {
                break;
            }
        }
        out
    }

    #[cfg(unix)]
    #[test]
    fn runs_steps_in_order_and_reports_failures() {
        let events = collect(&run(
            ToolKind::Npm,
            vec![
                Step::Command(vec!["/bin/echo".into(), "merhaba; rm -rf /".into()]),
                Step::Command(vec!["/usr/bin/false".into()]),
                Step::Command(vec!["/nonexistent/tool".into()]),
            ],
        ));
        // Arguments are not interpreted by a shell.
        assert!(events.contains(&RunEvent::Output("merhaba; rm -rf /".into())));
        let done: Vec<bool> = events
            .iter()
            .filter_map(|e| match e {
                RunEvent::Done { ok, .. } => Some(*ok),
                _ => None,
            })
            .collect();
        assert_eq!(done, vec![true, false, false]);
        assert_eq!(events.last(), Some(&RunEvent::Finished));
    }

    #[test]
    fn describes_steps_without_full_paths() {
        let step = command(
            Path::new("/usr/local/bin/docker"),
            &["image", "prune", "-f"],
        );
        assert_eq!(step.describe(), "docker image prune -f");
        let step = Step::TrashContents(PathBuf::from("/h/.gradle/caches"));
        assert_eq!(step.describe(), "/h/.gradle/caches/* → çöp kutusu");
    }
}
