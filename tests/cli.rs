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
