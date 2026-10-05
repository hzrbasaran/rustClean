# Changelog

All notable changes to this project are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.5.0] - 2026-10-05

### Added
- "Similar images" report, after the duplicates in the menu (the reports
  and tools below it move down by one).
  - It finds resized, re-compressed or re-saved copies of the same picture,
    whose bytes differ.
  - It reads JPEG, PNG, WebP, GIF, TIFF and BMP images of 100 KiB or more.
    HEIC and RAW files are not read.
  - Each image gets a 64-bit gradient hash. Images within 4 bits of each
    other are grouped (`[reports] similar_distance`, 0–16).
  - The search runs in the background with progress, and `Esc` cancels it.
  - A group shows how many images it holds and the largest in pixels.
    `Space` adds all but the largest image to the basket, and `Enter`
    lists each image with its size in pixels.
  - It skips the same folders as the clutter report: hidden folders,
    bundles such as a Photos library, `Library` and system folders.
  - It works with `rustclean report similar-images` and with `o`.
  - The image decoders sit behind the default `similar-images` cargo
    feature (about 1.2 MB). `--no-default-features` builds without them.
  ([#13](https://github.com/hzrbasaran/rustClean/issues/13))
- `rustclean check` warns when a disk is fuller than a threshold: 90 % by
  default, or `--threshold`, or `[watch] threshold` in `config.toml`.
  - It watches the disk with your home folder, plus the ones listed under
    `[watch] disks`. It prints one line per disk and exits with code 3 when
    one is over.
  - `rustclean check --install` runs it every hour in the background: a
    launchd agent on macOS, a systemd user timer on Linux. Before doing
    anything it shows what it will write and run, and asks you to type
    `yes` (`evet`). On Windows it shows the `schtasks` command instead.
    `--uninstall` removes it.
  - Background runs show a system notification (`osascript` / `notify-send`)
    at most once a day per disk while the disk stays over.
  ([#23](https://github.com/hzrbasaran/rustClean/issues/23))

### Changed
- For contributors: `o` and `w` save files through one module,
  `newfile.rs` ([#74](https://github.com/hzrbasaran/rustClean/issues/74)).
  CI also checks the build without the `similar-images` feature.
- The Playwright cleanup moves the browsers folder's contents to the trash
  instead of running `npx --yes playwright uninstall --all`, which could
  download Playwright first just to remove the browsers. It works offline,
  and the deletion log records it
  ([#79](https://github.com/hzrbasaran/rustClean/issues/79)).

### Fixed
- The duplicates report no longer looks inside packages or version control
  folders: apps, frameworks, a Photos library, Logic and iMovie libraries,
  and `.git`, `.hg` and `.svn` folders. A file in them could be listed as a
  copy and offered for deletion, which breaks the package or repository even
  when the same bytes exist elsewhere. Run from inside such a folder, the
  report says why it finds nothing
  ([#83](https://github.com/hzrbasaran/rustClean/issues/83)).
- The menu fits small terminals. At 80×24 its lower items and the
  description of the selected one were cut off. Now the list scrolls with
  the selection and shows how many items are above or below. The
  description and the note always stay visible, and long ones wrap instead
  of being cut. A mouse click picks the item drawn where you click
  ([#73](https://github.com/hzrbasaran/rustClean/issues/73)).
- Saving the language or theme can no longer drop the other one when two
  saves happen at once, and the settings file is replaced in one step, so it
  is never read half written
  ([#72](https://github.com/hzrbasaran/rustClean/issues/72)).

## [0.4.0] - 2026-10-05

### Added
- Optional mouse support: `M` turns it on or off (off by default and not
  saved, as mouse capture stops the terminal from selecting text). A click
  selects a row in the lists, the menu and the summary, or a treemap block; a
  double click opens it like `Enter`; the wheel moves through lists and
  scrolls the help, the deletion log and the failure list. Questions before
  deleting, uninstalling or running a cleanup take no mouse input
  ([#24](https://github.com/hzrbasaran/rustClean/issues/24)).
- "iPhone / iPad backups" report, the last of the reports in the menu (the
  tools move down by one): one row per backup folder in `MobileSync/Backup`
  (macOS, and iTunes / Apple Devices on Windows) with the device name and
  model, the date of the backup, the iOS version and whether it is
  encrypted, read from the backup's `Info.plist` and `Manifest.plist`. The
  newest backup of each device is marked, and the note warns that a deleted
  backup cannot restore the device. The age filter goes by the backup date.
  When the folder cannot be read (Full Disk Access) or was not scanned, the
  report says so and how to fix it
  ([#11](https://github.com/hzrbasaran/rustClean/issues/11)).
- An empty report shows its note (why nothing was found) in place of the
  list; before, only "No results." was shown.
- `w` in the folder list and the treemap saves the current folder as a
  self-contained HTML page with a zoomable treemap (click a folder to zoom
  in, the path or `Esc` to go back), colored by type or by age, in the
  current language. The data and script are inside the file and it makes no
  network requests, so it works offline and on a phone. At most 4 levels and
  3,000 blocks; small entries share an "other" block. The file goes to the
  working directory (else the home folder) as
  `rustclean-treemap-YYYYMMDD-HHMMSS.html` and never replaces an existing one
  ([#27](https://github.com/hzrbasaran/rustClean/issues/27)).
- Linux system caches on the tools screen: the apt, dnf and pacman package
  caches, the systemd journal (`journalctl --disk-usage`) and disabled snap
  revisions (`snap list --all`). They need root, so rustClean shows the exact
  `sudo` command (`apt-get clean`, `dnf clean all`, `paccache -rk1` /
  `pacman -Scc`, `journalctl --vacuum-time=2weeks`,
  `snap remove <name> --revision=<rev>`) for you to run instead of running it
  ([#18](https://github.com/hzrbasaran/rustClean/issues/18)).
- Windows folders on the tools screen: `%TEMP%` is moved to the Recycle Bin
  after you confirm, entry by entry, skipping files in use;
  `C:\Windows\Temp` and the Windows Update download cache are measured when
  readable and show the commands for an administrator PowerShell; the Recycle
  Bin shows its size per drive, with `Clear-RecycleBin` to run yourself
  ([#19](https://github.com/hzrbasaran/rustClean/issues/19)).
- `rustclean report <kind> [PATH]` runs a report without the interface and
  prints it as a readable table, or with `--csv` / `--json` for scripts.
  Every report has a kebab-case name (`largest-files`, `dev-junk`,
  `duplicates`…); `--older DAYS` applies the age filter where the report has
  one, and `--limit N` caps the rows. `PATH` defaults to the current folder.
  It only reads: nothing is deleted
  ([#20](https://github.com/hzrbasaran/rustClean/issues/20)).
- `o` saves the list on screen as CSV or JSON: the folder list, the treemap,
  the summary lists and every result list. The file goes to the working
  folder (or the home folder when that is not writable) as
  `rustclean-<list>-YYYYMMDD-HHMMSS.csv|json` and never replaces an existing
  file. Columns: absolute path, apparent size, size on disk, file count,
  modified and created (ISO 8601, UTC), the group and the row's detail; a
  group is saved as its members
  ([#21](https://github.com/hzrbasaran/rustClean/issues/21)).
- Configuration file: an optional `config.toml` in the data directory sets
  folders the scan skips (`[scan] exclude`, `~` allowed; also for
  `--summary` and `report`), the thresholds of "Old and large files" and of the duplicate
  search (`[reports]`), and the size and sort order a scan opens with
  (`[view]`). The menu and report notes show the values in effect. A broken
  file never stops rustClean: it starts with the defaults and names the file
  and the error's line and column; unknown keys only give a warning.
  `rustclean --config` prints the file's path and the values in effect
  ([#22](https://github.com/hzrbasaran/rustClean/issues/22)).
- More developer caches in the tools cleanup: Go (`go clean -modcache`,
  `go clean -cache`), Maven (`~/.m2/repository`), Bun, uv (`uv cache clean`),
  conda (`conda clean --all`, sized with its own dry run), the Flutter / Dart
  pub cache and Playwright's browsers
  ([#14](https://github.com/hzrbasaran/rustClean/issues/14)).
- Xcode archives: their size, and moving archives older than 12 months to
  the trash (data loss: they are needed to symbolicate crash reports).
  Simulators never used, or not used for over a year, can be deleted all at
  once or one by one
  ([#16](https://github.com/hzrbasaran/rustClean/issues/16)).
- Android SDK: emulators with their sizes, deleting one with `avdmanager`
  (or moving it to the trash without it), leftover `.avd` folders, and
  system images no emulator uses
  ([#15](https://github.com/hzrbasaran/rustClean/issues/15)).
- Docker Desktop's `Docker.raw`: its size on disk next to its apparent size,
  and a note on how the space comes back after pruning
  ([#17](https://github.com/hzrbasaran/rustClean/issues/17)).
- Tool cleanups refuse to empty or trash a relative path, a root, the
  home folder or a folder above it, or a standard folder such as Desktop,
  Documents or `~/Library`, for example when `PUB_CACHE` points to the
  home folder.
- The tools screen shortens long step lists to fit, the action picker
  scrolls, and a step chosen twice runs once. Some tools show a note under
  their actions.

### Fixed
- English counts use the singular for one: "1 item", "1 row", "1 file"
  instead of "1 items" (and "1 item was inaccessible", "the parent folder of
  1 item").
- The clutter report no longer lists a folder it could not look into as
  empty. A folder without read permission, on another disk or skipped by
  the scan showed nothing below it and was offered as an empty folder,
  although it may hold files. Such folders, and the folders around them,
  are now never counted as empty.

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

[Unreleased]: https://github.com/hzrbasaran/rustClean/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/hzrbasaran/rustClean/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/hzrbasaran/rustClean/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/hzrbasaran/rustClean/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/hzrbasaran/rustClean/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/hzrbasaran/rustClean/releases/tag/v0.1.0
