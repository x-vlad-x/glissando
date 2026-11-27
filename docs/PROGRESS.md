# Roadmap

The scope of 1.0.0 is in [RELEASE-1.0.md](RELEASE-1.0.md). Its work is tracked
in the [1.0.0 milestone](https://github.com/x-vlad-x/glissando/milestone/1).
This page is a short summary. Device results are in [DEVICE.md](DEVICE.md).

## In the code, with automated tests

* Wayland keyboard: layer shell, input-method-v2, virtual keyboard fallback,
  `sm.puri.OSK0`, portrait and landscape
* Layouts for English, German and Russian, symbols, number, phone and PIN pads,
  email and URL rows
* Prediction, auto-correction, multilingual typing, on-device learning
* Glide typing, space bar cursor, glide delete
* Emoji panel, search and suggestions; clipboard history; quick menu
* Terminal keys, apps without automatic showing, home bar long press
* Registration with gnome-session and clean end of session
* Screen keyboard setting and colour scheme applied while running
* Atomic, private personal data files; config changes from the keyboard and the
  settings app no longer overwrite each other
* CI: formatting, clippy, unit tests, dictionary hashes, e2e in portrait and
  landscape, privacy checks

The settings app has no e2e test. The voice e2e test needs a speech model, so it
reports `SKIP` in CI.

## Checked on the device

* Debian packages with a non-default Phosh-OSK alternative: install, upgrade
  from 0.3.0, switching, removal and installing again (#10)
* Fallback to the system keyboard after three crashes (#9)
* Manual checks on the FLX1s with phoc 0.58 and GTK 4.24: GTK 4, Qt 6,
  Firefox, Chromium, Electron, terminal, voice, lock screen, rotation (#7, #8)
* Performance: start, touch latency, memory and CPU (#22)
* Updates from the signed APT repository (#21)

## In the repository, not yet proven on a device

* Release workflow for arm64 packages, triggered by a `v*` tag

## Open before 1.0.0

* Logout and login with the candidate, and an external display (#8)
* Public test period for the release candidate (#30), then the release (#31)
  and the `1.0.x` branch (#32)

## Later

Not in 1.0; these can return in a later 1.x release:

* Split layout
* FUTO neural glide backend ([ARCHITECTURE.md](ARCHITECTURE.md) explains why it
  is deferred)
* New languages and layouts
* Partial damage rendering, unless device measurements show a battery problem
* Flatpak packaging
