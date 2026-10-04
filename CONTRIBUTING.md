# Contributing to rustClean

Thanks for your interest! Bug reports, ideas, documentation fixes and code are
all welcome. Discussion in Turkish or English is fine.

## Before you start

- For anything bigger than a small fix, please open an issue first so we can
  agree on the approach.
- Read the [architecture overview](docs/ARCHITECTURE.md). It explains where
  things live.
- Be kind: this project follows the [Code of Conduct](CODE_OF_CONDUCT.md).

## Development setup

You need a recent stable Rust toolchain ([rustup](https://rustup.rs)).

```bash
git clone https://github.com/hzrbasaran/rustClean
cd rustClean
cargo run --release -- ~/some/folder
```

Use `RUSTCLEAN_DATA_DIR=/tmp/rc-dev` while developing, so test runs do not
mix with your real scan history and settings.

## Checks

CI runs these on macOS, Linux and Windows; please run them before opening a
pull request:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

### Lint policy

The lints live in `Cargo.toml` (`[lints]`), so your editor shows the same
warnings as CI, where `-D warnings` turns them into errors. On top of the
default clippy set:

- `unsafe_code` is denied. The only exception is the `getattrlistat(2)` code
  in `clones.rs`, and every `unsafe` block needs a `// SAFETY:` comment
  (`clippy::undocumented_unsafe_blocks`).
- `rust_2018_idioms`, `unused_qualifications` and `trivial_casts`.
- A few `clippy::pedantic` lints that catch real mistakes or keep the code
  uniform: `needless_pass_by_value`, `cast_lossless`, `manual_let_else`,
  `map_unwrap_or`, `redundant_closure_for_method_calls`,
  `semicolon_if_nothing_returned`, `doc_markdown`, `items_after_statements`.

Left out on purpose: the `cast_possible_truncation` / `cast_sign_loss` /
`cast_precision_loss` group (layout math casts on purpose), `match_same_arms`
(the leftover rules in `apps/orphans.rs` keep one arm per case so they are
easy to review), and the style-only `too_many_lines`, `similar_names` and
`single_match_else`.

A `Dependencies` workflow runs [cargo-deny](https://github.com/EmbarkStudios/cargo-deny)
(`deny.toml`) on every pull request and once a week. It checks security
advisories, licenses compatible with MIT OR Apache-2.0, banned crates and
sources. A new dependency with a license not in `deny.toml` needs a note in
the pull request.

### Tests

Unit tests sit next to the code. For behavior that spans modules, add an
integration test in `src/integration/`:

```rust
let f = Fixture::standard(); // or Fixture::new() and f.file(...)
let mut app = open(f.root()); // scans and opens the browser
menu(&mut app, 8);            // m, ↓ × 8, Enter: the Downloads report
tick_until(&mut app, "the report", |a| browser(a).results.is_some());
assert_eq!(rows(&app), ["Downloads/setup.dmg", "Downloads/photos.zip"]);
```

#### Screen snapshots

`src/integration/screens.rs` draws every screen at 100×30 in Turkish and
English and compares it with `src/integration/snapshots/*.snap`. A snapshot
holds the text and the colors of each line. When you change a screen on
purpose, the test fails with a diff; accept the new version with

```bash
cargo install cargo-insta   # once
cargo insta review          # or: INSTA_UPDATE=always cargo test
```

and commit the `.snap` changes with the code, so the review shows what the
screen looks like now.

Colors come from `ui/theme.rs` by role (`.normal()`, `.muted()`, `.warn()`,
…); do not use color names in the screens. To compare the themes in a
browser:

```bash
RUSTCLEAN_PREVIEW=preview.html cargo test theme_preview -- --ignored
``` Snapshot tests run on macOS and Linux (paths use `/`).
CI never writes snapshots; a missing or different one fails the build.

`cargo test` never touches your history, settings or trash: test builds use
folders under the system temp directory instead (see `docs/ARCHITECTURE.md`).

## Guidelines

**Safety comes first.** rustClean deletes files, so changes that touch
deletion are held to a high standard:
- Never delete without an explicit confirmation that lists what will go.
- Move to the trash; never remove permanently.
- Never run commands through a shell or with `sudo`.
- When a heuristic decides what can be deleted (app leftovers, developer
  junk), err on the side of keeping things, and add a test for the case that
  must *not* be listed.

**Every user-visible text in both languages.** Write texts where they are
used, with the macros from `src/i18n.rs`:

```rust
self.set_status(t!("Sepet boş.", "The basket is empty."), false);
let title = tf!("{n} öğe taşındı", "{n} items moved");
```

Use `fmt_count`, `fmt_size`, `fmt_date` and `fmt_pct` from `ui/format.rs` for
numbers, so they follow the language. If you do not speak Turkish, write the
English text and leave a note in the pull request; we will help with the
translation.

**Tests.**
- Add unit tests next to the code.
- Build a small `Tree` by hand for report logic.
- Use `tempfile` for anything that touches the file system.
- Use fixtures in `tests/fixtures/` for parsers of command output.
- Tests run in Turkish (the default) and must pass on all three platforms:
  compare paths with `/` normalized, and gate Unix-only tests with
  `#[cfg(unix)]`.

**Performance.** Code that walks the tree may run over tens of millions of
entries. Avoid per-node allocations (`path_of`, `to_lowercase`) in hot loops.
Measure on a large folder when in doubt (`rustclean --summary <path>`).

**Style.** Follow the surrounding code: doc comments on modules and public
items, comments that explain *why*, and small focused functions.

## Pull requests

- One topic per pull request, with a clear description of what changed and
  how you tested it. Screenshots help for UI changes.
- Keep commits focused; write commit messages in English, imperative mood
  ("Add …", "Fix …").
- Update `CHANGELOG.md` under *Unreleased*, plus the README or
  `docs/USAGE.md` when behavior changes.

## Reporting bugs

Please use the issue templates and include:
- the operating system and version
- how you started rustClean (arguments)
- what you expected and what happened

For anything involving data being deleted unexpectedly, see
[SECURITY.md](SECURITY.md).

## Releasing (maintainers)

1. Bump `version` in `Cargo.toml`, run `cargo check` (updates `Cargo.lock`),
   and turn *Unreleased* in `CHANGELOG.md` into the new version with its date.
2. After the release PR is merged, tag `main` and push the tag:
   `git tag vX.Y.Z && git push origin vX.Y.Z`.
3. The release workflow builds the binaries, publishes the GitHub release with
   the CHANGELOG section as notes, and updates the Homebrew formula in
   [hzrbasaran/homebrew-tap](https://github.com/hzrbasaran/homebrew-tap)
   (needs the `HOMEBREW_TAP_TOKEN` secret).
4. Publish the crate: `cargo publish`.

## License

By contributing, you agree that your contributions are dual licensed under the
MIT and Apache-2.0 licenses, as described in the [README](README.md#license).
