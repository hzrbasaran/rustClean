# Changelog

All notable changes to this project are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed
- On APFS, pure clones (`cp -c`, Finder duplicates) are counted once in the
  on-disk size, so a folder of clones no longer looks many times larger than
  it is. The duplicates report shows how many copies are clones, since
  deleting them frees nothing (#6).

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

[Unreleased]: https://github.com/hzrbasaran/rustClean/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hzrbasaran/rustClean/releases/tag/v0.1.0
