#!/usr/bin/env bash
# Run lintian over the packages built by build-deb.sh. Needs a cross image built
# from the current packaging/cross/Containerfile.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
ver="$("$root/packaging/debian/deb-version.sh" "$(grep -m1 '^version' "$root/Cargo.toml" | cut -d'"' -f2)")"
podman run --rm -v "${root}/dist:/dist:ro" glissando-cross:forky \
    lintian --allow-root --fail-on error --info --display-info \
    "/dist/glissando_${ver}_arm64.deb" "/dist/glissando-voice_${ver}_arm64.deb"
