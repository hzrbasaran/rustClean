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

/// A Rust project with build output, a file with a comma and a quote in
/// its name, and a small file.
fn report_fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::create_dir_all(p.join("app/target/debug")).unwrap();
    std::fs::write(p.join("app/Cargo.toml"), "[package]\n").unwrap();
    std::fs::write(p.join("app/target/debug/app"), vec![0u8; 20_000]).unwrap();
    // Windows does not allow `"` in file names.
    let odd = if cfg!(windows) {
        "a, b.bin"
    } else {
        "a, \"b\".bin"
    };
    std::fs::write(p.join(odd), vec![1u8; 9_000]).unwrap();
    std::fs::write(p.join("small.txt"), b"hi").unwrap();
    dir
}

#[test]
fn report_prints_a_table() {
    let dir = report_fixture();
    let root = dir.path().to_str().unwrap();
    let out = rustclean(&["report", "dev-junk", root, "--lang", "en"]);
    assert!(out.status.success(), "{out:?}");
    let text = stdout(&out);
    assert!(text.starts_with("Developer junk — "), "{text}");
    assert!(text.contains("1 row,"), "{text}");
    assert!(text.replace('\\', "/").contains("app/target/"), "{text}");
    // Read-only: nothing was touched.
    assert!(dir.path().join("app/target/debug/app").exists());
}

#[test]
fn an_empty_report_says_why() {
    let dir = report_fixture();
    let root = dir.path().to_str().unwrap();
    let out = rustclean(&["report", "downloads", root, "--lang", "en"]);
    assert!(out.status.success(), "{out:?}");
    let text = stdout(&out);
    assert!(
        text.contains("No Downloads folder below this one"),
        "{text}"
    );
}

#[cfg(unix)] // the file name has a `"`, and paths use `/`
#[test]
fn report_as_csv_quotes_paths() {
    let dir = report_fixture();
    let root = dir.path().to_str().unwrap();
    let out = rustclean(&["report", "largest-files", root, "--csv", "--limit", "2"]);
    assert!(out.status.success(), "{out:?}");
    let text = stdout(&out);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[0],
        "path,apparent_size,disk_size,files,modified,created,group,detail"
    );
    assert_eq!(lines.len(), 3, "{text}");
    assert!(lines[1].contains("/app,20000,"), "{text}");
    // `a, "b".bin`: quoted, with the quote doubled.
    assert!(lines[2].starts_with('"'), "{text}");
    assert!(lines[2].contains("a, \"\"b\"\".bin\",9000,"), "{text}");
}

#[test]
fn report_as_json() {
    let dir = report_fixture();
    let root = dir.path().to_str().unwrap();
    let out = rustclean(&["--lang", "en", "report", "largest-files", root, "--json"]);
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["title"], "Largest files");
    assert_eq!(v["truncated"], false);
    let entries = v["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0]["apparent_size"], 20_000);
    assert_eq!(entries[0]["files"], 1);
    assert!(std::path::Path::new(entries[0]["path"].as_str().unwrap()).is_absolute());
    assert!(entries[0]["modified"].as_str().unwrap().ends_with('Z'));
}

#[test]
fn report_errors() {
    let unknown = rustclean(&["report", "biggest"]);
    assert_eq!(unknown.status.code(), Some(2));
    let err = String::from_utf8_lossy(&unknown.stderr);
    assert!(
        err.contains("dev-junk") && err.contains("largest-files"),
        "{err}"
    );

    let dir = report_fixture();
    let root = dir.path().to_str().unwrap();
    let no_age = rustclean(&["report", "old-big", root, "--older", "30", "--lang", "en"]);
    assert_eq!(no_age.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&no_age.stderr).contains("no age filter"),
        "{no_age:?}"
    );

    let both = rustclean(&["report", "apps", root, "--csv", "--json"]);
    assert_eq!(both.status.code(), Some(2));

    let mixed = rustclean(&["--list-disks", "report", "apps", root]);
    assert_eq!(mixed.status.code(), Some(2));

    let missing = dir.path().join("nope");
    let out = rustclean(&[
        "report",
        "caches",
        missing.to_str().unwrap(),
        "--lang",
        "en",
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("could not read"),
        "{out:?}"
    );
}

#[test]
fn report_age_filter() {
    let dir = report_fixture();
    let root = dir.path().to_str().unwrap();
    // Everything was just written: nothing is 90 days old.
    let out = rustclean(&["report", "largest-files", root, "--older", "90", "--json"]);
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["entries"].as_array().unwrap().is_empty(), "{v}");
}

#[test]
fn report_of_the_current_folder() {
    let dir = report_fixture();
    let data = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rustclean"))
        .args(["report", "largest-files", "--csv"])
        .current_dir(dir.path())
        .env("RUSTCLEAN_DATA_DIR", data.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(stdout(&out).lines().count(), 5, "{out:?}");
}
