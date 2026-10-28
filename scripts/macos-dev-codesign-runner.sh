#!/usr/bin/env bash
# Cargo target runner for macOS (see .cargo/config.toml).
# Signs the dev app binary with a stable self-signed identity so its
# designated requirement (identifier + leaf cert) does not change between
# rebuilds, then execs it. Never fails the run because of signing.
set -u

bin="${1:?usage: $0 <binary> [args...]}"
identity="${PR_NEXUS_DEV_SIGN_IDENTITY:-PR Nexus Dev}"

# Only the app binary; tests, examples and other bins run untouched.
if [ "$(basename -- "$bin")" = "pr-nexus" ] && [ "$(uname -s)" = "Darwin" ] \
  && [ "${PR_NEXUS_DEV_SIGN:-1}" != "0" ]; then
  if security find-identity -v -p codesigning 2>/dev/null | grep -Fq "\"$identity\""; then
    if ! out=$(codesign --force --sign "$identity" --identifier dev.pr-nexus "$bin" 2>&1); then
      printf 'warning: dev codesign with "%s" failed, running unsigned-stable binary:\n%s\n' \
        "$identity" "$out" >&2
    fi
  fi
fi

exec "$@"
