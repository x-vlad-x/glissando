# Running glissando on the FuriPhone

The user-prefix setup below needs no root and touches no system files.
Everything lives in `~/.local/opt/glissando` on the phone. Only the Debian
packages are installed system-wide.

## Build and copy

```bash
packaging/cross/build-arm64.sh          # rootless podman, Debian forky arm64 libraries
python3 tools/build-dicts.py            # once: data/dict/*.gld
packaging/device/deploy.sh furios       # ssh host from ~/.ssh/config
```

## Try it (temporary)

On the phone (SSH or terminal):

```bash
~/.local/opt/glissando/bin/glissando-trial start 30   # 30 minutes
~/.local/opt/glissando/bin/glissando-trial stop       # back to the system keyboard now
```

`start` stops `mobi.phosh.OSK.service` and runs glissando as the transient
user unit `glissando-trial` with a run-time limit. Its `ExecStopPost`
restarts the system keyboard whenever glissando ends: stop, crash, timeout
or logout. Logs: `journalctl --user -u glissando-trial`.

## Use it as the keyboard (per user, reversible)

```bash
~/.local/opt/glissando/bin/glissando-session enable
~/.local/opt/glissando/bin/glissando-session disable  # restores Stevia/Squeekboard
```

`enable` writes one drop-in, `~/.config/systemd/user/mobi.phosh.OSK.service.d/glissando.conf`,
that points the existing keyboard service at `glissando-launch`. The
launcher starts the system keyboard instead if glissando crashed three
times within five minutes or is missing. Removing the file (from any shell,
also via SSH) restores the original behaviour.

## Voice typing

```bash
packaging/voice/build-whisper.sh        # whisper.cpp for arm64 (once)
packaging/device/deploy.sh --voice furios
```

Installs `whisper-cli` and the multilingual Whisper base model (q8_0, 82 MB,
checksum verified) into the user prefix. The microphone appears in the
suggestion strip; tap it, speak, and pause (or tap again). On the FLX1s an
11 s utterance is recognised in about 4 s. Recording uses the first of
`parecord`, `pw-record` and `arecord` that starts. The audio stays in memory and
in an unnamed private file in `$XDG_RUNTIME_DIR`, which ends with the helper.

## System-wide installation (Debian packages)

```bash
packaging/debian/build-deb.sh           # dist/glissando_*.deb, dist/glissando-voice_*.deb
```

On the phone:

```bash
sudo apt install ./glissando_*_arm64.deb ./glissando-voice_*_arm64.deb
# switch (takes effect after re-login, or restart the keyboard service):
sudo update-alternatives --set Phosh-OSK /usr/share/glissando/io.github.x_vlad_x.Glissando.desktop
systemctl --user daemon-reload && systemctl --user restart mobi.phosh.OSK.service
# back to Stevia:
sudo update-alternatives --set Phosh-OSK /usr/share/phosh-osk-stevia/mobi.phosh.Stevia.desktop
systemctl --user daemon-reload && systemctl --user restart mobi.phosh.OSK.service
```

The package registers a Phosh-OSK alternative with priority 20 (Stevia has
30), so installing it never changes the active keyboard by itself. The
system unit has the same network ban and crash fallback as the user setup.
The package cycle was run on a FuriPhone FLX1s: upgrade from 0.3.0 with the
selection and the user files unchanged, switching to Stevia and back, removal
while glissando was the active keyboard (the alternative fell back to Stevia
in auto mode) and installing again (Stevia stayed selected until the manual
switch).

To upgrade, install the new packages the same way, then restart the keyboard
service. The prerm script removes the alternative only on removal, and the
postinst script registers it again, so the selection stays. `glissando-voice`
depends on the exact same version of `glissando`.

To remove, switch back to the system keyboard first, then run
`sudo apt remove glissando-voice glissando`. The prerm script removes the
alternative. There is no postrm script: `~/.config/glissando`,
`~/.local/share/glissando` and `~/.local/state/glissando` stay until you delete
them.

To remove the user-prefix setup, run `glissando-session disable`, then delete
`~/.local/opt/glissando`, the settings entry in `~/.local/share/applications`
and the icon in `~/.local/share/icons/hicolor/scalable/apps`.

## Emergency rollback without a working keyboard

* via SSH: `~/.local/opt/glissando/bin/glissando-session disable` or `rm -r ~/.config/systemd/user/mobi.phosh.OSK.service.d && systemctl --user daemon-reload && systemctl --user restart mobi.phosh.OSK.service`
* with the packages, select the system keyboard with `update-alternatives`
  over SSH, as above.
* automatic: after three failures within five minutes the launcher starts
  Stevia or Squeekboard. An exit with status 0 or 143 does not count as a
  failure. Once the failures are older than five minutes, the next start tries
  glissando again. Checked on the FuriPhone FLX1s by killing glissando three
  times with `kill -ABRT`: systemd restarted it twice, then Stevia started.
  Use SIGABRT for this test: the Rust runtime catches the first SIGSEGV sent
  with `kill`, so that signal does not end the process.

## Verified on the device

| Check | Status |
|---|---|
| arm64 binary starts, binds globals, creates the layer surface | done (2026-10-26) |
| fractional scale 1.74, 413 px logical width | done |
| first full render 23–37 ms on the phone CPU | measured |
| seccomp socket restriction under systemd user units | done |
| typing through input-method-v2 with Stevia stopped (`glissando-trial`) | done by the user (2026-10-26), no errors in the journal |
| local voice pipeline (recording via PulseAudio under the network ban, whisper.cpp) | done with a recorded sample; live speech at the phone not yet tried |

## Manual device checks before a release

Run these on the supported system with the candidate package. Touches can
come from a finger or from a virtual touchscreen (`/dev/uinput`, needs root):
phoc treats it like the real screen, while a virtual *pointer* does not make
GTK 4 fields ask for the keyboard. `grim` cannot capture the lock screen.

- [ ] GTK 4 (GNOME Text Editor): typing, correction and its undo, glide, autocap
- [ ] Qt 6 (`QLineEdit`, `QTextEdit`): focus, cursor moved by touch, password
- [ ] Firefox and Chromium: text, password and textarea fields
- [ ] Electron (VSCodium flatpak with `--ozone-platform=wayland`)
- [ ] Flutter (FluffyChat): first letters stay
- [ ] Terminal (GNOME Console): terminal row, Tab, Ctrl+C, history
- [ ] Voice in English, German and Russian, with noise, cancel, focus loss
- [ ] Lock screen: keyboard hides, PIN unlock, nothing typed in the log
- [ ] Rotation with the keyboard open
- [ ] Suspend and resume, reboot, logout and login
- [ ] Learned words survive a stop of the keyboard service
- [ ] `journalctl` shows no crash, no `failed to register before timeout`

### Results for 1.0.0-rc.3 (2026-11-27)

FuriPhone FLX1s, FuriOS 14.0, phoc 0.58.0, phosh 0.58.0, GTK 4.24.0,
gnome-session 51.0; test build of `main` after #50 and #52, touches from a
virtual touchscreen.

| Check | Result |
|---|---|
| GTK 4 Text Editor | pass: typing, correction and undo, glide, autocap after a full stop (#7) |
| Qt 6 `QLineEdit` and password | pass: focus, cursor moved by touch, password neither suggested nor logged |
| Qt 6 `QTextEdit` | pass with a limit: Qt sends no surrounding text in the first commit of an empty `QTextEdit`, and phoc then never forwards it. Deletions go out as keys, so text stays correct, but backspace does not undo a correction there |
| Firefox, Chromium | pass: text, password (not logged), textarea with glide and correction |
| Electron (VSCodium 1.x, flatpak) | pass: command palette and editor, glide, correction, Enter |
| Flutter (FluffyChat, in headless sway) | pass after #50; `main` before it gave "ello old" for "hello world" |
| GNOME Console | pass: terminal row, Tab completion, Ctrl+C, history |
| Voice, synthetic speech through a virtual microphone | pass after #52: English and German with `auto`, Russian with the Russian layout, English with noise; cancel and focus loss insert nothing and leave no helper process or file |
| Lock screen | pass: the keyboard hides; Phosh's own keypad takes the numeric PIN, and phoc raises the keyboard only over fields with text input |
| Rotation | pass: 413×260 portrait, 918×192 landscape and back |
| Learned words after SIGTERM | pass: written, mode 0600 |
| Session registration | pass: no registration error in 3 days of journal |
| Package upgrade rc.1 → rc.2 from the APT repository | pass, keyboard choice kept (#21) |
| Suspend, resume, reboot | done by the user on 0.3.1~test (#8) |
| Logout and login | not run: it would end the user's running session |
| External display | not run: no display connected |

### Performance (#22)

| Measure | Result |
|---|---|
| Cold start to the first committed frame (`--show`, caches dropped, 5 runs) | 222–304 ms |
| Touch down to frame committed (30 letters) | median 3.4 ms, p90 19 ms, max 24 ms |
| Touch up to text sent to the app | median 2.3 ms, p90 12 ms, max 21 ms |
| Memory (RSS) | 36 MB after start, 60–70 MB after hours of use |
| CPU of the keyboard service | 40 s over 14 h 50 min of normal use |

Rendering a frame takes a few milliseconds and the idle keyboard uses no CPU,
so partial damage rendering is not needed for 1.0. Battery drain over an hour
of typing was not measured: the phone was on the charger, and the CPU figures
above bound the keyboard's share.
