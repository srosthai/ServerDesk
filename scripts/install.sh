#!/usr/bin/env bash
set -euo pipefail

REPO="srosthai/ServerDesk"
VERSION="latest"
PREFIX=""
BASE_URL="${SERVERDESK_BASE_URL:-https://github.com/${REPO}/releases/download}"
CURL_PROTO="${SERVERDESK_CURL_PROTO:-=https}"

usage() {
  cat <<'EOF'
Install ServerDesk from a GitHub Release.

Usage:
  install.sh [--version vX.Y.Z] [--prefix DIR]

Options:
  --version VERSION  Release tag to install. Defaults to the latest release.
  --prefix DIR       Install location. Defaults to ~/.local/bin on Linux
                     and ~/Applications on macOS.
  --help             Show this help.

The script downloads the release asset and SHA256SUMS, verifies the checksum,
and installs without requiring root.
EOF
}

die() {
  printf 'ServerDesk install: %s\n' "$*" >&2
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version|-v)
      [[ $# -ge 2 ]] || die "--version requires a value"
      VERSION="$2"
      shift 2
      ;;
    --prefix)
      [[ $# -ge 2 ]] || die "--prefix requires a value"
      PREFIX="$2"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      die "unknown option: $1"
      ;;
  esac
done

command -v curl >/dev/null 2>&1 || die "curl is required"

case "$(uname -s)" in
  Linux) OS="linux" ;;
  Darwin) OS="macos" ;;
  *) die "unsupported operating system: $(uname -s)" ;;
esac

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64) ARCH="x86_64" ;;
  arm64|aarch64) ARCH="aarch64" ;;
  *) die "unsupported architecture: $ARCH" ;;
esac

if [[ "$VERSION" == "latest" ]]; then
  VERSION="$(
    curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null \
      | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
      | head -n 1 \
      || true
  )"
  if [[ -z "$VERSION" ]]; then
    effective_url="$(
      curl -fsSL -o /dev/null -w '%{url_effective}' \
        "https://github.com/${REPO}/releases/latest" 2>/dev/null || true
    )"
    VERSION="${effective_url##*/}"
  fi
fi

[[ -n "$VERSION" && "$VERSION" != "latest" ]] || die "could not determine the latest release; pass --version v0.1.0"
[[ "$VERSION" == v* ]] || VERSION="v${VERSION}"
VERSION_NUMBER="${VERSION#v}"

if [[ "$OS" == "linux" ]]; then
  [[ "$ARCH" == "x86_64" ]] || die "Linux command install currently supports x86_64 only"
  ASSET="ServerDesk_${VERSION_NUMBER}_amd64.AppImage"
  INSTALL_DIR="${PREFIX:-${XDG_BIN_HOME:-$HOME/.local/bin}}"
  TARGET="${INSTALL_DIR}/serverdesk"
else
  ASSET="ServerDesk.app.zip"
  INSTALL_DIR="${PREFIX:-$HOME/Applications}"
  TARGET="${INSTALL_DIR}/ServerDesk.app"
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

download() {
  curl -fL --proto "$CURL_PROTO" --tlsv1.2 --retry 3 --retry-delay 2 \
    -o "$2" "$1"
}

printf 'Downloading ServerDesk %s for %s...\n' "$VERSION" "$OS"
download "${BASE_URL}/${VERSION}/${ASSET}" "${TMP_DIR}/${ASSET}"
download "${BASE_URL}/${VERSION}/SHA256SUMS" "${TMP_DIR}/SHA256SUMS"

EXPECTED="$(
  awk -v asset="$ASSET" '$2 == asset { print $1; exit }' "${TMP_DIR}/SHA256SUMS"
)"
[[ -n "$EXPECTED" ]] || die "SHA256SUMS does not contain ${ASSET}"

if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL="$(sha256sum "${TMP_DIR}/${ASSET}" | awk '{ print $1 }')"
else
  ACTUAL="$(shasum -a 256 "${TMP_DIR}/${ASSET}" | awk '{ print $1 }')"
fi

if [[ "$ACTUAL" != "$EXPECTED" ]]; then
  die "checksum mismatch for ${ASSET}"
fi

mkdir -p "$INSTALL_DIR"

if [[ "$OS" == "linux" ]]; then
  install -m 0755 "${TMP_DIR}/${ASSET}" "$TARGET"
  printf 'Installed ServerDesk at %s\n' "$TARGET"
  if ! command -v serverdesk >/dev/null 2>&1; then
    printf 'Add %s to PATH, then run: serverdesk\n' "$INSTALL_DIR"
  else
    printf 'Run: serverdesk\n'
  fi
else
  rm -rf "$TARGET"
  ditto -x -k "${TMP_DIR}/${ASSET}" "$INSTALL_DIR"
  printf 'Installed ServerDesk at %s\n' "$TARGET"

  if codesign --verify --deep --strict "$TARGET" >/dev/null 2>&1; then
    printf 'Code signature verified.\n'
  else
    printf 'Warning: this macOS build is unsigned. If Gatekeeper blocks it, open System Settings > Privacy & Security and allow ServerDesk, or use a signed release.\n'
  fi

  printf 'Open with: open "%s"\n' "$TARGET"
fi
