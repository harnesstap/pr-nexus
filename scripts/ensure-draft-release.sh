#!/usr/bin/env bash
# Create a draft GitHub release or refresh its notes. Safe to re-run.
# Does not publish the release and does not upload assets.
#
# Environment: TAG (vX.Y.Z), VERSION (X.Y.Z), GH_TOKEN, GITHUB_REPOSITORY
set -euo pipefail

tag="${TAG:?TAG is required}"
version="${VERSION:?VERSION is required}"
repo="${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -f "${root}/.changes/v${version}.md" ]]; then
  notes="${root}/.changes/v${version}.md"
elif [[ -f "${root}/.changes/${version}.md" ]]; then
  notes="${root}/.changes/${version}.md"
else
  echo "missing release notes: .changes/v${version}.md" >&2
  exit 1
fi

cargo_version="$(python3 "${root}/scripts/workspace_version.py" read --manifest "${root}/Cargo.toml")"
if [[ "${cargo_version}" != "${version}" ]]; then
  echo "Cargo.toml version ${cargo_version} does not match ${version}" >&2
  exit 1
fi

title="PR-Nexus ${tag}"
if gh release view "${tag}" --repo "${repo}" >/dev/null 2>&1; then
  gh release edit "${tag}" --repo "${repo}" --title "${title}" --notes-file "${notes}"
  echo "updated draft notes for ${tag}"
else
  gh release create "${tag}" \
    --repo "${repo}" \
    --draft \
    --title "${title}" \
    --notes-file "${notes}" \
    --verify-tag
  echo "created draft ${tag}"
fi
