#!/usr/bin/env bash
# One-time, idempotent: create a self-signed code-signing identity
# "PR Nexus Dev" in the login keychain, used by
# scripts/macos-dev-codesign-runner.sh to sign dev builds when no
# "Apple Development" identity exists. Only an Apple-issued identity stops
# the keychain prompt after each rebuild; see CONTRIBUTING.md.
#
# Manual fallback: Keychain Access > Certificate Assistant > Create a
# Certificate..., Name "PR Nexus Dev", Identity Type "Self-Signed Root",
# Certificate Type "Code Signing".
set -euo pipefail

identity="${PR_NEXUS_DEV_SIGN_IDENTITY:-PR Nexus Dev}"
keychain="${HOME}/Library/Keychains/login.keychain-db"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "This script is for macOS only." >&2
  exit 1
fi

has_identity() {
  security find-identity -v -p codesigning "$keychain" 2>/dev/null | grep -Fq "\"$identity\""
}

if has_identity; then
  echo "Code-signing identity \"$identity\" already present; nothing to do."
  exit 0
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

cat >"$tmp/cert.cnf" <<CNF
[req]
distinguished_name = dn
prompt = no
x509_extensions = ext
[dn]
CN = $identity
[ext]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
subjectKeyIdentifier = hash
CNF

openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -config "$tmp/cert.cnf" -keyout "$tmp/key.pem" -out "$tmp/cert.pem" >/dev/null 2>&1

pass="pr-nexus-dev-$$"
# -legacy (OpenSSL 3) keeps the PKCS#12 readable by `security import`;
# LibreSSL (the macOS default openssl) has no -legacy flag.
p12_flags=()
if openssl pkcs12 -help 2>&1 | grep -q -- '-legacy'; then p12_flags=(-legacy); fi
openssl pkcs12 -export "${p12_flags[@]}" -name "$identity" \
  -inkey "$tmp/key.pem" -in "$tmp/cert.pem" -out "$tmp/id.p12" -passout "pass:$pass"

# -T /usr/bin/codesign: codesign may use the private key without a prompt.
security import "$tmp/id.p12" -k "$keychain" -P "$pass" -T /usr/bin/codesign

if ! has_identity; then
  echo "Trusting the certificate for code signing (macOS may ask for your password)..."
  security add-trusted-cert -r trustRoot -p codeSign -k "$keychain" "$tmp/cert.pem"
fi

if has_identity; then
  echo "OK: \"$identity\" is a valid code-signing identity:"
  security find-identity -v -p codesigning "$keychain" | grep -F "\"$identity\""
  cat <<NEXT

Next steps:
  1. bun run tauri dev   (the dev binary is now signed with "$identity")
  2. On the keychain prompt, enter your password and click "Always Allow".
     A self-signed identity has no Team ID, so macOS asks again after each
     rebuild. An "Apple Development" identity (free Apple ID in Xcode) stops
     that; the dev runner prefers it. See CONTRIBUTING.md.
  Opt out for one run: PR_NEXUS_DEV_SIGN=0 bun run tauri dev
NEXT
else
  echo "error: \"$identity\" not listed by 'security find-identity -v -p codesigning'." >&2
  echo "Use the manual fallback in CONTRIBUTING.md." >&2
  exit 1
fi
