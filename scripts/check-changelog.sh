#!/usr/bin/env bash
# Require a Changie fragment on pull requests and merge-queue builds.
# Skips when the PR has the no-changelog label.
#
# Environment:
#   EVENT_NAME       pull_request or merge_group
#   BASE_SHA         commit the diff starts from
#   LABELS           newline-separated label names (pull_request)
#   HEAD_REF         merge queue head ref, used to recover the PR number
#   GITHUB_REPOSITORY
set -euo pipefail

event_name="${EVENT_NAME:?EVENT_NAME is required}"
base_sha="${BASE_SHA:?BASE_SHA is required}"
labels="${LABELS:-}"

if [[ "${event_name}" == "merge_group" ]]; then
  pr="$(printf '%s' "${HEAD_REF:-}" | sed -n 's|.*/pr-\([0-9][0-9]*\)-.*|\1|p')"
  if [[ -n "${pr}" ]]; then
    labels="$(gh api "repos/${GITHUB_REPOSITORY}/pulls/${pr}" --jq '[.labels[].name] | join("\n")')"
  fi
fi

if printf '%s\n' "${labels}" | grep -qx 'no-changelog'; then
  echo "no-changelog label set; fragment not required"
  exit 0
fi

files="$(git diff --name-only "${base_sha}" HEAD -- .changes/unreleased | grep -v '/\.gitkeep$' || true)"
if [[ -z "${files}" ]]; then
  echo "No Changie fragment under .changes/unreleased." >&2
  echo "Add one with changie, or label the PR no-changelog." >&2
  exit 1
fi

echo "Changie fragments:"
printf '%s\n' "${files}"
