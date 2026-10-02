mod app;
mod apps;
mod delete;
mod disks;
mod lists;
mod reports;
mod scanner;
mod search;
mod stats;
mod tree;
mod ui;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{self, Event, KeyEventKind};

use app::App;

/// Disk kullanım analizcisi: diskleri bulur, klasör ve dosya boyutlarını gösterir.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Doğrudan taranacak klasör (verilmezse disk seçim ekranı açılır)
    path: Option<PathBuf>,

    /// Diskleri listeleyip çık
    #[arg(long)]
    list_disks: bool,

    /// Arayüz açmadan YOL'u tarayıp özetini yazdır
    #[arg(long, requires = "path")]
    summary: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

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
        "  diskte: {} ({} B)   görünen: {} ({} B)",
        ui::fmt_size(root.size.get(SizeMode::Disk)),
        root.size.disk,
        ui::fmt_size(root.size.apparent),
        root.size.apparent
    );
    println!(
        "  {} dosya, {} öğe, {} erişilemeyen, {:.1} sn",
        root.file_count,
        t.len() - 1,
        res.errors,
        res.elapsed.as_secs_f64()
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
