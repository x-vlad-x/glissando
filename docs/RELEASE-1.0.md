# Scope of glissando 1.0.0

1.0.0 is a reliability release: it makes what already exists dependable and
adds no new typing features. This document defines what the release covers
and what stays stable afterwards.

## Supported systems

| Tier | Systems | Meaning |
|---|---|---|
| Supported | FuriOS on the FuriPhone FLX1s | Tested before each release; bugs block the release |
| May work | Mobian, postmarketOS, PureOS and other Phosh systems | Not verified; fixes are best effort |

A system moves to *Supported* only after someone runs the manual device
checks and the package install, upgrade and removal cycle on it. Until then,
documentation must not promise that it works.

Phosh is the only supported desktop. Flatpak is not a distribution format for
1.0: a sandboxed client cannot see the Wayland protocols a keyboard depends on.

## Not in 1.0

- Split layout
- FUTO neural glide backend
- New languages and layouts beyond English, German and Russian
- Partial damage rendering, unless device measurements show a battery problem
- Flatpak packaging

These can return in a later 1.x release. Bug fixes for existing features are
always in scope.

## Required for 1.0

- **Session:** register with gnome-session as `sm.puri.OSK0` (PR #1), with
  clean logout and no loss of learned words.
- **Behaviour:** the settings app no longer overwrites changes made from the
  keyboard, and theme and `screen-keyboard-enabled` changes apply while
  running. Personal data files are written atomically and privately.
- **Device checks:** Qt and Chromium apps, live speech, terminals, password
  fields, rotation, login, suspend, and the fallback to the system keyboard.
- **Packaging:** the full install, upgrade and removal cycle on a real device,
  accurate dependencies and licence files, AppStream metadata and man pages.
- **Project:** CI runs the e2e and privacy checks, arm64 packages are built by
  a release workflow, updates come from an APT repository, and the
  documentation matches the code.

## Compatibility promise

From 1.0.0, versions follow [SemVer](https://semver.org). Everything in this
section keeps working across all 1.x releases. A change that breaks it needs
either an automatic migration or a 2.0.0 release.

### Configuration

`$XDG_CONFIG_HOME/glissando/config.toml`, or the path in `GLISSANDO_CONFIG`.

- Existing keys keep their name, type and meaning. Adding a key with a default
  is allowed.
- A rename or removal needs a migration: unknown keys are ignored on load, so
  the old value would otherwise be lost silently.
- A partial file stays valid; missing keys use their defaults.
- Accepted values may only widen. A new `theme` or `one_handed` value makes the
  whole file invalid for older versions, so it must be listed in the changelog.

Saving writes only known keys, so a downgrade followed by a save drops keys
added by the newer version. An invalid file falls back to the defaults, and the
next save replaces it.

### Personal data

| Data | Location | Format |
|---|---|---|
| Learned words | `$XDG_DATA_HOME/glissando/user/<lang>.tsv` | Tab-separated `w word count last flags` and `b first second count` lines; flags are `p` and `b` |
| Pinned clips | `$XDG_STATE_HOME/glissando/pinned-clips` | Text entries separated by U+001E |
| Recent emoji | `$XDG_STATE_HOME/glissando/recent-emoji` | One emoji per line, newest first |

New versions must read files written by any earlier 1.x version. Unknown TSV
lines are skipped on load and dropped on the next save.

### Dictionaries

`.gld` files with the magic `GLDICT01` stay readable. An incompatible layout
needs a new magic, and readers must keep accepting `GLDICT01` throughout 1.x.

### Identifiers

- App IDs: `io.github.x_vlad_x.Glissando` and
  `io.github.x_vlad_x.Glissando.Settings`
- Package names: `glissando` and `glissando-voice`
- D-Bus: owns `sm.puri.OSK0` and implements its interface
- Session integration: the `Phosh-OSK` alternative, `sm.puri.OSK0.desktop` and
  `mobi.phosh.OSK.service`
- Launcher: `/usr/libexec/glissando/glissando-launch`
- Installation must not change the active keyboard: the alternative stays below
  the priority of the system keyboards

### Command line and environment

- `glissando`: `--replace`, `--show`, `--allow-replacement`, `--no-dbus`,
  `--version`, `--help`
- Environment: `GLISSANDO_CONFIG`, `GLISSANDO_DATA_DIR`

### Not covered

These can change in any release:

- `--dump-geometry`, `--bench-render` and the `GLISSANDO_DEBUG*`,
  `GLISSANDO_AUTOSHOW` and `GLISSANDO_VOICE_TEST_INPUT` variables, which exist
  for testing and debugging
- The `glissando-voice` helper protocol and arguments, which are internal
- The `glissando-dictc` command line
- `glissando-session`, `glissando-trial` and `deploy.sh`, which are development
  tools for the user-prefix install
- The layout file format, until it is documented
- Rust crate APIs, which are not published as libraries

## Release criteria

1.0.0 ships from a release candidate when all of these hold:

- No open bug causes data loss, a failed session or a privacy leak.
- CI is green, including the e2e and privacy checks.
- The device checks are complete on the supported system.
- The candidate ran through the test period with no crashes in normal use.
- The install, upgrade and removal cycle passes on the final package.
