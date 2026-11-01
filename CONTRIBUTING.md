# Contributing to glissando

Bug reports and fixes are welcome. Before starting on a feature, read
[docs/RELEASE-1.0.md](docs/RELEASE-1.0.md): 1.0 adds no new typing features, and
its "Not in 1.0" list says what is out of scope. Bug fixes for existing features
are always in scope.

For security problems, see [SECURITY.md](SECURITY.md). Do not open a public
issue.

## Build

You need Rust 1.92 or newer, and the development files of GLib, cairo, Pango,
GTK 4 and libadwaita. On Debian or Ubuntu:

```bash
sudo apt install libglib2.0-dev libcairo2-dev libpango1.0-dev libgtk-4-dev libadwaita-1-dev
cargo build --workspace
```

Dictionaries are not in the repository. This downloads pinned word lists once
and writes `data/dict/*.gld`:

```bash
python3 tools/build-dicts.py
```

The arm64 build and the packages are described in the
[README](README.md#building) and [docs/DEVICE.md](docs/DEVICE.md).

## Test

CI runs these on every push to `main` and on every pull request. Run them before
you open one:

```bash
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked --workspace
cargo test --locked --workspace
```

Clippy warnings fail the build. `rustfmt.toml` sets the format, so use
`cargo fmt --all` rather than formatting by hand.

The end-to-end tests run the real keyboard in a headless sway. They need the
dictionaries and a built workspace. `fetch-sway.sh` needs `pacman`, so it works
on Arch-based hosts only:

```bash
tools/devenv/fetch-sway.sh
dbus-run-session -- python3 -I tests/e2e/e2e.py
E2E_LANDSCAPE=1 dbus-run-session -- python3 -I tests/e2e/e2e.py
```

`-k NAME` runs one test. The tests use private directories and do not touch your
desktop.

The privacy checks build release binaries and run the e2e suite twice more, with
a socket guard and in an empty network namespace. They need `gcc`:

```bash
tests/privacy/check.sh
```

If you change `tools/build-dicts.py`, CI checks that the output still matches
`data/dict/SOURCES.md`.

## Changes that need care

[docs/RELEASE-1.0.md](docs/RELEASE-1.0.md) lists what must keep working across
1.x: configuration keys, personal data files, dictionaries, identifiers and the
command line. Do not edit the format fixtures to make a test pass: add a
migration instead. Nothing may add network access; see
[docs/PRIVACY.md](docs/PRIVACY.md).

## Commits

Write the subject as a short sentence in the imperative, with a capital letter
and no full stop or prefix. Describe the effect, not the file:

```
Keep the recording unnamed and tied to the voice helper
Pin the dictionary sources by SHA-256
```

Keep each commit to one change. Add user-visible changes to the `Unreleased`
section of [CHANGELOG.md](CHANGELOG.md).

## Licence

glissando is licensed under GPL-3.0-or-later. By contributing, you agree that
your work is released under that licence. Data you add must have a compatible
licence and be listed in `data/dict/SOURCES.md`, `data/emoji/` or
`THIRD-PARTY.md`, as fits.
