# Validation Report

Generated: 2026-05-16

## Completed in this archive-generation environment

The environment used to produce this archive does not have a Rust toolchain installed, so Rust compilation and tests could not be executed here. Static validation was completed instead.

Completed checks:

- Required root files exist and are non-empty.
- Required source, docs, schemas, examples, scripts, hooks, prompts, and test directories exist.
- JSON schemas in `schemas/*.json` parse successfully.
- JSON examples in `examples/*.json` parse successfully.
- JSONL golden fixtures in `tests/golden/**/*.jsonl` parse successfully line by line.
- Schema-instance validation was executed where Python `jsonschema` was available.
- Shell scripts and Git hooks pass `bash -n`.
- Shell scripts and Git hooks use `set -euo pipefail`.
- Git hooks are executable.
- Production Rust source guardrails were checked for forbidden panic/debug shortcuts.
- Workspace dependency policy was checked.
- Zip archive integrity was checked after packaging.

Command used here:

```bash
python3 scripts/validate_all.py --static-only
```

## Required validation on a Rust-capable machine

Run the full gate after extracting the archive:

```bash
./scripts/install_hooks.sh
python3 scripts/validate_all.py --full
```

Equivalent Make target:

```bash
make quality-gate
```

The full gate runs:

```bash
cargo fmt --all --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p ori -- doctor
cargo run -p ori -- check --json examples/hello.ori
cargo run -p ori -- agent map --budget 2000 --json examples/fullstack/users.ori
cargo run -p ori -- agent explain sym:store.users.fetch_user --json examples/fullstack/users.ori
cargo run -p ori -- capsule --json examples/fullstack/users.ori
cargo run -p ori -- patch check --json examples/agent_patch.json
```

## Validation policy

No future agent or maintainer should mark a feature complete unless `python3 scripts/validate_all.py --full` passes on a Rust-capable machine. If the environment cannot run Rust, the response must say so explicitly and include the static validation result.
