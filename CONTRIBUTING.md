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
