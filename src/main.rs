#[macro_use]
mod i18n;

mod app;
mod apps;
mod basket;
mod cli;
mod clones;
mod delete;
mod disks;
mod duplicates;
mod export;
mod history;
mod lists;
mod paths;
mod reports;
mod scanner;
mod search;
mod settings;
mod stats;
mod system;
mod tools;
mod toolsview;
mod trashlog;
mod tree;
mod treemap;
mod ui;

#[cfg(test)]
mod integration;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::event::{self, Event, KeyEventKind};

use app::App;

/// Disk kullanım analizcisi / disk usage analyzer: diskleri bulur, klasör ve dosya boyutlarını
/// gösterir / finds disks and shows folder and file sizes.
#[derive(Parser)]
#[command(
    version,
    override_usage = "rustclean [OPTIONS] [PATH]\n       rustclean report <KIND> [PATH] [OPTIONS]"
)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    /// Doğrudan taranacak klasör (verilmezse disk seçim ekranı açılır) / folder to scan directly
    /// (without it, the disk list opens)
    path: Option<PathBuf>,

    /// Diskleri listeleyip çık / list the disks and exit
    #[arg(long)]
    list_disks: bool,

    /// Arayüz dili / interface language: tr, en (programda L ile değişir / switch with L)
    #[arg(long, value_name = "tr|en", global = true)]
    lang: Option<String>,

    /// Renk teması / color theme: dark, light, colorblind (programda T ile değişir / switch with T)
    #[arg(long, value_name = "dark|light|colorblind", value_parser = ["dark", "light", "colorblind"], global = true)]
    theme: Option<String>,

    #[arg(
        long,
        global = true,
        help = "Renk kullanma (NO_COLOR ortam değişkeni de olur) / no colors (NO_COLOR works too)"
    )]
    no_color: bool,

    /// Arayüz açmadan YOL'u tarayıp özetini yazdır / scan PATH and print a summary, no interface
    #[arg(long, requires = "path")]
    summary: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Arayüz açmadan bir rapor yazdır (yalnızca okur, silmez) / print a report without the
    /// interface (read-only, never deletes)
    #[command(after_help = "rustclean report dev-junk ~/Projects --older 90 --json")]
    Report(cli::ReportArgs),
}

fn main() -> Result<()> {
    let args = Args::parse();
    i18n::init(args.lang.as_deref());
    ui::theme::init(args.theme.as_deref(), args.no_color);

    if let Some(Command::Report(report)) = &args.command {
        if args.path.is_some() || args.summary || args.list_disks {
            use clap::CommandFactory;
            Args::command()
                .error(
                    clap::error::ErrorKind::ArgumentConflict,
                    t!(
                        "report; YOL, --summary ve --list-disks ile birlikte kullanılamaz",
                        "report cannot be combined with PATH, --summary or --list-disks"
                    ),
                )
                .exit();
        }
        return cli::report(report);
    }

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
    // Raw numbers, as scripts may read them.
    let (files, entries) = (root.file_count, t.len() - 1);
    let secs = res.elapsed.as_secs_f64();
    let counts = i18n::pick(
        format!(
            "  {files} dosya, {entries} öğe, {} erişilemeyen, {secs:.1} sn",
            res.errors
        ),
        format!(
            "  {files} {}, {entries} {}, {} inaccessible, {secs:.1} s",
            if files == 1 { "file" } else { "files" },
            if entries == 1 { "entry" } else { "entries" },
            res.errors
        ),
    );
    println!("{counts}");
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
