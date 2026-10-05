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
   │    ├─ mouse.rs      optional mouse (`M`): clicks, double clicks, wheel
   │    │  results, dashboard, basket, trash, uninstall, snapshots, rescan,
   │    │  export (`o`)
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
   │       names, clutter (empty folders, broken links, temporary files),
   │       backups (iPhone / iPad backups, read from their Info.plist)
   ├─ apps/ ────── apps and their data, orphaned leftovers
   │    ├─ mod.rs        App, DataDir, the Apps report, uninstall checks
   │    ├─ find.rs       installed apps (in the scan and on disk)
   │    ├─ data.rs       data folders and matching them to apps
   │    └─ orphans.rs    leftovers of removed apps (errs on keeping data)
   ├─ duplicates.rs  identical-content search (background thread)
   ├─ stats.rs ─── summary statistics, age groups, file categories
   ├─ search.rs ── name patterns
   ├─ lists.rs ─── generic result list (rows, groups, drill-down)
   ├─ export.rs ── lists as CSV / JSON records
   ├─ newfile.rs ─ saving a new file that never replaces one (`o`, `w`)
   ├─ cli.rs ───── `rustclean report`: scan, run one report, print it
   ├─ treemap.rs ─ squarified layout and block navigation
   ├─ htmlmap/ ─── the treemap as a self-contained HTML page (`w`)
   │    ├─ mod.rs        picks the blocks (depth, block and "other" limits),
   │    │                fills the page, saves it with `newfile`
   │    └─ page.html     the page: styles, JSON data, layout and zoom script
   │
   ├─ history.rs ─ scan snapshots and "what changed"
   ├─ tools.rs ─── developer tool measurement and cleanup commands
   │    ├─ tools_linux.rs    apt/dnf/pacman caches, journal, snaps (#18)
   │    └─ tools_windows.rs  %TEMP%, Windows temp, update cache, Recycle Bin (#19)
   ├─ tools/ ───── later tools: caches (Go, Maven, Bun, uv, conda, pub,
   │               Playwright), xcode (archives, old simulators), android,
   │               docker_vm (Docker.raw sizes)
   ├─ system.rs ── macOS system data (diskutil, tmutil, sysctl)
   ├─ delete.rs ── safety checks and moving to the trash
   ├─ trashlog.rs  the deletion log (deletions.jsonl)
   ├─ disks.rs ─── disk discovery (sysinfo)
   ├─ paths.rs ─── data directory (history, settings, deletion log, config)
   ├─ settings.rs  saved choices (language, theme) as key=value lines
   │               (rewritten under a lock, via a temp file and a rename)
   ├─ config.rs ── config.toml: excluded folders, report thresholds, the
   │               starting size and sort (read once at start)
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
- Folders excluded in `config.toml` are not entered either. Both go into the
  scanner's skip list, built by `config::scan_skip` for the full scan, the
  `R` rescan and `--summary`. The scan root itself is never skipped.
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

## Exporting and the report command

`export.rs` turns a list into `Record`s, one per entry: the folder list
gives its entries, a result list its rows, and a group row one record per
member with the group's label. The same records are written as CSV or JSON
by `o` in the interface (`app/export.rs` picks the list on screen and saves
it with `export::save`) and by `rustclean report` (`cli.rs`), which scans like
`--summary`, runs the report with the interface's own code (the report
methods of a `Browser` that is never drawn; duplicates with
`duplicates::find_groups` on the same thread) and prints to stdout. Column names are fixed English identifiers, so scripts do not
depend on the language.

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

Before a tool step empties or trashes a folder, `tools::run` checks it
with `tools::safe_target`: no relative paths, roots, the home folder or its
ancestors, or the home folder's standard folders (Desktop, Documents,
`Library`, `.config`…). Measuring uses the same check for folders taken
from environment variables, so a refused folder is never offered.

Every successful move to the trash is appended to the deletion log
(`trashlog::append`): by `Browser::poll_delete` for deletions from the
interface, and by `tools::run` for a tool's "move the contents to the trash"
step. A failed move is never written.

Reports that only read the tree run synchronously. They take under a second
even on a full disk. A "preparing" message is drawn first so the UI never looks
frozen.

## Mouse

Mouse support is off until `M`. `App::mouse` (`app/mouse.rs`) holds whether
it is on and the clickable areas of the last frame: while drawing, the
screens record a `(Rect, Hit)` for every visible table row (from the area
they drew into and `TableState::offset`), treemap block and menu line
(`Mouse::add_rows`, `ui::table_rows`, `ui::panel_rows`). `ui::render` clears
them first; a popup clears what it covers, and the confirmation, uninstall
and failure dialogs and the help record nothing. A click looks up the
topmost area under it (`Mouse::hit_at`), checks that the screen it belongs
to is still showing, and moves the selection; a second click on the same
`Hit` within 500 ms sends `Enter` through `on_key`. The wheel sends `↑`/`↓`
to lists and scrolls text views directly. While a question is open, text is
typed or work runs, `App::mouse_blocked` drops every mouse event, so nothing
can be confirmed by mouse. All of this works on a `TestBackend` without a
terminal, which the tests in `src/integration/mouse.rs` use.

`main.rs` turns the terminal's mouse capture on and off to follow
`app.mouse.on` after each event, and off again on quit and in the panic hook
(ratatui's own hook restores the terminal but not mouse capture). Pointer
motion events are read in a row without redrawing.

## The treemap page

`w` writes the current folder as an HTML page (`htmlmap`, called from
`app/htmlmap.rs`). Rust only picks the data: breadth first and largest first
within `htmlmap::LIMITS` (4 levels, 3,000 blocks, 60 per folder, small
entries merged into "other"), so a full disk still gives a small file. The
layout runs in the page: it is redone on every zoom and window size, and
only the browser knows the pixel size, so the script ports the squarified
layout of `treemap.rs` instead of shipping precomputed rectangles.

The data is inlined as JSON in a `<script type="application/json">`
element, with `<`, `>` and `&` escaped, so a file name cannot end the
element; the script inserts names with `textContent` only. A Content
Security Policy (`default-src 'none'`) keeps the page from loading anything.
Test builds write into the temp directory unless a test sets
`Browser::html_dir`.

Both `o` and `w` save through `newfile::create`. It opens the file with
`create_new`, so nothing is ever replaced: a taken name gets `-1`, `-2`…
The file goes to the first folder of `newfile::default_dirs` that takes it
(the working folder, else the home folder). The content is buffered and
synced to disk. A write that fails removes the half-written file and tries
the next folder.

## Configuration

`main` reads `config.toml` once (`config::load`) before anything else and
stores it with `config::init`; code asks `config::get()`. Nothing in the file
can stop the program: invalid TOML gives the defaults, a bad value gives that
key's default, and an unknown key is ignored. Each case is a
`config::Problem`, shown by `App::config_notice` on the status line or
printed to stderr by `--summary` and `--config`.

Where the values are used:

| Key | Read by | Default |
|---|---|---|
| `scan.exclude` | `config::scan_skip` (scanner skip list) | none |
| `reports.old_big_min_mib`, `old_big_min_days` | `reports::size::old_big`, its note and `ReportKind::description` | `OLD_BIG_SIZE`, `OLD_BIG_AGE` |
| `reports.duplicates_min_mib` | `duplicates::candidates`, the "no duplicates" note, `ReportKind::description` | `duplicates::MIN_SIZE` |
| `view.size`, `view.sort` | `Browser::new` | on disk, by size |

The constants stay as the defaults. Texts that name a threshold build it from
the configuration, so they always show the value in effect. Theme and
language stay in `settings` (saved by `T` / `L`); the configuration file is
only read, never written.

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
does: reports, duplicates, the basket, moving to the trash, uninstalling,
comparing with a saved scan, and exporting with `o`. `tests/cli.rs` runs the
binary itself (`--summary`, `report` in every format, `--help`, errors).
does: reports, duplicates, the basket, moving to the trash, uninstalling, and
comparing with a saved scan. `tests/cli.rs` runs the binary itself
(`--summary`, `--config`, `--help`, errors), each run with its own
`RUSTCLEAN_DATA_DIR`.

Tests that need other settings use `config::with`, another per-thread
switch; `config::load` is never called with the real data directory, so a
`config.toml` there cannot change test results.

`src/integration/screens.rs` stores a snapshot of every screen (text plus the
styles of each line) in Turkish and English. It uses a hand-built tree, fake
disks, tools and system data, and two test-only, per-thread switches:
`i18n::with_lang` and `ui::with_fixed_now`. The latter also shows dates in
UTC, so snapshots match on every machine.

Test builds never touch the user's data. `paths::data_dir` points to a
folder under the temp directory, `Deletion` renames entries into
`delete::test_trash` instead of calling the real trash, and `o` saves into a
folder under the temp directory (`newfile::default_dirs`, also used by
`w`), or into `Browser::export_dir` / `Browser::html_dir` when a test sets
them.

Tests run in the default language (Turkish) and must pass on macOS, Linux and
Windows. Compare paths with `/` normalized, and gate Unix-only tests with
`#[cfg(unix)]`.
