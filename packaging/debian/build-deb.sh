#!/usr/bin/env bash
# Build glissando_<ver>_arm64.deb and glissando-voice_<ver>_arm64.deb into dist/.
# Requires: packaging/cross/build-arm64.sh, data/dict/*.gld; for the voice
# package also packaging/voice/build-whisper.sh and the model in .devtools/voice.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
ver="$("$root/packaging/debian/deb-version.sh" "$(grep -m1 '^version' "$root/Cargo.toml" | cut -d'"' -f2)")"
bin="$root/target/aarch64-unknown-linux-gnu/release"
out="$root/dist"
mkdir -p "$out"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
deb="$root/packaging/debian"
image=glissando-cross:forky

# The arm64 tools and libraries live in the cross image, so the binaries are
# stripped and their dependencies computed there. Dependencies come from the
# libraries the binaries really use: a hand-written list lets the package
# install on a system too old to run it.
in_image() {
    podman run --rm -v "$work:$work:Z" -w "$work" "$image" "$@"
}
strip_binaries() {
    in_image aarch64-linux-gnu-strip --strip-unneeded --remove-section=.comment --remove-section=.note "$@"
}
# Files every package carries in its doc directory. The dates come from the
# last commit, so two builds of a commit produce the same files.
stamp="$(git -C "$root" log -1 --format=%ct)"
docs() {
    local name="$1" doc="$2/usr/share/doc/$1"
    install -Dm644 "$root/THIRD-PARTY.md" "$doc/THIRD-PARTY.md"
    gzip -9n -c "$root/CHANGELOG.md" > "$doc/NEWS.gz"
    {
        echo "$name ($ver) unstable; urgency=medium"
        echo
        echo "  * See /usr/share/doc/$name/NEWS.gz."
        echo
        echo " -- Vladislav Shadiuk <6992531+x-vlad-x@users.noreply.github.com>  $(date -u -R -d "@$stamp")"
    } | gzip -9n > "$doc/changelog.gz"
}
man_page() {
    install -d "$2/usr/share/man/man1"
    gzip -9n -c "$root/packaging/man/$1.1" > "$2/usr/share/man/man1/$1.1.gz"
}
mkdir -p "$work/debian"
printf 'Source: glissando\n\nPackage: glissando\nArchitecture: arm64\n' > "$work/debian/control"
shlib_deps() {
    local deps
    deps="$(in_image env CC=aarch64-linux-gnu-gcc dpkg-architecture -aarm64 -c dpkg-shlibdeps -O "$@" | sed -n 's/^shlibs:Depends=//p')"
    [ -n "$deps" ] || { echo "dpkg-shlibdeps found no dependencies for $*" >&2; exit 1; }
    echo "$deps"
}

# --- glissando ---------------------------------------------------------------
p="$work/debian/glissando"
install -Dm755 "$bin/glissando" "$p/usr/bin/glissando"
install -Dm755 "$bin/glissando-settings" "$p/usr/bin/glissando-settings"
install -Dm755 "$deb/glissando-launch" "$p/usr/libexec/glissando/glissando-launch"
mkdir -p "$p/usr/share/glissando/dict"
cp -r "$root/data/layouts" "$root/data/emoji" "$p/usr/share/glissando/"
cp "$root"/data/dict/*.gld "$p/usr/share/glissando/dict/"
install -Dm644 "$deb/io.github.x_vlad_x.Glissando.desktop" "$p/usr/share/glissando/io.github.x_vlad_x.Glissando.desktop"
install -Dm644 "$deb/mobi.phosh.OSK.service" "$p/usr/share/glissando/mobi.phosh.OSK.service"
install -Dm644 "$root/packaging/desktop/io.github.x_vlad_x.Glissando.Settings.desktop" "$p/usr/share/applications/io.github.x_vlad_x.Glissando.Settings.desktop"
install -Dm644 "$root/data/icons/io.github.x_vlad_x.Glissando.svg" "$p/usr/share/icons/hicolor/scalable/apps/io.github.x_vlad_x.Glissando.svg"
install -Dm644 "$root/README.md" "$p/usr/share/doc/glissando/README.md"
install -Dm644 "$root/docs/PRIVACY.md" "$p/usr/share/doc/glissando/PRIVACY.md"
install -Dm644 "$root/data/dict/SOURCES.md" "$p/usr/share/doc/glissando/DICTIONARY-SOURCES.md"
install -Dm644 "$root/data/emoji/LICENSE-UNICODE" "$p/usr/share/doc/glissando/LICENSE-UNICODE"
install -Dm644 "$deb/copyright" "$p/usr/share/doc/glissando/copyright"
install -Dm644 "$root/packaging/desktop/io.github.x_vlad_x.Glissando.metainfo.xml" "$p/usr/share/metainfo/io.github.x_vlad_x.Glissando.metainfo.xml"
docs glissando "$p"
man_page glissando "$p"
man_page glissando-settings "$p"
strip_binaries "$p/usr/bin/glissando" "$p/usr/bin/glissando-settings"
depends="$(shlib_deps -e"$p/usr/bin/glissando" -e"$p/usr/bin/glissando-settings")"
mkdir -p "$p/DEBIAN"
size=$(du -sk "$p/usr" | cut -f1)
cat > "$p/DEBIAN/control" <<CONTROL
Package: glissando
Version: $ver
Architecture: arm64
Maintainer: Vladislav Shadiuk <6992531+x-vlad-x@users.noreply.github.com>
Installed-Size: $size
Depends: $depends
Recommends: feedbackd, fonts-noto-color-emoji
Suggests: glissando-voice
Section: utils
Priority: optional
Homepage: https://github.com/x-vlad-x/glissando
Description: private on-screen keyboard for Phosh
 Glide typing, prediction and auto-correction, multilingual typing (English,
 German, Russian), emoji and clipboard panels. Works fully offline: no
 network access, no telemetry. Registers itself as a Phosh-OSK alternative
 without becoming the default.
CONTROL
cat > "$p/DEBIAN/postinst" <<'POSTINST'
#!/bin/sh
set -e
if [ "$1" = configure ]; then
    update-alternatives --install /usr/share/applications/sm.puri.OSK0.desktop Phosh-OSK \
        /usr/share/glissando/io.github.x_vlad_x.Glissando.desktop 20 \
        --slave /usr/lib/systemd/user/mobi.phosh.OSK.service Phosh-OSK.service \
        /usr/share/glissando/mobi.phosh.OSK.service
fi
POSTINST
cat > "$p/DEBIAN/prerm" <<'PRERM'
#!/bin/sh
set -e
if [ "$1" = remove ] || [ "$1" = deconfigure ]; then
    update-alternatives --remove Phosh-OSK /usr/share/glissando/io.github.x_vlad_x.Glissando.desktop
fi
PRERM
chmod 755 "$p/DEBIAN/postinst" "$p/DEBIAN/prerm"

# --- glissando-voice -----------------------------------------------------------
v="$work/debian/glissando-voice"
install -Dm755 "$bin/glissando-voice" "$v/usr/bin/glissando-voice"
install -Dm755 "$root/build/whisper-arm64/bin/whisper-cli" "$v/usr/libexec/glissando/whisper-cli"
install -Dm644 "$root/.devtools/voice/ggml-base-q8_0.bin" "$v/usr/share/glissando/voice/ggml-base-q8_0.bin"
install -Dm644 "$deb/copyright-voice" "$v/usr/share/doc/glissando-voice/copyright"
docs glissando-voice "$v"
man_page glissando-voice "$v"
strip_binaries "$v/usr/bin/glissando-voice" "$v/usr/libexec/glissando/whisper-cli"
depends="$(shlib_deps -e"$v/usr/bin/glissando-voice" -e"$v/usr/libexec/glissando/whisper-cli")"
mkdir -p "$v/DEBIAN"
size=$(du -sk "$v/usr" | cut -f1)
cat > "$v/DEBIAN/control" <<CONTROL
Package: glissando-voice
Version: $ver
Architecture: arm64
Maintainer: Vladislav Shadiuk <6992531+x-vlad-x@users.noreply.github.com>
Installed-Size: $size
Depends: $depends, pulseaudio-utils | pipewire-bin | alsa-utils, glissando (= $ver)
Section: utils
Priority: optional
Homepage: https://github.com/x-vlad-x/glissando
Description: local voice typing for the glissando keyboard
 Offline speech recognition with whisper.cpp and the multilingual Whisper
 base model. Audio never leaves the device.
CONTROL

for pkg in "$p" "$v"; do
    (cd "$pkg" && find usr -type f -print0 | sort -z | xargs -0 md5sum > DEBIAN/md5sums)
    podman run --rm -v "$work:$work:Z" -v "$out:$out:Z" "$image" \
        env SOURCE_DATE_EPOCH="$stamp" fakeroot dpkg-deb --root-owner-group -Zxz --build "$pkg" "$out" | sed 's/^/  /'
done
ls -la "$out"
