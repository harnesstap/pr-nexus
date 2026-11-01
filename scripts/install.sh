#!/bin/sh
# Install PR Nexus on Linux from the latest GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/harnesstap/pr-nexus/main/scripts/install.sh | sh
#
# Picks the .deb (apt), the .rpm (dnf, yum or zypper), or falls back to the
# AppImage in ~/.local/bin with a desktop entry. Every download is checked
# against the release's SHA256SUMS. On macOS it points to Homebrew.
#
# Environment:
#   PR_NEXUS_VERSION   release to install, e.g. 0.1.1 (default: latest)
#   PR_NEXUS_METHOD    deb, rpm or appimage (default: detected)
#   PR_NEXUS_DRY_RUN   1 = download and verify, print the install commands, change nothing
#   PR_NEXUS_BASE_URL  where release files live (default: the GitHub release; for tests)
#   PR_NEXUS_OS, PR_NEXUS_ARCH, PR_NEXUS_OS_RELEASE  override detection (for tests)
set -eu

REPO="harnesstap/pr-nexus"
DRY_RUN="${PR_NEXUS_DRY_RUN:-0}"

say() { printf '%s\n' "pr-nexus: $*"; }
die() { printf '%s\n' "pr-nexus: error: $*" >&2; exit 1; }

run() {
  if [ "$DRY_RUN" = 1 ]; then
    say "would run: $*"
  else
    "$@"
  fi
}

as_root() {
  if [ "$(id -u)" = 0 ]; then
    run "$@"
  elif command -v sudo >/dev/null 2>&1; then
    run sudo "$@"
  else
    die "need root to run: $* (install sudo or rerun as root)"
  fi
}

fetch() {
  # fetch URL DEST
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$2" "$1"
  else
    die "curl or wget is required"
  fi
}

latest_version() {
  url="https://api.github.com/repos/$REPO/releases/latest"
  if command -v curl >/dev/null 2>&1; then
    body="$(curl -fsSL "$url")"
  else
    body="$(wget -q -O - "$url")"
  fi
  printf '%s\n' "$body" | sed -n 's/.*"tag_name": *"v\{0,1\}\([0-9][0-9.]*\)".*/\1/p' | head -n 1
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d ' ' -f 1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d ' ' -f 1
  else
    die "sha256sum or shasum is required"
  fi
}

os="${PR_NEXUS_OS:-$(uname -s)}"
arch="${PR_NEXUS_ARCH:-$(uname -m)}"

case "$os" in
  Darwin)
    say "on macOS, install with Homebrew:"
    say "  brew install --cask harnesstap/tap/pr-nexus"
    exit 0
    ;;
  Linux) ;;
  *) die "unsupported OS $os; on Windows, download the installer from https://github.com/$REPO/releases/latest" ;;
esac

case "$arch" in
  x86_64 | amd64) deb_arch=amd64 rpm_arch=x86_64 app_arch=amd64 ;;
  aarch64 | arm64) deb_arch=arm64 rpm_arch=aarch64 app_arch=aarch64 ;;
  *) die "unsupported architecture $arch (x86_64 and aarch64 only)" ;;
esac

method="${PR_NEXUS_METHOD:-}"
if [ -z "$method" ]; then
  os_release="${PR_NEXUS_OS_RELEASE:-/etc/os-release}"
  ids=""
  if [ -r "$os_release" ]; then
    # shellcheck disable=SC1090
    ids="$(. "$os_release" && printf '%s %s' "${ID:-}" "${ID_LIKE:-}")"
  fi
  case " $ids " in
    *" debian "* | *" ubuntu "*) method=deb ;;
    *" fedora "* | *" rhel "* | *" centos "* | *" suse "* | *" opensuse "* | *" sles "*) method=rpm ;;
    *) method=appimage ;;
  esac
  # Detected a family but its package tool is missing: fall back.
  if [ "$method" = deb ] && ! command -v apt-get >/dev/null 2>&1; then method=appimage; fi
  if [ "$method" = rpm ] && ! command -v dnf >/dev/null 2>&1 && ! command -v zypper >/dev/null 2>&1 && ! command -v yum >/dev/null 2>&1; then method=appimage; fi
fi

version="${PR_NEXUS_VERSION:-}"
if [ -z "$version" ]; then
  version="$(latest_version)"
  [ -n "$version" ] || die "could not find the latest release of $REPO"
fi
version="${version#v}"

case "$method" in
  deb) asset="PR-Nexus_${version}_${deb_arch}.deb" ;;
  rpm) asset="PR-Nexus-${version}-1.${rpm_arch}.rpm" ;;
  appimage) asset="PR-Nexus_${version}_${app_arch}.AppImage" ;;
  *) die "PR_NEXUS_METHOD must be deb, rpm or appimage, got $method" ;;
esac

base="${PR_NEXUS_BASE_URL:-https://github.com/$REPO/releases/download/v$version}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

say "installing PR Nexus $version ($method, $arch)"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || die "could not download $base/SHA256SUMS (Linux installers ship from v0.1.1 on)"
fetch "$base/$asset" "$tmp/$asset" || die "could not download $base/$asset"

want="$(awk -v f="$asset" '$2 == f || $2 == "*" f { print $1 }' "$tmp/SHA256SUMS")"
[ -n "$want" ] || die "$asset is not listed in SHA256SUMS"
got="$(sha256 "$tmp/$asset")"
[ "$want" = "$got" ] || die "checksum mismatch for $asset (expected $want, got $got)"
say "checksum ok: $asset"

case "$method" in
  deb)
    as_root apt-get install -y "$tmp/$asset"
    ;;
  rpm)
    if command -v dnf >/dev/null 2>&1; then
      as_root dnf install -y "$tmp/$asset"
    elif command -v zypper >/dev/null 2>&1; then
      as_root zypper --non-interactive install --allow-unsigned-rpm "$tmp/$asset"
    elif command -v yum >/dev/null 2>&1; then
      as_root yum install -y "$tmp/$asset"
    else
      as_root rpm -U --replacepkgs "$tmp/$asset"
    fi
    ;;
  appimage)
    bin="$HOME/.local/bin"
    apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    run mkdir -p "$bin" "$apps"
    run install -m 0755 "$tmp/$asset" "$bin/pr-nexus.AppImage"
    desktop="[Desktop Entry]
Type=Application
Name=PR Nexus
Comment=Track your open pull requests and request reviews
Exec=$bin/pr-nexus.AppImage
Icon=pr-nexus
Categories=Development;
Terminal=false"
    if [ "$DRY_RUN" = 1 ]; then
      say "would write $apps/pr-nexus.desktop"
    else
      printf '%s\n' "$desktop" > "$apps/pr-nexus.desktop"
    fi
    case ":$PATH:" in
      *":$bin:"*) ;;
      *) say "add $bin to your PATH to run pr-nexus.AppImage from a terminal" ;;
    esac
    say "the AppImage needs FUSE 2 (libfuse2); without it, run with APPIMAGE_EXTRACT_AND_RUN=1"
    ;;
esac

if [ "$DRY_RUN" = 1 ]; then
  say "dry run: nothing was installed"
else
  say "done. Tokens are stored with the Secret Service (GNOME Keyring or KWallet); start one if PR Nexus reports no keyring backend."
fi
