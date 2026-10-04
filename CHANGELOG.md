# Changelog

All notable changes to this project are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed
- `L` now rebuilds the open result list in the new language: a report (with
  its age filter), search results, a comparison with an earlier scan, the
  duplicates (without searching again) or the basket. The cursor stays on
  the same entry, and an open group is opened again
  ([#8](https://github.com/hzrbasaran/rustClean/issues/8)).

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

[Unreleased]: https://github.com/hzrbasaran/rustClean/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/hzrbasaran/rustClean/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/hzrbasaran/rustClean/releases/tag/v0.1.0
