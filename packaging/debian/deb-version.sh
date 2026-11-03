#!/usr/bin/env bash
# Prints the Debian version for a SemVer version: 1.0.0-rc.1 -> 1.0.0~rc1-1,
# so that a release candidate sorts before the final release (1.0.0-1).
# The Debian revision (DEB_REVISION, default 1) is required by the APT
# repository at deb.xvlad.com, which expects <version>-<revision> packages.
set -euo pipefail
ver="${1:?usage: deb-version.sh SEMVER}"
rev="${DEB_REVISION:-1}"
if [[ ! "$ver" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
    echo "not a release version: $ver" >&2
    exit 1
fi
if [[ ! "$rev" =~ ^[0-9]+$ ]]; then
    echo "not a Debian revision: $rev" >&2
    exit 1
fi
base="${ver%%-*}"
if [ "$base" = "$ver" ]; then
    echo "$base-$rev"
else
    pre="${ver#*-}"
    echo "$base~${pre//./}-$rev"
fi
