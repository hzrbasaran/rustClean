# Architecture

rustClean is a single binary crate. Everything lives in `src/`; each module
has a doc comment at the top explaining its job.

```text
main.rs ─ CLI (clap), terminal setup, event loop (50 ms tick)
   │
   ├─ app.rs ───── state machine: disk list → scanning → browser
   │    │           Browser: current folder, result lists, dialogs, keys
   │    ├─ toolsview.rs   developer tools screen state and keys
   │    └─ basket.rs      entries collected for deletion
   │
   ├─ ui/ ──────── all drawing (ratatui); no state of its own
   │    ├─ mod.rs        picks the screen; shared title, key, popup, panel helpers
   │    ├─ format.rs     sizes, counts, dates, ages, paths
   │    ├─ style.rs      shared colors and styles
   │    └─ one module per screen: disks, scanning, browser, map, results,
   │       dashboard, tools, system, menus, dialogs
   │
   ├─ scanner.rs ─ parallel scan → tree.rs
   ├─ tree.rs ──── compact arena tree of the scan
   ├─ clones.rs ── APFS clone ids and private sizes (getattrlistat)
   │
   ├─ reports.rs ─ tree-based reports, age filter, menu items
   ├─ apps.rs ──── apps and their data, orphaned leftovers
   ├─ duplicates.rs  identical-content search (background thread)
   ├─ stats.rs ─── summary statistics, age groups, file categories
   ├─ search.rs ── name patterns
   ├─ lists.rs ─── generic result list (rows, groups, drill-down)
   ├─ treemap.rs ─ squarified layout and block navigation
   │
   ├─ history.rs ─ scan snapshots and "what changed"
   ├─ tools.rs ─── developer tool measurement and cleanup commands
   ├─ system.rs ── macOS system data (diskutil, tmutil, sysctl)
   ├─ delete.rs ── safety checks and moving to the trash
   ├─ disks.rs ─── disk discovery (sysinfo)
   └─ i18n.rs ──── Turkish / English texts
```

## Scanning

`scanner::scan` reads directories on a rayon thread pool. A worker reads one
directory, stats its entries and sends the listing to the thread that owns the
tree. That thread inserts the entries and queues a job for each subdirectory.
There is no ordering requirement, so workers never wait for each other. This
replaced `jwalk`, whose ordered output kept most threads idle, and made scans
about 6× faster.

Rules applied while scanning:
- Symlinks are not followed.
- Other filesystems and other volumes' mount points are not entered. The latter
  matters on macOS, where `/` and `/System/Volumes/Data` share a device id.
- Hard-linked files count once, by `(dev, inode)`.
- Both the apparent size and the allocated size are recorded.
- APFS pure clones count once on disk (macOS). For files of at least 64 KiB,
  `clones::Dir` asks `getattrlistat` for the clone id and share count, and
  only for shared files for the private size, which is slow: the kernel walks
  the file's extents. The first file of a clone id keeps its allocated size;
  later ones count only their private bytes. The tree remembers clone ids so
  the duplicates report can mark them. Asking every file made scans about 5×
  slower; with the threshold the cost is not measurable.

## The tree

`tree::Tree` is an arena of `Node`s (≤ 56 bytes each):
- `u32` ids
- children as a sibling-linked list
- all names in one shared `String`
- times as `u32` epoch seconds

A child always has a larger id than its parent, so `finalize` aggregates sizes,
file counts and the newest modification time in one reverse pass. A full disk
with 12 million entries takes about 800 MiB.

`Tree::remove` detaches a node and subtracts it from its ancestors, so deletions
update the view without rescanning.

## Result lists

Search results and every report are a `lists::ResultList` of `Row`s. A row is
either a single entry or a group, which can be drilled into with `Enter`. Rows
store their members' sizes so they can be recomputed after deletions or a
size-mode switch.

Whether a row shows `[✓]` is not stored in the row. The UI derives it from the
basket (`Browser::row_in_basket`), which is the single source of truth for what
is selected.

## Background work

Long operations run on their own threads and report through channels. The UI
polls them on every tick:

| Work | Where |
|---|---|
| scanning | `scanner::start` → `ScanHandle` |
| moving to the trash | `delete::Deletion` |
| duplicate search | `duplicates::DupJob` (progress in atomics) |
| tool measurement and commands | `tools::measure_all`, `tools::run` |
| saving history | a detached thread in `Browser::record_history` |

Reports that only read the tree run synchronously. They take under a second
even on a full disk. A "preparing" message is drawn first so the UI never looks
frozen.

## Interface texts

Every text shown to the user is written in both languages where it is used:
- `t!("Türkçe", "English")` returns a `&'static str`.
- `tf!("{n} öğe", "{n} items")` formats.

The language is a global set by `i18n::init` and `L`. Formatting helpers in
`ui/format.rs` (`fmt_count`, `fmt_date`, `fmt_pct`, `fmt_ago`) follow it.

## Tests

Unit tests sit next to the code (`#[cfg(test)]`):
- Most build a synthetic `Tree` by hand.
- Scanner, duplicate and history tests use temporary directories.
- Parsers are tested against real command output in `tests/fixtures/`.

Tests run in the default language (Turkish) and must pass on macOS, Linux and
Windows. Compare paths with `/` normalized, and gate Unix-only tests with
`#[cfg(unix)]`.
