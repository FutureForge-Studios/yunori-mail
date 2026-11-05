#!/bin/sh
# Yunori Mail Server — bootstrap installer (served at https://get.yunori.email/mail).
#
#   curl -fsSL https://get.yunori.email/mail | sudo sh
#   curl -fsSL https://get.yunori.email/mail | sudo sh -s -- --domain example.com --admin-email you@gmail.com
#
# What it does, and nothing else:
#   1. checks this is a supported 64-bit Linux and you are root,
#   2. downloads ONE pinned yunori-mail release for your CPU,
#   3. verifies its SHA-256 (and its minisign signature when minisign is installed),
#   4. installs it to /usr/local/bin/yunori-mail and runs `yunori-mail install` with your arguments.
# It never pipes further remote code into a shell. Read it before running it.

set -eu

YUNORI_VERSION="${YUNORI_VERSION:-0.1.0}"
BASE_URL="${YUNORI_BASE_URL:-https://github.com/FutureForge-Studios/yunori-mail/releases/download/v${YUNORI_VERSION}}"
# Release signing key (minisign). Replaced by the real key at the first signed release.
MINISIGN_PUBKEY="${YUNORI_MINISIGN_PUBKEY:-}"

say() { printf '%s\n' "$*"; }
fail() { printf 'yunori-mail: %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" = "0" ] || fail "please run as root (pipe into: sudo sh)"
[ "$(uname -s)" = "Linux" ] || fail "Yunori Mail Server runs on Linux"

case "$(uname -m)" in
  x86_64 | amd64) ARCH="x86_64" ;;
  aarch64 | arm64) ARCH="aarch64" ;;
  *) fail "unsupported CPU $(uname -m) — use an x86-64 or ARM64 server" ;;
esac

if [ -r /etc/os-release ]; then
  # shellcheck disable=SC1091
  . /etc/os-release
  case "${ID:-}:${VERSION_ID:-}" in
    debian:12 | debian:13 | ubuntu:22.04 | ubuntu:24.04 | ubuntu:26.04) ;;
    *) say "Note: ${PRETTY_NAME:-this OS} is not a supported release; the installer will tell you what it finds." ;;
  esac
fi

command -v curl >/dev/null 2>&1 || fail "curl is required"
command -v sha256sum >/dev/null 2>&1 || fail "sha256sum is required"
command -v tar >/dev/null 2>&1 || fail "tar is required"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT INT TERM

ASSET="yunori-mail-${ARCH}-linux.tar.gz"
say "Downloading Yunori Mail Server ${YUNORI_VERSION} (${ARCH})…"
curl -fsSL --proto '=https' --tlsv1.2 -o "$TMP/$ASSET" "$BASE_URL/$ASSET"
curl -fsSL --proto '=https' --tlsv1.2 -o "$TMP/$ASSET.sha256" "$BASE_URL/$ASSET.sha256"

(cd "$TMP" && sha256sum -c "$ASSET.sha256" >/dev/null) || fail "checksum mismatch — refusing to install"

if [ -n "$MINISIGN_PUBKEY" ] && command -v minisign >/dev/null 2>&1; then
  curl -fsSL --proto '=https' --tlsv1.2 -o "$TMP/$ASSET.minisig" "$BASE_URL/$ASSET.minisig"
  minisign -Vm "$TMP/$ASSET" -P "$MINISIGN_PUBKEY" >/dev/null || fail "signature check failed — refusing to install"
  say "Signature verified."
fi

tar -xzf "$TMP/$ASSET" -C "$TMP" yunori-mail
install -m 0755 "$TMP/yunori-mail" /usr/local/bin/yunori-mail
say "Installed /usr/local/bin/yunori-mail."
say ""

# Hand over to the real installer (interactive unless flags are given).
if [ -t 1 ] && [ ! -t 0 ] && [ -r /dev/tty ]; then
  exec /usr/local/bin/yunori-mail install "$@" </dev/tty
fi
exec /usr/local/bin/yunori-mail install "$@"
