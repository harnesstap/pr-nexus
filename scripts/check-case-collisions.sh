#!/usr/bin/env bash
# Fail when two tracked files in the same directory would be the same module
# specifier on a case-insensitive filesystem (foo.ts vs Foo.tsx).
# That lets Vite on macOS resolve "./Foo" to foo.ts, which has no Foo export.
#
# Only JS/TS module extensions are compared. Cargo.toml vs Cargo.lock, or
# icon.png vs icon.ico, are not module specifiers and are ignored.
#
#   scripts/check-case-collisions.sh
#   scripts/check-case-collisions.sh --self-test
#
# GIT               git binary (default: git)
# CASE_CHECK_ROOT   repository root (default: this script's parent)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${CASE_CHECK_ROOT:-"$SCRIPT_DIR/.."}" && pwd)"
GIT="${GIT:-git}"
SCRIPT="$SCRIPT_DIR/$(basename "${BASH_SOURCE[0]}")"

usage() {
  echo "Usage: scripts/check-case-collisions.sh [--self-test]" >&2
  exit 1
}

if [ "${1:-}" = "--self-test" ]; then
  tmp="$(mktemp -d)"
  cleanup() { rm -rf "$tmp"; }
  trap cleanup EXIT
  git -C "$tmp" init -q
  git -C "$tmp" config user.email "dev@example.com"
  git -C "$tmp" config user.name "Dev"
  mkdir -p "$tmp/src"
  printf 'export const x = 1;\n' >"$tmp/src/reviewAge.ts"
  printf 'export function ReviewAge() { return null; }\n' >"$tmp/src/ReviewAge.tsx"
  printf 'export const ok = 1;\n' >"$tmp/src/reviewWait.ts"
  git -C "$tmp" add src
  git -C "$tmp" commit -qm "seed"
  if CASE_CHECK_ROOT="$tmp" bash "$SCRIPT"; then
    echo "expected a collision between reviewAge.ts and ReviewAge.tsx" >&2
    exit 1
  fi
  rm -f "$tmp/src/reviewAge.ts"
  git -C "$tmp" add -A
  git -C "$tmp" commit -qm "fix"
  CASE_CHECK_ROOT="$tmp" bash "$SCRIPT"
  echo "self-test ok"
  exit 0
fi

if [ "$#" -ne 0 ]; then
  usage
fi

python3 - "$ROOT" "$GIT" <<'PY'
import os
import subprocess
import sys
from collections import defaultdict

root, git = sys.argv[1], sys.argv[2]
# Extensions Vite and TypeScript resolve when an import omits the suffix.
MODULE_EXTS = {".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts"}

files = subprocess.check_output([git, "-C", root, "ls-files", "-z"]).split(b"\0")
paths = [f.decode() for f in files if f]
groups = defaultdict(list)
for path in paths:
    directory, name = os.path.split(path)
    _, ext = os.path.splitext(name)
    if ext.lower() not in MODULE_EXTS:
        continue
    stem = name[: -len(ext)] if ext else name
    groups[(directory, stem.casefold())].append(path)

collisions = [items for items in groups.values() if len(items) > 1]
if not collisions:
    print(f"No case-insensitive module collisions in {len(paths)} tracked files.")
    sys.exit(0)

print("These tracked files share a directory and a basename, ignoring case and extension.", file=sys.stderr)
print("On a case-insensitive filesystem, an extensionless import can resolve the wrong file.", file=sys.stderr)
for items in sorted(collisions):
    print(file=sys.stderr)
    for path in sorted(items):
        print(f"  {path}", file=sys.stderr)
sys.exit(1)
PY
