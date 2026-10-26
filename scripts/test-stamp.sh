#!/usr/bin/env bash
# Exercise the version stamp without touching the real workspace manifest.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
stamp="${root}/scripts/stamp-release-version.sh"
tmp="$(mktemp)"
trap 'rm -f "${tmp}"' EXIT

cat >"${tmp}" <<'EOF'
[workspace]
members = ["crates/example"]

[workspace.package]
version = "0.1.0"
edition = "2021"

[package]
name = "decoy"
version = "9.9.9"
EOF

"${stamp}" --skip-update --manifest "${tmp}" v1.2.3
python3 - "${tmp}" <<'PY'
import pathlib, sys
text = pathlib.Path(sys.argv[1]).read_text()
workspace = text.split("[package]", 1)[0]
package = text.split("[package]", 1)[1]
if 'version = "1.2.3"' not in workspace:
    raise SystemExit("workspace version was not stamped")
if 'version = "9.9.9"' not in package:
    raise SystemExit("a version outside [workspace.package] changed")
PY

"${stamp}" --skip-update --manifest "${tmp}" 1.2.3
python3 - "${tmp}" <<'PY'
import pathlib, sys
text = pathlib.Path(sys.argv[1]).read_text()
if text.count('version = "1.2.3"') != 1:
    raise SystemExit("stamping the same version was not idempotent")
PY

if "${stamp}" --skip-update --manifest "${tmp}" 01 >/dev/null 2>&1; then
  echo "invalid version was accepted" >&2
  exit 1
fi

echo "stamp script ok"
