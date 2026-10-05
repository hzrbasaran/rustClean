# Changelog

All notable changes to this project are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- Optional mouse support: `M` turns it on or off (off by default and not
  saved, as mouse capture stops the terminal from selecting text). A click
  selects a row in the lists, the menu and the summary, or a treemap block; a
  double click opens it like `Enter`; the wheel moves through lists and
  scrolls the help, the deletion log and the failure list. Questions before
  deleting, uninstalling or running a cleanup take no mouse input
  ([#24](https://github.com/hzrbasaran/rustClean/issues/24)).

### Fixed
- English counts use the singular for one: "1 item", "1 row", "1 file"
  instead of "1 items" (and "1 item was inaccessible", "the parent folder of
  1 item").

## [0.3.0] - 2026-10-04

### Added
- Themes: `T` switches between dark (the default), light (for white
  terminal backgrounds) and a color-blind friendly palette (Okabe–Ito: blue
  and orange instead of green and red). The choice is saved;
  `--theme dark|light|colorblind` overrides it
  ([#25](https://github.com/hzrbasaran/rustClean/issues/25)).
- `--no-color` and the `NO_COLOR` environment variable turn colors off; the
  selection is shown reversed and treemap blocks get borders.
- `?` opens a help screen with every key: the current screen's first, then
  the keys that work everywhere, then the other screens. The bottom line
  starts with `?  help`
  ([#26](https://github.com/hzrbasaran/rustClean/issues/26)).
- "Empty folders, broken links, temporary files" report, in three groups:
  folders with nothing below them, symbolic links whose target is gone
  (checked when the report runs), and `.DS_Store`, `Thumbs.db`, `*.tmp`,
  Office `~$…` locks and unfinished downloads untouched for a day. Hidden
  folders, bundles, `Library`, `AppData`, build output and dependencies,
  system folders and the standard home folders are left alone
  ([#12](https://github.com/hzrbasaran/rustClean/issues/12)).
- Deletion log: every entry moved to the trash is written to
  `deletions.jsonl` in the data directory (time, path, size and how: list,
  summary, report, search, basket, uninstall or a developer tool). Failed
  moves are not written; the newest 10 000 entries are kept. "Deletion log"
  in the menu shows it by day, newest first
  ([#9](https://github.com/hzrbasaran/rustClean/issues/9)).

### Changed
- `L` now rebuilds the open result list in the new language: a report (with
  its age filter), search results, a comparison with an earlier scan, the
  duplicates (without searching again) or the basket. The cursor stays on
  the same entry, and an open group is opened again
  ([#8](https://github.com/hzrbasaran/rustClean/issues/8)).
- Releases are published to crates.io by the release workflow, together
  with the binaries and the Homebrew formula
  ([#29](https://github.com/hzrbasaran/rustClean/issues/29)).
- For contributors: the code is split into one module per screen, job and
  report ([#37](https://github.com/hzrbasaran/rustClean/issues/37)–[#40](https://github.com/hzrbasaran/rustClean/issues/40)),
  lints live in `Cargo.toml` and `cargo-deny` checks dependencies weekly
  ([#41](https://github.com/hzrbasaran/rustClean/issues/41)), integration
  tests run on real folders and every screen has a snapshot in both
  languages ([#42](https://github.com/hzrbasaran/rustClean/issues/42),
  [#43](https://github.com/hzrbasaran/rustClean/issues/43)). Test builds
  never touch your history, settings or trash. See CONTRIBUTING.md.

### Fixed
- The basket's bottom line lists its own keys (`c` empties it, `x` moves
  everything to the trash) instead of report keys that did nothing there.
- The selected row of lists in the dark theme is white on dark gray: colored
  dates and folder names were hard to read on the old gray.
- Summary: the file types column fits the longest label in the current
  language ("Archives / disk images" ran into its bar), and on narrow
  terminals labels are cut instead of the numbers.
- System data: the usage percentage after the bar is no longer cut off, and
  it is written the English way (`73%`) in English.
- Items that could not be moved to the trash: wrapped explanations, paths
  and errors continue indented under their own text
  ([#51](https://github.com/hzrbasaran/rustClean/issues/51)).

## [0.2.0] - 2026-10-03

### Added
- Uninstall an app with its data (macOS): `u` in "Applications and their
  data" lists the bundle, its data folders and preference files, all checked
  and each one can be unchecked, then moves them to the trash. System apps
  are refused, a running app is flagged, and the report is refreshed
  ([#7](https://github.com/hzrbasaran/rustClean/issues/7)).
- "Installers and archives in Downloads" report: disk images, installers
  and archives in Downloads folders, largest first, with the age filter
  ([#10](https://github.com/hzrbasaran/rustClean/issues/10)).
- `R` rescans only the current folder in the background and updates the
  tree in place (sizes, counts and dates of the folder and its parents), so
  refreshing after a cleanup no longer needs a full rescan
  ([#5](https://github.com/hzrbasaran/rustClean/issues/5)).
- Install with Homebrew: `brew install hzrbasaran/tap/rustclean`. The
  release workflow updates the formula
  ([#28](https://github.com/hzrbasaran/rustClean/issues/28)).

### Changed
- On APFS, pure clones (`cp -c`, Finder duplicates) are counted once in the
  on-disk size, so a folder of clones no longer looks many times larger than
  it is. The duplicates report shows how many copies are clones, since
  deleting them frees nothing
  ([#6](https://github.com/hzrbasaran/rustClean/issues/6)).
- The apps report includes preference files (`Library/Preferences`, matched
  by bundle id).
- English confirmation footers show `y` / `n`, like the dialogs.
- Dependencies: ratatui 0.30, sysinfo 0.39; release workflow actions updated.

## [0.1.0] - 2026-10-03

First public release.

### Scanning and browsing
- Parallel disk/folder scan that does not follow symlinks or cross into other
  volumes, and counts hard links once.
- Apparent and on-disk sizes (`a` switches between them).
- Browsable list with share bars, file counts and color-coded modification
  and creation dates; sorting by size, name, file count or age.
- Treemap view (`t`) with per-folder colors, color by type or age, and
  keyboard navigation between blocks.
- Folder summary (`i`): file types, age distribution, largest files and
  fullest folders, disk usage.
- Name search with `*` / `?` wildcards (`/`).

### Reports (`m`)
- Largest files and folders, most repeated file names.
- Applications with their data, and orphaned app leftovers (works from a
  home-folder scan).
- Developer junk (node_modules, Cargo target, build/dist, Pods, DerivedData,
  .venv, …), cache folders, old and large files.
- Duplicate files by content (size → partial hash → full hash), keeping the
  oldest copy.
- Minimum-age filter (`f`); project age for developer junk.

### Cleaning
- Basket: collect entries anywhere with `Space`, review with `S`, trash with
  `x`.
- Everything goes to the system trash after confirmation; mount points and
  other volumes are refused; a readable dialog explains failures (e.g. Full
  Disk Access).
- Developer tools cleanup for Docker, Xcode simulators, DerivedData /
  DeviceSupport, npm, pnpm, Yarn, pip, Gradle, CocoaPods, Homebrew and Cargo,
  with exact commands shown and `yes` required for data-loss actions.

### More
- Scan history with "changes since the last scan".
- macOS system data panel: APFS volumes, local snapshots, swap, simulator
  images, and why the scan differs from the disk.
- Turkish and English interface (`L`, `--lang`).
- `--list-disks` and `--summary` command line modes.

[Unreleased]: https://github.com/hzrbasaran/rustClean/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/hzrbasaran/rustClean/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/hzrbasaran/rustClean/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/hzrbasaran/rustClean/releases/tag/v0.1.0
