#!/bin/sh
# Smoke test for the release infrastructure.
#
# Validates:
#   1. scripts/install.sh --dry-run exits 0 and prints the expected download
#      URL pattern.
#   2. Dockerfile lints clean via `docker build --target builder` (skipped if
#      Docker is unavailable).
#   3. Formula/ori.rb declares sha256, url, and version.
#
# This is intentionally a POSIX shell script with no Bash-isms. It exits
# non-zero on the first failing check.

set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
INSTALL_SH="${ROOT}/scripts/install.sh"
DOCKERFILE="${ROOT}/Dockerfile"
FORMULA="${ROOT}/Formula/ori.rb"

FAIL=0
note() { printf '[smoke] %s\n' "$*"; }
fail() { printf '[smoke][FAIL] %s\n' "$*" >&2; FAIL=1; }
pass() { printf '[smoke][ ok ] %s\n' "$*"; }

# ----------------------------------------------------------------------------
# Check 1: install.sh --dry-run
# ----------------------------------------------------------------------------
note "checking install.sh exists and is syntactically valid"
[ -f "$INSTALL_SH" ] || { fail "missing $INSTALL_SH"; exit 1; }
sh -n "$INSTALL_SH" || { fail "install.sh syntax error"; exit 1; }
pass "install.sh syntax ok"

note "running install.sh --dry-run --version v0.1.1"
DRY_OUT="$(mktemp)"
DRY_ERR="$(mktemp)"
trap 'rm -f "$DRY_OUT" "$DRY_ERR"' EXIT INT TERM

if sh "$INSTALL_SH" --dry-run --version v0.1.1 >"$DRY_OUT" 2>"$DRY_ERR"; then
  pass "install.sh --dry-run exited 0"
else
  rc=$?
  fail "install.sh --dry-run exited $rc"
  cat "$DRY_ERR" >&2
fi

# Expected download URL pattern on stdout: PLAN download_url=https://github.com/<repo>/releases/download/v0.1.1/ori-<os>-<arch>.<ext>
if grep -E '^PLAN download_url=https://github\.com/[^/]+/[^/]+/releases/download/v0\.1\.1/ori-[a-z]+-(x86_64|aarch64)\.(tar\.gz|zip)$' "$DRY_OUT" >/dev/null; then
  pass "install.sh printed expected download URL pattern"
else
  fail "install.sh did not print expected download URL pattern"
  echo "--- stdout ---" >&2; cat "$DRY_OUT" >&2
  echo "--- stderr ---" >&2; cat "$DRY_ERR" >&2
fi

if grep -E '^PLAN install_path=' "$DRY_OUT" >/dev/null; then
  pass "install.sh printed install_path"
else
  fail "install.sh did not print install_path"
fi

# ----------------------------------------------------------------------------
# Check 2: Dockerfile lints
# ----------------------------------------------------------------------------
note "checking Dockerfile exists"
[ -f "$DOCKERFILE" ] || { fail "missing $DOCKERFILE"; exit 1; }

# Minimal syntactic sanity: it must declare at least one FROM and an
# ENTRYPOINT. This is the only "lint" we can do without external tools.
if grep -E '^FROM ' "$DOCKERFILE" >/dev/null; then
  pass "Dockerfile has FROM directive"
else
  fail "Dockerfile missing FROM"
fi

if grep -E '^ENTRYPOINT ' "$DOCKERFILE" >/dev/null; then
  pass "Dockerfile has ENTRYPOINT"
else
  fail "Dockerfile missing ENTRYPOINT"
fi

if command -v docker >/dev/null 2>&1; then
  note "docker available; attempting builder-stage build"
  # Use --target builder so we do not pull the runtime image too. This is
  # mostly to catch Dockerfile parse errors. The build itself may be slow;
  # gate it behind ORI_SMOKE_DOCKER=1 so CI does not pay the cost by default.
  if [ "${ORI_SMOKE_DOCKER:-0}" = "1" ]; then
    if (cd "$ROOT" && docker build --target builder -f Dockerfile . >/dev/null); then
      pass "docker build --target builder succeeded"
    else
      fail "docker build --target builder failed"
    fi
  else
    pass "skipping docker build (set ORI_SMOKE_DOCKER=1 to enable)"
  fi
else
  pass "skipping docker build (docker not installed)"
fi

# ----------------------------------------------------------------------------
# Check 3: Formula/ori.rb
# ----------------------------------------------------------------------------
note "checking Formula/ori.rb fields"
[ -f "$FORMULA" ] || { fail "missing $FORMULA"; exit 1; }

if grep -E '^[[:space:]]*version "' "$FORMULA" >/dev/null; then
  pass "formula has version"
else
  fail "formula missing version"
fi

if grep -E '^[[:space:]]*url "' "$FORMULA" >/dev/null; then
  pass "formula has url"
else
  fail "formula missing url"
fi

if grep -E '^[[:space:]]*sha256 "' "$FORMULA" >/dev/null; then
  pass "formula has sha256"
else
  fail "formula missing sha256"
fi

if grep -E '^class Ori < Formula' "$FORMULA" >/dev/null; then
  pass "formula declares class Ori < Formula"
else
  fail "formula missing 'class Ori < Formula'"
fi

# ----------------------------------------------------------------------------
if [ "$FAIL" -ne 0 ]; then
  note "FAILED"
  exit 1
fi
note "all checks passed"
exit 0
