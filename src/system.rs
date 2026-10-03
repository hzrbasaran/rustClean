//! "System data" panel: what the disk holds besides the files a scan can
//! see (macOS). Information comes from `diskutil`, `tmutil` and `sysctl`;
//! nothing here changes the system.

use std::fs;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    pub name: String,
    pub role: String,
    pub used: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub reference: String,
    pub total: u64,
    pub free: u64,
    pub volumes: Vec<Volume>,
}

impl Container {
    pub fn has_role(&self, role: &str) -> bool {
        self.volumes.iter().any(|v| v.role == role)
    }

    pub fn used(&self) -> u64 {
        self.volumes.iter().map(|v| v.used).sum()
    }
}

#[derive(Debug, Default)]
pub struct SystemInfo {
    /// The container holding the System and Data volumes.
    pub main: Option<Container>,
    /// Simulator runtime images, each in its own container.
    pub simulators: Vec<Container>,
    pub snapshots: Vec<String>,
    /// Swap total and used, in bytes.
    pub swap: Option<(u64, u64)>,
    pub sleepimage: Option<u64>,
    /// Commands that could not be run or parsed.
    pub problems: Vec<String>,
}

impl SystemInfo {
    /// Used space of the System and Data volumes: what a scan of "/" sees.
    pub fn scannable_used(&self) -> Option<u64> {
        let main = self.main.as_ref()?;
        Some(
            main.volumes
                .iter()
                .filter(|v| v.role == "System" || v.role == "Data")
                .map(|v| v.used)
                .sum(),
        )
    }
}

pub fn collect() -> SystemInfo {
    let mut info = SystemInfo::default();
    if !cfg!(target_os = "macos") {
        info.problems.push(
            t!(
                "Sistem verileri paneli yalnızca macOS'ta çalışır.",
                "The system data panel only works on macOS.",
            )
            .into(),
        );
        return info;
    }
    match run(&["diskutil", "apfs", "list", "-plist"]).and_then(|out| parse_apfs(out.as_bytes())) {
        Some(containers) => {
            for c in containers {
                if c.has_role("System") || c.has_role("Data") {
                    info.main = Some(c);
                } else if c.volumes.iter().any(|v| v.name.contains("Simulator")) {
                    info.simulators.push(c);
                }
            }
        }
        None => info.problems.push(
            t!(
                "diskutil apfs list okunamadı",
                "could not read diskutil apfs list"
            )
            .into(),
        ),
    }
    match run(&["tmutil", "listlocalsnapshots", "/"]) {
        Some(out) => info.snapshots = parse_snapshots(&out),
        None => info
            .problems
            .push(t!("tmutil çalıştırılamadı", "could not run tmutil").into()),
    }
    info.swap = run(&["sysctl", "-n", "vm.swapusage"]).and_then(|s| parse_swap(&s));
    info.sleepimage = fs::metadata("/private/var/vm/sleepimage")
        .ok()
        .map(|m| m.len());
    info
}

fn run(cmd: &[&str]) -> Option<String> {
    let out = Command::new(cmd[0]).args(&cmd[1..]).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn parse_apfs(bytes: &[u8]) -> Option<Vec<Container>> {
    let root = plist::Value::from_reader(std::io::Cursor::new(bytes)).ok()?;
    let containers = root.as_dictionary()?.get("Containers")?.as_array()?;
    let int = |d: &plist::Dictionary, k: &str| d.get(k).and_then(|v| v.as_unsigned_integer());
    let text = |d: &plist::Dictionary, k: &str| {
        d.get(k)
            .and_then(|v| v.as_string())
            .unwrap_or_default()
            .to_string()
    };
    let out = containers
        .iter()
        .filter_map(|c| {
            let c = c.as_dictionary()?;
            let volumes = c
                .get("Volumes")?
                .as_array()?
                .iter()
                .filter_map(|v| {
                    let v = v.as_dictionary()?;
                    let role = v
                        .get("Roles")
                        .and_then(|r| r.as_array())
                        .and_then(|r| r.first())
                        .and_then(|r| r.as_string())
                        .unwrap_or("")
                        .to_string();
                    Some(Volume {
                        name: text(v, "Name"),
                        role,
                        used: int(v, "CapacityInUse")?,
                    })
                })
                .collect();
            Some(Container {
                reference: text(c, "ContainerReference"),
                total: int(c, "CapacityCeiling")?,
                free: int(c, "CapacityFree")?,
                volumes,
            })
        })
        .collect();
    Some(out)
}

/// Snapshot names from `tmutil listlocalsnapshots /`.
pub fn parse_snapshots(out: &str) -> Vec<String> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("Snapshots for"))
        .map(str::to_string)
        .collect()
}

/// "total = 8192.00M  used = 7025.62M  free = 1166.38M  (encrypted)"
pub fn parse_swap(out: &str) -> Option<(u64, u64)> {
    let value = |key: &str| -> Option<u64> {
        let rest = out.split(&format!("{key} = ")).nth(1)?;
        let num = rest.split_whitespace().next()?;
        let (digits, unit) = num.split_at(num.find(|c: char| c.is_ascii_alphabetic())?);
        let mult = match unit {
            "K" => 1u64 << 10,
            "M" => 1 << 20,
            "G" => 1 << 30,
            _ => return None,
        };
        Some((digits.parse::<f64>().ok()? * mult as f64) as u64)
    };
    Some((value("total")?, value("used")?))
}

/// Name for an APFS volume role, in the interface language.
pub fn role_label(role: &str) -> &str {
    match role {
        "System" => t!("Sistem (salt okunur macOS)", "System (read-only macOS)"),
        "Data" => t!(
            "Veri (uygulamalar ve dosyalarınız)",
            "Data (apps and your files)"
        ),
        "VM" => t!("Sanal bellek (takas)", "Virtual memory (swap)"),
        "Preboot" => t!("Önyükleme", "Preboot"),
        "Recovery" => t!("Kurtarma", "Recovery"),
        "Update" => t!("Güncelleme", "Update"),
        "" => "—",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APFS: &[u8] = include_bytes!("../tests/fixtures/apfs_list.plist");

    #[test]
    fn parses_diskutil_plist() {
        let containers = parse_apfs(APFS).unwrap();
        assert_eq!(containers.len(), 9);
        let main = containers.iter().find(|c| c.has_role("Data")).unwrap();
        assert_eq!(main.reference, "disk3");
        assert_eq!(main.total, 994_662_584_320);
        let data = main.volumes.iter().find(|v| v.role == "Data").unwrap();
        assert_eq!(data.name, "Macintosh HD - Data");
        assert_eq!(data.used, 786_977_054_720);
        let sims = containers
            .iter()
            .filter(|c| c.volumes.iter().any(|v| v.name.contains("Simulator")))
            .count();
        assert!(sims > 0);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_apfs(b"not a plist").is_none());
    }

    #[test]
    fn parses_tmutil_and_swap() {
        assert!(parse_snapshots("Snapshots for disk /:\n").is_empty());
        assert_eq!(
            parse_snapshots(
                "Snapshots for disk /:\ncom.apple.TimeMachine.2026-10-01-101500.local\n"
            ),
            vec!["com.apple.TimeMachine.2026-10-01-101500.local"]
        );
        let (total, used) =
            parse_swap("total = 8192.00M  used = 7025.62M  free = 1166.38M  (encrypted)").unwrap();
        assert_eq!(total, 8192 << 20);
        assert_eq!(used, (7025.62 * (1u64 << 20) as f64) as u64);
        assert!(parse_swap("garbage").is_none());
    }
}
