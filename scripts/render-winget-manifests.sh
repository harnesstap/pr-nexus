#!/usr/bin/env bash
# Renders the WinGet manifests for a release from packaging/winget/*.tmpl.
# Usage: scripts/render-winget-manifests.sh <vX.Y.Z> <SHA256SUMS> <out-dir> [release-date]
# Writes <out-dir>/manifests/h/harnesstap/PRNexus/<X.Y.Z>/harnesstap.PRNexus*.yaml.
set -euo pipefail

[ "$#" -ge 3 ] || { echo "Usage: $0 <vX.Y.Z> <SHA256SUMS> <out-dir> [release-date]" >&2; exit 1; }
TAG="$1"
SUMS="$2"
OUT="$3"
RELEASE_DATE="${4:-$(date -u +%Y-%m-%d)}"
if ! [[ "$TAG" =~ ^v([0-9]+\.[0-9]+\.[0-9]+)$ ]]; then
  echo "not a stable vX.Y.Z tag: $TAG" >&2
  exit 1
fi
VERSION="${BASH_REMATCH[1]}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

sum_of() {
  local value
  value="$(awk -v f="$1" '$2 == f || $2 == "*" f { print toupper($1) }' "$SUMS")"
  [ -n "$value" ] || { echo "missing $1 in $SUMS" >&2; exit 1; }
  printf '%s' "$value"
}
SHA256_EXE="$(sum_of "PR-Nexus_${VERSION}_x64-setup.exe")"
SHA256_MSI="$(sum_of "PR-Nexus_${VERSION}_x64_en-US.msi")"

dest="$OUT/manifests/h/harnesstap/PRNexus/$VERSION"
mkdir -p "$dest"
for tmpl in "$ROOT"/packaging/winget/*.yaml.tmpl; do
  name="$(basename "$tmpl" .tmpl)"
  sed -e "s/\${VERSION}/$VERSION/g" \
    -e "s/\${RELEASE_DATE}/$RELEASE_DATE/g" \
    -e "s/\${SHA256_EXE}/$SHA256_EXE/g" \
    -e "s/\${SHA256_MSI}/$SHA256_MSI/g" \
    "$tmpl" > "$dest/$name"
done
# shellcheck disable=SC2016 # a literal ${ placeholder
if grep -qF '${' "$dest"/*.yaml; then
  echo "unrendered placeholder in $dest" >&2
  exit 1
fi
echo "$dest"
