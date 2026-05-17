#!/usr/bin/env bash
set -euo pipefail
# Local quality loop. Mirrors the full CI gate.
cargo fmt --all --check
cargo test --workspace
cargo run -p ori -- check --json examples/hello.ori
cargo run -p ori -- check --json examples/fullstack/users.ori
cargo run -p ori -- agent map --budget 2000 --json examples/fullstack/users.ori
cargo run -p ori -- agent explain sym:store.users.fetch_user --json examples/fullstack/users.ori
cargo run -p ori -- capsule --json examples/fullstack/users.ori
cargo run -p ori -- patch check --json examples/agent_patch.json
