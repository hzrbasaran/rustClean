//! The treemap as a self-contained HTML page (`w`): the current folder a few
//! levels deep, inlined as JSON next to a small script that lays it out and
//! zooms. The page makes no requests (a Content Security Policy forbids
//! them), so it works offline and can be shared as a single file.
//!
//! The layout runs in the page, not here: it has to be redone on every zoom
//! and window size, and the browser knows the pixel size. The script ports
//! the squarified layout of `treemap.rs`. This module only picks the data,
//! within limits that keep the file small on a full disk.

use std::io;
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::newfile;
use crate::stats::{self, Category};
use crate::tree::{NodeId, SizeMode, Tree};

/// The page; `{{KEY}}` markers are filled by `render`.
const TEMPLATE: &str = include_str!("page.html");

/// How much of the tree goes into the page.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Levels below the exported folder.
    pub depth: usize,
    /// Blocks in the whole page, the exported folder included.
    pub nodes: usize,
    /// Blocks per folder; the rest share one "other" block.
    pub children: usize,
    /// Entries smaller than this share of their folder go to "other".
    pub min_share: f64,
}

pub const LIMITS: Limits = Limits {
    depth: 4,
    nodes: 3000,
    children: 60,
    min_share: 0.002,
};

/// What a block stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File(Category),
    /// Small entries of a folder merged into one block.
    Other {
        count: usize,
    },
}

/// One block of the page.
#[derive(Debug)]
pub struct Block {
    pub name: String,
    pub size: u64,
    pub kind: Kind,
    /// Age group of `stats::age_label` (`stats::AGE_UNKNOWN` without a date).
    pub age: usize,
    /// Largest first; an "other" block comes last.
    pub children: Vec<Block>,
    /// A folder whose contents are left out (depth or block limit).
    pub cut: bool,
}

/// A block before the tree is assembled: children by index.
struct Flat {
    block: Block,
    children: Vec<usize>,
}

/// The blocks of `dir` within `limits`. Folders are filled breadth first,
/// largest first, so when the block budget runs out it is the deepest and
/// smallest folders that are cut.
pub fn extract(tree: &Tree, dir: NodeId, mode: SizeMode, now: u64, limits: &Limits) -> Block {
    let block = |id: NodeId, name: String| {
        let n = tree.node(id);
        Block {
            name,
            size: n.size.get(mode),
            kind: if n.is_dir {
                Kind::Dir
            } else {
                Kind::File(Category::of(tree.name(id)))
            },
            age: if n.modified == 0 {
                stats::AGE_UNKNOWN
            } else {
                stats::age_group(now.saturating_sub(u64::from(n.modified)))
            },
            children: Vec::new(),
            cut: false,
        }
    };
    let root_name = tree.path_of(dir).file_name().map_or_else(
        || tree.path_of(dir).display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let mut flat = vec![Flat {
        block: block(dir, root_name),
        children: Vec::new(),
    }];
    // (tree node, index in `flat`, depth)
    let mut queue = std::collections::VecDeque::from([(dir, 0usize, 0usize)]);
    while let Some((id, at, depth)) = queue.pop_front() {
        let mut kids: Vec<(u64, NodeId)> = tree
            .children(id)
            .map(|c| (tree.node(c).size.get(mode), c))
            .filter(|&(s, _)| s > 0)
            .collect();
        if kids.is_empty() {
            continue;
        }
        let room = limits.nodes.saturating_sub(flat.len());
        if depth >= limits.depth || room == 0 {
            flat[at].block.cut = true;
            continue;
        }
        kids.sort_unstable_by_key(|&(s, _)| std::cmp::Reverse(s));
        let total = flat[at].block.size.max(1) as f64;
        let mut keep = kids
            .iter()
            .take(limits.children)
            .take_while(|&&(s, _)| s as f64 / total >= limits.min_share)
            .count()
            .min(room);
        // Leftovers need a block of their own.
        if keep < kids.len() && keep == room {
            keep -= 1;
        }
        for &(_, c) in &kids[..keep] {
            let i = flat.len();
            flat.push(Flat {
                block: block(c, tree.name(c).to_string()),
                children: Vec::new(),
            });
            flat[at].children.push(i);
            if tree.node(c).is_dir {
                queue.push_back((c, i, depth + 1));
            }
        }
        let rest = &kids[keep..];
        if !rest.is_empty() {
            let count = rest.len();
            let i = flat.len();
            flat.push(Flat {
                block: Block {
                    name: tf!(
                        "diğer ({})",
                        "other ({})",
                        crate::i18n::count(count as u64, "küçük öğe", "small item", "small items")
                    ),
                    size: rest.iter().map(|&(s, _)| s).sum(),
                    kind: Kind::Other { count },
                    age: stats::AGE_UNKNOWN,
                    children: Vec::new(),
                    cut: false,
                },
                children: Vec::new(),
            });
            flat[at].children.push(i);
        }
    }
    // Children come after their parent, so assemble from the end.
    let mut done: Vec<Option<Block>> = Vec::with_capacity(flat.len());
    let mut kids: Vec<Vec<usize>> = Vec::with_capacity(flat.len());
    for f in flat {
        done.push(Some(f.block));
        kids.push(f.children);
    }
    for i in (0..done.len()).rev() {
        let children: Vec<Block> = kids[i]
            .iter()
            .map(|&c| done[c].take().expect("each block has one parent"))
            .collect();
        if let Some(b) = &mut done[i] {
            b.children = children;
        }
    }
    done[0].take().expect("the root block")
}

impl Block {
    /// Blocks in this subtree, this one included.
    #[cfg(test)]
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(Block::count).sum::<usize>()
    }

    /// Short keys keep the file small: n name, s size, t type (d folder,
    /// f file, o other), c category, a age group, k children, x cut, m count
    /// of an "other" block.
    fn to_json(&self) -> Value {
        let mut v = json!({ "n": self.name, "s": self.size, "a": self.age });
        match self.kind {
            Kind::Dir => v["t"] = "d".into(),
            Kind::File(c) => {
                v["t"] = "f".into();
                v["c"] = (c as usize).into();
            }
            Kind::Other { count } => {
                v["t"] = "o".into();
                v["m"] = count.into();
            }
        }
        if !self.children.is_empty() {
            v["k"] = self.children.iter().map(Block::to_json).collect();
        }
        if self.cut {
            v["x"] = 1.into();
        }
        v
    }
}

/// What the page says besides the blocks.
pub struct PageInfo<'a> {
    /// The exported folder, for the heading.
    pub path: &'a str,
    pub mode: SizeMode,
    /// When the page was written, already formatted.
    pub date: &'a str,
    pub limits: Limits,
}

/// The whole page.
pub fn render(root: &Block, info: &PageInfo<'_>) -> String {
    let lim = info.limits;
    let texts = json!({
        "title": t!("Harita", "Treemap"),
        "up": t!("↑ Üst klasör", "↑ Up"),
        "color": t!("Renk:", "Color:"),
        "byType": t!("Türe göre", "By type"),
        "byAge": t!("Yaşa göre", "By age"),
        "folders": t!("Klasörler (her biri ayrı renk)", "Folders (each its own color)"),
        "folder": t!("klasör", "folder"),
        "other": t!("küçük öğeler birlikte", "small entries together"),
        "here": t!("kendi klasöründe {p}", "{p} of its folder"),
        "whole": t!("tümünde {p}", "{p} of the whole page"),
        "zoom": t!("Tıklayın: içine girer", "Click to zoom in"),
        "cut": t!(
            "Daha derin seviyeler bu sayfada yok",
            "Deeper levels are not in this page"
        ),
        "empty": t!(
            "Gösterilecek bir şey yok: buradaki her şey 0 bayt.",
            "Nothing to show: everything here is 0 bytes."
        ),
        "hint": t!(
            "Bir klasöre tıklayın: içine girer. Geri: üstteki yol, ↑ düğmesi, Esc ya da ⌫.",
            "Click a folder to zoom in. Back: the path above, the ↑ button, Esc or ⌫."
        ),
        "size": match info.mode {
            SizeMode::Disk => t!("Boyut: diskte kapladığı", "Size: on disk"),
            SizeMode::Apparent => t!("Boyut: görünen", "Size: apparent"),
        },
        "limits": tf!(
            "Sayfa boyutu küçük kalsın diye: en fazla {} seviye ve {} blok. Bir klasörün %{}'sinden küçük öğeler ve ilk {} öğeden sonrakiler \"diğer\" bloğunda birleşir. Çizgili klasörlerin içi sayfada yok.",
            "To keep the page small: at most {} levels and {} blocks. Entries under {}% of their folder, and all after the first {}, share an \"other\" block. Striped folders have no contents in the page.",
            lim.depth,
            crate::ui::fmt_count(lim.nodes as u64),
            lim.min_share * 100.0,
            lim.children
        ),
        "offline": t!(
            "Bu sayfa kendi içinde tamdır: internete bağlanmaz, dış dosya yüklemez. Yalnızca gösterir; hiçbir şeyi değiştirmez.",
            "This page is self-contained: it makes no network requests and loads no other files. It only shows; it changes nothing."
        ),
        "made": tf!(
            "rustClean {} ile oluşturuldu: {}",
            "Made with rustClean {} on {}",
            env!("CARGO_PKG_VERSION"),
            info.date
        ),
    });
    let cats: Vec<&str> = Category::ALL.iter().map(|c| c.label()).collect();
    let ages: Vec<&str> = (0..=stats::AGE_UNKNOWN).map(stats::age_label).collect();
    let data = json!({
        "lang": crate::i18n::lang().code(),
        "path": info.path,
        "root": root.to_json(),
        "texts": texts,
        "cats": cats,
        "ages": ages,
    });
    let title = format!(
        "{} — {}",
        t!("rustClean harita", "rustClean treemap"),
        info.path
    );
    fill(TEMPLATE, |key| match key {
        "LANG" => crate::i18n::lang().code().to_string(),
        "TITLE" => escape_html(&title),
        "NOSCRIPT" => escape_html(t!(
            "Bu sayfa JavaScript ister.",
            "This page needs JavaScript."
        )),
        "DATA" => script_json(&data),
        _ => String::new(),
    })
}

/// Replaces each `{{KEY}}` of `template` in one pass, so a value can never
/// bring in a marker of its own.
fn fill(template: &str, value: impl Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start..].find("}}") else {
            break;
        };
        out.push_str(&rest[..start]);
        out.push_str(&value(&rest[start + 2..start + len]));
        rest = &rest[start + len + 2..];
    }
    out.push_str(rest);
    out
}

/// Text for HTML content and attribute values.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// JSON safe inside a `<script>` element: no `<`, `>` or `&` (so no
/// `</script>` or `<!--`), and no line separators. These only occur in
/// strings, where the `\u` escapes mean the same.
fn script_json(v: &Value) -> String {
    let mut out = String::new();
    for c in v.to_string().chars() {
        match c {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            _ => out.push(c),
        }
    }
    out
}

/// Writes `page` as `rustclean-treemap-<stamp>.html` into the first of
/// `dirs` that takes it. An existing file is never replaced (see `newfile`).
pub fn save(page: &str, stamp: &str, dirs: &[PathBuf]) -> io::Result<PathBuf> {
    newfile::create(dirs, &format!("rustclean-treemap-{stamp}"), "html", |w| {
        w.write_all(page.as_bytes())
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::tree::Size;

    const DAY: u64 = 86_400;
    const NOW: u64 = 2_000_000_000;

    fn size(n: u64) -> Size {
        Size {
            apparent: n,
            disk: n,
        }
    }

    fn file(t: &mut Tree, parent: NodeId, name: &str, n: u64, age_days: u64) -> NodeId {
        let id = t.push(parent, name, false, size(n));
        let when = (NOW - age_days * DAY) as u32;
        t.set_times(id, when, when);
        id
    }

    fn info() -> PageInfo<'static> {
        PageInfo {
            path: "~/demo",
            mode: SizeMode::Disk,
            date: "2026-10-05 12:00",
            limits: LIMITS,
        }
    }

    #[test]
    fn keeps_sizes_kinds_and_ages() {
        let mut t = Tree::new(Path::new("/r"));
        let docs = t.push(0, "Docs", true, Size::default());
        file(&mut t, docs, "a.pdf", 1000, 3);
        file(&mut t, 0, "movie.mov", 5000, 400);
        t.push(0, "empty.txt", false, Size::default());
        t.finalize();
        let b = extract(&t, 0, SizeMode::Disk, NOW, &LIMITS);
        assert_eq!(b.size, 6000);
        let names: Vec<&str> = b.children.iter().map(|c| c.name.as_str()).collect();
        // Largest first; empty entries are left out.
        assert_eq!(names, ["movie.mov", "Docs"]);
        assert_eq!(b.children[0].kind, Kind::File(Category::Video));
        assert_eq!(b.children[0].age, 3);
        assert_eq!(b.children[1].kind, Kind::Dir);
        assert_eq!(b.children[1].children[0].age, 0);
        assert!(!b.children[1].cut);
    }

    #[test]
    fn small_and_surplus_entries_merge_into_other() {
        let mut t = Tree::new(Path::new("/r"));
        file(&mut t, 0, "big", 1_000_000, 0);
        for i in 0..5 {
            file(&mut t, 0, &format!("tiny{i}"), 10, 0);
        }
        t.finalize();
        let b = extract(&t, 0, SizeMode::Disk, NOW, &LIMITS);
        assert_eq!(b.children.len(), 2);
        let other = &b.children[1];
        assert_eq!(other.kind, Kind::Other { count: 5 });
        assert_eq!(other.size, 50);
        assert_eq!(other.name, "diğer (5 küçük öğe)");

        // Equal sizes: only the first `children` get their own block.
        let mut t = Tree::new(Path::new("/r"));
        for i in 0..100 {
            file(&mut t, 0, &format!("f{i}"), 100, 0);
        }
        t.finalize();
        let b = extract(&t, 0, SizeMode::Disk, NOW, &LIMITS);
        assert_eq!(b.children.len(), LIMITS.children + 1);
        assert_eq!(
            b.children.last().unwrap().kind,
            Kind::Other {
                count: 100 - LIMITS.children
            }
        );
        let sum: u64 = b.children.iter().map(|c| c.size).sum();
        assert_eq!(sum, b.size, "nothing is lost");
    }

    #[test]
    fn stops_at_the_depth_and_block_limits() {
        // /r/d1/d2/d3/d4/d5/f
        let mut t = Tree::new(Path::new("/r"));
        let mut dir = 0;
        for i in 1..=5 {
            dir = t.push(dir, &format!("d{i}"), true, Size::default());
        }
        file(&mut t, dir, "f", 10, 0);
        t.finalize();
        let b = extract(&t, 0, SizeMode::Disk, NOW, &LIMITS);
        let mut level = &b;
        for _ in 0..4 {
            level = &level.children[0];
        }
        assert_eq!(level.name, "d4");
        assert!(level.cut && level.children.is_empty());

        // 30 folders of 30 files, but only 100 blocks.
        let mut t = Tree::new(Path::new("/r"));
        for d in 0..30 {
            let dir = t.push(0, &format!("d{d}"), true, Size::default());
            for f in 0..30 {
                file(&mut t, dir, &format!("f{f}"), 100 + f, 0);
            }
        }
        t.finalize();
        let limits = Limits {
            nodes: 100,
            ..LIMITS
        };
        let b = extract(&t, 0, SizeMode::Disk, NOW, &limits);
        assert!(b.count() <= 100, "{}", b.count());
        assert!(b.children.iter().any(|c| c.cut));
        // Every folder that has contents adds up.
        for c in b.children.iter().filter(|c| !c.children.is_empty()) {
            assert_eq!(c.children.iter().map(|k| k.size).sum::<u64>(), c.size);
        }
    }

    #[test]
    fn hostile_names_cannot_break_out() {
        let mut t = Tree::new(Path::new("/r"));
        let evil = "</script><script>alert(\"x\")</script> & <!-- 'q' \u{2028} ünïcødé 🗂";
        file(&mut t, 0, evil, 100, 0);
        t.finalize();
        let b = extract(&t, 0, SizeMode::Disk, NOW, &LIMITS);
        let page = render(
            &b,
            &PageInfo {
                path: "/x/</title><b>\"&",
                ..info()
            },
        );
        assert_eq!(page.matches("</script>").count(), 2, "only the page's own");
        assert_eq!(page.matches("<script").count(), 2);
        assert!(!page.contains("<!--"));
        assert!(!page.contains('\u{2028}'));
        assert!(page
            .contains("<title>rustClean harita — /x/&lt;/title&gt;&lt;b&gt;&quot;&amp;</title>"));
        // The name comes back intact from the JSON.
        let start = page.find("type=\"application/json\">").unwrap() + 24;
        let end = start + page[start..].find("</script>").unwrap();
        let data: Value = serde_json::from_str(&page[start..end]).unwrap();
        assert_eq!(data["root"]["k"][0]["n"], evil);
        assert_eq!(data["path"], "/x/</title><b>\"&");
    }

    #[test]
    fn page_loads_nothing_from_outside() {
        let mut t = Tree::new(Path::new("/r"));
        file(&mut t, 0, "a.txt", 100, 0);
        t.finalize();
        let page = render(&extract(&t, 0, SizeMode::Disk, NOW, &LIMITS), &info());
        for bad in ["http://", "https://", "//cdn", "src=", "@import", "url("] {
            assert!(!page.contains(bad), "{bad}");
        }
        assert!(page.contains("default-src 'none'"));
        assert!(page.starts_with("<!doctype html>"));
        assert!(page.contains("<html lang=\"tr\">"));
    }

    #[cfg(unix)] // `with_lang` is Unix-only, like the screen snapshots
    #[test]
    fn page_follows_the_language() {
        let mut t = Tree::new(Path::new("/r"));
        file(&mut t, 0, "a.txt", 100, 0);
        t.finalize();
        let en = crate::i18n::with_lang(crate::i18n::Lang::En, || {
            render(&extract(&t, 0, SizeMode::Disk, NOW, &LIMITS), &info())
        });
        assert!(en.contains("<html lang=\"en\">"));
        assert!(en.contains("Click a folder to zoom in"));
    }

    #[test]
    fn saves_as_a_treemap_page() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = [dir.path().to_path_buf()];
        let a = save("one", "20261005-120000", &dirs).unwrap();
        let b = save("two", "20261005-120000", &dirs).unwrap();
        let name = |p: &Path| p.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name(&a), "rustclean-treemap-20261005-120000.html");
        assert_eq!(name(&b), "rustclean-treemap-20261005-120000-1.html");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "one");
    }
}
