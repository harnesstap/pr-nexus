#!/usr/bin/env bash
# Resolve the release tag and open or reuse a draft GitHub release.
#
# Dry run (DRY_RUN=1) checks the workspace version, the asset-name contract,
# and the reseal script's non-macOS skip. It does not call the GitHub API.
#
# Real run expects CHANGIE_LATEST, EVENT_NAME, and (for workflow_dispatch)
# INPUT_TAG. GH_TOKEN must be able to create releases.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${PREPARE_RELEASE_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"
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

if [ "${DRY_RUN:-}" = "1" ]; then
  version="$(workspace_version)"
  bash scripts/check-release-assets.sh "$version" "PR-Nexus_${version}_aarch64.dmg"
  bash scripts/reseal-macos-app.sh "${TMPDIR:-/tmp}/pr-nexus-missing-bundle"
  write_output "dry_run=true"
  write_output "skip=true"
  write_output "tag="
  exit 0
fi

write_output "dry_run=false"

LATEST="${CHANGIE_LATEST:-}"
if [ -z "$LATEST" ] || [ "$LATEST" = "v0.0.0" ] || [ "$LATEST" = "0.0.0" ] || [ ! -f ".changes/${LATEST}.md" ]; then
  echo "No batched changelog for ${LATEST:-<empty>}; skipping."
  write_output "skip=true"
  write_output "tag="
  exit 0
fi

EVENT="${EVENT_NAME:?EVENT_NAME is required}"
if [ "$EVENT" = "push" ]; then
  TAG="${GITHUB_REF_NAME:?}"
  if [ "$TAG" != "$LATEST" ]; then
    echo "Tag $TAG does not match changie latest $LATEST"
    exit 1
  fi
elif [ "$EVENT" = "workflow_dispatch" ]; then
  TAG="${INPUT_TAG:-$LATEST}"
  if [ ! -f ".changes/${TAG}.md" ]; then
    echo "No batched changelog for $TAG; skipping."
    write_output "skip=true"
    write_output "tag="
    exit 0
  fi
else
  CREATED=""
  ARTIFACT_TAG=""
  RESULT="${TAG_RELEASE_RESULT:-}"
  if [ -n "$RESULT" ] && [ -f "$RESULT" ]; then
    CREATED="$(sed -n 's/^created=//p' "$RESULT")"
    ARTIFACT_TAG="$(sed -n 's/^tag=//p' "$RESULT")"
  fi
  if [ -n "$CREATED" ] && [ "$CREATED" != "true" ]; then
    echo "Tag release did not create a tag; skipping."
    write_output "skip=true"
    write_output "tag="
    exit 0
  fi
  TAG="${ARTIFACT_TAG:-$LATEST}"
  if [ ! -f ".changes/${TAG}.md" ]; then
    echo "No batched changelog for $TAG; skipping."
    write_output "skip=true"
    write_output "tag="
    exit 0
  fi
  if ! git rev-parse "$TAG" >/dev/null 2>&1; then
    echo "Tag $TAG does not exist; skipping."
    write_output "skip=true"
    write_output "tag="
    exit 0
  fi
  TAG_COMMIT="$(git rev-parse "$TAG^{commit}")"
  TRIGGER="${WORKFLOW_RUN_HEAD_SHA:-}"
  if [ -n "$CREATED" ]; then
    if [ "$TAG_COMMIT" != "$TRIGGER" ] && ! git merge-base --is-ancestor "$TRIGGER" "$TAG_COMMIT"; then
      echo "Tag $TAG ($TAG_COMMIT) is not this Tag release cut ($TRIGGER); skipping."
      write_output "skip=true"
      write_output "tag="
      exit 0
    fi
  fi
fi

EXPECTED="${TAG#v}"
ACTUAL="$(workspace_version)"
if [ "$ACTUAL" != "$EXPECTED" ]; then
  echo "Cargo.toml workspace version ($ACTUAL) does not match tag ($EXPECTED)"
  exit 1
fi

if IS_DRAFT="$(gh release view "$TAG" --repo "$GITHUB_REPOSITORY" --json isDraft --jq .isDraft 2>/dev/null)"; then
  echo "Reusing release $TAG (draft=${IS_DRAFT})."
else
  gh release create "$TAG" \
    --repo "$GITHUB_REPOSITORY" \
    --draft \
    --verify-tag \
    --title "$TAG" \
    --notes-file ".changes/${TAG}.md"
  echo "Created draft release $TAG."
fi

write_output "skip=false"
write_output "tag=$TAG"
