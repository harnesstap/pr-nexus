#!/usr/bin/env bash
# Smoke-launch the packaged Linux build: start it under a virtual X display and
# check it is still running after a few seconds (no immediate crash, e.g. on a
# machine without a Secret Service).
#
# Usage: scripts/smoke-linux.sh <bundle-dir> [seconds]
# Installs the .deb when apt-get is usable, and also runs the AppImage.
set -euo pipefail

BUNDLE="${1:?usage: scripts/smoke-linux.sh <bundle-dir> [seconds]}"
SECONDS_UP="${2:-15}"

alive_for() {
  # alive_for <label> <cmd...>: run cmd under xvfb, require it to stay up.
  local label="$1"
  shift
  local log
  log="$(mktemp)"
  xvfb-run --auto-servernum "$@" >"$log" 2>&1 &
  local pid=$!
  sleep "$SECONDS_UP"
  if ! kill -0 "$pid" 2>/dev/null; then
    echo "::error::$label exited within ${SECONDS_UP}s"
    cat "$log"
    return 1
  fi
  echo "$label still running after ${SECONDS_UP}s"
  pkill -TERM -x pr-nexus || true
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  tail -n 20 "$log"
}

shopt -s nullglob
debs=("$BUNDLE"/deb/*.deb)
appimages=("$BUNDLE"/appimage/*.AppImage)
rpms=("$BUNDLE"/rpm/*.rpm)
[ ${#debs[@]} -gt 0 ] || { echo "::error::no .deb in $BUNDLE/deb"; exit 1; }
[ ${#appimages[@]} -gt 0 ] || { echo "::error::no .AppImage in $BUNDLE/appimage"; exit 1; }
[ ${#rpms[@]} -gt 0 ] || { echo "::error::no .rpm in $BUNDLE/rpm"; exit 1; }

# A throwaway HOME keeps the run away from any real config.
HOME="$(mktemp -d)"
export HOME

sudo DEBIAN_FRONTEND=noninteractive apt-get install -y "${debs[0]}"
alive_for ".deb install (/usr/bin/pr-nexus)" /usr/bin/pr-nexus

chmod +x "${appimages[0]}"
APPIMAGE_EXTRACT_AND_RUN=1 alive_for "AppImage" "${appimages[0]}"

rpm -qip "${rpms[0]}" 2>/dev/null | head -n 5 || true
echo "Linux smoke test passed."
