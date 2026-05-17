#!/usr/bin/env bash
set -euo pipefail

# Wire the repository's tracked Git hooks (.githooks/*) into Git by setting
# core.hooksPath. This is idempotent; running it again is a no-op when the
# hooks path is already correct.
#
# Symmetric counterpart: scripts/uninstall_hooks.sh
#
# After install, every `git commit` runs `.githooks/pre-commit` and every
# `git push` runs `.githooks/pre-push`. Both invoke `scripts/validate_all.py`
# at the appropriate gate level (see CONTRIBUTING.md).
#
# Usage:
#   ./scripts/install_hooks.sh
#
# Equivalent Make target:
#   make install-hooks

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if ! git rev-parse --git-dir >/dev/null 2>&1; then
    printf 'install_hooks: not inside a Git working tree; nothing to wire\n' >&2
    exit 1
fi

git config core.hooksPath .githooks
chmod +x .githooks/pre-commit .githooks/pre-push \
         scripts/validate_all.py scripts/check_json_contracts.sh \
         scripts/install_hooks.sh scripts/uninstall_hooks.sh
printf 'installed Orison git hooks from .githooks\n'
printf 'to remove: ./scripts/uninstall_hooks.sh (or: make uninstall-hooks)\n'
