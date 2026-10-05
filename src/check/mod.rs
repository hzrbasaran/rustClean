//! `rustclean check`: warns when a watched disk is fuller than the threshold
//! (90 % by default). It only reads: it prints one line per watched disk and
//! exits with code 3 when one is over. When it runs in the background (its
//! output is not a terminal), it also shows a system notification, at most
//! once a day per disk while the disk stays over.
//!
//! `--install` sets it up to run every hour (`agent.rs`); `--uninstall`
//! removes that again.

mod agent;

use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Result;

use crate::disks::{self, DiskInfo};
use crate::ui::fmt_size;

/// The exit code when a watched disk is over the threshold.
pub const OVER: i32 = 3;

/// A notification for a disk that stays over is repeated after this long.
const REPEAT: u64 = 24 * 3600;

#[derive(clap::Args)]
pub struct CheckArgs {
    /// Uyarı eşiği, doluluk yüzdesi (varsayılan: config.toml ya da 90) / warn above this
    /// percentage full (default: config.toml, else 90)
    #[arg(long, value_name = "PERCENT", value_parser = clap::value_parser!(u8).range(1..=99))]
    pub threshold: Option<u8>,

    /// Bildirim gösterme, yalnızca yazdır / print only, never show a notification
    #[arg(long)]
    pub no_notify: bool,

    /// Saatte bir arka planda çalışacak şekilde kur / set up to run every hour in the background
    #[arg(long, conflicts_with = "uninstall")]
    pub install: bool,

    /// Arka plan kurulumunu kaldır / remove the background setup
    #[arg(long)]
    pub uninstall: bool,

    /// --install / --uninstall için onay sorma / do not ask before --install / --uninstall
    #[arg(long)]
    pub yes: bool,
}

/// Runs `rustclean check`; returns the exit code.
pub fn run(args: &CheckArgs) -> Result<i32> {
    if args.install {
        return agent::install(args.yes);
    }
    if args.uninstall {
        return agent::uninstall(args.yes);
    }
    let config = crate::config::get();
    let threshold = args.threshold.unwrap_or(config.watch_threshold);
    let home = dirs::home_dir();
    let watched = watched(&disks::list_disks(), home.as_deref(), &config.watch_disks);
    if watched.is_empty() {
        println!(
            "{}",
            t!("İzlenecek disk bulunamadı.", "No disk to watch was found.")
        );
        return Ok(0);
    }
    let over: Vec<&DiskInfo> = watched.iter().filter(|d| is_over(d, threshold)).collect();
    for d in &watched {
        println!("{}", line(d, threshold));
    }

    // In the background nobody reads the output, so a notification says it.
    // The notification memory changes only with notifications, so a check
    // in the terminal never silences the next one in the background.
    let notify = !args.no_notify && !std::io::stdout().is_terminal();
    if notify {
        let mut state = State::load();
        let due = state.update(&over, crate::ui::now_secs());
        state.save();
        for d in due {
            show_notification(
                t!(
                    "rustClean: disk dolmak üzere",
                    "rustClean: disk almost full"
                ),
                &message(d),
            );
        }
    }
    Ok(if over.is_empty() { 0 } else { OVER })
}

/// The disks to check: the one holding the home folder, and the ones
/// holding the folders in `[watch] disks`.
fn watched(all: &[DiskInfo], home: Option<&Path>, extra: &[PathBuf]) -> Vec<DiskInfo> {
    let mut out: Vec<DiskInfo> = Vec::new();
    for path in home.into_iter().chain(extra.iter().map(PathBuf::as_path)) {
        if let Some(d) = disks::disk_for(path, all) {
            if !out.iter().any(|o| o.mount_point == d.mount_point) {
                out.push(d.clone());
            }
        }
    }
    out
}

/// Whole percent used, rounded down, so "90 %" is shown only once it is
/// really 90 % or more.
fn percent(d: &DiskInfo) -> u64 {
    if d.total == 0 {
        return 0;
    }
    (u128::from(d.used()) * 100 / u128::from(d.total)) as u64
}

fn is_over(d: &DiskInfo, threshold: u8) -> bool {
    d.total > 0 && percent(d) >= u64::from(threshold)
}

/// One line of output: name, how full, what is free, and the verdict.
fn line(d: &DiskInfo, threshold: u8) -> String {
    let verdict = if is_over(d, threshold) {
        tf!(
            "eşiğin (%{threshold}) üstünde",
            "over the threshold ({threshold} %)"
        )
    } else {
        t!("tamam", "ok").to_string()
    };
    tf!(
        "{}  {}  %{} dolu, {} boş  {}",
        "{}  {}  {} % full, {} free  {}",
        d.name,
        d.mount_point.display(),
        percent(d),
        fmt_size(d.available),
        verdict
    )
}

/// The notification's text for a disk over the threshold.
fn message(d: &DiskInfo) -> String {
    tf!(
        "{} %{} dolu, {} boş. Yer açmak için rustclean'i çalıştırın.",
        "{} is {} % full, {} free. Run rustclean to free space.",
        d.name,
        percent(d),
        fmt_size(d.available)
    )
}

/// When each disk over the threshold was last notified, by mount point, in
/// `check-state` in the data directory.
#[derive(Debug, Default, PartialEq, Eq)]
struct State(BTreeMap<String, u64>);

impl State {
    fn file() -> Option<PathBuf> {
        Some(crate::paths::data_dir()?.join("check-state"))
    }

    fn load() -> Self {
        let text = Self::file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .unwrap_or_default();
        Self::parse(&text)
    }

    fn parse(text: &str) -> Self {
        State(
            text.lines()
                .filter_map(|l| {
                    let (time, mount) = l.split_once(' ')?;
                    Some((mount.to_string(), time.parse().ok()?))
                })
                .collect(),
        )
    }

    fn text(&self) -> String {
        self.0.iter().map(|(m, t)| format!("{t} {m}\n")).collect()
    }

    fn save(&self) {
        let Some(file) = Self::file() else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, self.text());
    }

    /// Records `now` for the disks in `over` that are due a notification
    /// (new, or last notified a day ago) and returns them. Disks no longer
    /// over are forgotten, so going over again notifies at once.
    fn update<'a>(&mut self, over: &[&'a DiskInfo], now: u64) -> Vec<&'a DiskInfo> {
        let mounts: Vec<String> = over
            .iter()
            .map(|d| d.mount_point.display().to_string())
            .collect();
        self.0.retain(|m, _| mounts.contains(m));
        let mut due = Vec::new();
        for (d, m) in over.iter().zip(mounts) {
            let last = self.0.get(&m).copied();
            if last.is_none_or(|t| now.saturating_sub(t) >= REPEAT) {
                self.0.insert(m, now);
                due.push(*d);
            }
        }
        due
    }
}

/// A system notification: `osascript` on macOS, `notify-send` on Linux when
/// it is installed. The texts go in as arguments, never into a script, so a
/// disk name cannot change what runs. Failures are ignored: the output and
/// the exit code still say it.
fn show_notification(title: &str, body: &str) {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("/usr/bin/osascript");
        c.args([
            "-e",
            "on run argv",
            "-e",
            "display notification (item 2 of argv) with title (item 1 of argv)",
            "-e",
            "end run",
            title,
            body,
        ]);
        c
    } else if cfg!(target_os = "linux") {
        let Some(exe) = crate::tools::which("notify-send") else {
            return;
        };
        let mut c = Command::new(exe);
        c.args(["--app-name=rustClean", title, body]);
        c
    } else {
        return;
    };
    let _ = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    fn disk(name: &str, mount: &str, total_gib: u64, free_gib: u64) -> DiskInfo {
        DiskInfo {
            name: name.into(),
            mount_point: mount.into(),
            fs_type: "apfs".into(),
            total: total_gib * GIB,
            available: free_gib * GIB,
            removable: false,
        }
    }

    #[test]
    fn watches_the_home_disk_and_the_listed_ones() {
        let all = [
            disk("Macintosh HD", "/", 500, 40),
            disk("Data", "/Volumes/Data", 1000, 900),
            disk("Backup", "/Volumes/Backup", 2000, 10),
        ];
        let home = Path::new("/Users/demo");
        let names = |v: Vec<DiskInfo>| v.into_iter().map(|d| d.name).collect::<Vec<_>>();
        assert_eq!(names(watched(&all, Some(home), &[])), ["Macintosh HD"]);
        // A folder on a disk stands for that disk; the same disk counts once.
        let extra = [PathBuf::from("/Volumes/Data/Projects"), PathBuf::from("/")];
        assert_eq!(
            names(watched(&all, Some(home), &extra)),
            ["Macintosh HD", "Data"]
        );
        // The backup disk is full, but nobody asked to watch it.
        assert!(!names(watched(&all, Some(home), &extra)).contains(&"Backup".to_string()));
    }

    #[test]
    fn over_means_at_least_the_threshold() {
        let d = disk("D", "/", 100, 10); // 90 % used
        assert_eq!(percent(&d), 90);
        assert!(is_over(&d, 90));
        assert!(!is_over(&d, 91));
        let empty = disk("E", "/e", 0, 0);
        assert!(!is_over(&empty, 1));
    }

    #[test]
    fn notifies_once_a_day_while_over() {
        let a = disk("A", "/", 100, 5);
        let b = disk("B", "/Volumes/B", 100, 1);
        let mut state = State::default();
        let names = |v: Vec<&DiskInfo>| v.iter().map(|d| d.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(state.update(&[&a], 1000)), ["A"]);
        // An hour later: still over, already told.
        assert!(state.update(&[&a], 1000 + 3600).is_empty());
        // B goes over: only B is new.
        assert_eq!(names(state.update(&[&a, &b], 1000 + 7200)), ["B"]);
        // A day after A's notification it is repeated.
        assert_eq!(names(state.update(&[&a, &b], 1000 + REPEAT)), ["A"]);
        // A drops below and goes over again: told at once.
        state.update(&[&b], 1000 + REPEAT + 10);
        assert_eq!(names(state.update(&[&a, &b], 1000 + REPEAT + 20)), ["A"]);
    }

    #[test]
    fn state_round_trips() {
        let mut state = State::default();
        let a = disk("A", "/Volumes/My Disk", 100, 1);
        state.update(&[&a], 42);
        assert_eq!(state.text(), "42 /Volumes/My Disk\n");
        assert_eq!(State::parse(&state.text()), state);
        assert_eq!(
            State::parse("garbage\n7 /x\n"),
            State([("/x".into(), 7)].into())
        );
    }

    #[cfg(unix)]
    #[test]
    fn lines_say_how_full() {
        crate::i18n::with_lang(crate::i18n::Lang::En, || {
            let d = disk("Macintosh HD", "/", 100, 7);
            assert_eq!(
                line(&d, 90),
                "Macintosh HD  /  93 % full, 7.0 GiB free  over the threshold (90 %)"
            );
            assert_eq!(
                message(&d),
                "Macintosh HD is 93 % full, 7.0 GiB free. Run rustclean to free space."
            );
            assert!(line(&disk("D", "/d", 100, 50), 90).ends_with("ok"));
        });
    }
}
