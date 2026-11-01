#!/usr/bin/env bash
# Checks that a release carries every installer, with the stable (space-free)
# asset names the upload steps produce:
#   macOS   PR-Nexus_<v>_aarch64.dmg
#   Linux   PR-Nexus_<v>_amd64.AppImage, PR-Nexus_<v>_amd64.deb, PR-Nexus-<v>-1.x86_64.rpm
#           PR-Nexus_<v>_aarch64.AppImage, PR-Nexus_<v>_arm64.deb, PR-Nexus-<v>-1.aarch64.rpm
#   Windows PR-Nexus_<v>_x64-setup.exe, PR-Nexus_<v>_x64_en-US.msi
# SHA256SUMS is added by the publish job after this check and is not required.
# Updater artifacts (.app.tar.gz, .sig) are not required yet.
#
# Usage: scripts/check-release-assets.sh <version> <asset-name>...
#        scripts/check-release-assets.sh --list <version>
# version is MAJOR.MINOR.PATCH, with or without a leading v.
set -euo pipefail

usage() {
  echo "Usage: scripts/check-release-assets.sh [--list] <version> <asset-name>..." >&2
  exit 1
}

LIST=false
if [ "${1:-}" = "--list" ]; then
  LIST=true
  shift
fi
[ "${1:-}" != "" ] || usage

VERSION="${1#v}"
shift
if ! printf '%s' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "version must be MAJOR.MINOR.PATCH, got: $VERSION" >&2
  exit 1
fi

EXPECTED=(
  "PR-Nexus_${VERSION}_aarch64.dmg"
  "PR-Nexus_${VERSION}_amd64.AppImage"
  "PR-Nexus_${VERSION}_amd64.deb"
  "PR-Nexus-${VERSION}-1.x86_64.rpm"
  "PR-Nexus_${VERSION}_aarch64.AppImage"
  "PR-Nexus_${VERSION}_arm64.deb"
  "PR-Nexus-${VERSION}-1.aarch64.rpm"
  "PR-Nexus_${VERSION}_x64-setup.exe"
  "PR-Nexus_${VERSION}_x64_en-US.msi"
)

if [ "$LIST" = true ]; then
  printf '%s\n' "${EXPECTED[@]}"
  exit 0
fi

missing=0
for want in "${EXPECTED[@]}"; do
  found=false
  for name in "$@"; do
    if [ "$name" = "$want" ]; then
      found=true
      break
    fi
  done
  if [ "$found" != true ]; then
    echo "missing $want" >&2
    missing=1
  fi
done
[ "$missing" -eq 0 ] || exit 1

echo "release assets ok: ${EXPECTED[*]}"
