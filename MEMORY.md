# MEMORY.md

Persistent architectural memory for Orison.

Agents must treat this file as a compact source of project truth. Update it only when an architectural decision changes or a new durable decision is made.

## Durable decisions

### D001 — Language name and extension

- Working language name: **Orison**.
- File extension: `.ori`.
- CLI command: `ori`.

### D002 — Bootstrap implementation language

- The initial reference compiler scaffold is written in Rust.
- Foundational JSON contract dependencies are permitted: `serde` and `serde_json`.
- Additional dependencies require explicit rationale, tests, and a changelog entry.

### D003 — Core invariant

Anything the compiler knows should be available to tools and agents through stable structured output.

### D004 — Language safety model

- No null.
- No exceptions.
- `Option[T]` and `Result[T, E]` are first-class.
- Safe code must prevent memory unsafety and data races.
- Mutation and effects are explicit.

### D005 — Agent compatibility is a first-class feature

Agent-facing compiler output is not auxiliary. It is part of the language product.

Required artifacts:

- JSON diagnostics
- semantic capsules
- symbol cards
- agent maps
- patch IR
- change manifests
- affected-test graph

### D006 — Standard distribution, not only standard library

Orison should ship an official distribution with these layers:

- `core`
- `std`
- `app`
- `platform`
- `labs`

The goal is to reduce dependency pressure for common production applications.

### D007 — Capability-secured effects

Effects and capabilities are required for both security and context compression.

Examples:

- `uses fs.read`
- `uses net.outbound`
- `uses db.write`
- `uses StripeApi`

### D008 — Compiler architecture

The intended compiler is a query-based incremental compiler with:

- error-tolerant CST
- stable AST node IDs
- typed HIR
- effect graph
- borrow graph
- MIR
- dev backend
- release backend
- Wasm component backend

### D009 — Dev/release split

Safety rules are identical in dev and release modes. Optimization depth changes; safety does not.

### D010 — Patch-native repairs

Agents should be able to apply structural patches instead of rewriting full files.

Patch IR is a public schema and must remain versioned.

### D011 — Contract JSON must be typed

Public compiler-agent JSON must be produced through typed serialization. Manual JSON string concatenation is not acceptable for diagnostics, capsules, patch checks, agent maps, or symbol cards.

## Current scaffold status

The current compiler scaffold can (after waves 1–4 of the multi-agent
build-out on 2026-05-16, ending in 455 passing tests with the full
quality gate green):

- read `.ori` files and lex source.
- extract module declarations, imports, top-level symbols, signatures,
  and `uses` effects.
- produce an error-tolerant CST with stable, content-derived node IDs that
  survive whitespace edits (`crates/ori-compiler/src/cst.rs`).
- resolve multi-module symbol tables, separate value/type/protocol/service
  namespaces, and detect import cycles
  (`crates/ori-compiler/src/resolver.rs`).
- run a baseline signature-level type checker against builtins, declared
  types, and the permitted generics (`Option`, `Result`, `List`, `Pair`,
  `Fn`, `Iter`, `Query`, `Map`, `Set`) — `type_check.rs`.
- emit a capability manifest plus a policy diff and reject undeclared
  ambient effects (`effect_check.rs`).
- format source idempotently from the CST, preserving comments and blank
  lines (`formatter.rs`).
- apply structural patches with stable-id targeting, partial-apply
  semantics, and `sym:` / `mod:` / `node:` id tolerance
  (`patch_apply.rs`).
- emit OpenAPI 3.1, UI manifests, Wasm component manifests, capability
  manifests, and build reports (`openapi.rs`, `ui_check.rs`,
  `wasm_component.rs`, `effect_check.rs`).
- track changes per file via an in-memory FNV-1a hash cache and select
  affected tests (`incremental.rs`).
- lower to a minimal HIR / MIR and run a tree-walking interpreter that
  reports observed effects and callee candidates (`hir.rs`, `mir.rs`,
  `interp.rs`).
- run a deterministic benchmark suite covering eight metrics
  (`bench.rs`, see `BENCHMARKS.md`).
- emit `ori.agent_symbol_list.v1`, `ori.agent_diagnose.v1`,
  `ori.agent_tests.v1`, `ori.benchmark.v1`, `ori.doctor.v1`, and 12 other
  schema-versioned contracts via the `ori-agent` extras and the
  `ori-cli` subcommands listed below.
- expose CLI commands for `check`, `fmt`, `capsule`, `agent map`,
  `agent explain`, `agent symbols`, `agent diagnose`,
  `agent tests --affected`, `patch check`, `patch apply`,
  `patch dry-run`, `patch explain`, `lsp --stdio`, `package check`,
  `audit`, `sbom`, `provenance verify`, `run`, `build`, `bench`,
  `openapi`, `ui`, `wasm`, `capability`, `test`, and `doctor`.

It also now (since wave 4):

- runs a cooperative async scheduler stub with A0001–A0003 diagnostics
  (`async_runtime.rs`).
- imports GraphQL SDL and gRPC `.proto` subset, emitting Orison source
  that parses clean through `Compiler::check_source`.
- estimates per-function test coverage with word-boundary matching
  (`coverage.rs`).
- exposes a local-filesystem registry stub
  (`crates/ori-pkg/src/registry.rs`) supporting publish / fetch / list /
  yank with FNV-1a checksums.
- responds to LSP `workspace/symbol`, `documentSymbol`, `definition`,
  and `references` (`crates/ori-lsp/src/server.rs`).
- enforces design-token references in `view` bodies with D0010 + D0020
  diagnostics.
- emits a mobile-target manifest with effect→permission mapping for
  iOS/Android.
- runs a string-literal-aware preprocessor for `${ENV}` /
  `@orison/<const>` substitution (`preproc.rs`).

It still **cannot** (intentionally, per `docs/ROADMAP.md`):

- compile to native code with optimisation passes — the bootstrap ships
  only textual LLVM-IR-style scaffold output, and a 39-byte hand-rolled
  wasm hello-module.
- run an M:N async runtime — the scheduler is single-threaded
  cooperative.
- sign packages cryptographically — the lockfile checksum is FNV-1a
  and `signature: "self-attested:bootstrap"`.
- infer types bidirectionally inside arbitrary expression bodies — the
  type checker handles signature plus body literals/var/call/block/if/
  match/return/try/construct; no binary-operator typing yet.
- perform region-inference borrow checking — the checker is
  signature-level (B0010–B0050).
- execute arbitrary user expressions with side-effects (the interpreter
  evaluates the body parser's expression set; no I/O, no concurrency).

## Current priority

Land an error-tolerant body-recovery pass on top of the existing CST so
the type checker, effect propagation, and interpreter can move from
"signature-level" to "expression-level". That unlocks real inference,
borrow checking, and `ori run` execution.

### D012 — Canonical validation gate and hooks

The canonical repository validation entry point is:

```bash
python3 scripts/validate_all.py --full
```

The static archive-only gate is:

```bash
python3 scripts/validate_all.py --static-only
```

Local Git hooks live in `.githooks/` and are installed with:

```bash
./scripts/install_hooks.sh
```

Future agents must not weaken these gates to pass a change. Fix the change instead.

### D013 — Continuation handoff is authoritative

`ORISON_AGENT_DEVELOPMENT_HANDOFF.md` is the authoritative continuation file for agents. It defines remaining work, quality gates, validation policy, schema policy, test matrix, implementation tickets, and release readiness criteria.

### D014 — Honest scope statement

Date: 2026-05-16.

Orison is **bootstrap plus early-alpha**, not production-ready. Documentation, prompts,
agent guidance, and any external communication must reflect that. The canonical capability
matrix lives in `README.md` under "What's actually implemented"; cross-reference it before
asserting any feature works. Subsystems listed as "Skeleton" or "Not yet" must not be
described as working.

This decision exists so future agents do not promote in-progress milestones to "done"
in user-facing material before tests, schemas, and the full quality gate confirm the
behavior.

### D015 — Bootstrap apply engine targets the active file only

Date: 2026-05-16.

`patch_apply::apply_patch` resolves operation targets against the CST and AST of
the *single file* it is invoked on. Cross-file patch ops (such as a patch that
inserts into both `catalog.ori` and `ui.ori`) skip the foreign ops with `P1010`
and apply the resident ops. The partial-apply policy is intentional and lives in
`patch_apply.rs`: `P1000`/`P1001`/`P1002`/`P1003` are fatal (no diff is emitted);
`P1010` is per-op (other ops still apply and the partial `after` is returned).
Multi-file orchestration belongs in a higher-level driver and is not in the
bootstrap.

### D016 — Tests may use `assert!(false, ...)` to fail when panic is forbidden

Date: 2026-05-16.

The repository-wide guardrail forbids `panic!`, `unreachable!`, `todo!`,
`unimplemented!`, `unwrap()`, and `expect()` in `crates/*/src/**`. Test bodies
in `ori-pkg` that need to fail with a contextual message use
`assert!(false, "...")` paired with `#[allow(clippy::assertions_on_constants)]`
and a returning `_ => return;` arm. This is the sanctioned escape hatch; do not
introduce `panic!` in tests just to satisfy clippy.
