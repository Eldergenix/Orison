# Orison Roadmap

This roadmap is the *delta* between the current bootstrap and a production
language. It tracks what is left after the bootstrap drop.

## Currently shipping (waves 1–4)

The bootstrap ships:

- Lexer + source manager + error-tolerant CST + stable node IDs.
- Item parser + body parser (literals, vars, calls, blocks, if, match,
  return, try, lambda, record, tuple, construct).
- Multi-module resolver (namespaces, duplicates, cycles, imports).
- Signature-level type checker + expression-level type inference
  (Lit/Var/Block/If/Match/Return/Try/Call/Construct) with W0530–W0541.
- Effect propagation through call graph with `change_signature` Patch IR
  fix suggestions (E0420).
- Capability manifest + policy diff (E0410).
- Borrow checker prototype B0010–B0050.
- Exhaustive match checker (E0540) + constant folding pass.
- HIR / MIR / tree-walking executing interpreter (R0001–R0005).
- Hand-rolled wasm bytecode encoder (39-byte hello-module) + textual
  LLVM-IR-style codegen.
- Async cooperative scheduler stub (A0001–A0003).
- Patch IR validation + apply + dry-run + explain with partial-apply
  semantics + cross-format ID resolution.
- Formatter (CST-preserving, idempotent).
- Standard distribution: 24+ modules across core/std/app/platform/labs.
- Backend framework (service, route, OpenAPI 3.1).
- UI framework (view DSL + a11y heuristics + design-token enforcement).
- Database framework (typed query shape check + migration toposort).
- Package manager (manifest, lockfile, SBOM, audit, provenance) +
  local registry stub (publish/fetch/list/yank).
- GraphQL SDL importer + gRPC `.proto` subset importer.
- Test coverage estimator + affected-test selector + query engine.
- LSP server (hover, completion, rename, code actions, workspace/symbol,
  documentSymbol, definition, references).
- Documentation generator + edition migration tool.
- Macro pre-processor stub (`${ENV}` / `@orison/<const>` substitution).
- Mobile build manifest pipeline (iOS / Android permission mapping).
- Security audit suite (capability bypass, lockfile tamper, SBOM shape,
  provenance failure, unsafe surface report, capability runtime denial).
- Benchmark harness with real numbers in BENCHMARKS.md +
  BENCHMARKS.results.json.
- 348+ workspace tests, `python3 scripts/validate_all.py --full` green.
- 26+ schema-versioned JSON contracts.
- 27+ CLI subcommands.
- 6 example apps + 24+ stdlib modules.
- CI: static / test / release / sbom workflows + Makefile targets + PR
  template + issue templates.

## What is *not* yet shipping to production grade

These are the gaps between the current bootstrap and a Rust/Swift/Go-equivalent
production language. Each is named honestly with an estimate of remaining
work that exceeds a single session's scope.

### Type system

- Full bidirectional type inference (HM-style) inside arbitrary expression
  bodies, including binary operators, generic instantiation by usage, trait
  resolution, default method bodies, and associated types.
- Type classes / protocols with coherence rules.
- Higher-kinded types.
- Refinement / dependent types (out of scope — research project).
- GADT-style variants.
- Row polymorphism for records.

### Memory model

- Region inference for the borrow checker — currently signature-level only.
- Lifetime parameters on functions and types.
- Move-after-use checking inside expression bodies (currently signature
  scope).
- Arena lifetime tracking.
- Safe wrapper contracts for `Shared` / `Weak`.
- Borrow checker integration with the type checker for path-sensitive
  type narrowing.

### Codegen

- Native AOT codegen (LLVM, Cranelift, or custom IR). The bootstrap
  ships an LLVM-IR-shaped textual emitter; no native binary executor.
- Optimisation passes (constant propagation, dead-code elimination,
  inlining, register allocation).
- ABI stability tests.
- Wasm component encoder beyond the hello-module shape (multi-function,
  data section, imports, memory).
- Wasm component WIT (interface definition language) generator.
- Linker integration.

### Runtime

- Real async runtime with M:N scheduling (the bootstrap scheduler is
  cooperative + single-threaded).
- Garbage collector (not planned; ownership model is the alternative).
- Panic / unwind semantics.
- Stack traces in `RuntimeError`.

### Package manager

- Real cryptographic signing (Sigstore / GPG). The bootstrap checksum
  is FNV-1a and `signature: "self-attested:bootstrap"`.
- Distributed registry with HTTPS protocol.
- Resolver with version-range constraints and SAT-style solving.
- Build script sandboxing.
- Mirror / vendor / lockfile-tamper-detection at the wire layer.

### Editor tooling

- LSP semantic-token highlighting.
- LSP code-lens.
- LSP inlay hints for inferred types.
- LSP refactorings beyond rename (extract function, inline variable,
  move to module).
- LSP test runner integration.

### Standard distribution

- Concurrency primitives (channels, async queues, mutexes — the bootstrap
  scheduler is single-threaded).
- TLS / HTTP client with full RFC compliance.
- SQL DSL with parameterised statements + connection pool.
- Full crypto suite (X25519, Ed25519, ChaCha20-Poly1305, KDF).
- Graphics / GPU bindings.
- File I/O with async.
- Datetime with timezone awareness (not just ISO-8601 strings).

### Framework

- Full backend service runtime (the bootstrap parses routes; no
  dispatcher yet).
- Auth / session middleware with cookie + JWT support.
- Database driver wiring.
- UI render pipeline beyond the manifest extractor.
- Mobile native UI bindings (UIKit / Jetpack Compose).
- Native form validation flows.

### Conformance

- Full grammar coverage in `tests/golden/parser/` (current coverage:
  hello, full_signatures, variants, records, services, views).
- Negative-test corpus per diagnostic ID.
- ABI stability tests across compiler versions.
- Edition compatibility test corpus.

### Performance

- Incremental cache backed by on-disk artefacts (currently in-memory).
- Per-query memoisation.
- Parallel compilation (currently single-thread).
- Benchmark regression budgets enforced in CI.

## Honest scope statement

The bootstrap is **alpha-shaped**: every shipping feature has a stable
schema, tests, and a CLI surface. The "Not yet shipping" list above is
the work between alpha and production. None of it is hidden; the
contracts shipping today are designed to remain stable across those
implementations.

Future contributors should consult this roadmap before promising any
feature beyond what is listed in "Currently shipping".
