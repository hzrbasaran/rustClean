//! The command line, run as a separate process.

use std::process::{Command, Output};

fn rustclean(args: &[&str]) -> Output {
    let data = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_rustclean"))
        .args(args)
        // Never read or write the user's settings.
        .env("RUSTCLEAN_DATA_DIR", data.path())
        .output()
        .unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn summary_prints_the_totals() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("a.bin"), vec![0u8; 5000]).unwrap();
    std::fs::write(dir.path().join("sub/b.bin"), vec![1u8; 3000]).unwrap();

    let out = rustclean(&["--summary", "--lang", "en", dir.path().to_str().unwrap()]);
    assert!(out.status.success(), "{out:?}");
    let text = stdout(&out);
    assert!(text.contains("apparent: 7.8 KiB (8000 B)"), "{text}");
    assert!(
        text.contains("2 files, 3 entries, 0 inaccessible"),
        "{text}"
    );
}

#[test]
fn summary_speaks_turkish() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.bin"), vec![0u8; 1000]).unwrap();
    let out = rustclean(&["--summary", "--lang", "tr", dir.path().to_str().unwrap()]);
    assert!(
        stdout(&out).contains("1 dosya, 1 öğe, 0 erişilemeyen"),
        "{out:?}"
    );
}

#[test]
fn summary_of_a_missing_folder_fails() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope");
    let out = rustclean(&["--summary", "--lang", "en", missing.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("could not read"),
        "{out:?}"
    );
}

#[test]
fn summary_needs_a_path() {
    let out = rustclean(&["--summary"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("<PATH>"),
        "{out:?}"
    );
}

#[test]
fn help_and_version() {
    let help = rustclean(&["--help"]);
    assert!(help.status.success());
    assert!(stdout(&help).contains("--summary"));
    let version = rustclean(&["--version"]);
    assert_eq!(
        stdout(&version).trim(),
        format!("rustclean {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn theme_options() {
    let help = stdout(&rustclean(&["--help"]));
    assert!(
        help.contains("--theme") && help.contains("--no-color"),
        "{help}"
    );
    let bad = rustclean(&["--theme", "pink", "--list-disks"]);
    assert_eq!(bad.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&bad.stderr).contains("colorblind"),
        "{bad:?}"
    );
    let ok = rustclean(&["--theme", "light", "--no-color", "--list-disks"]);
    assert!(ok.status.success(), "{ok:?}");
}

/// Runs rustclean with `config` as its `config.toml`. Returns the output,
/// the file, and the data directory (removed when dropped).
fn with_config(config: &str, args: &[&str]) -> (Output, std::path::PathBuf, tempfile::TempDir) {
    let data = tempfile::tempdir().unwrap();
    let file = data.path().join("config.toml");
    std::fs::write(&file, config).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rustclean"))
        .args(args)
        .env("RUSTCLEAN_DATA_DIR", data.path())
        .output()
        .unwrap();
    (out, file, data)
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn summary_skips_excluded_folders() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("skip")).unwrap();
    std::fs::write(dir.path().join("a.bin"), vec![0u8; 5000]).unwrap();
    std::fs::write(dir.path().join("skip/b.bin"), vec![1u8; 3000]).unwrap();
    let skip = std::path::absolute(dir.path().join("skip")).unwrap();
    // A literal string, so Windows backslashes stay as they are.
    let config = format!("[scan]\nexclude = ['{}']\n", skip.display());
    let path = dir.path().to_str().unwrap();
    let (out, _, _data) = with_config(&config, &["--summary", "--lang", "en", path]);
    assert!(out.status.success(), "{out:?}");
    let text = stdout(&out);
    assert!(text.contains("apparent: 4.9 KiB (5000 B)"), "{text}");
    assert!(text.contains("1 file, 2 entries"), "{text}");
    assert_eq!(stderr(&out), "");
}

#[test]
fn a_broken_config_does_not_stop_the_summary() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.bin"), vec![0u8; 1000]).unwrap();
    let path = dir.path().to_str().unwrap();
    let (out, file, _data) = with_config("[scan\n", &["--summary", "--lang", "en", path]);
    assert!(out.status.success(), "{out:?}");
    assert!(stdout(&out).contains("1 file, 1 entry"), "{out:?}");
    let err = stderr(&out);
    assert!(err.contains(&file.display().to_string()), "{err}");
    assert!(err.contains("using the defaults"), "{err}");
    assert!(err.contains("line 1, column"), "{err}");
}

#[test]
fn config_prints_the_path_and_the_values() {
    // No file: the path and the defaults.
    let out = rustclean(&["--config", "--lang", "en"]);
    assert!(out.status.success(), "{out:?}");
    let text = stdout(&out);
    assert!(text.contains("config.toml"), "{text}");
    assert!(text.contains("No file; the defaults apply"), "{text}");
    assert!(text.contains("old_big_min_mib = 100"), "{text}");
    assert!(text.contains("size = \"disk\""), "{text}");

    // A file with a value and an unknown key.
    let (out, file, _data) = with_config(
        "[reports]\nold_big_min_days = 30\nnope = 1\n",
        &["--config", "--lang", "en"],
    );
    assert!(out.status.success(), "{out:?}");
    let text = stdout(&out);
    assert!(
        text.starts_with(&format!("# {}\n", file.display())),
        "{text}"
    );
    assert!(text.contains("old_big_min_days = 30"), "{text}");
    assert!(
        stderr(&out).contains("Unknown key ignored: reports.nope"),
        "{out:?}"
    );
}
