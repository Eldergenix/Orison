# Quality Gates

Quick-reference card for the four validation layers used in this repository. The
authoritative version of this policy lives in `ORISON_AGENT_DEVELOPMENT_HANDOFF.md`
under "Required quality gates". When that file disagrees with this card, the handoff wins.

## Validation pyramid

```text
                CI gate (GitHub Actions)
              ----------------------------
                Pre-push gate (local hook)
              ----------------------------
                Pre-commit gate (local hook)
              ----------------------------
                Static gate (always available)
```

Each layer is a strict superset of the one below it. Higher layers must never be weakened to
let a change pass; fix the change instead.

## Commands

### Static gate

Runs everywhere, including in environments without a Rust toolchain.

```bash
python3 scripts/validate_all.py --static-only
```

Checks: required files and directories exist, JSON schemas parse and declare Draft 2020-12,
JSON examples parse, JSONL golden fixtures parse line-by-line, schema-instance validation
runs when `jsonschema` is installed, shell scripts and Git hooks use strict mode and pass
`bash -n`, Git hooks are executable, Rust production source has no `unwrap()`, `expect()`,
`panic!`, `todo!`, `unimplemented!`, `dbg!`, or unsafe Rust, workspace dependencies remain
within the approved bootstrap set.

### Pre-commit gate

Installed by `./scripts/install_hooks.sh`. Runs on `git commit`.

```bash
python3 scripts/validate_all.py --pre-commit
```

Adds on top of the static gate:

```bash
cargo fmt --all --check
cargo check --workspace --all-targets
```

### Pre-push gate

Installed by `./scripts/install_hooks.sh`. Runs on `git push`.

```bash
python3 scripts/validate_all.py --full
```

Equivalent Make target:

```bash
make quality-gate
```

Adds on top of the pre-commit gate:

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p ori -- doctor
cargo run -p ori -- check --json examples/hello.ori
cargo run -p ori -- agent map --budget 2000 --json examples/fullstack/users.ori
cargo run -p ori -- agent explain sym:store.users.fetch_user --json examples/fullstack/users.ori
cargo run -p ori -- capsule --json examples/fullstack/users.ori
cargo run -p ori -- patch check --json examples/agent_patch.json
```

### CI gate

`.github/workflows/ci.yml` runs the full gate. CI must remain at least as strict as the
pre-push gate. If a CI-only check is added, mirror it locally so contributors can reproduce
the failure.

## Installation

Install the hooks once per clone:

```bash
./scripts/install_hooks.sh
```

This wires `.githooks/pre-commit` and `.githooks/pre-push` into `.git/hooks/`.

## Source guardrails

The static gate rejects any of the following in `crates/*/src/**/*.rs`:

- `unwrap()` / `expect()`
- `panic!` / `todo!` / `unimplemented!` / `dbg!`
- `unsafe fn` / `unsafe impl` / `unsafe trait` / `unsafe {`

Workspace dependencies must remain within the approved bootstrap set:

- `serde`
- `serde_json`

Adding a new workspace dependency requires a `MEMORY.md` decision entry, a `CHANGELOG.md`
note, and tests that the new dependency does not destabilize public JSON contracts.

## Schema policy

Every public contract under `schemas/` must:

1. Declare `"$schema": "https://json-schema.org/draft/2020-12/schema"`.
2. Include `title`, `type`, `required`, and an `additionalProperties` policy.
3. Have at least one parseable example under `examples/` or `tests/golden/`.
4. Be wired into `SCHEMA_MAP` in `scripts/validate_all.py` once a canonical instance exists.

Changing a public schema requires a version bump, a migration note in `CHANGELOG.md`, and
either an additive change or a new schema version that coexists with the old one.

## When a gate fails

1. Read the error. The static gate prints actionable error lines prefixed with `error:`.
2. Reproduce locally with the narrowest command from the gate.
3. Fix the underlying issue. Do not loosen the gate, skip a hook, or comment out a check.
4. Re-run the gate from the same layer that failed.
