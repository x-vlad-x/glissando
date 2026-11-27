#!/usr/bin/env bash
# Fetches sway/grim/wtype binaries for a nested test compositor without root.
# Packages are verified against the SHA256 recorded in the local pacman sync DB
# and unpacked into .devtools/ (gitignored). Arch/CachyOS hosts only.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
dest="$root/.devtools"
mkdir -p "$dest/pkgs" "$dest/root"
for pkg in sway grim wtype; do
    url="$(pacman -Sp "$pkg" | tail -1)"
    file="$dest/pkgs/$(basename "$url")"
    [ -f "$file" ] || curl -sfL --retry 6 --retry-all-errors --retry-delay 20 --connect-timeout 30 --speed-limit 1 --speed-time 60 -o "$file" "$url"
    name="$(basename "$file")"
    want=""
    for db in /var/lib/pacman/sync/*.db; do
        want="$(bsdtar -xOf "$db" --include='*/desc' 2>/dev/null | awk -v f="$name" '
            /^%FILENAME%$/ {getline; cur=$0}
            /^%SHA256SUM%$/ {getline; if (cur==f) {print; exit}}')" || true
        [ -n "$want" ] && break
    done
    have="$(sha256sum "$file" | cut -d' ' -f1)"
    if [ -z "$want" ] || [ "$want" != "$have" ]; then
        echo "checksum mismatch for $name (db: '$want', file: $have)" >&2
        exit 1
    fi
    bsdtar -xf "$file" -C "$dest/root" usr
    echo "ok $name"
done
