#!/usr/bin/env bash
set -euo pipefail

# Symmetric counterpart to scripts/install_hooks.sh.
# Removes the .githooks wiring by unsetting core.hooksPath so Git falls back to
# .git/hooks. This script does NOT delete the .githooks/ directory; the hooks
# remain in the working tree for inspection and re-installation.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if ! git rev-parse --git-dir >/dev/null 2>&1; then
    printf 'uninstall_hooks: not inside a Git working tree, nothing to do\n' >&2
    exit 0
fi

current_path="$(git config --get core.hooksPath || true)"
if [ -z "$current_path" ]; then
    printf 'uninstall_hooks: core.hooksPath is not set, nothing to do\n'
    exit 0
fi

git config --unset core.hooksPath
printf 'uninstalled Orison git hooks (cleared core.hooksPath, was: %s)\n' "$current_path"
