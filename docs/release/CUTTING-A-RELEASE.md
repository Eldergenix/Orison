# Cutting an Orison release

This document is the maintainer's runbook for cutting a tagged release of the
Orison toolchain. It assumes you have push access to the
[`Eldergenix/Orison`](https://github.com/Eldergenix/Orison) repository and a
local clone tracking `main`.

A release is a single git tag of the form `vMAJOR.MINOR.PATCH`. Pushing that
tag is the only action that triggers the public artefact pipeline. Everything
else is preparation, verification, and post-release housekeeping.

## 0. Scope of a release

A tagged release publishes:

1. **GitHub Release artefacts** — prebuilt `ori` binaries for the five-target
   matrix (linux/x86_64, linux/aarch64, macos/x86_64, macos/aarch64,
   windows/x86_64), plus `.sha256` files and a release body listing all
   checksums.
2. **A Homebrew formula bump** — the `Formula/ori.rb` file in this repo,
   updated by hand once the artefact SHA256s are known.
3. **An optional container image** — built from `Dockerfile`. Container
   publishing is a follow-up action, not part of the tag workflow yet.

What a tag explicitly does **not** do:

- It does not publish to `crates.io`. Crate publishing is gated on a stable
  public API and tracked separately.
- It does not rotate signing keys or modify git config.

## 1. Pre-flight: confirm the tree is releasable

Run these checks on a clean clone of `main`. Do not skip any of them — the
tagged workflow does not re-run the full validation gate.

```sh
git switch main
git pull --ff-only
git status                                # must be clean
```

Then:

```sh
cargo fmt --all -- --check
cargo test --workspace --all-features
python3 scripts/validate_all.py --full
cargo build --release -p ori
target/release/ori --help                 # must exit 0
target/release/ori bench --samples 50 --json > /tmp/bench.json
```

If any of these fail, **stop**. Fix on `main` via PR first, then restart this
playbook from step 0.

## 2. Decide the version number

Orison follows SemVer 2.0.0:

- **Patch** (`v0.1.X`): bug fixes, doc fixes, perf wins with no API change.
- **Minor** (`v0.X.0`): backwards-compatible additions to the CLI, schemas
  (additive only), or stdlib.
- **Major** (`vX.0.0`): breaking changes. Pre-1.0, every minor bump is
  effectively allowed to break; once we hit `v1.0.0` this stops.

Schemas are immutable: adding a new schema is fine; changing an existing one
requires a major bump.

Confirm there are no `MEMORY.md` or `STABILITY.md` entries flagging the next
version as blocked.

## 3. Bump the version

The workspace version lives in `Cargo.toml` under `[workspace.package]`. Bump
it, plus any per-crate `version` overrides:

```sh
NEW=0.1.2
sed -i.bak -E "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+\"$/version = \"${NEW}\"/" Cargo.toml
rm Cargo.toml.bak
cargo update -p ori                       # refresh Cargo.lock
```

Verify the bump:

```sh
grep '^version' Cargo.toml
grep '^version' crates/*/Cargo.toml
```

The `ori` package version in `crates/ori-cli/Cargo.toml` must match the new
workspace version. If it has its own `version` line, bump it too.

## 4. Regenerate the changelog

Add a new section at the top of `CHANGELOG.md`:

```
## v0.1.2 — YYYY-MM-DD

### Added
- ...

### Changed
- ...

### Fixed
- ...

### Schemas
- (additive only; list any new schema files)
```

Collect entries by walking `git log v0.1.1..HEAD --oneline` and categorising
each commit. Anything user-visible belongs in the changelog; pure refactors
do not.

## 5. Re-run the full quality gate

Now that the version is bumped and the changelog is updated, repeat:

```sh
python3 scripts/validate_all.py --full
cargo build --release -p ori
sh tests/release_smoke.sh                 # release-infra smoke tests
```

The smoke test validates that `install.sh --dry-run` works, the `Dockerfile`
parses, and `Formula/ori.rb` has the required fields.

## 6. Open the version-bump PR

Commit and push the version + changelog changes via a PR:

```sh
git switch -c release/v0.1.2
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "release: v0.1.2"
git push -u origin release/v0.1.2
gh pr create --fill --base main
```

Wait for CI to pass, then merge. **Do not tag from the branch**; tag from
`main` after the merge lands.

## 7. Tag and push

After the PR is merged:

```sh
git switch main
git pull --ff-only
git tag -a v0.1.2 -m "Orison v0.1.2"
git push origin v0.1.2
```

Pushing the tag is the trigger for `.github/workflows/release-publish.yml`.
Nothing about the tag push itself can be undone cleanly once artefacts have
been uploaded — if you must withdraw a release, cut a new patch version
rather than rewriting the tag.

## 8. Watch the workflow

```sh
gh run watch --workflow release-publish.yml
```

The workflow has two jobs:

1. **build** — a matrix of five OS/arch combinations. Each produces an
   archive (`ori-<os>-<arch>.tar.gz` or `.zip`) plus a `.sha256` sidecar.
2. **publish** — collects all matrix artefacts, composes a release body
   that lists every SHA256, and creates the GitHub Release via
   `softprops/action-gh-release@v2`.

If a single matrix entry fails, **re-run only that entry**. If the failure
is reproducible (e.g. a flaky cross-compile linker), open an issue and patch
the workflow on `main` before retrying.

## 9. Verify the artefacts

Once the workflow is green, manually verify at least one artefact end-to-end:

```sh
curl -fsSL -o /tmp/ori.tar.gz \
  https://github.com/Eldergenix/Orison/releases/download/v0.1.2/ori-linux-x86_64.tar.gz
curl -fsSL -o /tmp/ori.tar.gz.sha256 \
  https://github.com/Eldergenix/Orison/releases/download/v0.1.2/ori-linux-x86_64.tar.gz.sha256
sha256sum -c <(awk '{print $1"  /tmp/ori.tar.gz"}' /tmp/ori.tar.gz.sha256)
tar -tzf /tmp/ori.tar.gz                  # should list ori, LICENSE, README.md
```

Also smoke-test the installer:

```sh
sh scripts/install.sh --dry-run --version v0.1.2
sh scripts/install.sh --version v0.1.2    # only on a throwaway box
ori --help
```

## 10. Bump the Homebrew formula

The release workflow does **not** update `Formula/ori.rb` — it cannot mutate
git. Update it manually:

1. For each platform pair in the formula, replace the `version`, the four
   `url`s, and the four `sha256`s with the values from the release body.
2. The SHA256s in the formula are for the archives, identical to the
   contents of the `.sha256` sidecar files.
3. Open a PR titled `homebrew: bump ori to v0.1.2`, get it merged.

If the formula is mirrored into a tap repository (e.g.
`Eldergenix/homebrew-orison`), open the equivalent PR there.

## 11. Optional: publish the container image

If container distribution is enabled this cycle:

```sh
docker build -t orison/ori:v0.1.2 -t orison/ori:latest .
docker push orison/ori:v0.1.2
docker push orison/ori:latest
```

The final image must be < 200 MB (`docker image ls orison/ori`) and must
respond to `docker run --rm orison/ori:v0.1.2 --help`.

## 12. Announce

In order:

1. Post the GitHub Release URL in the project's release channel.
2. Update the docsite version selector if applicable.
3. Close the milestone for this release on GitHub; open the milestone for
   the next one.
4. Add a note to `MEMORY.md` summarising what shipped, in case any future
   bisection needs to walk back through tagged versions.

## Rollback policy

Releases are immutable. If a critical bug ships, do **not** delete the tag.
Instead:

1. Mark the broken release as a pre-release in the GitHub UI and add a
   prominent note in the release body pointing at the fix.
2. Cut a new patch version following this playbook from step 0.
3. Yank the broken Homebrew formula by reverting it to the previous SHA256s.

## Things this playbook does not cover

- Crates.io publishing (deferred).
- Signed releases (cosign / minisign). Tracked separately.
- Backporting fixes to older minor lines. We do not currently maintain
  release branches; only the latest minor is supported.
