# Orison CI

This document is the source of truth for how Orison's continuous-integration
matrix is wired, what each workflow guarantees, the order in which gates run,
and how to reproduce any of them on a local machine.

It is read alongside:

- `.github/workflows/*.yml` — the workflow definitions themselves.
- `scripts/validate_all.py` — the static + full validation gate.
- `scripts/compare_bench.py` — the benchmark regression comparator.
- `BENCHMARKS.md` / `BENCHMARKS.results.json` — the in-tree perf baseline.

## Workflows at a glance

| Workflow | File | Triggers | Purpose | Blocks merge? |
|----------|------|----------|---------|---------------|
| `ci` | `ci.yml` | push, PR | Minimal quality-gate (rustfmt, clippy, full validate_all). | Yes |
| `static` | `static.yml` | push, PR, `workflow_call` | No-Rust static gate (file layout, schema contracts, source guardrails, hook checks, bench-compare CLI health). | Yes |
| `test` | `test.yml` | push, PR, `workflow_call` | Cross-platform matrix: {ubuntu, macos, windows} × {stable, nightly}. Runs fmt, clippy, `cargo test --workspace`, and `validate_all.py --full`. | Stable rows: yes. Nightly rows: informational (`continue-on-error: true`). |
| `conformance` | `conformance.yml` | push, PR | `cargo test -p ori-compiler --test conformance`; uploads failing-fixture diff on failure. | Yes |
| `security-audit` | `security-audit.yml` | push, PR, weekly cron | Per-test gates for unsafe-surface, capability runtime denial, capability bypass, lockfile tamper, SBOM schema, provenance failure. | Yes |
| `bench-regression` | `bench-regression.yml` | push to main, PR | Builds release `ori`, runs `ori bench --samples 50 --json`, compares against main's last successful bench artefact (PRs) or the committed `BENCHMARKS.results.json` (main pushes). Fails if any suite regresses p50 by more than 20%. | Yes |
| `release` | `release.yml` | `workflow_dispatch` | Full gate + release build + bench artefact. | N/A (manual) |
| `sbom` | `sbom.yml` | `workflow_dispatch` | Emits an SBOM artefact from the release binary. | N/A (manual) |

## Gate ordering

For a push or PR the effective dependency graph is:

```
                 +----------+
                 |  static  |  <-- runs first; no Rust needed
                 +----+-----+
                      |
        +-------------+-------------+
        |             |             |
        v             v             v
+---------------+ +-----------+ +------------------+
| test (matrix) | | conformance| | security-audit  |
+-------+-------+ +-----------+ +------------------+
        |
        v
+---------------+
| bench-regression
+---------------+
```

- `test.yml` declares `needs: static` and re-uses `static.yml` via
  `workflow_call`, so the static gate is the single entry point for the
  Rust-aware matrix.
- `conformance.yml` and `security-audit.yml` are intentionally independent
  jobs so they fail with their own status checks (easier to triage).
- `bench-regression.yml` is the last gate because it depends on a green
  release build; a regression there should never mask a test failure.
- Nightly matrix rows use `continue-on-error: true`. They surface as
  yellow checks if rustc nightly breaks something but do not block merges.

## Cross-platform matrix details

`test.yml`'s matrix is:

| OS | Toolchain | Blocks merge? |
|----|-----------|---------------|
| ubuntu-latest | stable | Yes |
| macos-latest | stable | Yes |
| windows-latest | stable | Yes |
| ubuntu-latest | nightly | No (informational) |
| macos-latest | nightly | No (informational) |
| windows-latest | nightly | No (informational) |

`fail-fast: false` is set so one OS or toolchain breaking doesn't mask
problems on another.

Every job:

1. Checks out the repo.
2. Installs the requested rustc with `dtolnay/rust-toolchain@master`
   (`components: rustfmt, clippy`).
3. Caches `~/.cargo/registry/{index,cache}`, `~/.cargo/git/db`, and the
   workspace `target/` directory under a key that includes the OS, the
   toolchain, and `hashFiles('**/Cargo.lock', '**/Cargo.toml', 'rust-toolchain.toml')`.
4. Installs Python 3.13 and `jsonschema` for the full validation gate.
5. Runs `cargo fmt --all --check`, then
   `cargo clippy --workspace --all-targets -- -D warnings`, then
   `cargo test --workspace`, then `python3 scripts/validate_all.py --full`.

All `run:` steps invoke tools via their workspace-relative paths (no
hard-coded forward-slash separators) so Windows runners do not need any
special shell handling.

## Benchmark regression gate

`bench-regression.yml` produces `bench-current.json` from a release-mode
`ori bench --samples 50 --json` run. The comparator (`scripts/compare_bench.py`)
joins metrics by `(suite_name, metric_key)`, computes the percentage delta on
the p50, and exits 1 if any metric exceeds the configured threshold
(default 20%, override via the `BENCH_REGRESSION_THRESHOLD` env var).

Baseline selection logic:

- **Push to `main`:** compare against the committed `BENCHMARKS.results.json`
  (kept fresh by maintainers using `make bench` or the release workflow).
- **Pull request:** look up the most recent successful `bench-regression`
  run on `main` via `gh run list` and download its `bench-current-*`
  artefact. If none exists (e.g. brand-new repo), fall back to the
  committed `BENCHMARKS.results.json`.

The current artefact is always uploaded (`if: always()`) so reviewers can
inspect it even when the gate fails.

### Tolerance for new/removed suites

`compare_bench.py` never blocks on:

- A suite or metric that exists in the current run but not the baseline
  (emitted as `info (new suite/metric)`).
- A suite or metric that exists in the baseline but not the current run
  (emitted as `info (removed suite/metric)`).
- A non-numeric or zero p50 (avoids divide-by-zero).

This keeps the gate compatible with normal evolution of the bench surface.

## Reproducing locally

You need:

- Rust stable (matching `rust-toolchain.toml`).
- Python 3.13 with `jsonschema` (`pip install jsonschema`).
- `gh` CLI only if you want to mimic the cross-run artefact lookup.

### Static + full validation gate

```bash
python3 scripts/validate_all.py --static-only
python3 scripts/validate_all.py --full
```

### Cargo gates

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

### Conformance suite

```bash
cargo test -p ori-compiler --test conformance -- --nocapture
```

### Security audit suite

```bash
cargo test -p ori-compiler --test unsafe_surface_report -- --nocapture
cargo test -p ori-compiler --test capability_runtime_denial -- --nocapture
cargo test -p ori-pkg      --test capability_bypass         -- --nocapture
cargo test -p ori-pkg      --test lockfile_tamper           -- --nocapture
cargo test -p ori-pkg      --test sbom_schema               -- --nocapture
cargo test -p ori-pkg      --test provenance_failure        -- --nocapture
```

### Benchmark regression gate

```bash
cargo build --release -p ori
target/release/ori bench --samples 50 --json > bench-current.json

# Compare against the committed baseline.
python3 scripts/compare_bench.py \
    --baseline BENCHMARKS.results.json \
    --current  bench-current.json \
    --threshold 20 \
    --markdown bench-report.md
```

The script writes a markdown table to stdout (and to `--markdown`) and
exits 1 if any metric regresses past the threshold.

## Adding a new gate

1. Create the workflow file under `.github/workflows/` with a `name:`,
   explicit `on:` triggers, a `permissions:` block, and `runs-on:` per
   job.
2. Add `actions/cache@v4` with a key that includes
   `hashFiles('**/Cargo.lock', '**/Cargo.toml', 'rust-toolchain.toml')`.
3. If the new gate uses Python, pin to `python-version: '3.13'` to match
   the toolchain used by `validate_all.py`.
4. Update the table at the top of this document.
5. If the gate is meant to block merges, add it to the required-checks
   list in the repo's branch protection settings.
