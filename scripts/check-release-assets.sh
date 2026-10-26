#!/usr/bin/env bash
# v1 ships one installer: PR-Nexus_<version>_aarch64.dmg.
# Updater artifacts (.app.tar.gz, .sig) are not required yet.
#
# Usage: scripts/check-release-assets.sh <version> <asset-name>...
# version is MAJOR.MINOR.PATCH, with or without a leading v.
set -euo pipefail

if [ "${1:-}" = "" ]; then
  echo "Usage: scripts/check-release-assets.sh <version> <asset-name>..." >&2
  exit 1
fi

VERSION="${1#v}"
shift
if ! printf '%s' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "version must be MAJOR.MINOR.PATCH, got: $VERSION" >&2
  exit 1
fi

EXPECTED="PR-Nexus_${VERSION}_aarch64.dmg"
found=false
for name in "$@"; do
  if [ "$name" = "$EXPECTED" ]; then
    found=true
  fi
done

if [ "$found" != true ]; then
  echo "missing $EXPECTED" >&2
  exit 1
fi

echo "release assets ok: $EXPECTED"
