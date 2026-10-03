#[macro_use]
mod i18n;

mod app;
mod apps;
mod basket;
mod clones;
mod delete;
mod disks;
mod duplicates;
mod history;
mod lists;
mod reports;
mod scanner;
mod search;
mod stats;
mod system;
mod tools;
mod toolsview;
mod tree;
mod treemap;
mod ui;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{self, Event, KeyEventKind};

use app::App;

/// Disk kullanım analizcisi / disk usage analyzer: diskleri bulur, klasör ve dosya boyutlarını
/// gösterir / finds disks and shows folder and file sizes.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Doğrudan taranacak klasör (verilmezse disk seçim ekranı açılır) / folder to scan directly
    /// (without it, the disk list opens)
    path: Option<PathBuf>,

    /// Diskleri listeleyip çık / list the disks and exit
    #[arg(long)]
    list_disks: bool,

    /// Arayüz dili / interface language: tr, en (programda L ile değişir / switch with L)
    #[arg(long, value_name = "tr|en")]
    lang: Option<String>,

    /// Arayüz açmadan YOL'u tarayıp özetini yazdır / scan PATH and print a summary, no interface
    #[arg(long, requires = "path")]
    summary: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    i18n::init(args.lang.as_deref());

    if args.list_disks {
        for d in disks::list_disks() {
            println!(
                "{:<24} {:<30} {:<8} {:>11} / {:>11}",
                d.name,
                d.mount_point.display(),
                d.fs_type,
                ui::fmt_size(d.used()),
                ui::fmt_size(d.total)
            );
        }
        return Ok(());
    }

    if args.summary {
        return print_summary(&args.path.expect("clap enforces path"));
    }

    // ratatui::init installs a panic hook that restores the terminal.
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, App::new(args.path));
    ratatui::restore();
    result
}

fn print_summary(path: &std::path::Path) -> Result<()> {
    use tree::{SizeMode, ROOT};

    let res = scanner::scan(path, disks::all_mount_points(), &Default::default(), |_| {})?;
    let t = &res.tree;
    let root = t.node(ROOT);
    println!("{}", t.root_path().display());
    println!(
        "{}",
        tf!(
            "  diskte: {} ({} B)   görünen: {} ({} B)",
            "  on disk: {} ({} B)   apparent: {} ({} B)",
            ui::fmt_size(root.size.get(SizeMode::Disk)),
            root.size.disk,
            ui::fmt_size(root.size.apparent),
            root.size.apparent
        )
    );
    println!(
        "{}",
        tf!(
            "  {} dosya, {} öğe, {} erişilemeyen, {:.1} sn",
            "  {} files, {} entries, {} inaccessible, {:.1} s",
            root.file_count,
            t.len() - 1,
            res.errors,
            res.elapsed.as_secs_f64()
        )
    );
    Ok(())
}

fn run(terminal: &mut ratatui::DefaultTerminal, mut app: App) -> Result<()> {
    while !app.should_quit {
        terminal.draw(|f| ui::render(f, &mut app))?;
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                // Windows reports key releases too.
                if key.kind == KeyEventKind::Press {
                    app.on_key(key);
                }
            }
        }
        app.on_tick();
    }
    Ok(())
}
