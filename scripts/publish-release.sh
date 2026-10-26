#!/usr/bin/env bash
# Publish a draft release after the expected asset is present.
# Re-running replaces nothing here; uploads use --clobber in the build job.
# --latest is set only when this version is the highest semver already released
# (including this draft).
#
# Environment: TAG, VERSION, GH_TOKEN, GITHUB_REPOSITORY
set -euo pipefail

tag="${TAG:?TAG is required}"
version="${VERSION:?VERSION is required}"
repo="${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"
expected="PR-Nexus_${version}_aarch64.dmg"

assets="$(gh release view "${tag}" --repo "${repo}" --json assets --jq '.assets[].name')"
if ! printf '%s\n' "${assets}" | grep -qx "${expected}"; then
  echo "missing release asset ${expected}" >&2
  echo "assets:" >&2
  printf '%s\n' "${assets}" >&2
  exit 1
fi

highest="$(
  gh release list --repo "${repo}" --limit 200 --json tagName --jq '.[].tagName' \
    | sed 's/^v//' \
    | sort -V \
    | tail -n 1
)"

if [[ -z "${highest}" || "${version}" == "${highest}" ]]; then
  gh release edit "${tag}" --repo "${repo}" --draft=false --latest
  echo "published ${tag} as latest"
else
  gh release edit "${tag}" --repo "${repo}" --draft=false --latest=false
  echo "published ${tag} without moving the latest marker"
fi
