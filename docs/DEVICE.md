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
`apt-get install -s` of both packages was checked on the device. The real
install, upgrade and removal cycle has not been run on a device yet.

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
  glissando again. This has not been verified on a device yet.

## Verified on the device

| Check | Status |
|---|---|
| arm64 binary starts, binds globals, creates the layer surface | done (2026-10-26) |
| fractional scale 1.74, 413 px logical width | done |
| first full render 23–37 ms on the phone CPU | measured |
| seccomp socket restriction under systemd user units | done |
| typing through input-method-v2 with Stevia stopped (`glissando-trial`) | done by the user (2026-10-26), no errors in the journal |
| local voice pipeline (recording via PulseAudio under the network ban, whisper.cpp) | done with a recorded sample; live speech at the phone not yet tried |
