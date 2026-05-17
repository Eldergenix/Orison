# Code Review Checklist

- [ ] Does the change preserve agent-facing JSON contracts?
- [ ] Are public JSON outputs emitted through typed serialization rather than string concatenation?
- [ ] Are diagnostics stable, parseable, and useful?
- [ ] Are spans correct enough for structural repair?
- [ ] Is the smallest possible context exposed to agents?
- [ ] Are new capabilities/effects explicit?
- [ ] Are invalid inputs tested, not only happy paths?
- [ ] Are tests added or updated?
- [ ] Are docs and schemas updated where needed?
- [ ] Does `cargo fmt --all --check` pass?
- [ ] Does `cargo clippy --workspace --all-targets -- -D warnings` pass?
- [ ] Does `cargo test` pass?
