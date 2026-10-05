//! `rustclean report <kind> [PATH]`: runs one report without the interface
//! and prints it as a table, CSV or JSON. It only reads; nothing is deleted.

use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use anyhow::{bail, Result};
use clap::builder::PossibleValue;

use crate::app::Browser;
use crate::export::{self, Format, Meta};
use crate::lists::ResultList;
use crate::reports::ReportKind;
use crate::tree::{SizeMode, Tree, ROOT};
use crate::{config, duplicates, scanner, ui};

impl clap::ValueEnum for ReportKind {
    fn value_variants<'a>() -> &'a [Self] {
        &ReportKind::ALL
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        Some(PossibleValue::new(self.slug()))
    }
}

#[derive(clap::Args)]
pub struct ReportArgs {
    /// Rapor türü / report kind
    #[arg(value_name = "KIND")]
    pub kind: ReportKind,

    /// Taranacak klasör (verilmezse bulunulan klasör) / folder to scan (default: the current
    /// folder)
    pub path: Option<PathBuf>,

    /// Yalnızca en az bu kadar gündür dokunulmamış öğeler / only entries untouched for at least
    /// this many days
    #[arg(long, value_name = "DAYS")]
    pub older: Option<u32>,

    /// JSON olarak yazdır / print as JSON
    #[arg(long, conflicts_with = "csv")]
    pub json: bool,

    /// CSV olarak yazdır / print as CSV
    #[arg(long)]
    pub csv: bool,

    /// En fazla bu kadar satır / at most this many rows
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,
}

/// Scans the folder, runs the report and prints it to stdout.
pub fn report(args: &ReportArgs) -> Result<()> {
    let kind = args.kind;
    let min_age = args.older.unwrap_or(0);
    if min_age > 0 && !kind.supports_age() {
        bail!(
            "{}",
            tf!(
                "„{}” raporunda yaş filtresi yok; --older olmadan çalıştırın.",
                "The {} report has no age filter; run it without --older.",
                kind.slug()
            )
        );
    }
    let path = match &args.path {
        Some(p) => p.clone(),
        None => std::env::current_dir()?,
    };
    let res = scanner::scan(&path, config::scan_skip(), &Default::default(), |_| {})?;
    // The interface's own report code, over a browser that is never shown
    // and records no history. The table shows sizes on disk, so the reports
    // rank by them too, whatever size the configuration opens scans with.
    let mut browser = Browser::new(res);
    browser.size_mode = SizeMode::Disk;
    let tree = &browser.tree;
    let mut list = build(&browser, kind, min_age);
    if let Some(n) = args.limit {
        if list.rows.len() > n {
            list.rows.truncate(n);
            list.truncated = true;
        }
    }

    let format = if args.json {
        Some(Format::Json)
    } else if args.csv {
        Some(Format::Csv)
    } else {
        None
    };
    let mut out = BufWriter::new(io::stdout().lock());
    let written = match format {
        Some(format) => {
            let meta = Meta {
                title: list.title.clone(),
                root: tree.path_of(list.base),
                truncated: list.truncated,
            };
            export::write(&mut out, format, &meta, &export::list_records(tree, &list))
        }
        None => print_table(&mut out, tree, &list),
    };
    match written.and_then(|()| out.flush()) {
        // `rustclean report … | head` closes the pipe early: not an error.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        other => Ok(other?),
    }
}

/// The report over the whole scan. Duplicates are searched right here, not
/// on a background thread as in the interface.
fn build(b: &Browser, kind: ReportKind, min_age: u32) -> ResultList {
    if kind != ReportKind::Duplicates {
        return b.report_list(kind, ROOT, min_age);
    }
    let cands = duplicates::candidates(&b.tree, ROOT);
    if cands.is_empty() {
        return b.no_duplicates_list(ROOT);
    }
    let groups = duplicates::find_groups(
        &cands,
        &AtomicBool::new(false),
        &duplicates::Progress::default(),
    );
    b.duplicates_list(ROOT, groups)
}

/// Members of a group shown under it in the table.
const MEMBERS: usize = 10;

/// A readable table: size, files, last change, path and the row's detail.
/// A group's members follow it, indented.
fn print_table(w: &mut impl Write, tree: &Tree, list: &ResultList) -> io::Result<()> {
    let mode = SizeMode::Disk;
    writeln!(w, "{} — {}", list.title, tree.path_of(list.base).display())?;
    if list.rows.is_empty() {
        // The note says why nothing was found, as in the interface.
        let why = if list.note.is_empty() {
            t!("Sonuç yok.", "No results.")
        } else {
            list.note.as_str()
        };
        return writeln!(w, "{why}");
    }
    writeln!(
        w,
        "{}",
        tf!(
            "{}, toplam {} (diskte)",
            "{}, {} in total (on disk)",
            crate::i18n::count(list.rows.len() as u64, "satır", "row", "rows"),
            ui::fmt_size(list.total_size())
        )
    )?;
    writeln!(w)?;
    let width = list
        .rows
        .iter()
        .map(|r| r.label.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(10, 60);
    writeln!(
        w,
        "{:>10}  {:>9}  {:<16}  {:<width$}  {}",
        t!("Boyut", "Size"),
        t!("Dosya", "Files"),
        t!("Değişiklik", "Modified"),
        t!("Konum", "Location"),
        t!("Bilgi", "Detail"),
    )?;
    let date = |secs: u32| {
        if secs == 0 {
            String::new()
        } else {
            ui::fmt_date(secs)
        }
    };
    // Folders show how many files they hold; for a file it is always 1.
    let count = |files: u64, dir: bool| {
        if dir {
            ui::fmt_count(files)
        } else {
            String::new()
        }
    };
    for row in &list.rows {
        let (files, modified) = if row.group {
            let files: u64 = row
                .nodes
                .iter()
                .map(|&id| u64::from(tree.node(id).file_count))
                .sum();
            (count(files, true), 0)
        } else {
            let n = tree.node(row.nodes[0]);
            (count(u64::from(n.file_count), n.is_dir), n.modified)
        };
        let line = format!(
            "{:>10}  {:>9}  {:<16}  {:<width$}  {}",
            ui::fmt_size(row.size()),
            files,
            date(modified),
            row.label,
            row.detail,
        );
        writeln!(w, "{}", line.trim_end())?;
        if row.group {
            // Largest first, as when the group is opened in the interface.
            let mut ids = row.nodes.clone();
            ids.sort_by_key(|&id| std::cmp::Reverse(tree.node(id).size.get(mode)));
            for &id in ids.iter().take(MEMBERS) {
                let n = tree.node(id);
                let label = crate::lists::relative_label(tree, list.base, id);
                let line = format!(
                    "{:>10}  {:>9}  {:<16}    {label}",
                    ui::fmt_size(n.size.get(mode)),
                    count(u64::from(n.file_count), n.is_dir),
                    date(n.modified),
                );
                writeln!(w, "{line}")?;
            }
            if ids.len() > MEMBERS {
                let more = (ids.len() - MEMBERS) as u64;
                let text = tf!(
                    "… ve {} öğe daha (hepsi için --csv / --json)",
                    "… and {} more (all of them with --csv / --json)",
                    ui::fmt_count(more)
                );
                writeln!(w, "{:>10}  {:>9}  {:<16}    {text}", "", "", "")?;
            }
        }
    }
    if list.truncated {
        writeln!(w)?;
        writeln!(
            w,
            "{}",
            t!(
                "Daha fazla sonuç var; yalnızca ilkler gösterildi.",
                "There are more results; only the first ones are shown."
            )
        )?;
    }
    Ok(())
}
