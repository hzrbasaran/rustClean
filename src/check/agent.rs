//! `rustclean check --install` / `--uninstall`: runs the check every hour in
//! the background. On macOS a launchd agent in `~/Library/LaunchAgents`, on
//! Linux a systemd user timer in `~/.config/systemd/user`. Windows gets the
//! `schtasks` command to run, as rustClean does not register tasks itself.
//!
//! Everything is shown first (the files with their content, the commands)
//! and done only after the user types the confirmation word, unless
//! `--yes` is given. Nothing needs root, and no shell is used.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

const LABEL: &str = "io.github.hzrbasaran.rustclean.check";
const UNIT: &str = "rustclean-check";
const TASK: &str = "rustClean check";

/// What installing or uninstalling does.
#[derive(Debug, Default, PartialEq, Eq)]
struct Plan {
    /// Files written, with their content.
    write: Vec<(PathBuf, String)>,
    /// Files removed (when they exist).
    remove: Vec<PathBuf>,
    /// Commands run after the files, in order. A failing one stops the rest,
    /// unless it is in `may_fail`.
    run: Vec<Vec<String>>,
    /// Indexes into `run` whose failure is expected (stopping an agent that
    /// is not running).
    may_fail: Vec<usize>,
    /// Commands only shown, for the user to run (Windows).
    show: Vec<String>,
}

pub fn install(yes: bool) -> Result<i32> {
    let exe = stable_exe()?;
    let home = dirs::home_dir().context("no home folder")?;
    let plan = install_plan(&exe, &home)?;
    carry_out(
        &plan,
        yes,
        t!(
            "rustClean saatte bir disk doluluğunu kontrol edecek. Yapılacaklar:",
            "rustClean will check how full the disks are every hour. This will be done:"
        ),
    )
}

pub fn uninstall(yes: bool) -> Result<i32> {
    let home = dirs::home_dir().context("no home folder")?;
    let plan = uninstall_plan(&home);
    if plan.show.is_empty() && !plan.remove.iter().any(|p| p.exists()) {
        println!(
            "{}",
            t!(
                "Kurulu bir arka plan kontrolü yok.",
                "No background check is set up."
            )
        );
        return Ok(0);
    }
    carry_out(
        &plan,
        yes,
        t!(
            "Arka plan kontrolü kaldırılacak. Yapılacaklar:",
            "The background check will be removed. This will be done:"
        ),
    )
}

/// The path the agent runs. A package manager's link on `PATH` (Homebrew's
/// `/opt/homebrew/bin/rustclean`) survives upgrades, where the versioned
/// path the program runs from would not.
fn stable_exe() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("cannot find the running program")?;
    let real = std::fs::canonicalize(&exe).unwrap_or_else(|_| exe.clone());
    let name = if cfg!(windows) {
        "rustclean.exe"
    } else {
        "rustclean"
    };
    if let Some(on_path) = crate::tools::which(name.trim_end_matches(".exe")) {
        if std::fs::canonicalize(&on_path).ok().as_ref() == Some(&real) {
            return Ok(on_path);
        }
    }
    Ok(exe)
}

fn install_plan(exe: &Path, home: &Path) -> Result<Plan> {
    let exe_text = exe.display().to_string();
    if exe_text.contains(['"', '\n', '\r']) {
        bail!(
            "{}",
            tf!(
                "Programın yolu kurulum için uygun değil: {exe_text}",
                "The program's path cannot be used for the setup: {exe_text}"
            )
        );
    }
    if cfg!(target_os = "macos") {
        let plist = home.join(format!("Library/LaunchAgents/{LABEL}.plist"));
        let domain = launchd_domain(home);
        Ok(Plan {
            write: vec![(plist.clone(), launchd_plist(&exe_text))],
            run: vec![
                // Replace one loaded earlier (fails when there is none).
                vec![
                    "/bin/launchctl".into(),
                    "bootout".into(),
                    format!("{domain}/{LABEL}"),
                ],
                vec![
                    "/bin/launchctl".into(),
                    "bootstrap".into(),
                    domain,
                    plist.display().to_string(),
                ],
            ],
            may_fail: vec![0],
            ..Plan::default()
        })
    } else if cfg!(windows) {
        Ok(Plan {
            show: vec![format!(
                "schtasks /Create /F /SC HOURLY /TN \"{TASK}\" /TR \"\\\"{exe_text}\\\" check\""
            )],
            ..Plan::default()
        })
    } else {
        let dir = systemd_dir(home);
        let systemctl = systemctl()?;
        Ok(Plan {
            write: vec![
                (
                    dir.join(format!("{UNIT}.service")),
                    systemd_service(&exe_text),
                ),
                (dir.join(format!("{UNIT}.timer")), systemd_timer()),
            ],
            run: vec![
                vec![systemctl.clone(), "--user".into(), "daemon-reload".into()],
                vec![
                    systemctl,
                    "--user".into(),
                    "enable".into(),
                    "--now".into(),
                    format!("{UNIT}.timer"),
                ],
            ],
            ..Plan::default()
        })
    }
}

fn uninstall_plan(home: &Path) -> Plan {
    if cfg!(target_os = "macos") {
        let plist = home.join(format!("Library/LaunchAgents/{LABEL}.plist"));
        Plan {
            run: vec![vec![
                "/bin/launchctl".into(),
                "bootout".into(),
                format!("{}/{LABEL}", launchd_domain(home)),
            ]],
            may_fail: vec![0],
            remove: vec![plist],
            ..Plan::default()
        }
    } else if cfg!(windows) {
        Plan {
            show: vec![format!("schtasks /Delete /F /TN \"{TASK}\"")],
            ..Plan::default()
        }
    } else {
        let dir = systemd_dir(home);
        let systemctl = systemctl().unwrap_or_else(|_| "systemctl".into());
        Plan {
            run: vec![
                vec![
                    systemctl.clone(),
                    "--user".into(),
                    "disable".into(),
                    "--now".into(),
                    format!("{UNIT}.timer"),
                ],
                vec![systemctl, "--user".into(), "daemon-reload".into()],
            ],
            may_fail: vec![0, 1],
            remove: vec![
                dir.join(format!("{UNIT}.timer")),
                dir.join(format!("{UNIT}.service")),
            ],
            ..Plan::default()
        }
    }
}

/// `gui/<uid>`: the logged-in user's launchd domain. The uid is the home
/// folder's owner, so no extra dependency is needed.
fn launchd_domain(home: &Path) -> String {
    #[cfg(unix)]
    let uid = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(home).map_or(501, |m| m.uid())
    };
    #[cfg(not(unix))]
    let uid = {
        let _ = home;
        0
    };
    format!("gui/{uid}")
}

fn systemd_dir(home: &Path) -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"))
        .join("systemd/user")
}

fn systemctl() -> Result<String> {
    match crate::tools::which("systemctl") {
        Some(p) => Ok(p.display().to_string()),
        None => bail!(
            "{}",
            t!(
                "systemctl bulunamadı. Zamanlamayı kendiniz kurun, örneğin crontab'a: 0 * * * * rustclean check",
                "systemctl was not found. Schedule it yourself, for example in crontab: 0 * * * * rustclean check"
            )
        ),
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn launchd_plist(exe: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{}</string>
		<string>check</string>
	</array>
	<key>StartInterval</key>
	<integer>3600</integer>
	<key>RunAtLoad</key>
	<true/>
	<key>ProcessType</key>
	<string>Background</string>
</dict>
</plist>
"#,
        xml_escape(exe)
    )
}

fn systemd_service(exe: &str) -> String {
    format!(
        "[Unit]\n\
         Description=rustClean disk space check\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         ExecStart=\"{exe}\" check\n\
         # Exit code 3 means a disk is over the threshold, not a failure.\n\
         SuccessExitStatus=3\n"
    )
}

fn systemd_timer() -> String {
    format!(
        "[Unit]\n\
         Description=rustClean disk space check, every hour\n\
         \n\
         [Timer]\n\
         OnCalendar=hourly\n\
         Persistent=true\n\
         Unit={UNIT}.service\n\
         \n\
         [Install]\n\
         WantedBy=timers.target\n"
    )
}

/// Shows the plan, asks, and carries it out.
fn carry_out(plan: &Plan, yes: bool, intro: &str) -> Result<i32> {
    if !plan.show.is_empty() {
        println!(
            "{}",
            t!(
                "rustClean Windows'ta zamanlanmış görev kaydetmez. Şu komutu kendiniz çalıştırın:",
                "rustClean does not register scheduled tasks on Windows. Run this command yourself:"
            )
        );
        for c in &plan.show {
            println!("  {c}");
        }
        return Ok(0);
    }
    println!("{intro}");
    for (path, content) in &plan.write {
        println!(
            "\n{} {}:",
            t!("Yazılacak dosya", "File to write"),
            path.display()
        );
        for l in content.lines() {
            println!("    {l}");
        }
    }
    for path in &plan.remove {
        println!(
            "\n{} {}",
            t!("Silinecek dosya:", "File to remove:"),
            path.display()
        );
    }
    if !plan.run.is_empty() {
        println!("\n{}", t!("Çalıştırılacak komutlar:", "Commands to run:"));
        for c in &plan.run {
            println!("  {}", c.join(" "));
        }
    }
    if !yes && !confirmed()? {
        println!(
            "{}",
            t!(
                "Vazgeçildi; hiçbir şey değişmedi.",
                "Cancelled; nothing changed."
            )
        );
        return Ok(1);
    }
    apply(plan)?;
    println!("{}", t!("Tamam.", "Done."));
    Ok(0)
}

fn confirmed() -> Result<bool> {
    let word = crate::toolsview::confirm_word();
    print!(
        "\n{} ",
        tf!(
            "Devam etmek için \"{word}\" yazın:",
            "Type \"{word}\" to go ahead:"
        )
    );
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(answer.trim().eq_ignore_ascii_case(word))
}

fn apply(plan: &Plan) -> Result<()> {
    for (path, content) in &plan.write {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("{}", dir.display()))?;
        }
        std::fs::write(path, content).with_context(|| format!("{}", path.display()))?;
    }
    for path in &plan.remove {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("{}", path.display())),
        }
    }
    for (i, c) in plan.run.iter().enumerate() {
        let out = Command::new(&c[0]).args(&c[1..]).output();
        let ok = out.as_ref().is_ok_and(|o| o.status.success());
        if !ok && !plan.may_fail.contains(&i) {
            let detail = match out {
                Ok(o) => String::from_utf8_lossy(&o.stderr).trim().to_string(),
                Err(e) => e.to_string(),
            };
            bail!("{}: {detail}", c.join(" "));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_runs_check_with_an_escaped_path() {
        let plist = launchd_plist("/Users/a&b/bin/rustclean");
        assert!(plist.contains("<string>/Users/a&amp;b/bin/rustclean</string>"));
        assert!(plist.contains("<string>check</string>"));
        assert!(plist.contains("<integer>3600</integer>"));
        let service = systemd_service("/home/a b/.cargo/bin/rustclean");
        assert!(service.contains("ExecStart=\"/home/a b/.cargo/bin/rustclean\" check"));
        assert!(service.contains("SuccessExitStatus=3"));
        assert!(systemd_timer().contains("OnCalendar=hourly"));
    }

    #[test]
    fn paths_that_cannot_be_quoted_are_refused() {
        let home = Path::new("/home/demo");
        assert!(install_plan(Path::new("/odd\"name/rustclean"), home).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_installs_a_launch_agent() {
        let home = Path::new("/Users/demo");
        let plan = install_plan(Path::new("/opt/homebrew/bin/rustclean"), home).unwrap();
        let plist = home.join(format!("Library/LaunchAgents/{LABEL}.plist"));
        assert_eq!(plan.write[0].0, plist);
        assert_eq!(plan.run[1][1], "bootstrap");
        assert_eq!(plan.may_fail, [0]);
        let undo = uninstall_plan(home);
        assert_eq!(undo.remove, [plist]);
        assert_eq!(undo.run[0][1], "bootout");
    }

    #[cfg(windows)]
    #[test]
    fn windows_only_shows_the_command() {
        let home = Path::new("C:\\Users\\demo");
        let plan = install_plan(Path::new("C:\\Tools\\rustclean.exe"), home).unwrap();
        assert!(plan.write.is_empty() && plan.run.is_empty());
        assert!(plan.show[0].starts_with("schtasks /Create"));
        assert!(plan.show[0].contains("C:\\Tools\\rustclean.exe"));
    }

    #[test]
    fn applying_writes_and_removes_files() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("agents/x.plist");
        let plan = Plan {
            write: vec![(file.clone(), "content".into())],
            ..Plan::default()
        };
        apply(&plan).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "content");
        let undo = Plan {
            remove: vec![file.clone(), tmp.path().join("missing")],
            ..Plan::default()
        };
        apply(&undo).unwrap();
        assert!(!file.exists());
        // A failing command stops, unless its failure is expected.
        let missing = tmp.path().join("no-such-program").display().to_string();
        let fails = Plan {
            run: vec![vec![missing.clone()]],
            ..Plan::default()
        };
        assert!(apply(&fails).is_err());
        let tolerated = Plan {
            run: vec![vec![missing]],
            may_fail: vec![0],
            ..Plan::default()
        };
        apply(&tolerated).unwrap();
    }
}
