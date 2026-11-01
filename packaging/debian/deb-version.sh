#!/usr/bin/env bash
# Prints the Debian version for a SemVer version: 1.0.0-rc.1 -> 1.0.0~rc1, so
# that a release candidate sorts before the final release.
set -euo pipefail
ver="${1:?usage: deb-version.sh SEMVER}"
if [[ ! "$ver" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
    echo "not a release version: $ver" >&2
    exit 1
fi
base="${ver%%-*}"
if [ "$base" = "$ver" ]; then
    echo "$base"
else
    pre="${ver#*-}"
    echo "$base~${pre//./}"
fi
