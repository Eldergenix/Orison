#!/usr/bin/env bash
set -euo pipefail
cargo fmt --all --check
cargo test
cargo run -p ori -- doctor
cargo run -p ori -- check --json examples/hello.ori
cargo run -p ori -- agent map --budget 2000 --json examples/fullstack/users.ori
cargo run -p ori -- capsule --json examples/fullstack/users.ori
cargo run -p ori -- patch check --json examples/agent_patch.json
