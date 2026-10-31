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
telemetry, no accounts, no cloud. Built for and tested on the
**FuriPhone FLX1s** running **FuriOS**. It may also work on other Phosh-based
systems (Mobian, postmarketOS, PureOS…), but these are not verified yet.

> **Preview release.** This is the first public version. It is already
> pleasant for daily typing, but expect rough edges — feedback is very
> welcome!

| | | |
|:-:|:-:|:-:|
| ![Glide typing, dark theme](docs/screenshots/dark-glide.png) | ![Suggestions, light theme](docs/screenshots/light-typing.png) | ![Russian layout](docs/screenshots/dark-russian.png) |
| Glide typing | Suggestions | Russian layout |
| ![Emoji panel](docs/screenshots/light-emoji.png) | ![Settings](docs/screenshots/settings.png) | |
| Emoji with search | Settings app | |

## Features

**Typing**
- Glide (swipe) typing with a fading trail — 87.7 % of real-world gestures
  decoded as the top suggestion, 95 % within the strip
- Word suggestions, next-word prediction and auto-correction that adapts to
  your touch; backspace right after a correction undoes it
- Multilingual typing: English, German and Russian; several Latin languages
  at once with automatic language detection
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
- Nothing you type is logged. Password and PIN fields get no suggestions and
  are never learned from.
- Learned words, recent emoji and pinned clips stay in your home directory and
  can be deleted from the settings.
- Voice typing records into memory and transcribes on the phone.

Automated checks prove it: the full end-to-end test suite runs with a socket
guard that fails on any network attempt, and again inside an empty network
namespace. Details: [docs/PRIVACY.md](docs/PRIVACY.md).

## Install on FuriOS / Phosh

Download the packages from the
[latest release](https://github.com/x-vlad-x/glissando/releases/latest) on the
phone, then:

```bash
sudo apt install ./glissando_*_arm64.deb ./glissando-voice_*_arm64.deb   # voice is optional
sudo update-alternatives --set Phosh-OSK /usr/share/glissando/io.github.x_vlad_x.Glissando.desktop
systemctl --user daemon-reload && systemctl --user restart mobi.phosh.OSK.service
```

Installing does not change your keyboard by itself; the second command does.
To go back to Stevia:

```bash
sudo update-alternatives --set Phosh-OSK /usr/share/phosh-osk-stevia/mobi.phosh.Stevia.desktop
systemctl --user daemon-reload && systemctl --user restart mobi.phosh.OSK.service
```

If glissando ever fails to start three times in a row, the system keyboard
starts instead, so you can't lock yourself out. Prefer to try it without
root? See the trial mode in [docs/DEVICE.md](docs/DEVICE.md).

## Building

```bash
cargo build --release
cargo test
python3 tools/build-dicts.py        # dictionaries (downloads public word lists once)
packaging/cross/build-arm64.sh      # aarch64 build in a rootless podman container
packaging/debian/build-deb.sh       # .deb packages
```

End-to-end tests run against a real headless Wayland compositor:

```bash
tools/devenv/fetch-sway.sh
dbus-run-session -- python3 tests/e2e/e2e.py
tests/privacy/check.sh
```

Architecture overview: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) ·
FuriOS input stack: [docs/research/furios.md](docs/research/furios.md) ·
Reference behaviour: [docs/research/reference-behaviour.md](docs/research/reference-behaviour.md)

## Contributing

Bug reports, layouts for more languages and translations are very welcome.
Layouts are small TOML files in [`data/layouts`](data/layouts).

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

glissando — экранная клавиатура для Phosh (FuriOS и другие), работающая
полностью на устройстве: ввод свайпом, подсказки и автоисправление, русская,
английская и немецкая раскладки с одновременным вводом на нескольких языках,
эмодзи, буфер обмена, локальный голосовой ввод. Никакой сети, телеметрии и
учётных записей. Установка — выше, в разделе «Install»; настройки и
интерфейс переведены на русский.
