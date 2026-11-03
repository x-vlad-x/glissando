<div align="center">

<img src="data/icons/io.github.x_vlad_x.Glissando.svg" width="96" alt="">

# glissando

**A private, offline on-screen keyboard for Phosh**

Glide typing · prediction & auto-correction · multilingual · emoji · clipboard · local voice typing

[![CI](https://github.com/x-vlad-x/glissando/actions/workflows/ci.yml/badge.svg)](https://github.com/x-vlad-x/glissando/actions/workflows/ci.yml)
[![License: GPL v3+](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)
![Platform: Phosh / Wayland](https://img.shields.io/badge/platform-Phosh%20%2F%20Wayland-4a86cf)

</div>

glissando brings the comfort of a modern phone keyboard to Linux phones —
glide typing, smart suggestions, cursor control on the space bar — while
staying fully on the device. It has **no network access at all**: no
telemetry, no accounts, no cloud.

## Supported systems

| Tier | Systems | Meaning |
|---|---|---|
| Supported | FuriOS on the FuriPhone FLX1s | Tested before each release; bugs block the release |
| May work | Mobian, postmarketOS, PureOS and other Phosh systems | Not verified; fixes are best effort |

A system moves to *Supported* only after someone runs the manual device checks
and the package install, upgrade and removal cycle on it. Phosh is the only
supported desktop. Open release work is listed in
[docs/PROGRESS.md](docs/PROGRESS.md).

| | | |
|:-:|:-:|:-:|
| ![Glide typing, dark theme](docs/screenshots/dark-glide.png) | ![Suggestions, light theme](docs/screenshots/light-typing.png) | ![Russian layout](docs/screenshots/dark-russian.png) |
| Glide typing | Suggestions | Russian layout |
| ![Emoji panel](docs/screenshots/light-emoji.png) | ![Settings](docs/screenshots/settings.png) | |
| Emoji with search | Settings app | |

## Features

**Typing**
- Glide (swipe) typing with a fading trail — on recorded English gestures,
  87.7 % decoded as the top suggestion and 95.1 % within the top three
- Word suggestions, next-word prediction and auto-correction that adapts to
  your touch; backspace right after a correction undoes it
- Multilingual typing: English, German and Russian; enabled Latin layouts
  work at once with automatic language detection
- Learns new words on the device; personal dictionary you can review and edit
- Auto-capitalisation, double-space period, smart punctuation

**Gestures & keys**
- Slide on the space bar to move the cursor, slide left from backspace to
  delete whole words, optional trackpad mode
- Long-press for accented letters, digits and symbols; key pop-ups;
  multitouch rollover; shift chording and caps lock
- Number row, one-handed mode, adjustable height, portrait and landscape
- Layouts for email and URL fields, number/phone/PIN pads
- Terminal keys (Esc, Tab, Ctrl, Alt, arrows) in terminal apps
- Choose apps where the keyboard does not open by itself (e.g. Calculator)
- Long press on the Phosh home bar shows and hides the keyboard

**More**
- Emoji panel with categories, recents and search (EN/DE/RU keywords),
  emoji suggestions while typing
- Clipboard history with retention time and pinned items
- Local voice typing with whisper.cpp (optional package)
- Light and dark themes following the system, haptic feedback with adjustable
  strength via feedbackd
- Settings app (GTK4/libadwaita) in English, German and Russian

## Privacy

- No networking code; the keyboard service runs with the network forbidden by
  the kernel (`RestrictAddressFamilies=AF_UNIX AF_NETLINK`).
- Nothing you type is logged, unless you turn on the touch log meant for test
  sessions. Password and PIN fields get no prediction or auto-correction and
  are never learned from.
- Learned words, recent emoji and pinned clips stay in your home directory and
  can be deleted from the settings.
- Voice typing records into memory and transcribes on the phone. The file
  handed to the speech engine has no name and ends with the voice helper.

Automated checks cover this: the end-to-end test suite runs with a socket guard
that fails on any network attempt from the keyboard, and again inside an empty
network namespace. Details: [docs/PRIVACY.md](docs/PRIVACY.md).

## Install on FuriOS / Phosh

The current version is the release candidate **1.0.0-rc.1**. Please report
any problem in the [issues](https://github.com/x-vlad-x/glissando/issues).

### From the APT repository (recommended)

The packages are published in the signed APT repository
[deb.xvlad.com](https://deb.xvlad.com), so updates arrive with
`apt upgrade`. On the phone:

```bash
sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://deb.xvlad.com/xvlad.gpg | sudo tee /etc/apt/keyrings/xvlad.gpg >/dev/null
curl -fsSL https://deb.xvlad.com/xvlad.sources | sudo tee /etc/apt/sources.list.d/xvlad.sources >/dev/null
sudo apt update
sudo apt install glissando glissando-voice   # glissando-voice is optional
```

Check the key before trusting it: `gpg --show-keys /etc/apt/keyrings/xvlad.gpg`
must print the fingerprint `3AEF 2C40 39D6 4C00 8CBA 5F03 2850 6009 1652 C539`
(also published at <https://deb.xvlad.com/fingerprint.txt>). The key is
attached to this repository only through `Signed-By` and is not trusted for
any other source. The packages themselves are downloaded from the GitHub
releases of this project; apt checks each file against the signed index.

Then make glissando the active keyboard:

```bash
sudo update-alternatives --set Phosh-OSK /usr/share/glissando/io.github.x_vlad_x.Glissando.desktop
systemctl --user daemon-reload && systemctl --user restart mobi.phosh.OSK.service
```

### From downloaded packages

Download the `.deb` files from the
[releases page](https://github.com/x-vlad-x/glissando/releases) on the phone,
then, in that directory, install them and switch as above:

```bash
sudo apt install ./glissando_*_arm64.deb ./glissando-voice_*_arm64.deb   # voice is optional
```

Installing does not change your keyboard by itself; the `update-alternatives`
command does. To go back to Stevia:

```bash
sudo update-alternatives --set Phosh-OSK /usr/share/phosh-osk-stevia/mobi.phosh.Stevia.desktop
systemctl --user daemon-reload && systemctl --user restart mobi.phosh.OSK.service
```

If glissando fails three times within five minutes, its launcher starts Stevia
or Squeekboard instead. Prefer to try it without root? See the trial mode in
[docs/DEVICE.md](docs/DEVICE.md).

## Updating

With the APT repository, updates come with the rest of the system:

```bash
sudo apt update && sudo apt upgrade
systemctl --user restart mobi.phosh.OSK.service
```

Already installed glissando from downloaded packages? Add the repository as
above; `sudo apt update && sudo apt upgrade` then brings the newer version.
Without the repository, download both new packages into an otherwise empty
directory and run `sudo apt install ./glissando_*_arm64.deb
./glissando-voice_*_arm64.deb` there.

`glissando-voice` needs the exact same version of `glissando`, so update them
together; apt does this by itself. Your keyboard choice, settings, learned
words and pinned clips stay. To remove the repository later:
`sudo rm /etc/apt/sources.list.d/xvlad.sources /etc/apt/keyrings/xvlad.gpg && sudo apt update`.

## Removing

Switch back to the system keyboard first (see above), then:

```bash
sudo apt remove glissando-voice glissando
```

Removing `glissando` also removes its Phosh-OSK alternative. The packages never
touch your home directory, so your settings and personal data stay. To delete
them too:

```bash
rm -r ~/.config/glissando ~/.local/share/glissando ~/.local/state/glissando
```

## Known limitations

- Phosh only. Other desktops are not supported.
- Packages exist for arm64 only.
- No Flatpak: a sandboxed client cannot see the Wayland protocols a keyboard
  depends on.
- English, German and Russian only. Russian is not mixed with the Latin
  languages: switch layouts to change between them.
- Not in 1.0: split layout, the FUTO neural glide backend, new languages and
  layouts, and partial damage rendering. These can return in a later 1.x
  release.
- The launcher's fallback only knows Stevia and Squeekboard.
- Keeping the keyboard closed in chosen apps needs a compositor with
  wlr-foreign-toplevel.

## Building

```bash
cargo build --release
cargo test
python3 tools/build-dicts.py        # dictionaries (downloads pinned word lists once)
packaging/cross/build-arm64.sh      # aarch64 build in a rootless podman container
packaging/debian/build-deb.sh       # .deb packages
```

End-to-end tests run against a real headless Wayland compositor:

```bash
tools/devenv/fetch-sway.sh
dbus-run-session -- python3 tests/e2e/e2e.py
tests/privacy/check.sh
```

Both run in CI. `fetch-sway.sh` works on Arch-based hosts only. The voice test
reports `SKIP` unless its model and test recording are in `.devtools/voice` and
`packaging/voice/build-whisper-host.sh` has been run.

Architecture overview: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) ·
FuriOS input stack: [docs/research/furios.md](docs/research/furios.md) ·
Reference behaviour: [docs/research/reference-behaviour.md](docs/research/reference-behaviour.md)

## Contributing

Bug reports and translation fixes are very welcome. See
[CONTRIBUTING.md](CONTRIBUTING.md). Layouts are small TOML files, described in
[docs/ADDING-A-LAYOUT.md](docs/ADDING-A-LAYOUT.md); new languages are outside
the scope of 1.0. To report a security problem, see [SECURITY.md](SECURITY.md).

## Acknowledgements

- Word lists: Android Open Source Project (Apache-2.0)
- Word pair statistics: [Leipzig Corpora Collection](https://wortschatz.uni-leipzig.de) (CC BY 4.0)
- Emoji data and names: Unicode emoji and CLDR (Unicode License v3)
- Glide evaluation data: [FUTO swipe dataset](https://huggingface.co/datasets/futo-org/swipe.futo.org) (MIT)
- Voice typing: [whisper.cpp](https://github.com/ggml-org/whisper.cpp) and the Whisper models (MIT)
- Thanks to the Phosh, Squeekboard and Stevia projects for the input method
  infrastructure glissando plugs into.

## Licence

GPL-3.0-or-later. See [LICENSE](LICENSE) and the data licences in
`data/*/` and `data/dict/SOURCES.md`.

---

### По-русски

glissando — экранная клавиатура для Phosh, работающая
полностью на устройстве: ввод свайпом, подсказки и автоисправление, русская,
английская и немецкая раскладки с одновременным вводом на нескольких языках,
эмодзи, буфер обмена, локальный голосовой ввод. Никакой сети, телеметрии и
учётных записей. Поддерживается FuriOS на FuriPhone FLX1s; на других системах
с Phosh клавиатура может работать, но это не проверено. Другие среды и Flatpak
не поддерживаются. Установка, обновление и удаление — выше, в разделах
«Install», «Updating» и «Removing»; ограничения — в разделе «Known limitations».
Настройки и интерфейс переведены на русский.
