# Code Review Remediation Report

This file records the issues a strict senior reviewer would reasonably reject in the first scaffold and the fixes applied in v0.1.1.

## Review-blocking issues fixed

| Issue | Why it would be rejected | Fix |
|---|---|---|
| Public JSON built with string concatenation | Fragile escaping, schema drift risk, hard to evolve safely | Added typed `serde` serialization for diagnostics, capsules, agent maps, symbol cards, and patch checks |
| Patch validation used substring checks | Invalid JSON could pass or valid JSON could fail for irrelevant substrings | Added `serde_json` parsing and semantic validation for schema, intent, operation arrays, known operation names, and required operation fields |
| CLI could not emit capsules even though the compiler generated them | Dead functionality and incomplete agent ABI surface | Added `ori capsule` and `ori agent capsule` |
| Parser emitted malformed signatures like `fn main () -> Unit:` | Agent maps and capsules depend on exact compact signatures | Fixed signature compaction and removed trailing declaration colons |
| `null` detection scanned raw lines | False positives inside strings and comments | Moved reserved-runtime checks to lexer tokens |
| Imports were ignored | Agent context and capsules lacked dependency context | Added import parsing and capsule/agent-map import output |
| No toolchain pin or editor normalization | Reproducibility and formatting drift risk | Added `rust-toolchain.toml` and `.editorconfig` |
| Weak tests | Public agent contracts were not parse-tested | Added JSON parse tests for diagnostics, capsules, patch checks, agent maps, and symbol cards |
| Schemas incomplete for actual CLI outputs | Agents need stable machine contracts | Added `patch-check.schema.json` and `symbol-card.schema.json`; expanded `agent-map.schema.json` |
| Stated dependency policy conflicted with contract quality | “No dependencies” encouraged worse JSON handling | Updated policy to allow foundational serialization dependencies only |

## Still intentionally not implemented

These are not review fixes; they are future implementation work tracked in `TASKS.md`:

- real CST/AST pipeline
- type checker
- borrow checker
- native/Wasm backends
- structural patch application
- package manager
- standard distribution implementation
- full UI/backend framework implementation

The scaffold must not claim these are complete.
