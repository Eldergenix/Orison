# Contributing to Orison

Thanks for working on Orison. This document is the developer entry point. It
points at the authoritative policies and describes the workflow every
contributor — human or agent — is expected to follow.

Read these first, in order:

1. [`README.md`](../README.md)
2. [`GOAL.md`](../GOAL.md)
3. [`AGENTS.md`](../AGENTS.md) — mandatory for AI coding agents.
4. [`ORISON_AGENT_DEVELOPMENT_HANDOFF.md`](../ORISON_AGENT_DEVELOPMENT_HANDOFF.md)
   — authoritative roadmap, quality gates, and "Required command loop".
5. [`docs/QUALITY_GATES.md`](./QUALITY_GATES.md)
6. [`docs/CI.md`](./CI.md)
7. [`BENCHMARKS.md`](../BENCHMARKS.md)

## One-time setup

```bash
make install-hooks
```

This wires the pre-commit and pre-push hooks from `.githooks/` into Git via
`core.hooksPath`. To revert:

```bash
make uninstall-hooks
```

You also need a working Rust toolchain that matches `rust-toolchain.toml` and
Python 3.11+ for the validation script. `cargo` and `python3` are the only
hard prerequisites; everything else is invoked by Make.

## Required command loop

The canonical loop is defined in
[`AGENTS.md` "Required command loop"](../AGENTS.md#required-command-loop) and
in [`ORISON_AGENT_DEVELOPMENT_HANDOFF.md` "Required quality gates"](../ORISON_AGENT_DEVELOPMENT_HANDOFF.md#required-quality-gates).
Mirror them here for convenience; if they disagree, the handoff wins.

### Before changes

```bash
make install-hooks
make gate-fast
make fmt-check
make test
make check
make agent-map
```

### After changes

```bash
make fmt-check
make test
make check
cargo run -p ori -- check --json examples/fullstack/users.ori
make capsule
make patch-check
make gate-full
```

If the change affects CLI behaviour:

```bash
make doctor
cargo run -p ori -- help
```

## Pull request expectations

Every PR must:

- Pass the static gate (`make gate-fast`).
- Pass the full local gate (`make gate-full`) on a Rust-capable machine. CI
  re-runs the full matrix; do not rely on CI to discover failures.
- Update [`CHANGELOG.md`](../CHANGELOG.md) for any externally visible change.
- Update [`TASKS.md`](../TASKS.md) when task status changes.
- Update [`MEMORY.md`](../MEMORY.md) when an architectural decision changes,
  a new workspace dependency is added, or a quality gate is changed.
- Update the relevant `docs/` files when semantics change (see
  [`AGENTS.md` "Documentation update rules"](../AGENTS.md#documentation-update-rules)).
- Update or extend tests for new behaviour. JSON contract changes require
  schema updates and example refreshes.
- Avoid touching unrelated files. Small, coherent diffs are easier to review
  and easier to revert.

The PR template at
[`.github/PULL_REQUEST_TEMPLATE.md`](../.github/PULL_REQUEST_TEMPLATE.md)
enforces the above with a checklist. Fill in every section; the reviewer is
explicitly empowered to bounce a PR that leaves sections blank.

## Source guardrails

The static gate rejects, in any file under `crates/*/src/**/*.rs`:

- `unwrap()`, `expect()`
- `panic!`, `todo!`, `unimplemented!`, `dbg!`
- `unsafe fn`, `unsafe impl`, `unsafe trait`, `unsafe { ... }`

Workspace dependencies are restricted to the approved bootstrap set
(currently `serde`, `serde_json`). Adding one requires a `MEMORY.md` decision
entry and a `CHANGELOG.md` note. See
[`docs/QUALITY_GATES.md` "Source guardrails"](./QUALITY_GATES.md#source-guardrails).

## JSON contract discipline

Public schemas live under `schemas/` and use JSON Schema Draft 2020-12. Any
change to a public schema is a contract change. Follow
[`AGENTS.md` "JSON contract rules"](../AGENTS.md#json-contract-rules):
add a new schema version, keep the old one, refresh examples, and write a
migration note in `CHANGELOG.md`.

## Filing issues

Use the templates under [`.github/ISSUE_TEMPLATE/`](../.github/ISSUE_TEMPLATE/).
Bug reports must include a minimal reproducer plus the output of `ori doctor`.
Feature requests must explain the agent-visible contract impact, not only the
human-facing UX.

## When in doubt

Read [`AGENTS.md`](../AGENTS.md) again. The "Forbidden shortcuts" section
exists because every entry in it has been tried at least once. Don't be the
next entry.
