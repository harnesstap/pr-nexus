#!/usr/bin/env bash
# Decide whether Tag release should batch+stamp, only tag, or skip.
# Writes key=value lines to stdout and to $GITHUB_OUTPUT when set.
#
# The first cut has no .changes/v*.md, so batch uses v0.1.0.
# Later cuts use `changie batch auto`.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${TAG_RELEASE_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"
cd "$ROOT"

write_output() {
  echo "$1"
  if [ -n "${GITHUB_OUTPUT:-}" ]; then
    echo "$1" >> "$GITHUB_OUTPUT"
  fi
}

workspace_version() {
  awk '
    $0 == "[workspace.package]" { in_pkg = 1; next }
    in_pkg && /^\[/ { exit }
    in_pkg && $1 == "version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' Cargo.toml
}

has_unreleased=false
shopt -s nullglob
unreleased=(.changes/unreleased/*.yaml .changes/unreleased/*.yml)
if [ ${#unreleased[@]} -gt 0 ]; then
  has_unreleased=true
fi

batch_args=""
if [ "$has_unreleased" = true ]; then
  existing=(.changes/v*.md)
  if [ ${#existing[@]} -gt 0 ]; then
    batch_args="auto"
  else
    batch_args="v0.1.0"
  fi
fi

LATEST="${CHANGIE_LATEST:-}"
batched_version=""
if [ -n "$LATEST" ] && [ "$LATEST" != "v0.0.0" ] && [ "$LATEST" != "0.0.0" ] && [ -f ".changes/${LATEST}.md" ]; then
  batched_version="$LATEST"
fi

versions_match=false
if [ -n "$batched_version" ]; then
  expected="${batched_version#v}"
  cargo_version="$(workspace_version)"
  if [ "$cargo_version" = "$expected" ]; then
    versions_match=true
  fi
fi

action=skip
if [ "$has_unreleased" = true ]; then
  action=prepare
elif [ -n "$batched_version" ] && [ "$versions_match" = false ]; then
  action=prepare
elif [ -n "$batched_version" ] && [ "$versions_match" = true ]; then
  action=tag
fi

write_output "has_unreleased=$has_unreleased"
write_output "batch_args=$batch_args"
write_output "batched_version=$batched_version"
write_output "versions_match=$versions_match"
write_output "action=$action"
