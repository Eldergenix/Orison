# Compiler Architecture

The Orison compiler is designed for fast safe iteration and agent-native code repair.

## Pipeline

```text
source
  ↓
lexer
  ↓
error-tolerant CST
  ↓
AST with stable node IDs
  ↓
name resolution
  ↓
typed HIR
  ↓
effect graph
  ↓
ownership / borrow graph
  ↓
MIR
  ↓
dev backend / release backend / Wasm backend
  ↓
binary, component, bundle, capsule graph
```

## Design requirements

- Every phase is query-addressable.
- Every phase can emit JSON diagnostics.
- Every syntax node has a stable structural ID.
- Every public symbol can emit a symbol card.
- Every module can emit a semantic capsule.
- Every diagnostic can include structured repair candidates.
- Public JSON contracts are emitted through typed serialization.

## Incrementality

The compiler should cache by:

| Unit | Cache key |
|---|---|
| Lexed file | file hash |
| CST | file hash + grammar version |
| AST | CST hash + lowering version |
| Name resolution | import graph hash |
| Types | symbol signature hash |
| Effects | typed body hash |
| Borrow graph | typed/effect body hash |
| MIR | borrow-checked HIR hash |
| Codegen | MIR hash + target triple |
| Capsule | public API hash + effect hash |

## Stable node IDs

Stable node IDs should be derived from:

```text
module path + declaration kind + nearest symbol + local structural hash
```

Do not use raw line numbers as identity. Line numbers are spans, not identities.

## Dev and release modes

Safety is identical in all modes.

- Dev mode prioritizes low latency.
- Release mode prioritizes optimized output.

## Agent-facing outputs

The compiler must expose:

```bash
ori check --json
ori capsule --json FILE
ori agent map --budget N --json FILE
ori agent explain SYMBOL --json FILE
ori agent diagnose --json
ori patch check PATCH.json
ori patch apply PATCH.json
```

## Current scaffold

The current Rust code implements only the first bootstrap subset:

- source model with spans;
- lexer;
- symbol-oriented parser;
- import extraction;
- basic forbidden-construct diagnostics;
- serde-backed JSON diagnostics;
- semantic capsule generation;
- agent map generation;
- symbol-card generation;
- Patch IR semantic validation.

It does not yet implement a real CST, AST, type checker, borrow checker, or backend.
