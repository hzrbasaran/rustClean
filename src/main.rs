#[macro_use]
mod i18n;

mod app;
mod apps;
mod basket;
mod clones;
mod config;
mod delete;
mod disks;
mod duplicates;
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

    /// Renk teması / color theme: dark, light, colorblind (programda T ile değişir / switch with T)
    #[arg(long, value_name = "dark|light|colorblind", value_parser = ["dark", "light", "colorblind"])]
    theme: Option<String>,

    #[arg(
        long,
        help = "Renk kullanma (NO_COLOR ortam değişkeni de olur) / no colors (NO_COLOR works too)"
    )]
    no_color: bool,

    /// Arayüz açmadan YOL'u tarayıp özetini yazdır / scan PATH and print a summary, no interface
    #[arg(long, requires = "path")]
    summary: bool,

    /// Ayar dosyasının yerini ve geçerli değerleri yazdır / print the config file's path and the
    /// values in effect
    #[arg(long)]
    config: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    i18n::init(args.lang.as_deref());
    ui::theme::init(args.theme.as_deref(), args.no_color);
    // A broken file never stops the program: the defaults apply instead.
    let loaded = config::load();
    config::init(loaded.config.clone());

    if args.config {
        print_config(&loaded);
        return Ok(());
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
        for message in loaded.messages() {
            eprintln!("{message}");
        }
        return print_summary(&args.path.expect("clap enforces path"));
    }

    let mut app = App::new(args.path);
    if let Some(text) = loaded.status() {
        app.config_notice(text);
    }
    // ratatui::init installs a panic hook that restores the terminal.
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, app);
    ratatui::restore();
    result
}

fn print_summary(path: &std::path::Path) -> Result<()> {
    use tree::{SizeMode, ROOT};

    let res = scanner::scan(path, config::scan_skip(), &Default::default(), |_| {})?;
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

/// `--config`: where the file is, then the values in effect as a file that
/// can be copied there. Problems go to stderr.
fn print_config(loaded: &config::Loaded) {
    let path = loaded.path.as_ref().map_or_else(
        || t!("(veri klasörü bulunamadı)", "(no data directory)").to_string(),
        |p| p.display().to_string(),
    );
    println!("# {path}");
    let state = if !loaded.found {
        t!(
            "Dosya yok; varsayılanlar geçerli. Aşağıdakiler bu dosyaya kopyalanabilir.",
            "No file; the defaults apply. What follows can be copied into it."
        )
    } else if loaded.problems.is_empty() {
        t!("Geçerli değerler:", "Values in effect:")
    } else {
        t!(
            "Geçerli değerler (sorunlar stderr'de):",
            "Values in effect (problems on stderr):"
        )
    };
    println!("# {state}");
    println!();
    print!("{}", config::to_toml(&loaded.config));
    for message in loaded.messages() {
        eprintln!("{message}");
    }
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
