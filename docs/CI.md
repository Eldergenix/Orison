# Continuous Integration

This document describes the GitHub Actions workflow matrix that protects the
`main` branch and how to reproduce every gate locally. The authoritative
quality-gate policy lives in
[`ORISON_AGENT_DEVELOPMENT_HANDOFF.md`](../ORISON_AGENT_DEVELOPMENT_HANDOFF.md)
under "Required quality gates"; this document only describes how that policy is
wired into CI. When the two disagree, the handoff wins.

See also: [`docs/QUALITY_GATES.md`](./QUALITY_GATES.md),
[`docs/CONTRIBUTING.md`](./CONTRIBUTING.md),
[`BENCHMARKS.md`](../BENCHMARKS.md),
[`AGENTS.md`](../AGENTS.md).

## Workflow matrix

| Workflow                            | Trigger                       | Runners                          | Purpose                                                                                  |
| ----------------------------------- | ----------------------------- | -------------------------------- | ---------------------------------------------------------------------------------------- |
| `.github/workflows/static.yml`      | every push, every PR          | `ubuntu-latest`                  | Static gate (`validate_all.py --static-only`). No Rust toolchain. First and fastest gate. |
| `.github/workflows/test.yml`        | every push, every PR          | `ubuntu-latest`, `macos-latest`  | Rust matrix (`1.92` stable + `nightly`): `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`. Depends on static. |
| `.github/workflows/ci.yml`          | every push, every PR          | `ubuntu-latest`                  | Legacy "full gate" job (`validate_all.py --full`). Retained for backward compatibility with any external automation that pins the `ci` workflow name; will be removed once `test.yml` has stabilised. |
| `.github/workflows/release.yml`     | `workflow_dispatch` (manual)  | `ubuntu-latest`                  | Full gate, then `cargo build --release -p ori`, then `ori bench --json`. Publishes the binary and the benchmark JSON as artefacts. Does **not** publish to crates.io or to GitHub Releases — that path is gated on a future release-policy decision (recorded in `MEMORY.md`). |
| `.github/workflows/sbom.yml`        | `workflow_dispatch` (manual)  | `ubuntu-latest`                  | Builds the release binary, runs `ori sbom --json`, uploads `sbom.json`. The workflow exists so the supply-chain path is provable; production quality is gated on M18 (Security and supply chain). |

## Gate ordering

CI enforces a strictly nested validation pyramid; each layer is a superset of
the layers below it.

```text
static  <-  unit/integration  <-  release artefact  <-  bench  <-  sbom
```

1. **Static** — `validate_all.py --static-only`. Required-file presence, JSON
   schema and example parseability, JSONL golden fixture validity, source
   guardrails (no `unwrap`/`panic!`/`unsafe`), shell-script strict mode, hook
   executability, approved workspace dependency set.
2. **Unit / integration** — `cargo fmt --check`, `cargo clippy -D warnings`,
   `cargo test --workspace`. Runs across the Rust matrix
   (`1.92` stable and `nightly`) on Ubuntu and macOS.
3. **Release artefact** — `cargo build --release -p ori`. Uploaded as
   `ori-release-${{ github.sha }}`.
4. **Bench** — `target/release/ori bench --samples 50 --json`. Uploaded as
   `bench-${{ github.sha }}.json`. The schema is
   [`schemas/benchmark.schema.json`](../schemas/benchmark.schema.json).
5. **SBOM** — `target/release/ori sbom --json`. Uploaded as
   `sbom-${{ github.sha }}.json`. Gated on M18 maturity.

A change must pass every layer up to and including the layer that contains its
last-touched artefact. Do not weaken a gate to land a change; fix the change.

## Reproducing CI locally

Every CI job has a one-line local equivalent. The Make targets are documented
in `make help`.

| CI job                | Local command                                                                        |
| --------------------- | ------------------------------------------------------------------------------------ |
| `static.yml`          | `make gate-fast`        (= `python3 scripts/validate_all.py --static-only`)         |
| `test.yml` (matrix)   | `make gate-full`        (= `python3 scripts/validate_all.py --full`)                |
| `release.yml` build   | `make release-build`    (= `cargo build --release -p ori`)                          |
| `release.yml` bench   | `make bench-json`       (= writes `BENCHMARKS.results.json`)                        |
| `sbom.yml`            | `make sbom`             (= writes `sbom.json`)                                      |

The pre-commit and pre-push Git hooks are equivalent to:

```bash
make gate-pre-commit   # = python3 scripts/validate_all.py --pre-commit
make gate-full         # = python3 scripts/validate_all.py --full
```

Install the hooks once per clone:

```bash
make install-hooks
```

Remove them:

```bash
make uninstall-hooks
```

## Toolchain pinning

- Rust stable channel is pinned to **1.92** in both `test.yml` and
  `release.yml`. Bumping the stable channel requires a `MEMORY.md` entry.
- The repository's `rust-toolchain.toml` selects the version used for local
  `cargo` invocations; CI installs its own toolchain explicitly so the
  workflows are reproducible regardless of what is checked in.
- `nightly` runs in the matrix on a best-effort basis. It uses
  `fail-fast: false`, so a `nightly`-only regression does not block stable
  PRs. Any consistent `nightly` failure must still be triaged in a follow-up
  issue.

## Caching

All Rust jobs cache `~/.cargo/registry`, `~/.cargo/git/db`, and `target/`
keyed on `Cargo.lock`, `Cargo.toml`, and `rust-toolchain.toml`. The cache key
includes the toolchain version so the stable and nightly caches do not
collide. A miss on the primary key falls back to the most recent key with the
same toolchain.

## When CI fails

1. Identify the failed layer from the workflow name.
2. Reproduce locally using the table above.
3. Fix the underlying issue and re-run the same local command until it passes.
4. Push the fix in a new commit; never `--force` over a failing CI run.
5. If the failure is environmental (runner outage, transient network), re-run
   the workflow but record the incident in `MEMORY.md` if it happens twice.
