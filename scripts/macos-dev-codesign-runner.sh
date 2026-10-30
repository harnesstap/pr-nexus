#!/usr/bin/env bash
# Cargo target runner for macOS (see .cargo/config.toml). `tauri dev` runs
# `cargo run`, so this wraps every dev launch.
#
# Signs the dev app binary with a stable identity so its designated
# requirement (identifier + certificate) does not change between rebuilds,
# then execs it. Never fails the run because of signing.
#
# Identity, in order: $PR_NEXUS_DEV_SIGN_IDENTITY, else the first valid
# "Apple Development:" identity, else "PR Nexus Dev" (self-signed).
# Only an Apple-issued identity carries a Team ID; the keychain's partition
# list is keyed on it, so only then does "Always Allow" survive rebuilds.
# A self-signed identity gets a cdhash partition entry, which every rebuild
# invalidates. See CONTRIBUTING.md, "macOS dev: stop keychain prompts".
set -u

bin="${1:?usage: $0 <binary> [args...]}"

# Only the app binary; tests, examples and other bins run untouched.
if [ "$(basename -- "$bin")" = "pr-nexus" ] && [ "$(uname -s)" = "Darwin" ] \
  && [ "${PR_NEXUS_DEV_SIGN:-1}" != "0" ]; then
  identities=$(security find-identity -v -p codesigning 2>/dev/null || true)
  if [ -n "${PR_NEXUS_DEV_SIGN_IDENTITY:-}" ]; then
    identity="$PR_NEXUS_DEV_SIGN_IDENTITY"
    found=$(printf '%s\n' "$identities" | grep -F -e "\"$identity\"" -e " $identity " | head -n1)
  else
    found=$(printf '%s\n' "$identities" | grep -F '"Apple Development: ' | head -n1)
    if [ -n "$found" ]; then
      # Sign by SHA-1: several Apple Development identities may share a prefix.
      identity=$(printf '%s\n' "$found" | awk '{print $2}')
    else
      identity="PR Nexus Dev"
      found=$(printf '%s\n' "$identities" | grep -F "\"$identity\"" | head -n1)
    fi
  fi

  if [ -z "$found" ]; then
    printf 'note: no "%s" code-signing identity; macOS will ask for keychain access on every rebuild. See CONTRIBUTING.md "macOS dev: stop keychain prompts" (PR_NEXUS_DEV_SIGN=0 silences this).\n' \
      "$identity" >&2
  elif ! out=$(codesign --force --sign "$identity" --identifier dev.pr-nexus "$bin" 2>&1); then
    printf 'warning: dev codesign with "%s" failed, running the linker-signed binary:\n%s\n' \
      "$identity" "$out" >&2
  elif ! codesign -dv "$bin" 2>&1 | grep -q '^TeamIdentifier=[A-Z0-9]\{10\}$'; then
    printf 'note: dev binary signed with a self-signed identity (no Team ID); the keychain still asks once per rebuild. An "Apple Development" identity stops that, see CONTRIBUTING.md.\n' >&2
  fi
fi

exec "$@"
