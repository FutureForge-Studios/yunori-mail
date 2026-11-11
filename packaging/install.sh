#!/bin/sh
# Yunori Mail Server: one-line installer.
#
#   curl -fsSL https://yunori.email/server/install.sh | sudo sh
#
# (yunori.email serves this repository's releases; the GitHub release URL works the same:
#  https://github.com/FutureForge-Studios/yunori-mail/releases/latest/download/install.sh)
#
# Unattended (every answer as a flag; see `yunori-mail install --help`):
#   curl -fsSL …/install.sh | sudo sh -s -- --domain example.com --admin-email you@gmail.com
#
# Check a server first, installing nothing but the binary:
#   curl -fsSL …/install.sh | sudo sh -s -- preflight
#
# Run it again at any time: a paused install resumes, a finished one is updated in place.
# On an installed server, `sudo yunori-mail update` runs this same script.
#
# What it does, and nothing else:
#   1. checks this is 64-bit Linux with systemd, and that you are root;
#   2. downloads ONE yunori-mail release for your CPU (a single static binary);
#   3. verifies its SHA-256 (and its minisign signature when a key is pinned and minisign is installed);
#   4. installs it as /usr/local/bin/yunori-mail and runs `yunori-mail install` with your arguments
#      (or `yunori-mail preflight` when the first argument is `preflight`).
# It never pipes further remote code into a shell. Read it before running it.
#
# Environment overrides:
#   YUNORI_VERSION=0.2.1       install this release instead of the one this script was published with
#   YUNORI_SKIP_INSTALL=1      download, verify and install the binary only; don't run the installer
#   YUNORI_BASE_URL=…          download from a mirror (https:// or file://) instead of yunori.email/GitHub
#   YUNORI_FEED=…              the release feed (default https://yunori.email/server; GitHub is the fallback)
#   YUNORI_PREFIX=/usr/local   where bin/yunori-mail goes

set -eu

REPO="FutureForge-Studios/yunori-mail"
# yunori.email answers for this repository's releases: /latest.json and /v<version>/<file>.
FEED="${YUNORI_FEED:-https://yunori.email/server}"
# Release builds pin this line to their own version (see .github/workflows/release.yml).
DEFAULT_VERSION="latest"
# Release signing key (minisign). Empty until releases are signed.
MINISIGN_PUBKEY="${YUNORI_MINISIGN_PUBKEY:-}"

say() { printf '%s\n' "$*"; }
fail() {
  printf 'yunori-mail: %s\n' "$*" >&2
  exit 1
}

check_system() {
  [ "$(uname -s)" = "Linux" ] || fail "Yunori Mail Server runs on Linux."
  case "$(uname -m)" in
    x86_64 | amd64) ARCH="x86_64" ;;
    aarch64 | arm64) ARCH="aarch64" ;;
    *) fail "unsupported CPU $(uname -m): use an x86-64 or ARM64 server." ;;
  esac
  if [ "${YUNORI_SKIP_INSTALL:-}" = "1" ]; then
    return
  fi
  [ "$(id -u)" = "0" ] || fail "run it as root: curl -fsSL https://yunori.email/server/install.sh | sudo sh"
  [ -d /run/systemd/system ] || fail "this machine doesn't run systemd. Use a Debian 12+ or Ubuntu 22.04+ server."
  if [ -r /etc/os-release ]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    case "${ID:-}:${VERSION_ID:-}" in
      debian:12 | debian:13 | ubuntu:22.04 | ubuntu:24.04 | ubuntu:26.04) ;;
      *) say "Note: ${PRETTY_NAME:-this OS} isn't a tested release. The installer checks what it needs and tells you." ;;
    esac
  fi
}

pick_downloader() {
  command -v sha256sum >/dev/null 2>&1 || fail "sha256sum is required (coreutils)."
  command -v tar >/dev/null 2>&1 || fail "tar is required."
  command -v gzip >/dev/null 2>&1 || fail "gzip is required."
  if command -v curl >/dev/null 2>&1; then
    DL="curl"
  elif command -v wget >/dev/null 2>&1; then
    DL="wget"
  else
    fail "curl or wget is required."
  fi
}

# fetch URL FILE: HTTPS only (or file:// for a local mirror), TLS 1.2+, retried.
fetch() {
  case "$1" in
    https://*) proto="=https" ;;
    file://*) proto="=file" ;;
    *) fail "refusing to download from $1 (https:// or file:// only)" ;;
  esac
  if [ "$DL" = "curl" ]; then
    curl -fsSL --proto "$proto" --proto-redir '=https' --tlsv1.2 --retry 3 --connect-timeout 15 -o "$2" "$1"
  else
    case "$1" in file://*) cp "${1#file://}" "$2" ;; *) wget -q --https-only --tries=3 --timeout=15 -O "$2" "$1" ;; esac
  fi
}

# The newest release's version from the feed's latest.json, or nothing.
feed_version() {
  case "$FEED" in https://*) ;; *) return 0 ;; esac
  json="$(mktemp)"
  if fetch "$FEED/latest.json" "$json" 2>/dev/null; then
    sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$json" | head -n 1
  fi
  rm -f "$json"
}

resolve_version() {
  VERSION="${YUNORI_VERSION:-$DEFAULT_VERSION}"
  if [ "$VERSION" = "latest" ]; then
    VERSION="$(feed_version)"
  fi
  if [ -z "$VERSION" ] || [ "$VERSION" = "latest" ]; then
    url="https://github.com/$REPO/releases/latest"
    if [ "$DL" = "curl" ]; then
      final="$(curl -fsSLI --proto '=https' --tlsv1.2 -o /dev/null -w '%{url_effective}' "$url")" ||
        fail "couldn't reach GitHub to find the latest release. Set YUNORI_VERSION to install a specific one."
    else
      final="$(wget -q --https-only --max-redirect=0 -S --spider "$url" 2>&1 | awk '/^ *[Ll]ocation: / { print $2 }' | tail -n 1)"
    fi
    VERSION="${final##*/}"
  fi
  VERSION="${VERSION#v}"
  printf '%s' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$' ||
    fail "no published release found (got '$VERSION'). Set YUNORI_VERSION to install a specific one."
}

install_binary() {
  PREFIX="${YUNORI_PREFIX:-/usr/local}"
  BIN="$PREFIX/bin/yunori-mail"
  REPLACED=0
  TMP=""
  if [ -x "$BIN" ] && [ "$("$BIN" --version 2>/dev/null || true)" = "yunori-mail $VERSION" ]; then
    say "yunori-mail $VERSION is already installed."
    return
  fi

  ASSET="yunori-mail-$ARCH-linux.tar.gz"
  TMP="$(mktemp -d)"
  trap 'rm -rf "$TMP"' EXIT INT TERM

  say "Downloading Yunori Mail Server $VERSION ($ARCH)…"
  # A mirror, if one is set; else yunori.email, and GitHub if yunori.email can't be reached.
  # The checksum always comes from the same place as the binary it checks.
  if [ -n "${YUNORI_BASE_URL:-}" ]; then
    BASE_URL="$YUNORI_BASE_URL"
    fetch "$BASE_URL/$ASSET" "$TMP/$ASSET" || fail "download failed: $BASE_URL/$ASSET"
  else
    BASE_URL="$FEED/v$VERSION"
    if ! fetch "$BASE_URL/$ASSET" "$TMP/$ASSET" 2>/dev/null; then
      BASE_URL="https://github.com/$REPO/releases/download/v$VERSION"
      say "yunori.email didn't answer; downloading from GitHub instead."
      fetch "$BASE_URL/$ASSET" "$TMP/$ASSET" || fail "download failed: $BASE_URL/$ASSET"
    fi
  fi
  fetch "$BASE_URL/$ASSET.sha256" "$TMP/$ASSET.sha256" || fail "download failed: $ASSET.sha256"
  (cd "$TMP" && sha256sum -c "$ASSET.sha256" >/dev/null 2>&1) || fail "checksum mismatch. Refusing to install."

  if [ -n "$MINISIGN_PUBKEY" ] && command -v minisign >/dev/null 2>&1; then
    fetch "$BASE_URL/$ASSET.minisig" "$TMP/$ASSET.minisig" || fail "download failed: $ASSET.minisig"
    minisign -Vm "$TMP/$ASSET" -P "$MINISIGN_PUBKEY" >/dev/null || fail "signature check failed. Refusing to install."
    say "Signature verified."
  fi

  tar -xzf "$TMP/$ASSET" -C "$TMP" yunori-mail || fail "the download is not a yunori-mail release."
  [ "$("$TMP/yunori-mail" --version 2>/dev/null || true)" = "yunori-mail $VERSION" ] ||
    fail "the downloaded binary doesn't run here or isn't version $VERSION."

  # Atomic swap: a running service keeps its old binary until it restarts.
  mkdir -p "$PREFIX/bin"
  cp "$TMP/yunori-mail" "$PREFIX/bin/.yunori-mail.new"
  chmod 0755 "$PREFIX/bin/.yunori-mail.new"
  mv -f "$PREFIX/bin/.yunori-mail.new" "$BIN"
  REPLACED=1
  say "Installed $BIN (checksum verified)."
}

main() {
  check_system
  pick_downloader
  resolve_version
  install_binary

  if [ "${YUNORI_SKIP_INSTALL:-}" = "1" ]; then
    say "Next: sudo $BIN install"
    exit 0
  fi
  say ""
  # `exec` skips the EXIT trap, so clean up the download now.
  if [ -n "${TMP:-}" ]; then rm -rf "$TMP"; fi
  case "${1:-}" in
    preflight | check)
      # Report whether this server can run a mail server; change nothing else.
      shift
      exec "$BIN" preflight "$@"
      ;;
  esac
  export YUNORI_BINARY_REPLACED="$REPLACED"
  # When piped (curl … | sh), stdin is this script: answer the questions on the terminal instead.
  if [ ! -t 0 ] && (exec </dev/tty) 2>/dev/null; then
    exec "$BIN" install "$@" </dev/tty
  fi
  exec "$BIN" install "$@"
}

# Everything above only defines functions: nothing runs until the whole script
# has arrived, so a cut-off download can't run half an installer.
main "$@"
