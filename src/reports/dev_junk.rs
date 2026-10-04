//! Build output and dependency folders of developer projects.

use crate::tree::{NodeId, SizeMode, Tree};

use super::{by_size, singles, walk, AgeFilter, Report};

/// What kind of regenerable directory `id` is, if any.
pub(super) fn dev_junk_kind(tree: &Tree, id: NodeId) -> Option<&'static str> {
    let name = tree.name(id);
    let parent = tree.parent(id)?;
    let sibling = |pred: &dyn Fn(&str) -> bool| tree.children(parent).any(|c| pred(tree.name(c)));
    let has = |file: &str| sibling(&|n| n == file);
    let kind = match name {
        "node_modules" => t!("Node.js bağımlılıkları", "Node.js dependencies"),
        "__pycache__" => t!("Python önbelleği", "Python cache"),
        ".gradle" => t!("Gradle önbelleği", "Gradle cache"),
        "DerivedData" => t!("Xcode derleme verisi", "Xcode build data"),
        ".next" | ".nuxt" | ".turbo" | ".parcel-cache" | ".angular" => {
            t!("web derleme önbelleği", "web build cache")
        }
        ".dart_tool" => t!("Dart/Flutter araçları", "Dart/Flutter tooling"),
        "target" if has("Cargo.toml") => t!("Rust derleme çıktısı", "Rust build output"),
        "build"
            if has("build.gradle")
                || has("build.gradle.kts")
                || has("CMakeLists.txt")
                || has("package.json")
                || has("setup.py") =>
        {
            t!("derleme çıktısı", "build output")
        }
        "dist" if has("package.json") || has("setup.py") => t!("paket çıktısı", "package output"),
        "Pods" if has("Podfile") => t!("CocoaPods bağımlılıkları", "CocoaPods dependencies"),
        ".build" if has("Package.swift") => t!("Swift derleme çıktısı", "Swift build output"),
        ".venv" | "venv" if tree.children(id).any(|c| tree.name(c) == "pyvenv.cfg") => {
            t!("Python sanal ortamı", "Python virtual environment")
        }
        "vendor" if has("composer.json") => t!("PHP bağımlılıkları", "PHP dependencies"),
        "bin" | "obj" if sibling(&|n| n.ends_with(".csproj")) => {
            t!(".NET derleme çıktısı", ".NET build output")
        }
        _ => return None,
    };
    Some(kind)
}

/// When the project around a junk folder was last touched: the newest
/// change among its other entries. Reinstalling dependencies refreshes
/// `node_modules` but says nothing about whether the project is in use.
fn project_modified(tree: &Tree, junk: NodeId) -> u32 {
    tree.parent(junk).map_or(0, |p| {
        tree.children(p)
            .filter(|&c| c != junk)
            .map(|c| tree.node(c).modified)
            .max()
            .unwrap_or(0)
    })
}

pub(super) fn dev_junk(tree: &Tree, base: NodeId, mode: SizeMode, age: AgeFilter) -> Report {
    let mut found = Vec::new();
    walk(tree, base, |id| {
        if !tree.node(id).is_dir {
            return false;
        }
        match dev_junk_kind(tree, id) {
            Some(kind) => {
                let project = project_modified(tree, id);
                if age.ok(project) {
                    let detail = if project == 0 {
                        kind.to_string()
                    } else {
                        tf!(
                            "{kind} · proje: {}",
                            "{kind} · project: {}",
                            crate::ui::fmt_date(project)
                        )
                    };
                    found.push((id, detail));
                }
                false
            }
            None => true,
        }
    });
    let (items, truncated) = by_size(tree, mode, found);
    let rows = singles(tree, base, mode, items);
    (
        rows,
        truncated,
        t!(
            "Bu klasörler projeyi yeniden derleyince / kurunca geri gelir.",
            "These folders come back when the project is built / installed again."
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::reports::test_util::{labels, report, sz};
    use crate::reports::{run, ReportKind};
    use crate::stats::DAY;
    use crate::tree::{SizeMode, Tree, ROOT};

    #[test]
    fn dev_junk_needs_marker_files() {
        let mut t = Tree::new(Path::new("/r"));
        let rust = t.push(ROOT, "rust", true, sz(0));
        t.push(rust, "Cargo.toml", false, sz(1));
        let target = t.push(rust, "target", true, sz(0));
        t.push(target, "debug.bin", false, sz(500));
        // A "target" folder without Cargo.toml is not build output.
        let other = t.push(ROOT, "photos", true, sz(0));
        let not_junk = t.push(other, "target", true, sz(0));
        t.push(not_junk, "x.jpg", false, sz(900));
        let web = t.push(ROOT, "web", true, sz(0));
        let nm = t.push(web, "node_modules", true, sz(0));
        let nested = t.push(nm, "node_modules", true, sz(0));
        t.push(nested, "lib.js", false, sz(200));
        let py = t.push(ROOT, "py", true, sz(0));
        let venv = t.push(py, ".venv", true, sz(0));
        t.push(venv, "pyvenv.cfg", false, sz(1));
        let dotnet = t.push(ROOT, "app", true, sz(0));
        t.push(dotnet, "App.csproj", false, sz(1));
        let bin = t.push(dotnet, "bin", true, sz(0));
        t.push(bin, "app.dll", false, sz(50));
        t.finalize();

        let l = report(&t, ReportKind::DevJunk);
        assert_eq!(
            labels(&l),
            vec!["rust/target/", "web/node_modules/", "app/bin/", "py/.venv/"]
        );
        assert!(l.rows[0].detail.starts_with("Rust derleme çıktısı"));
    }

    #[test]
    fn age_filter_uses_project_age_for_dev_junk() {
        let now = 1000 * DAY;
        let ago = |d: u64| (now - d * DAY) as u32;
        let mut t = Tree::new(Path::new("/r"));
        // Old project whose node_modules was reinstalled yesterday.
        let old = t.push(ROOT, "eski", true, sz(0));
        let pkg = t.push(old, "package.json", false, sz(1));
        t.set_times(pkg, ago(400), 0);
        let nm = t.push(old, "node_modules", true, sz(0));
        let lib = t.push(nm, "lib.js", false, sz(500));
        t.set_times(lib, ago(1), 0);
        // Project in active use.
        let new = t.push(ROOT, "yeni", true, sz(0));
        let pkg = t.push(new, "package.json", false, sz(1));
        t.set_times(pkg, ago(3), 0);
        let nm = t.push(new, "node_modules", true, sz(0));
        let lib = t.push(nm, "lib.js", false, sz(900));
        t.set_times(lib, ago(3), 0);
        t.finalize();

        let all = run(&t, ROOT, SizeMode::Disk, now, ReportKind::DevJunk, 0);
        assert_eq!(
            labels(&all),
            vec!["yeni/node_modules/", "eski/node_modules/"]
        );
        let stale = run(&t, ROOT, SizeMode::Disk, now, ReportKind::DevJunk, 90);
        assert_eq!(labels(&stale), vec!["eski/node_modules/"]);
        assert!(stale.title.contains("≥ 90 gün"));
        assert_eq!(stale.min_age_days(), 90);

        // Files: the date of the file itself.
        let files = run(&t, ROOT, SizeMode::Disk, now, ReportKind::LargestFiles, 180);
        assert_eq!(labels(&files), vec!["eski/package.json"]);
        // Not supported: the filter is ignored.
        let old_big = run(&t, ROOT, SizeMode::Disk, now, ReportKind::OldBig, 90);
        assert_eq!(old_big.min_age_days(), 0);
    }
}
