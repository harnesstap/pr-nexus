#!/usr/bin/env bash
# Commit the stamped release and push the tag as github-actions[bot].
# Tags pushed with GITHUB_TOKEN do not trigger workflows; release.yml listens
# for the Tag release workflow_run event as well as real tag pushes.
set -euo pipefail

version="${1:?version X.Y.Z is required}"
tag="v${version}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${root}"

if git rev-parse "${tag}" >/dev/null 2>&1; then
  echo "tag ${tag} already exists" >&2
  exit 1
fi

git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
git add -A -- Cargo.toml Cargo.lock CHANGELOG.md .changes
if git diff --cached --quiet; then
  echo "release commit would be empty" >&2
  exit 1
fi

git commit -m "Release ${tag}"
git tag "${tag}"
git push origin HEAD:main
git push origin "${tag}"
echo "pushed ${tag}"
