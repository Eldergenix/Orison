#!/usr/bin/env bash
# Orison CLI installer.
set -euo pipefail
#
# POSIX sh, dep-light. Detects the host OS+arch, resolves the latest GitHub
# Release of `ori`, downloads the matching archive, verifies its SHA256, and
# installs the binary into a user-writable bin directory.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/Eldergenix/Orison/main/scripts/install.sh | sh
#   sh scripts/install.sh                # install latest
#   sh scripts/install.sh --version v0.1.2
#   sh scripts/install.sh --dry-run      # print plan, do not write
#   sh scripts/install.sh --prefix /opt  # install into /opt/bin/ori
#
# Exits non-zero on any failure. Idempotent on re-run (overwrites the binary).

set -eu

REPO="${ORI_REPO:-Eldergenix/Orison}"
BIN_NAME="ori"
VERSION=""
DRY_RUN=0
PREFIX=""

log() { printf '%s\n' "$*" >&2; }
err() { printf 'error: %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<USAGE
Orison installer

Options:
  --version <tag>   Install a specific release tag (e.g. v0.1.2). Default: latest.
  --prefix <dir>    Install into <dir>/bin/ori. Overrides auto-detection.
  --dry-run         Print what would happen and exit 0.
  -h, --help        Show this help.

Environment:
  ORI_REPO          Override the GitHub repo (default: ${REPO}).
USAGE
}

while [ $# -gt 0 ]; do
  case "$1" in
    --version)
      shift
      [ $# -gt 0 ] || err "--version requires an argument"
      VERSION="$1"
      ;;
    --prefix)
      shift
      [ $# -gt 0 ] || err "--prefix requires an argument"
      PREFIX="$1"
      ;;
    --dry-run)
      DRY_RUN=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      err "unknown option: $1"
      ;;
  esac
  shift
done

# --- detection ---------------------------------------------------------------

detect_os() {
  os="$(uname -s)"
  case "$os" in
    Linux*)  echo "linux" ;;
    Darwin*) echo "macos" ;;
    MINGW*|MSYS*|CYGWIN*) echo "windows" ;;
    *) err "unsupported OS: $os" ;;
  esac
}

detect_arch() {
  arch="$(uname -m)"
  case "$arch" in
    x86_64|amd64) echo "x86_64" ;;
    aarch64|arm64) echo "aarch64" ;;
    *) err "unsupported arch: $arch" ;;
  esac
}

OS="$(detect_os)"
ARCH="$(detect_arch)"

# --- archive name ------------------------------------------------------------

if [ "$OS" = "windows" ]; then
  EXT="zip"
  BIN_FILE="ori.exe"
else
  EXT="tar.gz"
  BIN_FILE="ori"
fi

ARCHIVE="ori-${OS}-${ARCH}.${EXT}"

# --- version resolution ------------------------------------------------------

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || err "missing required command: $1"
}

resolve_latest() {
  require_cmd curl
  # /releases/latest redirects to /releases/tag/<tag>. Follow the redirect and
  # pluck the tag from the final URL. No JSON parsing, no extra deps.
  url="$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
    "https://github.com/${REPO}/releases/latest")"
  case "$url" in
    *"/releases/tag/"*) ;;
    *) err "could not resolve latest release URL: $url" ;;
  esac
  echo "${url##*/releases/tag/}"
}

if [ -z "$VERSION" ]; then
  if [ "$DRY_RUN" -eq 1 ]; then
    VERSION="<latest>"
  else
    VERSION="$(resolve_latest)"
  fi
fi

DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE}"
SHA_URL="${DOWNLOAD_URL}.sha256"

# --- install location --------------------------------------------------------

choose_install_dir() {
  if [ -n "$PREFIX" ]; then
    echo "${PREFIX}/bin"
    return
  fi
  # Prefer /usr/local/bin if writable without sudo; else $HOME/.local/bin.
  if [ -w "/usr/local/bin" ] 2>/dev/null; then
    echo "/usr/local/bin"
  else
    echo "${HOME}/.local/bin"
  fi
}

INSTALL_DIR="$(choose_install_dir)"
INSTALL_PATH="${INSTALL_DIR}/${BIN_FILE}"

log "==> Orison installer"
log "    repo:        ${REPO}"
log "    os/arch:     ${OS}/${ARCH}"
log "    version:     ${VERSION}"
log "    archive:     ${ARCHIVE}"
log "    download:    ${DOWNLOAD_URL}"
log "    sha256 url:  ${SHA_URL}"
log "    install to:  ${INSTALL_PATH}"

if [ "$DRY_RUN" -eq 1 ]; then
  log "==> dry-run: no files written"
  printf 'PLAN download_url=%s\n' "$DOWNLOAD_URL"
  printf 'PLAN install_path=%s\n' "$INSTALL_PATH"
  exit 0
fi

require_cmd curl

# --- workspace ---------------------------------------------------------------

TMPDIR_WORK="$(mktemp -d 2>/dev/null || mktemp -d -t ori-install)"
trap 'rm -rf "$TMPDIR_WORK"' EXIT INT TERM

ARCHIVE_PATH="${TMPDIR_WORK}/${ARCHIVE}"
SHA_PATH="${ARCHIVE_PATH}.sha256"

log "==> downloading archive"
curl -fsSL --retry 3 -o "$ARCHIVE_PATH" "$DOWNLOAD_URL" \
  || err "failed to download ${DOWNLOAD_URL}"

log "==> downloading sha256"
curl -fsSL --retry 3 -o "$SHA_PATH" "$SHA_URL" \
  || err "failed to download ${SHA_URL}"

# --- verify sha256 -----------------------------------------------------------

expected="$(awk '{print $1}' "$SHA_PATH" | tr -d '\r\n[:space:]')"
[ -n "$expected" ] || err "empty sha256 for ${ARCHIVE}"

if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$ARCHIVE_PATH" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  actual="$(shasum -a 256 "$ARCHIVE_PATH" | awk '{print $1}')"
else
  err "no sha256 tool (sha256sum or shasum) available"
fi

if [ "$expected" != "$actual" ]; then
  err "sha256 mismatch: expected=${expected} actual=${actual}"
fi

log "==> sha256 verified: ${actual}"

# --- unpack ------------------------------------------------------------------

UNPACK_DIR="${TMPDIR_WORK}/unpack"
mkdir -p "$UNPACK_DIR"

case "$EXT" in
  tar.gz)
    require_cmd tar
    tar -xzf "$ARCHIVE_PATH" -C "$UNPACK_DIR"
    ;;
  zip)
    require_cmd unzip
    unzip -q "$ARCHIVE_PATH" -d "$UNPACK_DIR"
    ;;
  *)
    err "unknown archive extension: $EXT"
    ;;
esac

SRC_BIN="${UNPACK_DIR}/${BIN_FILE}"
[ -f "$SRC_BIN" ] || err "archive did not contain expected binary: ${BIN_FILE}"

# --- install -----------------------------------------------------------------

mkdir -p "$INSTALL_DIR"
# Move-or-copy with mode preservation. Re-run is idempotent: the destination
# is overwritten atomically via mv when possible.
cp "$SRC_BIN" "${INSTALL_PATH}.new"
chmod 0755 "${INSTALL_PATH}.new"
mv "${INSTALL_PATH}.new" "$INSTALL_PATH"

log "==> installed: ${INSTALL_PATH}"

# --- PATH hint ---------------------------------------------------------------

case ":${PATH}:" in
  *":${INSTALL_DIR}:"*)
    log "==> ${INSTALL_DIR} is already on PATH"
    ;;
  *)
    log ""
    log "    Note: ${INSTALL_DIR} is not on your PATH."
    log "    Add this line to your shell profile (~/.bashrc, ~/.zshrc, ...):"
    log ""
    log "      export PATH=\"${INSTALL_DIR}:\$PATH\""
    log ""
    ;;
esac

log "==> done. Try: ori --help"
