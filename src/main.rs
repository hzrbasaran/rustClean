#[macro_use]
mod i18n;

mod app;
mod apps;
mod basket;
mod check;
mod cli;
mod clones;
mod config;
mod delete;
mod disks;
mod duplicates;
mod export;
mod history;
mod htmlmap;
mod lists;
mod newfile;
mod paths;
mod reports;
mod scanner;
mod search;
mod settings;
mod similar;
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseEventKind,
};

use app::App;

/// Disk kullanım analizcisi / disk usage analyzer: diskleri bulur, klasör ve dosya boyutlarını
/// gösterir / finds disks and shows folder and file sizes.
#[derive(Parser)]
#[command(
    version,
    override_usage = "rustclean [OPTIONS] [PATH]\n       rustclean report <KIND> [PATH] [OPTIONS]\n       rustclean check [OPTIONS]"
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

    /// Ayar dosyasının yerini ve geçerli değerleri yazdır / print the config file's path and the
    /// values in effect
    #[arg(long)]
    config: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Arayüz açmadan bir rapor yazdır (yalnızca okur, silmez) / print a report without the
    /// interface (read-only, never deletes)
    #[command(after_help = "rustclean report dev-junk ~/Projects --older 90 --json")]
    Report(cli::ReportArgs),
    /// Disklerin doluluğunu kontrol et, eşiği geçince uyar (yalnızca okur) / check how full the
    /// disks are and warn above a threshold (read-only)
    #[command(after_help = "rustclean check --threshold 85\nrustclean check --install")]
    Check(check::CheckArgs),
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
        for message in loaded.messages() {
            eprintln!("{message}");
        }
        return cli::report(report);
    }

    if let Some(Command::Check(check)) = &args.command {
        if args.path.is_some() || args.summary || args.list_disks {
            use clap::CommandFactory;
            Args::command()
                .error(
                    clap::error::ErrorKind::ArgumentConflict,
                    t!(
                        "check; YOL, --summary ve --list-disks ile birlikte kullanılamaz",
                        "check cannot be combined with PATH, --summary or --list-disks"
                    ),
                )
                .exit();
        }
        for message in loaded.messages() {
            eprintln!("{message}");
        }
        let code = check::run(check)?;
        std::process::exit(code);
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
    // ratatui::init installs a panic hook that restores the terminal. It
    // does not know about mouse capture, so ours turns that off first.
    let mut terminal = ratatui::init();
    let restore_terminal = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = set_mouse_capture(false);
        restore_terminal(info);
    }));
    let result = run(&mut terminal, app);
    let mouse_off = set_mouse_capture(false);
    ratatui::restore();
    result.and(mouse_off.map_err(Into::into))
}

/// Whether the terminal currently reports the mouse to us.
static MOUSE_CAPTURED: AtomicBool = AtomicBool::new(false);

/// Turns mouse capture on or off, if it is not already.
fn set_mouse_capture(on: bool) -> std::io::Result<()> {
    if MOUSE_CAPTURED.swap(on, Ordering::SeqCst) == on {
        return Ok(());
    }
    let mut out = std::io::stdout();
    if on {
        crossterm::execute!(out, EnableMouseCapture)
    } else {
        crossterm::execute!(out, DisableMouseCapture)
    }
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
            let mut ev = event::read()?;
            // Moving the pointer sends a stream of events that change
            // nothing: read on without drawing after each of them.
            while is_pointer_motion(&ev) && event::poll(Duration::ZERO)? {
                ev = event::read()?;
            }
            match ev {
                // Windows reports key releases too.
                Event::Key(key) if key.kind == KeyEventKind::Press => app.on_key(key),
                Event::Mouse(m) => app.on_mouse(m),
                _ => {}
            }
        }
        set_mouse_capture(app.mouse.on)?;
        app.on_tick();
    }
    Ok(())
}

fn is_pointer_motion(ev: &Event) -> bool {
    matches!(
        ev,
        Event::Mouse(m) if matches!(m.kind, MouseEventKind::Moved | MouseEventKind::Drag(_))
    )
}
