#!/usr/bin/env bash
# Resolve-tag cases that do not need GitHub.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
resolve="${root}/scripts/resolve-release-tag.sh"
out="$(mktemp)"
trap 'rm -f "${out}"' EXIT

run_case() {
  : >"${out}"
  GITHUB_OUTPUT="${out}" "$@" "${resolve}"
}

run_case env EVENT_NAME=push REF_NAME=v1.2.3
grep -qx 'tag=v1.2.3' "${out}"
grep -qx 'version=1.2.3' "${out}"
grep -qx 'dry_run=false' "${out}"
grep -qx 'check_tree=true' "${out}"

run_case env EVENT_NAME=workflow_dispatch INPUT_TAG=v0.1.0 INPUT_DRY_RUN=true
grep -qx 'dry_run=true' "${out}"
grep -qx 'check_tree=true' "${out}"
grep -qx 'version=0.1.0' "${out}"

dir="$(mktemp -d)"
trap 'rm -f "${out}"; rm -rf "${dir}"' EXIT
printf 'v2.0.0\n' >"${dir}/tag"
printf 'false\n' >"${dir}/dry_run"
run_case env EVENT_NAME=workflow_run RESULT_DIR="${dir}"
grep -qx 'tag=v2.0.0' "${out}"
grep -qx 'version=2.0.0' "${out}"
grep -qx 'dry_run=false' "${out}"
grep -qx 'check_tree=true' "${out}"

printf 'true\n' >"${dir}/dry_run"
run_case env EVENT_NAME=workflow_run RESULT_DIR="${dir}"
grep -qx 'tag=v2.0.0' "${out}"
grep -qx 'dry_run=true' "${out}"
grep -qx 'check_tree=false' "${out}"

if run_case env EVENT_NAME=push REF_NAME=not-a-tag >/dev/null 2>&1; then
  echo "invalid tag was accepted" >&2
  exit 1
fi

echo "resolve-release-tag ok"
