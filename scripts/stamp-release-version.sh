#!/usr/bin/env bash
# Write the release version into [workspace.package] version in the root
# Cargo.toml (Tauri reads it from there) and into app/package.json, then
# refresh Cargo.lock.
#
# Usage: scripts/stamp-release-version.sh <version>
# version may be "1.2.3" or "v1.2.3".
#
# STAMP_RELEASE_ROOT  directory that contains Cargo.toml and app/package.json
#                     (default: repo root)
# STAMP_SKIP_CARGO=1  rewrite the files only, skip `cargo update` (tests)
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

python3 - "$ROOT/app/package.json" "$VERSION" <<'PY'
import pathlib, re, sys
path, version = sys.argv[1], sys.argv[2]
file = pathlib.Path(path)
text = file.read_text()
# The top-level "version" is the only key indented by exactly two spaces.
new, count = re.subn(r'^(  "version": )"[^"]*"', rf'\g<1>"{version}"', text, count=1, flags=re.M)
if count != 1:
    raise SystemExit(f"no top-level version in {path}")
file.write_text(new)
PY

if [ "${STAMP_SKIP_CARGO:-}" = "1" ]; then
  exit 0
fi

cd "$ROOT"
cargo update --workspace
