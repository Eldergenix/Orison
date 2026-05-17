# Contributing to Orison

Thanks for working on Orison. This guide gets a new contributor
productive in under five minutes.

## Required reading (in order)

1. [`README.md`](./README.md) — what the project does and what's shipping today.
2. [`docs/ROADMAP.md`](./docs/ROADMAP.md) — the explicit delta between bootstrap and production.
3. [`SECURITY.md`](./SECURITY.md) — the threat model the toolchain defends against.
4. [`BENCHMARKS.md`](./BENCHMARKS.md) — performance posture and regression policy.

## One-time setup

Requires Rust 1.92 (pinned in `rust-toolchain.toml`).

```bash
./scripts/install_hooks.sh         # wires pre-commit + pre-push gates
cargo build --release -p ori       # build the CLI once
```

## The local quality loop

Before opening a PR:

```bash
cargo fmt --all                                # format
cargo test --workspace                         # all tests must pass
python3 scripts/validate_all.py --full         # static + clippy + tests + CLI smoke
```

The `--full` gate runs:

1. **Static gate** — required-file layout, JSON contract parse, JSONL
   fixture parse, schema-instance validation (when `jsonschema` is
   installed), shell-hook strict mode, Rust source guardrails (no
   `.unwrap()` / `.expect()` / `panic!` / `todo!` / `unimplemented!` /
   `dbg!` / `unsafe` in production sources), workspace dependency
   allow-list (`serde` + `serde_json` only).
2. **`cargo fmt --all --check`**
3. **`cargo check --workspace --all-targets`**
4. **`cargo clippy --workspace --all-targets -- -D warnings`**
5. **`cargo test --workspace`**
6. **Six CLI contract smoke commands** — `doctor`, `check`, `agent map`,
   `agent explain`, `capsule`, `patch check`.

Don't weaken the gate to make a PR pass. Fix the PR.

## Source guardrails

These are enforced by `scripts/validate_all.py`:

- No `.unwrap()` / `.expect()` in `crates/*/src/**/*.rs` (tests
  included). Use `assert!(false, "...")` with
  `#[allow(clippy::assertions_on_constants)]` when a test must fail
  with context.
- No `panic!`, `todo!`, `unimplemented!`, `dbg!` in production code.
- No `unsafe fn` / `impl` / `trait` / block in any crate source.
- No new third-party dependency without a `CHANGELOG.md` entry
  explaining the rationale. The only currently-approved deps are
  `serde` and `serde_json`.

## JSON contract rules

Every public CLI envelope follows a schema-versioned contract
(`schemas/*.schema.json`):

1. JSON is built through typed `serde` structs, never string
   concatenation.
2. The struct contains a `schema: &'static str` field naming the
   contract id (e.g. `"ori.diagnostic.v1"`).
3. Adding a new contract means adding the schema file, the emitter,
   one round-trip conformance test, and a line in
   `crates/ori-agent/src/extras.rs`'s `doctor_report_json` map.
4. Breaking changes ship as `v2`; never overwrite a `v1`.

## Commit messages

Use the same shape the repo's existing commits use: subject in
imperative mood, body explaining the why. Reference relevant
diagnostic IDs (`E0100`, `B0010`, `P1010`, etc.) where applicable.

## Pull requests

Before requesting review:

- [ ] `python3 scripts/validate_all.py --full` is green.
- [ ] New code has tests.
- [ ] New diagnostics have a golden fixture under `tests/golden/diagnostics/`.
- [ ] New schemas have a Draft 2020-12 file with `examples` + a
      conformance round-trip test.
- [ ] CHANGELOG.md has an entry in the top section.

## Reporting bugs

Use the `.github/ISSUE_TEMPLATE/bug_report.md` template. Include the
exact CLI command, the JSON output (or first 20 lines), the expected
behaviour, and your `target/release/ori doctor` output.

## License

By contributing you agree your contribution is licensed under the
project's Apache-2.0 license (see `LICENSE`).
