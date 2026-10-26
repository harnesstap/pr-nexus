#!/usr/bin/env bash
# Resolve the release tag for release.yml and append outputs for GITHUB_OUTPUT.
#
# Environment:
#   EVENT_NAME     push, workflow_dispatch, or workflow_run
#   REF_NAME       github.ref_name (tag pushes)
#   INPUT_TAG      workflow_dispatch tag input
#   INPUT_DRY_RUN  true when workflow_dispatch should not publish
#   RESULT_DIR     artifact directory containing a `tag` file (workflow_run)
#   GITHUB_OUTPUT
set -euo pipefail

event="${EVENT_NAME:?EVENT_NAME is required}"
dry_run="false"
# A workflow_dispatch dry run still checks out an existing tag.
# A Tag release dry run never pushes a tag, so the tree is not checked.
check_tree="true"
tag=""

case "${event}" in
  workflow_dispatch)
    tag="${INPUT_TAG:?tag input is required}"
    if [[ "${INPUT_DRY_RUN:-}" == "true" ]]; then
      dry_run="true"
    fi
    ;;
  push)
    tag="${REF_NAME:?REF_NAME is required}"
    ;;
  workflow_run)
    tag="$(tr -d '[:space:]' < "${RESULT_DIR:?RESULT_DIR is required}/tag")"
    if [[ -f "${RESULT_DIR}/dry_run" ]]; then
      flag="$(tr -d '[:space:]' < "${RESULT_DIR}/dry_run")"
      if [[ "${flag}" == "true" ]]; then
        dry_run="true"
        check_tree="false"
      fi
    fi
    ;;
  *)
    echo "unsupported event: ${event}" >&2
    exit 1
    ;;
esac

if [[ ! "${tag}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "tag must look like vX.Y.Z, got ${tag}" >&2
  exit 1
fi

{
  echo "tag=${tag}"
  echo "version=${tag#v}"
  echo "dry_run=${dry_run}"
  echo "check_tree=${check_tree}"
} >> "${GITHUB_OUTPUT:?GITHUB_OUTPUT is required}"
