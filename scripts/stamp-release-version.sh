#!/usr/bin/env bash
# Write the release version into the one place it lives:
# [workspace.package] version in the root Cargo.toml. Then refresh Cargo.lock.
#
# Usage: scripts/stamp-release-version.sh <version>
# version may be "1.2.3" or "v1.2.3".
#
# STAMP_RELEASE_ROOT  directory that contains Cargo.toml (default: repo root)
# STAMP_SKIP_CARGO=1  rewrite the manifest only (tests)
set -euo pipefail

if [ "${1:-}" = "" ]; then
  echo "Usage: scripts/stamp-release-version.sh <version>" >&2
  exit 1
fi

VERSION="${1#v}"
if ! printf '%s' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "version must be MAJOR.MINOR.PATCH, got: $1" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${STAMP_RELEASE_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"

python3 - "$ROOT/Cargo.toml" "$VERSION" <<'PY'
import pathlib, sys
path, version = sys.argv[1], sys.argv[2]
file = pathlib.Path(path)
lines = file.read_text().splitlines(keepends=True)
out = []
in_workspace_package = False
replaced = False
for line in lines:
    stripped = line.strip()
    if stripped.startswith("[") and stripped.endswith("]"):
        in_workspace_package = stripped == "[workspace.package]"
    if in_workspace_package and not replaced and stripped.startswith("version"):
        newline = "\n" if line.endswith("\n") else ""
        line = f'version = "{version}"{newline}'
        replaced = True
    out.append(line)
if not replaced:
    raise SystemExit(f"no [workspace.package] version in {path}")
file.write_text("".join(out))
PY

if [ "${STAMP_SKIP_CARGO:-}" = "1" ]; then
  exit 0
fi

cd "$ROOT"
cargo update --workspace
