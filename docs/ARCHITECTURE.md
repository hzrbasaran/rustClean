# Architecture

rustClean is a single binary crate. Everything lives in `src/`; each module
has a doc comment at the top explaining its job.

```text
main.rs ─ CLI (clap), terminal setup, event loop (50 ms tick)
   │
   ├─ app/ ─────── state machine: disk list → scanning → browser
   │    │           Browser: current folder, result lists, dialogs, keys
   │    ├─ mod.rs        App, shared enums, and the Browser struct (kept here
   │    │                so the submodules can use its private fields)
   │    ├─ browser.rs    navigation, treemap moves, the key dispatcher
   │    ├─ one module per job, each adding an `impl Browser` block:
   │    │  results, dashboard, basket, trash, uninstall, snapshots, rescan
   │    ├─ toolsview.rs   developer tools screen state and keys
   │    └─ basket.rs      entries collected for deletion
   │
   ├─ ui/ ──────── all drawing (ratatui); no state of its own
   │    ├─ mod.rs        picks the screen; shared title, key, popup, panel helpers
   │    ├─ format.rs     sizes, counts, dates, ages, paths, wrapping
   │    ├─ theme.rs      the four themes (dark, light, color-blind, no color)
   │    ├─ style.rs      colors by role (`.muted()`, `.warn()`…) from the theme
   │    ├─ help.rs       the `?` screen: every key, by screen
   │    └─ one module per screen: disks, scanning, browser, map, results,
   │       dashboard, tools, system, menus, dialogs, log (deletion log)
   │
   ├─ scanner.rs ─ parallel scan → tree.rs
   ├─ tree.rs ──── compact arena tree of the scan
   ├─ clones.rs ── APFS clone ids and private sizes (getattrlistat)
   │
   ├─ reports/ ─── tree-based reports
   │    ├─ mod.rs        report kinds, menu items, age filter, run(), shared
   │    │                walk / top-N helpers
   │    └─ one module per report group: size, downloads, dev_junk, caches,
   │       names, clutter (empty folders, broken links, temporary files)
   ├─ apps/ ────── apps and their data, orphaned leftovers
   │    ├─ mod.rs        App, DataDir, the Apps report, uninstall checks
   │    ├─ find.rs       installed apps (in the scan and on disk)
   │    ├─ data.rs       data folders and matching them to apps
   │    └─ orphans.rs    leftovers of removed apps (errs on keeping data)
   ├─ duplicates.rs  identical-content search (background thread)
   ├─ stats.rs ─── summary statistics, age groups, file categories
   ├─ search.rs ── name patterns
   ├─ lists.rs ─── generic result list (rows, groups, drill-down)
   ├─ treemap.rs ─ squarified layout and block navigation
   │
   ├─ history.rs ─ scan snapshots and "what changed"
   ├─ tools.rs ─── developer tool measurement and cleanup commands
   │    ├─ tools_linux.rs    apt/dnf/pacman caches, journal, snaps (#18)
   │    └─ tools_windows.rs  %TEMP%, Windows temp, update cache, Recycle Bin (#19)
   ├─ system.rs ── macOS system data (diskutil, tmutil, sysctl)
   ├─ delete.rs ── safety checks and moving to the trash
   ├─ trashlog.rs  the deletion log (deletions.jsonl)
   ├─ disks.rs ─── disk discovery (sysinfo)
   ├─ paths.rs ─── data directory (history, settings, deletion log)
   ├─ settings.rs  saved choices (language, theme) as key=value lines
   └─ i18n.rs ──── Turkish / English texts, counts with their nouns
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

A list records its `Source`: a report and its age filter, a search pattern, a
saved scan, the duplicates (rebuilt from their own rows, without hashing
again), the basket, or the members of a group. `Browser::rebuild_results`
builds the open list again from it and puts the cursor back on the same entry,
re-opening groups level by level. It runs after `L` (so titles and details
follow the language), after the age filter changes, and after the basket or
the apps change.

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

Every successful move to the trash is appended to the deletion log
(`trashlog::append`): by `Browser::poll_delete` for deletions from the
interface, and by `tools::run` for a tool's "move the contents to the trash"
step. A failed move is never written.

Reports that only read the tree run synchronously. They take under a second
even on a full disk. A "preparing" message is drawn first so the UI never looks
frozen.

## Interface texts

Every text shown to the user is written in both languages where it is used:
- `t!("Türkçe", "English")` returns a `&'static str`.
- `tf!("{n} öğe", "{n} items")` formats.

The language is a global set by `i18n::init` and `L`. Formatting helpers in
`ui/format.rs` (`fmt_count`, `fmt_date`, `fmt_pct`, `fmt_ago`) follow it. For
a number with a noun use `i18n::count(n, "öğe", "item", "items")`: English
takes the singular for one, Turkish nouns do not change after a number. Files
store language-independent codes (`ReportKind::code`, `ToolKind::code`), not
labels.

## Colors

Screens never name colors. They ask for a role from `ui/style.rs`:
`.normal()`, `.muted()`, `.accent()`, `.success()`, `.warn()`, `.danger()`,
`.key()`, `.badge()`, and the age, usage, risk, category and treemap palettes.
`ui/theme.rs` answers from the current theme:
- **dark**: the 16 named colors, which follow the terminal's palette;
- **light** and **color-blind**: 256-color entries, which look the same
  everywhere;
- **no color** (`--no-color`, `NO_COLOR`): reversed and bold text instead,
  and borders around treemap blocks.

`T` cycles the themes and `--theme` picks one at start; `settings.rs`
remembers the choice. `--no-color` or `NO_COLOR` wins over both.

## Tests

Unit tests sit next to the code (`#[cfg(test)]`):
- Most build a synthetic `Tree` by hand.
- Scanner, duplicate and history tests use temporary directories.
- Parsers are tested against real command output in `tests/fixtures/`.

Integration tests live in `src/integration/` (test builds only, so they can
reach the internals without a library target). `Fixture` builds a real folder
tree in a temp directory with distinct file sizes and set ages. The tests scan
it with the real scanner and drive `App` by key presses, as the interface
does: reports, duplicates, the basket, moving to the trash, uninstalling, and
comparing with a saved scan. `tests/cli.rs` runs the binary itself
(`--summary`, `--help`, errors).

`src/integration/screens.rs` stores a snapshot of every screen (text plus the
styles of each line) in Turkish and English. It uses a hand-built tree, fake
disks, tools and system data, and two test-only, per-thread switches:
`i18n::with_lang` and `ui::with_fixed_now`. The latter also shows dates in
UTC, so snapshots match on every machine.

Test builds never touch the user's data. `paths::data_dir` points to a
folder under the temp directory, and `Deletion` renames entries into
`delete::test_trash` instead of calling the real trash.

Tests run in the default language (Turkish) and must pass on macOS, Linux and
Windows. Compare paths with `/` normalized, and gate Unix-only tests with
`#[cfg(unix)]`.
