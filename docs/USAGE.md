# Usage guide

This guide walks through every screen of rustClean. Each screen also lists its
keys on the bottom line, so you rarely need to look anything up.

- [Starting](#starting)
- [Browsing](#browsing)
- [Treemap](#treemap)
- [Summary](#summary)
- [Finding by name](#finding-by-name)
- [The basket and deleting](#the-basket-and-deleting)
- [Reports](#reports)
- [Tools](#tools)
- [Language](#language)
- [Deletion log](#deletion-log)
- [Themes and colors](#themes-and-colors)
- [Command line](#command-line)

## Starting

Run `rustclean` without arguments to get the list of disks with their usage;
pick one with `Enter`. Or pass a folder: `rustclean ~/Projects`.

Scanning shows live progress (files, folders, size, inaccessible entries).
`Esc` cancels it. Scanning a whole disk with millions of files takes a minute
or two on an SSD.

`?` on any screen lists every key: the current screen's first, then the keys
that work everywhere, then the other screens. The bottom line always starts
with `?  help` and shows the most used keys.

## Browsing

The list shows the current folder's entries with:

- **Size** and a **share** bar relative to the folder.
- **Files**: how many files a folder contains.
- **Modified / Created**: for folders, *Modified* is the newest change of
  anything inside, so long-untouched folders stand out. Dates are colored
  green (≤ 7 days), cyan (≤ 30 days), yellow (≤ 1 year) and red (older).
- **Name**: folders end with `/`; a `✓` marks entries in the basket.

| Key | Action |
|---|---|
| `↑` `↓` `PgUp` `PgDn` `g` `G` | move |
| `Enter` / `→` | open a folder |
| `⌫` / `←` / `Esc` | back to the parent folder |
| `s` | sort: size → name → file count → oldest change first |
| `a` | apparent size (file length) ↔ size on disk (what `du` reports; on APFS, pure clones are counted once) |
| `R` | rescan only the current folder (e.g. after a cleanup), in the background; `Esc` cancels |
| `r` | rescan everything from the root |
| `d` | back to the disk list |

## Treemap

`t` switches between the list and a treemap of the current folder. Block area
is proportional to size; entries too small for their own block share the
strip at the bottom ("other"). The selection is shared with the list.

| Key | Action |
|---|---|
| arrow keys | move to the neighboring block |
| `Enter` / `⌫` | open a folder / go back |
| `c` | color by type (folders each get their own color) or by age |
| `t` / `Esc` | back to the list |

`Enter` on the "other" strip opens the list at the first small entry.

## Summary

`i` shows a summary of the current folder:
- totals and the disk it is on
- **file types** (video, archives, build output, …)
- **age** distribution
- the **largest files**
- the **fullest folders**

"Fullest folders" ranks folders by the files *directly* inside them. Ranking by
total size would only list chains of nested parents.

`Tab` switches between the two lists, `Enter` goes to the entry, `Space`/`x`
work as everywhere else.

## Finding by name

`/` searches the current folder and everything below it. The match ignores
case:
- `deneme` matches the whole name exactly
- `deneme*` matches names starting with it
- `*.log` matches all `.log` files
- `test?` matches one extra character

Contents of a matching folder are not listed separately, since moving the
folder moves them too.

## The basket and deleting

- `Space` adds the entry under the cursor to the basket, or removes it. This
  works in the list, the treemap, the summary lists and every result list.
  `t` in a result list adds all its rows.
- The header shows the basket's size: `🧺 3 items, 1.2 GiB (S)`.
- `S` shows the basket. There, `Space` removes an entry and `c` empties the
  basket.
- `x` moves **the whole basket** to the trash after a confirmation. If the
  basket is empty, `x` acts on the entry under the cursor instead.

A folder in the basket covers everything below it, so nothing is counted or
trashed twice.

If some entries cannot be moved (for example protected locations without Full
Disk Access, or locked files), a dialog lists each one with the full error and
what to do about it.

## Reports

`m` opens the menu. Reports cover the folder you are in. Rows start
unselected. `Enter` opens a group or goes to an entry, and `Esc` goes back.

| Report | What it lists |
|---|---|
| Largest files | the 200 largest files |
| Largest folders | by total size, skipping folders that merely wrap one big subfolder |
| Most repeated file names | names that occur most often; `Enter` lists the files |
| Applications and their data | each app with its data folders in `~/Library` (support, caches, containers, logs…) and preference files; the whole scan is used; `u` uninstalls |
| Orphaned app leftovers | data folders that belong to no installed app (installed apps are read from `/Applications` as well, so scanning your home folder is enough) |
| Developer junk | `node_modules`, Cargo `target`, `build`, `dist`, `Pods`, `.build`, `DerivedData`, `.gradle`, `.venv`, `__pycache__`, `vendor`, .NET `bin`/`obj`… only when the project's marker file is present |
| Cache folders | entries of `Caches` / `.cache`, and `GPUCache`, `Code Cache`… |
| Old and large files | ≥ 100 MiB and unchanged for over a year |
| Installers and archives in Downloads | disk images (`.dmg`, `.iso`…), installers (`.pkg`, `.msi`, `.deb`…) and archives (`.zip`, `.xip`, `.tar.gz`…) in `Downloads` folders below the current one (or in the current folder when it is inside `Downloads`) |
| Duplicate files | files with identical content (≥ 1 MiB); `Space` on a group adds all but the oldest copy |
| Empty folders, broken links, temporary files | three groups: folders with nothing below them (only the topmost is listed), symbolic links whose target is gone, and temporary files (`.DS_Store`, `Thumbs.db`, `*.tmp`, Office `~$…` locks, unfinished downloads) untouched for a day; `Enter` opens a group, `Space` adds it whole |

**Age filter.** In most reports `f` cycles a minimum age: none → 30 → 90 → 180
→ 365 days since the last change. For developer junk the age is the
project's (the newest change next to the junk folder), so a freshly reinstalled
`node_modules` in an abandoned project still counts as old.

**Uninstalling an app** (macOS). In "Applications and their data", `u` on an
app (or inside its group) lists the bundle and every data folder and
preference file matched to it, all checked. `Space` unchecks an entry, `t`
toggles all, and `e` moves the checked ones to the trash. Apps under `/System`
are refused. If the app is running, the dialog says so; quit it first. The
report is listed again afterwards. Preference files
(`~/Library/Preferences/<bundle id>….plist`, also `ByHost`) are matched by
bundle id only.

**Leftovers are a best guess.** Matching data folders to apps uses names and
bundle identifiers. The report excludes shared and system folders and
anything that looks like it belongs to an installed app. Still, look inside
(`Enter`) before deleting.

**What the clutter report leaves alone.** Nothing is lost by removing an
empty folder or a broken link, unless something expects it. So the report
does not look inside hidden folders (a fresh repository's `.git/refs` is
empty and needed), app and package bundles (`.app`, `.framework`,
`.photoslibrary`…), `Library`, `AppData`, build output and dependencies
(`target`, `Pods`, `node_modules`, `vendor`…) and system folders,
and it never lists the standard folders of your home folder (`Desktop`,
`Documents`, `Downloads`…). A folder that holds only such a folder is not
empty. Temporary files changed in the last day may be in use (an open
document's lock file, a running download) and are left out; `.DS_Store` and
`Thumbs.db` are listed at any age, as the system writes them again.

**Duplicates and APFS.** Copies that are APFS clones (made by `cp -c`, Finder's
Duplicate and many apps) share their blocks. The group's detail says how many
copies are clones, and deleting them frees nothing. In disk size mode the
group's size already leaves them out.

## Tools

**Changes since the last scan.** Every completed scan is summarized and the
newest 10 per folder are kept. Pick a saved scan to see what grew, shrank or
appeared below the current folder. The removed entries are summarized at the
bottom.

**Removed app leftovers.** The same report as "Orphaned app leftovers".

**Developer tools cleanup.** Measures, read-only, what each installed tool
could free, then lets you choose actions:
- green: safe
- yellow: will be downloaded or built again
- red: may lose data

The confirmation shows the exact commands. Red actions require typing `yes`
(`evet` in Turkish). Commands run one after another with their output in a
log. The tool is then measured again.

| Tool | Actions |
|---|---|
| Docker | stopped containers + dangling images + build cache · all unused images · unused volumes (data loss) |
| Xcode simulators | delete unavailable simulators · delete individual runtimes |
| DerivedData / DeviceSupport | move contents to the trash |
| npm · pnpm · Yarn · pip | the tool's own cache commands |
| Gradle | stop daemons, move the cache to the trash |
| CocoaPods · Homebrew | `pod cache clean --all` · `brew cleanup --prune=all` |
| Cargo | move downloaded `.crate` files to the trash |

**System data** (macOS). Shows:
- the APFS container and every volume's usage
- how a scan of `/` compares to the System + Data volumes, and why
- Time Machine local snapshots (with the command to delete them; it needs
  `sudo`, so rustClean does not run it)
- swap and sleep image
- simulator runtime images, which live outside the scanned volume

## Language

`L` switches between Turkish and English anywhere (except while typing). An
open report, search, comparison or basket is built again in the new language,
with the cursor on the same entry and an open group still open. The choice is
saved. `--lang tr|en` overrides it for one run. Without either, the system
language is used.

## Deletion log

Every entry moved to the trash is written to `deletions.jsonl` in the data
directory: when, the path, its size, and how (the folder list, the summary, a
report, a search, the basket, uninstalling an app, or a developer tool's
cleanup). Moves that failed are not written. The newest 10 000 entries are
kept.

**Deletion log** in the menu (`m`) shows it newest first, grouped by day with
each day's total, and the total of the last 30 days. The entries themselves
are in the trash and can be restored from there until it is emptied.

## Themes and colors

`T` switches the colors anywhere (except while typing): **dark** (the
default), **light** for terminals with a white background, and
**color-blind friendly**, which uses blue and orange instead of green and red
(dates, disk usage, cleanup risks; each also has text beside it). The choice
is saved. `--theme dark|light|colorblind` overrides it for one run.

`--no-color`, or the `NO_COLOR` environment variable, turns colors off: the
selected row is shown reversed, headings in bold, and treemap blocks get
borders. `T` then leaves the colors off.

## Command line

```text
rustclean [PATH] [--lang tr|en] [--theme dark|light|colorblind] [--no-color]
          [--list-disks] [--summary]
```

- `PATH`: scan this folder directly instead of choosing a disk.
- `--list-disks`: print the disks and exit.
- `--summary`: scan `PATH` without the interface and print the totals.
- `RUSTCLEAN_DATA_DIR`: where history and settings are stored.
