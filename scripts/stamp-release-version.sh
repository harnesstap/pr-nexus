#!/usr/bin/env bash
# Write [workspace.package] version and refresh Cargo.lock.
# Usage: stamp-release-version.sh [--skip-update] [--manifest PATH] VERSION
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
skip_update=0
manifest="${root}/Cargo.toml"
version=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-update)
      skip_update=1
      shift
      ;;
    --manifest)
      manifest="${2:?--manifest requires a path}"
      shift 2
      ;;
    -h | --help)
      echo "usage: stamp-release-version.sh [--skip-update] [--manifest PATH] VERSION"
      exit 0
      ;;
    --)
      shift
      break
      ;;
    -*)
      echo "unknown flag: $1" >&2
      exit 2
      ;;
    *)
      if [[ -n "${version}" ]]; then
        echo "unexpected argument: $1" >&2
        exit 2
      fi
      version="$1"
      shift
      ;;
  esac
done

if [[ -z "${version}" ]]; then
  echo "version is required" >&2
  exit 2
fi

if [[ "${skip_update}" -eq 0 && "${manifest}" != "${root}/Cargo.toml" ]]; then
  echo "--manifest other than the workspace Cargo.toml requires --skip-update" >&2
  exit 2
fi

python3 "${root}/scripts/workspace_version.py" write "${version}" --manifest "${manifest}"

if [[ "${skip_update}" -eq 0 ]]; then
  (cd "${root}" && cargo update --workspace)
fi
