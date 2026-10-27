#!/usr/bin/env bash
# Render Casks/pr-nexus.rb for harnesstap/homebrew-tap from a published release.
#
# Usage: scripts/render-homebrew-cask.sh <tag> <assets.json> <tap-dir>
#
# assets.json is `gh release view <tag> --json assets`. The script picks the one
# asset named *_<version>_aarch64.dmg and uses the sha256 digest GitHub
# recorded for it, so the DMG file name comes from the release, not from here.
# The app name comes from productName in app/src-tauri/tauri.conf.json.
# It writes nothing when the tap already carries the same or a newer version,
# so a backfill of an older tag never downgrades the tap.
#
# RENDER_CASK_ROOT  directory with packaging/ and app/ (default: repo root)
# CASK_APP_NAME     override the app name (tests)
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "Usage: scripts/render-homebrew-cask.sh <tag> <assets.json> <tap-dir>" >&2
  exit 1
fi

TAG="$1"
ASSETS_JSON="$2"
TAP_DIR="$3"
VERSION="${TAG#v}"
if ! printf '%s' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "Homebrew cask: $TAG is not a stable vX.Y.Z tag." >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${RENDER_CASK_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"
TEMPLATE="$ROOT/packaging/homebrew/pr-nexus.rb.tmpl"
CASK="$TAP_DIR/Casks/pr-nexus.rb"

if [ -f "$CASK" ]; then
  current="$(sed -n 's/^  version "\([0-9.]*\)"$/\1/p' "$CASK" | head -n 1)"
  if [ -n "$current" ] && [ "$(printf '%s\n%s\n' "$current" "$VERSION" | sort -V | tail -n 1)" = "$current" ]; then
    echo "Homebrew tap already at pr-nexus $current; not rendering $VERSION."
    exit 0
  fi
fi

APP_NAME="${CASK_APP_NAME:-$(jq -r '.productName // empty' "$ROOT/app/src-tauri/tauri.conf.json")}"
if ! printf '%s' "$APP_NAME" | grep -Eq '^[A-Za-z0-9][A-Za-z0-9 ._-]*$'; then
  echo "Homebrew cask: unexpected app name '$APP_NAME'." >&2
  exit 1
fi

suffix="_${VERSION}_aarch64.dmg"
mapfile -t matches < <(jq -r --arg s "$suffix" '.assets[] | select(.name | endswith($s)) | "\(.name)\t\(.digest // "")"' "$ASSETS_JSON")
if [ "${#matches[@]}" -ne 1 ]; then
  echo "Homebrew cask: expected one *${suffix} asset, found ${#matches[@]}." >&2
  exit 1
fi
DMG_NAME="${matches[0]%%$'\t'*}"
DIGEST="${matches[0]#*$'\t'}"
SHA256="${DIGEST#sha256:}"
if [ "$DIGEST" = "$SHA256" ] || ! printf '%s' "$SHA256" | grep -Eq '^[0-9a-f]{64}$'; then
  echo "Homebrew cask: $DMG_NAME has no sha256 digest (got '$DIGEST')." >&2
  exit 1
fi
if ! printf '%s' "$DMG_NAME" | grep -Eq '^[A-Za-z0-9._-]+$'; then
  echo "Homebrew cask: asset name '$DMG_NAME' must not need URL encoding." >&2
  exit 1
fi
DMG_FILE="${DMG_NAME%"$suffix"}_#{version}_aarch64.dmg"

mkdir -p "$TAP_DIR/Casks"
sed \
  -e "s|@VERSION@|${VERSION}|g" \
  -e "s|@SHA256@|${SHA256}|g" \
  -e "s|@DMG_FILE@|${DMG_FILE}|g" \
  -e "s|@APP_NAME@|${APP_NAME}|g" \
  "$TEMPLATE" > "$CASK"
echo "wrote Casks/pr-nexus.rb for $VERSION ($DMG_NAME, app \"$APP_NAME.app\")"
