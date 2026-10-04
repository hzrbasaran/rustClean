//! The system data screen.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::app::Browser;
use crate::system;
use crate::system::SystemInfo;

use super::format::{fmt_count, fmt_size};
use super::style::usage_color;
use super::{bar, halves, panel};

pub(super) fn render_system(f: &mut Frame, b: &Browser, sys: &SystemInfo, area: Rect) {
    let vol_rows = sys.main.as_ref().map_or(1, |c| c.volumes.len()) as u16;
    let [top, mid, bottom] = Layout::vertical([
        Constraint::Length(vol_rows + 4),
        Constraint::Length(9),
        Constraint::Min(3),
    ])
    .areas(area);
    let [gap_area, other_area] = halves(mid);
    let line = |label: &str, value: String| {
        Line::from(vec![
            Span::raw(format!("{label:<30}")).white(),
            Span::raw(value).bold(),
        ])
    };

    // APFS container and its volumes
    let mut lines = Vec::new();
    let title = match &sys.main {
        Some(c) => {
            let used = c.total.saturating_sub(c.free);
            let ratio = used as f64 / c.total.max(1) as f64;
            let bar_w = (top.width as usize).saturating_sub(60).max(10);
            lines.push(Line::from(vec![
                Span::raw(tf!(
                    "Toplam {}  ·  kullanılan {}  ·  boş {}   ",
                    "Total {}  ·  used {}  ·  free {}   ",
                    fmt_size(c.total),
                    fmt_size(used),
                    fmt_size(c.free)
                )),
                Span::styled(bar(ratio, bar_w), Style::new().fg(usage_color(ratio))),
                Span::raw(format!(" %{:.0}", ratio * 100.0)).bold(),
            ]));
            lines.push(Line::from(""));
            let mut vols = c.volumes.clone();
            vols.sort_by_key(|v| std::cmp::Reverse(v.used));
            for v in vols {
                let ratio = v.used as f64 / c.total.max(1) as f64;
                lines.push(Line::from(vec![
                    Span::raw(format!("{:<26}", v.name)).white().bold(),
                    Span::raw(format!("{:<38}", system::role_label(&v.role))).gray(),
                    Span::raw(format!("{:>11}  ", fmt_size(v.used))),
                    Span::styled(bar(ratio, 20), Style::new().fg(Color::Cyan)),
                ]));
            }
            tf!(
                " APFS kapsayıcısı ({}) ",
                " APFS container ({}) ",
                c.reference
            )
        }
        None => {
            lines.push(
                Line::from(t!(
                    "APFS bilgisi alınamadı.",
                    "Could not read APFS information."
                ))
                .yellow(),
            );
            t!(" APFS kapsayıcısı ", " APFS container ").to_string()
        }
    };
    f.render_widget(Paragraph::new(lines).block(panel(&title, false)), top);

    // What the scan cannot see
    let scanned = b.tree.node(crate::tree::ROOT).size.disk;
    let mut lines = Vec::new();
    match sys.scannable_used() {
        Some(expected) if b.tree.root_path() == std::path::Path::new("/") => {
            lines.push(line(
                t!("Sistem + Veri bölümleri", "System + Data volumes"),
                fmt_size(expected),
            ));
            lines.push(line(
                t!("Taramanın bulduğu", "Found by the scan"),
                fmt_size(scanned),
            ));
            if expected >= scanned {
                lines.push(
                    line(
                        t!("Taramanın göremediği", "Not seen by the scan"),
                        fmt_size(expected - scanned),
                    )
                    .yellow(),
                );
                lines.push(Line::from(t!("Olası nedenler:", "Possible reasons:")).gray());
                if b.errors > 0 {
                    lines.push(Line::from(tf!(
                        " • erişilemeyen {} öğe — terminale Tam Disk Erişimi verin",
                        " • {} inaccessible items — give the terminal Full Disk Access",
                        fmt_count(b.errors)
                    )));
                }
                if !sys.snapshots.is_empty() {
                    lines.push(Line::from(tf!(
                        " • {} Time Machine yerel anlık görüntüsü",
                        " • {} local Time Machine snapshots",
                        sys.snapshots.len()
                    )));
                }
                lines.push(Line::from(t!(
                    " • silinebilir (purgeable) alan ve dosya sistemi meta verisi",
                    " • purgeable space and file system metadata",
                )));
            } else {
                lines.push(line(
                    t!("Fazla sayılan", "Counted extra"),
                    fmt_size(scanned - expected),
                ));
                lines.push(
                    Line::from(t!(
                        "Tam klonlar bir kez sayılır; kısmen değiştirilmiş klonlar ve küçük \
                         dosyaların klonları ise blok paylaşsa da ayrı sayılır.",
                        "Pure clones are counted once; partly modified clones and clones of \
                         small files share blocks but are still counted separately.",
                    ))
                    .gray(),
                );
            }
        }
        _ => {
            lines.push(Line::from(t!(
                "Bu karşılaştırma için diskin kökünü (/) tarayın:",
                "For this comparison, scan the disk root (/):",
            )));
            lines.push(Line::from("d → Macintosh HD").cyan());
        }
    }
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel(
                t!(" Taramanın göremediği ", " Not seen by the scan "),
                false,
            )),
        gap_area,
    );

    // Snapshots and memory files
    let mut lines = vec![line(
        t!("Time Machine anlık görüntüsü", "Time Machine snapshots"),
        tf!("{} adet", "{}", sys.snapshots.len()),
    )];
    for name in sys.snapshots.iter().take(2) {
        lines.push(Line::from(format!("  {name}")).gray());
    }
    if let Some(first) = sys.snapshots.first() {
        let date = first
            .trim_start_matches("com.apple.TimeMachine.")
            .trim_end_matches(".local");
        lines.push(
            Line::from(tf!(
                "  silmek için: sudo tmutil deletelocalsnapshots {date}",
                "  to delete: sudo tmutil deletelocalsnapshots {date}"
            ))
            .cyan(),
        );
    }
    if let Some((total, used)) = sys.swap {
        lines.push(line(
            t!("Takas (swap)", "Swap"),
            format!("{} / {}", fmt_size(used), fmt_size(total)),
        ));
    }
    if let Some(sleep) = sys.sleepimage {
        lines.push(line(
            t!("Uyku görüntüsü (sleepimage)", "Sleep image (sleepimage)"),
            fmt_size(sleep),
        ));
    }
    f.render_widget(
        Paragraph::new(lines).block(panel(
            t!(" Anlık görüntüler ve bellek ", " Snapshots and memory "),
            false,
        )),
        other_area,
    );

    // Simulator runtimes and problems
    let mut lines = Vec::new();
    if !sys.simulators.is_empty() {
        let total: u64 = sys.simulators.iter().map(|c| c.used()).sum();
        lines.push(line(
            t!("Simülatör çalışma zamanları", "Simulator runtimes"),
            tf!(
                "{} imaj, {}",
                "{} images, {}",
                sys.simulators.len(),
                fmt_size(total)
            ),
        ));
        lines.push(
            Line::from(t!(
                "  Ayrı disk imajlarında durur, taramada görünmez. Temizlik: m → Geliştirici araçları temizliği",
                "  Kept in separate disk images, not in the scan. Clean up: m → Developer tools cleanup",
            ))
                .gray(),
        );
    }
    for p in &sys.problems {
        lines.push(Line::from(format!("⚠ {p}")).yellow());
    }
    f.render_widget(
        Paragraph::new(lines).block(panel(t!(" Diğer ", " Other "), false)),
        bottom,
    );
}
