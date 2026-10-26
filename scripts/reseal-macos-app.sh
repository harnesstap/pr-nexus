#!/usr/bin/env bash
# Ad-hoc seal the macOS app and rebuild the DMG.
# Gatekeeper on Apple Silicon rejects an unsigned arm64 bundle. v1 has no
# Apple Developer ID, so the seal is `codesign --force --deep --sign -`
# (docs/SPEC.md §9 and §11). Notarization is intentionally absent.
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "reseal-macos-app.sh runs on macOS" >&2
  exit 1
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(python3 "${root}/scripts/workspace_version.py" read --manifest "${root}/Cargo.toml")"
# Workspace target directory from `cargo metadata`, not app/src-tauri/target.
app="${root}/target/release/bundle/macos/PR-Nexus.app"
dmg_dir="${root}/target/release/bundle/dmg"
dmg="${dmg_dir}/PR-Nexus_${version}_aarch64.dmg"

if [[ ! -d "${app}" ]]; then
  echo "missing app bundle: ${app}" >&2
  exit 1
fi

codesign --force --deep --sign - "${app}"
codesign --verify --verbose=2 "${app}"

stage="$(mktemp -d)"
trap 'rm -rf "${stage}"' EXIT
cp -R "${app}" "${stage}/"
ln -s /Applications "${stage}/Applications"

mkdir -p "${dmg_dir}"
rm -f "${dmg_dir}"/*.dmg
hdiutil create \
  -volname "PR-Nexus" \
  -srcfolder "${stage}" \
  -ov \
  -format UDZO \
  "${dmg}"

echo "ad-hoc sealed ${dmg}"
