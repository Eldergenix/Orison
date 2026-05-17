# Research Notes and Design Rationale

This file records durable research directions an agent should continue to review.

## Relevant existing systems to study

- Rust: ownership, borrow checking, diagnostics, JSON output, cargo ecosystem.
- Go: fast builds, simple syntax, standard library coherence.
- Zig: explicit allocation, comptime, small toolchain philosophy.
- Swift/Kotlin: modern application syntax and mobile ergonomics.
- TypeScript: structural tooling, language server ecosystem, web compatibility.
- Gleam/Elm: friendly functional type systems and helpful errors.
- Tree-sitter: incremental error-tolerant parsing.
- Salsa-style query engines: incremental compiler architecture.
- WebAssembly Component Model: portable component ABI.
- Language Server Protocol: editor JSON-RPC protocol model.
- Model Context Protocol and AGENTS.md: agent/tool integration conventions.
- Aider repo maps: token-budgeted repository context.
- SWE-bench-style evaluations: real-world coding-agent benchmarks.

## Research questions

1. How small can a language grammar be while supporting full-stack application work?
2. Which compiler facts most reduce agent token consumption?
3. Which diagnostics produce the highest patch success rate for small models?
4. How should capabilities be represented to be both secure and ergonomic?
5. How should UI consistency and accessibility be statically checked?
6. What backend architecture best balances dev build speed and release performance?
7. Can Patch IR reduce regressions compared with whole-file generation?

## Agent research rule

When this repository is connected to web access, agents should periodically update this file with current findings and links, then convert findings into concrete tasks in `TASKS.md`.
