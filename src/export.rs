//! Lists as CSV or JSON: the `o` key writes the list on screen to a file,
//! and `rustclean report … --csv / --json` prints a report. Both use the
//! records and writers here, so the columns are the same everywhere.
//!
//! One record is one entry: a file or a folder with its absolute path. A
//! group row (same name, same content, an app and its data) becomes one
//! record per member, with the group's label in `group`.

use std::io::{self, Write};
use std::path::PathBuf;

use crate::lists::{ResultList, Source};
use crate::newfile;
use crate::tree::{NodeId, Tree};

/// The columns, in order. The JSON keys use the same names.
pub const COLUMNS: [&str; 8] = [
    "path",
    "apparent_size",
    "disk_size",
    "files",
    "modified",
    "created",
    "group",
    "detail",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Csv,
    Json,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Json => "json",
        }
    }
}

/// One exported entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub path: PathBuf,
    /// Bytes.
    pub apparent: u64,
    /// Bytes allocated on disk.
    pub disk: u64,
    /// 1 for a file, the number of files below for a folder.
    pub files: u32,
    /// Seconds since the Unix epoch, 0 when unknown.
    pub modified: u32,
    pub created: u32,
    /// Label of the group row the entry belongs to; empty for single rows.
    pub group: String,
    /// The row's detail text (reports); may be empty.
    pub detail: String,
}

impl Record {
    pub fn of(tree: &Tree, id: NodeId, group: &str, detail: &str) -> Self {
        let n = tree.node(id);
        Self {
            path: tree.path_of(id),
            apparent: n.size.apparent,
            disk: n.size.disk,
            files: n.file_count,
            modified: n.modified,
            created: n.created,
            group: group.to_string(),
            detail: detail.to_string(),
        }
    }
}

/// What the file says about the list besides its entries (JSON only).
#[derive(Debug, Clone)]
pub struct Meta {
    /// The list's title, as shown on screen.
    pub title: String,
    /// The folder the list was made for.
    pub root: PathBuf,
    /// More results existed than were exported.
    pub truncated: bool,
}

/// Records for some entries, e.g. the folder list.
pub fn entries(tree: &Tree, ids: &[NodeId]) -> Vec<Record> {
    ids.iter().map(|&id| Record::of(tree, id, "", "")).collect()
}

/// Records for every row of a result list; group rows give one record per
/// member. In an opened group, every entry gets the group's label and
/// detail.
pub fn list_records(tree: &Tree, list: &ResultList) -> Vec<Record> {
    let opened = match (&list.source, &list.parent) {
        (Source::Members, Some(parent)) => parent.selected_row().filter(|r| r.group),
        _ => None,
    };
    let mut out = Vec::new();
    for row in &list.rows {
        let (group, detail) = match opened {
            _ if row.group => (row.label.as_str(), row.detail.as_str()),
            Some(g) if row.detail.is_empty() => (g.label.as_str(), g.detail.as_str()),
            Some(g) => (g.label.as_str(), row.detail.as_str()),
            None => ("", row.detail.as_str()),
        };
        for &id in &row.nodes {
            out.push(Record::of(tree, id, group, detail));
        }
    }
    out
}

/// `2026-10-01T12:00:00Z`; `None` for an unknown time (0).
pub fn iso(secs: u32) -> Option<String> {
    if secs == 0 {
        return None;
    }
    chrono::DateTime::from_timestamp(i64::from(secs), 0)
        .map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

/// A CSV field, quoted when it holds a comma, a quote or a line break
/// (RFC 4180): `a"b` → `"a""b"`.
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn write_csv(w: &mut impl Write, records: &[Record]) -> io::Result<()> {
    writeln!(w, "{}", COLUMNS.join(","))?;
    for r in records {
        let fields = [
            r.path.to_string_lossy().into_owned(),
            r.apparent.to_string(),
            r.disk.to_string(),
            r.files.to_string(),
            iso(r.modified).unwrap_or_default(),
            iso(r.created).unwrap_or_default(),
            r.group.clone(),
            r.detail.clone(),
        ];
        let line: Vec<String> = fields.iter().map(|f| csv_field(f)).collect();
        writeln!(w, "{}", line.join(","))?;
    }
    Ok(())
}

/// JSON with the keys in column order, one entry per line:
/// `{"title": …, "root": …, "truncated": …, "entries": [{…}, …]}`. Unknown
/// dates and empty texts are `null`.
pub fn write_json(w: &mut impl Write, meta: &Meta, records: &[Record]) -> io::Result<()> {
    let js = |s: &str| serde_json::Value::from(s).to_string();
    let opt = |s: Option<String>| s.map_or_else(|| "null".to_string(), |s| js(&s));
    let text = |s: &str| opt((!s.is_empty()).then(|| s.to_string()));
    writeln!(w, "{{")?;
    writeln!(w, "  \"title\": {},", js(&meta.title))?;
    writeln!(w, "  \"root\": {},", js(&meta.root.to_string_lossy()))?;
    writeln!(w, "  \"truncated\": {},", meta.truncated)?;
    write!(w, "  \"entries\": [")?;
    for (i, r) in records.iter().enumerate() {
        let values = [
            js(&r.path.to_string_lossy()),
            r.apparent.to_string(),
            r.disk.to_string(),
            r.files.to_string(),
            opt(iso(r.modified)),
            opt(iso(r.created)),
            text(&r.group),
            text(&r.detail),
        ];
        let fields: Vec<String> = COLUMNS
            .iter()
            .zip(values)
            .map(|(k, v)| format!("\"{k}\": {v}"))
            .collect();
        let sep = if i == 0 { "" } else { "," };
        write!(w, "{sep}\n    {{{}}}", fields.join(", "))?;
    }
    if !records.is_empty() {
        write!(w, "\n  ")?;
    }
    writeln!(w, "]")?;
    writeln!(w, "}}")
}

pub fn write(
    w: &mut impl Write,
    format: Format,
    meta: &Meta,
    records: &[Record],
) -> io::Result<()> {
    match format {
        Format::Csv => write_csv(w, records),
        Format::Json => write_json(w, meta, records),
    }
}

/// `rustclean-<what>-20261001-120000`, with `what` reduced to lowercase
/// letters, digits and dashes.
fn base_name(what: &str, stamp: &str) -> String {
    let mut slug = String::new();
    for c in what.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "list" } else { slug };
    format!("rustclean-{slug}-{stamp}")
}

/// Writes the list to a new file in the first of `dirs` that takes it and
/// returns its path. An existing file is never replaced (see `newfile`).
pub fn save(
    dirs: &[PathBuf],
    what: &str,
    format: Format,
    meta: &Meta,
    records: &[Record],
) -> io::Result<PathBuf> {
    let base = base_name(what, &newfile::stamp());
    newfile::create(dirs, &base, format.extension(), |mut w| {
        write(&mut w, format, meta, records)
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::lists::{Row, RowSize};
    use crate::tree::{Size, ROOT};

    fn tree() -> (Tree, [NodeId; 3]) {
        let mut t = Tree::new(Path::new("/r"));
        let dir = t.push(ROOT, "a, \"b\"\nc", true, Size::default());
        let f = t.push(
            dir,
            "f.txt",
            false,
            Size {
                apparent: 10,
                disk: 4096,
            },
        );
        t.set_times(f, 1_790_856_000, 0);
        let g = t.push(
            ROOT,
            "g.bin",
            false,
            Size {
                apparent: 7,
                disk: 8,
            },
        );
        t.finalize();
        (t, [dir, f, g])
    }

    /// Splits CSV text into records of fields, honoring quotes.
    fn parse_csv(text: &str) -> Vec<Vec<String>> {
        let (mut rows, mut row, mut field) = (Vec::new(), Vec::new(), String::new());
        let mut quoted = false;
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '"' if quoted && chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = !quoted,
                ',' if !quoted => row.push(std::mem::take(&mut field)),
                '\n' if !quoted => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                c => field.push(c),
            }
        }
        rows
    }

    #[test]
    fn csv_quotes_commas_quotes_and_newlines() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("two\nlines"), "\"two\nlines\"");

        let (t, [dir, f, _]) = tree();
        let mut out = Vec::new();
        write_csv(&mut out, &entries(&t, &[dir, f])).unwrap();
        let rows = parse_csv(&String::from_utf8(out).unwrap());
        assert_eq!(rows[0], COLUMNS);
        assert_eq!(rows.len(), 3);
        let dir_path = t.path_of(dir).to_string_lossy().into_owned();
        assert_eq!(rows[1][0], dir_path);
        assert!(rows[1][0].ends_with("a, \"b\"\nc"));
        assert_eq!(
            rows[2][1..6],
            ["10", "4096", "1", "2026-10-01T12:00:00Z", ""]
        );
    }

    #[test]
    fn groups_export_their_members() {
        let (t, [_, f, g]) = tree();
        let rows = vec![
            Row::group(
                "same".into(),
                "2 copies".into(),
                vec![(f, 10), (g, 7)],
                RowSize::Wasted,
                2,
            ),
            Row::single(&t, ROOT, g, crate::tree::SizeMode::Disk, "big".into()),
        ];
        let list = ResultList::new("t".into(), ROOT, rows);
        let recs = list_records(&t, &list);
        let summary: Vec<(&str, &str)> = recs
            .iter()
            .map(|r| (r.group.as_str(), r.detail.as_str()))
            .collect();
        assert_eq!(
            summary,
            [("same", "2 copies"), ("same", "2 copies"), ("", "big")]
        );

        // The opened group: its members keep the group's label and detail.
        let mut opened = ResultList::new("t".into(), ROOT, list.rows.clone());
        let mode = crate::tree::SizeMode::Disk;
        let members = [f, g].map(|id| Row::single(&t, ROOT, id, mode, String::new()));
        opened.drill_into(ResultList::new("m".into(), ROOT, members.to_vec()));
        let recs_opened = list_records(&t, &opened);
        assert!(recs_opened
            .iter()
            .all(|r| r.group == "same" && r.detail == "2 copies"));

        let mut out = Vec::new();
        let meta = Meta {
            title: "t".into(),
            root: "/r".into(),
            truncated: false,
        };
        write_json(&mut out, &meta, &recs).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["entries"].as_array().unwrap().len(), 3);
        assert_eq!(v["entries"][0]["group"], "same");
        assert_eq!(v["entries"][0]["modified"], "2026-10-01T12:00:00Z");
        assert!(v["entries"][0]["created"].is_null());
        assert!(v["entries"][2]["group"].is_null());
        assert_eq!(v["entries"][2]["disk_size"], 8);
    }

    #[test]
    fn saves_under_the_list_name() {
        let dir = tempfile::tempdir().unwrap();
        let (t, ids) = tree();
        let meta = Meta {
            title: "t".into(),
            root: "/r".into(),
            truncated: false,
        };
        let dirs = [dir.path().to_path_buf()];
        let json = save(&dirs, "Dev junk!", Format::Json, &meta, &entries(&t, &ids)).unwrap();
        let name = json.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("rustclean-dev-junk-"), "{name}");
        assert!(name.ends_with(".json"), "{name}");
        let text = fs::read_to_string(&json).unwrap();
        assert!(text.starts_with("{\n  \"title\": \"t\""), "{text}");
    }
}
