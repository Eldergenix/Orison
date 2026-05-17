# Recipe 06: Wire the formatter into a precommit hook and CI gate

**Goal.** Block any commit that contains unformatted Orison source. You
will install a Git precommit hook locally, add a CI step that runs the
same check on every PR, and learn how to recover when the hook trips.

**Prerequisites.** A working `ori` binary and a Git repository with
`.ori` files under version control.

**Time:** ~15 minutes.

## 1. What `ori fmt` does

`ori fmt <file.ori>` is a CST-preserving formatter. It rewrites
the file to canonical form (LF line endings, no trailing
whitespace, two-space indentation, one blank line between
top-level items), is *idempotent*, and never changes the parsed
AST. Whatever passed `ori check` before the format passes after.

The formatter prints the canonical form to stdout. To check
whether a file is already formatted, diff the output against the
file on disk. Exit 0 means the file parses; non-zero means the
file failed to parse.

## 2. The single-file check

```bash
ori fmt src/api.ori | diff - src/api.ori
echo "exit=$?"
```

If the file is already formatted, `diff` exits 0 and prints nothing.
If the file needs reformatting, `diff` exits 1 and prints the patch.
That exit-code contract is what the precommit hook and CI gate hinge
on.

A future Orison release will expose this as a single flag,
`ori fmt --check <file>`, with the same exit-code semantics. The
`diff`-based form documented here works today and will continue to
work; the `--check` flag is the more ergonomic way to wire CI when it
ships and is the form the rest of this recipe targets.

## 3. The repo-wide check

For a workspace with many `.ori` files:

```bash
#!/usr/bin/env bash
# scripts/fmt-check.sh
set -euo pipefail

failed=0
while IFS= read -r -d '' file; do
  if ! ori fmt "$file" | diff -q - "$file" > /dev/null; then
    echo "needs format: $file"
    failed=1
  fi
done < <(find . -name '*.ori' -not -path './target/*' -print0)

exit $failed
```

The script walks every tracked `.ori` file outside `target/`, runs
the format check, and exits 1 if any file would change. Stash it as
`scripts/fmt-check.sh` and `chmod +x` it.

When `ori fmt --check` ships, the entire script collapses to:

```bash
find . -name '*.ori' -not -path './target/*' -print0 \
  | xargs -0 ori fmt --check
```

with the same exit-code semantics.

## 4. The precommit hook

Git's precommit hook lives at `.git/hooks/pre-commit` and runs before
every commit. To install:

```bash
cat > .git/hooks/pre-commit <<'HOOK'
#!/usr/bin/env bash
set -euo pipefail

# Format-check only the staged .ori files
staged=$(git diff --cached --name-only --diff-filter=ACMR | grep '\.ori$' || true)
if [ -z "$staged" ]; then
  exit 0
fi

failed=0
for file in $staged; do
  if ! ori fmt "$file" | diff -q - "$file" > /dev/null; then
    echo "fmt: $file needs reformatting"
    failed=1
  fi
done

if [ $failed -ne 0 ]; then
  echo ""
  echo "Run: ori fmt <file> > <file>.new && mv <file>.new <file>"
  echo "Or:  scripts/fmt-fix.sh   (writes in place)"
  exit 1
fi
HOOK
chmod +x .git/hooks/pre-commit
```

Two design choices to call out. First, the hook only checks *staged*
files — uncommitted changes elsewhere in the tree do not block the
commit, which is what you want when you are working on a subset.
Second, it does not auto-format. Auto-formatting in the hook hides
information about what changed; the hook tells the developer what is
wrong and the developer fixes it. (For projects that *want* auto-fix,
swap the diff line for `ori fmt "$file" > "$file.tmp" && mv "$file.tmp" "$file"`
and re-stage.)

To share the hook across the team, commit it to `.githooks/pre-commit`
and tell each developer to run `git config core.hooksPath .githooks`.
The Orison repo itself uses this pattern; see `.githooks/`.

## 5. The CI gate

GitHub Actions example. Save as `.github/workflows/fmt.yml`:

```yaml
name: format

on:
  pull_request:
    paths: ['**/*.ori']
  push:
    branches: [main]
    paths: ['**/*.ori']

jobs:
  fmt-check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install ori
        run: |
          curl -sSL https://orison-lang.org/install.sh | bash
          echo "$HOME/.orison/bin" >> $GITHUB_PATH
      - name: Format check
        run: ./scripts/fmt-check.sh
```

The job runs only when `.ori` files change (the `paths:` filter), so
you do not burn CI minutes on doc-only PRs. The same script you run
locally runs in CI — there is no second source of truth.

When `ori fmt --check` lands, the last step simplifies to:

```yaml
      - name: Format check
        run: |
          find . -name '*.ori' -not -path './target/*' -print0 \
            | xargs -0 ori fmt --check
```

## 6. Pre-PR developer workflow

```bash
# Fix every file in one shot
find . -name '*.ori' -not -path './target/*' | while read f; do
  ori fmt "$f" > "$f.new" && mv "$f.new" "$f"
done

# Verify nothing else regressed
ori check --json src/*.ori
```

Stash this in `scripts/fmt-fix.sh` and `chmod +x` it. Now any
developer who hits the hook can recover with a single command.

## 7. Why a CI gate matters even with a precommit hook

The precommit hook is a developer-side convenience and can be
bypassed (`git commit --no-verify`). The CI gate is the
authoritative enforcement. The cost is low: `ori fmt` runs in
roughly 50 ms per file (see `BENCHMARKS.md`), so a full workspace
check finishes in under two seconds.

## 8. What the formatter does not change

The formatter is structural — it does not rename anything, does
not reorder declarations, does not collapse imports, does not
strip dead code. Those operations live in `ori patch apply` (see
[recipe 03](./03-agent-driven-refactor.md)).

Tier-1 promise: from 1.0, the formatter output is stable across
patch releases. A minor release may change the output (e.g. a new
style rule), and the migration tool fixes existing files
automatically — see [migration guide](../migration/README.md).
