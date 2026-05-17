# Orison Agent Development Handoff

**Project:** Orison programming language  
**Repository role:** bootstrap reference implementation and specification scaffold  
**Primary audience:** AI coding agents and senior engineers continuing implementation  
**Required behavior:** do not claim language completion until every quality gate, schema contract, CLI contract, and end-to-end acceptance test in this file passes.

---

## Current repository state

This repository is a continuation-ready scaffold, not a complete programming language.

The current implementation contains:

- Rust workspace with three crates:
  - `crates/ori-compiler`: lexer, minimal symbol parser, diagnostics, semantic capsule generation, Patch IR validation, source spans, formatting scaffold.
  - `crates/ori-agent`: agent map and symbol-card output.
  - `crates/ori-cli`: `ori` bootstrap CLI.
- Stable JSON schema contracts under `schemas/`.
- Example Orison source files under `examples/`.
- Golden diagnostic fixtures under `tests/golden/`.
- Language/compiler/framework/security/stdlib specs under `docs/`.
- Root operating instructions:
  - `README.md`
  - `AGENTS.md`
  - `MEMORY.md`
  - `TASKS.md`
  - `GOAL.md`
  - `CHANGELOG.md`
  - `VALIDATION.md`
- Quality gate infrastructure:
  - `scripts/validate_all.py`
  - `scripts/check_json_contracts.sh`
  - `scripts/install_hooks.sh`
  - `.githooks/pre-commit`
  - `.githooks/pre-push`
  - `.github/workflows/ci.yml`
  - `Makefile`

The current implementation intentionally recognizes only a small subset of Orison syntax. The next phase must replace the minimal parser with a real compiler frontend while preserving the public JSON contracts that agents consume.

---

## Mandatory first action for any AI agent

Before changing code, run this read sequence:

```bash
cat GOAL.md
cat MEMORY.md
cat TASKS.md
cat AGENTS.md
cat ORISON_AGENT_DEVELOPMENT_HANDOFF.md
cat VALIDATION.md
```

Then install hooks:

```bash
./scripts/install_hooks.sh
```

Then run the appropriate gate:

```bash
make quality-gate
```

If Rust is not installed, run only archive/static validation and explicitly record that Rust validation was not executed:

```bash
python3 scripts/validate_all.py --static-only
```

Do not skip validation silently.

---

## Design invariants that must not be broken

1. **Compiler truth is agent truth.** Anything the compiler knows must be available through stable structured output.
2. **JSON contracts are public APIs.** Diagnostics, patches, capsules, agent maps, symbol cards, package manifests, capabilities, and change manifests are schema-versioned APIs.
3. **Safe code is the default.** No null, no exceptions, no implicit ambient filesystem/network/process capabilities, no unchecked shared mutable state, and no unsafe Rust in the bootstrap implementation without an explicit reviewed exception.
4. **Small patches beat rewrites.** Agents should prefer structural Patch IR and narrow source edits over whole-file rewrites.
5. **Every new behavior requires tests.** Parser behavior, type behavior, diagnostics, schemas, CLI output, and agent output must all have tests.
6. **No silent dependency creep.** New Rust dependencies require rationale in `MEMORY.md`, a `CHANGELOG.md` entry, and tests proving the dependency does not destabilize public contracts.
7. **No raw public JSON assembly.** Public JSON output must use typed structs plus `serde`/`serde_json` serialization. The only exception is the emergency serialization fallback utility in `crates/ori-compiler/src/json.rs`.
8. **No panic-driven compiler behavior.** Production Rust source must not use `unwrap()`, `expect()`, `panic!`, `todo!`, `unimplemented!`, or `dbg!`.
9. **Diagnostics must be actionable.** Any compiler error exposed to users or agents must include stable ID, severity, message, span, expected/found data where applicable, docs references, and agent summary.
10. **Edit-check-repair loop is the product.** Runtime performance matters, but the product wedge is fast, safe, agent-native iteration.

---

## End-to-end remaining work

The language is incomplete until every milestone below is implemented, validated, and documented.

### M0 — Repository control plane and quality gates

**Status:** partially implemented. Continue hardening.

Required completion:

- Keep `.githooks/pre-commit` and `.githooks/pre-push` executable.
- Keep `scripts/validate_all.py` as the canonical local validation entry point.
- Keep CI equivalent to or stricter than local full validation.
- Add missing schema-instance validation as new schemas are introduced.
- Add license, contribution policy, security policy, release process, and RFC process when the project moves beyond private bootstrap.
- Add a repository health command:

```bash
ori doctor --json
```

Acceptance criteria:

```bash
python3 scripts/validate_all.py --static-only
make quality-gate
```

Both pass on a Rust-capable machine.

---

### M1 — Real source frontend

**Status:** minimal lexer and declaration parser exist. Not production-ready.

Build:

- Source manager with file IDs, stable spans, line/column lookup, and deterministic diagnostics.
- Lexer covering the full grammar in `docs/language/GRAMMAR.ebnf`.
- Error-tolerant concrete syntax tree (CST).
- AST lowering from CST.
- Stable node IDs for structural patches.
- Import/module graph resolver.
- Public/private visibility model.
- Parser recovery for incomplete code so editor/agent tooling still works mid-edit.
- Formatter based on CST/AST, not whitespace trimming.

Tests required:

- Lexer unit tests for every token class.
- Parser golden tests for valid syntax.
- Parser recovery golden tests for malformed syntax.
- Stable node ID tests proving IDs remain stable after unrelated edits.
- Formatter snapshot tests.
- CLI tests for `ori fmt`, `ori check`, and JSON diagnostics.

Acceptance commands:

```bash
cargo test -p ori-compiler lexer
cargo test -p ori-compiler parser
cargo test -p ori-compiler formatter
cargo run -p ori -- check --json examples/hello.ori
```

---

### M2 — Name resolution and module system

**Status:** top-level symbol extraction exists. Real resolution is not implemented.

Build:

- Symbol interner.
- Module path canonicalization.
- Import aliases.
- Re-exports.
- Visibility checks.
- Duplicate symbol handling by namespace.
- Type/value/protocol namespace separation.
- Dependency graph for modules and symbols.
- Cycle detection with actionable diagnostics.

Tests required:

- Duplicate symbol tests.
- Import alias tests.
- Missing import tests.
- Visibility violation tests.
- Module cycle tests.
- Agent map tests showing resolved dependencies.

Acceptance commands:

```bash
cargo test -p ori-compiler resolver
cargo run -p ori -- agent map --budget 2000 --json examples/fullstack/users.ori
```

---

### M3 — Type system

**Status:** type surface exists only as documentation and light parsing.

Build:

- Primitive types.
- Records.
- Variants/algebraic data types.
- Newtypes/wrapper types.
- `Option[T]` and `Result[T, E]` semantics.
- Local type inference.
- Function signatures.
- Generic types and functions.
- Protocol declarations and implementations.
- Trait/protocol bounds.
- Pattern typing.
- Exhaustive match checking.
- No-null enforcement.
- No-exception model.
- Public API explicit-signature requirement.

Tests required:

- Type inference unit tests.
- Generic instantiation tests.
- Protocol implementation tests.
- Newtype non-confusion tests.
- Exhaustive match diagnostics and suggested patches.
- `Result`/`Option` propagation tests.
- Golden diagnostics for type mismatch, missing return type, ambiguous inference, invalid generic arity.

Acceptance commands:

```bash
cargo test -p ori-compiler types
cargo run -p ori -- check --json examples/bad_null.ori
```

---

### M4 — Effects and capabilities

**Status:** simple effect-name validation exists. Real capability checking is not implemented.

Build:

- Effect grammar and semantic model.
- Capability declarations.
- Function effect propagation.
- Effect subtyping or inclusion rules.
- Package manifest capability declarations.
- Deny undeclared filesystem, network, environment, process, database, secret, GPU, UI, and unsafe effects.
- Capability manifests emitted by `ori check`/`ori build`.
- Agent summaries of security-sensitive effects.

Tests required:

- Unknown effect diagnostics.
- Missing capability diagnostics.
- Effect propagation tests.
- Capability manifest schema tests.
- Package capability diff tests.
- Security tests proving ambient access is rejected.

Acceptance commands:

```bash
cargo test -p ori-compiler effects
cargo run -p ori -- check --json examples/fullstack/users.ori
```

---

### M5 — Ownership, borrowing, and memory safety

**Status:** design documented. Not implemented.

Build:

- Move semantics.
- Borrow semantics.
- Explicit mutable borrow rules.
- Copy/clone model.
- Owned heap allocation model.
- Arena allocation model.
- `Shared[T]` / `Weak[T]` semantics.
- Data-race prevention for concurrent code.
- Safe FFI boundary policy.
- Unsafe Orison block model, even if initially rejected in MVP.

Tests required:

- Move-after-use diagnostics.
- Double mutable borrow diagnostics.
- Shared mutable state rejection.
- Arena lifetime tests.
- Safe wrapper contract tests.
- Golden diagnostics with Patch IR suggestions where repairable.

Acceptance commands:

```bash
cargo test -p ori-compiler ownership
cargo test -p ori-compiler borrow
```

---

### M6 — Intermediate representation and execution

**Status:** not implemented.

Build:

- Typed HIR.
- MIR suitable for optimization and codegen.
- Constant evaluation for simple pure expressions.
- Dev execution backend:
  - interpreter, bytecode VM, or baseline native codegen.
- Release backend plan:
  - Cranelift, LLVM, MLIR, or custom backend.
- Wasm component backend plan.
- Runtime ABI definition.
- ABI stability tests.

Tests required:

- HIR lowering tests.
- MIR golden tests.
- Constant evaluation tests.
- Dev execution smoke tests.
- Backend-independent semantic tests.
- ABI fixture tests.

Acceptance commands:

```bash
cargo test -p ori-compiler hir
cargo test -p ori-compiler mir
cargo run -p ori -- run examples/hello.ori
```

---

### M7 — Incremental compiler and build system

**Status:** design documented. Not implemented.

Build:

- Query engine or equivalent incremental cache.
- File hash cache.
- AST/HIR/type/effect/borrow/MIR cache layers.
- Per-symbol dependency graph.
- Affected-test computation.
- Build graph.
- Dev/release build modes.
- Artifact cache invalidation.
- Deterministic build outputs.

Tests required:

- Incremental invalidation unit tests.
- One-line edit only invalidates affected symbols.
- Cache corruption recovery test.
- Affected-test selection tests.
- Build artifact reproducibility test.

Acceptance commands:

```bash
cargo test -p ori-compiler incremental
cargo run -p ori -- test --changed --json
```

---

### M8 — Agent Context ABI

**Status:** basic agent map, symbol card, and capsule output exist. Needs real compiler integration.

Build:

- `ori agent map --budget N --json` with budget-respecting summarization.
- `ori agent symbols --changed --json`.
- `ori agent explain <symbol> --json`.
- `ori agent diagnose --json`.
- `ori agent tests --affected <symbol> --json`.
- `ori agent capsule <module> --json`.
- Semantic capsules with exports, imports, effects, invariants, tests, dependency edges, and token summaries.
- Agent map compression strategy.
- Stable schema versions.
- Context-budget tests.

Tests required:

- Schema validation for all agent outputs.
- Budget enforcement tests.
- Symbol lookup tests by ID and name.
- Changed-symbol tests.
- Minimal-context recommendation tests.
- Agent contract golden fixtures.

Acceptance commands:

```bash
cargo test -p ori-agent
cargo run -p ori -- agent map --budget 1000 --json examples/fullstack/users.ori
cargo run -p ori -- agent explain sym:store.users.fetch_user --json examples/fullstack/users.ori
```

---

### M9 — Patch IR and repair loop

**Status:** Patch IR validator exists. Patch application is not implemented.

Build:

- Structural patch parser.
- Patch validator with operation-specific schemas.
- Patch applier using stable node IDs.
- Patch dry-run command.
- Patch explanation command.
- Patch provenance metadata.
- Safety checks that reject destructive broad rewrites unless explicitly allowed.
- Patch-to-test binding.

Commands to implement:

```bash
ori patch check --json patch.json
ori patch apply --json patch.json
ori patch explain --json patch.json
ori patch dry-run --json patch.json
```

Tests required:

- Operation-specific validation tests.
- Apply patch to AST/CST tests.
- Reject stale node ID tests.
- Preserve formatting tests.
- Patch provenance tests.
- End-to-end diagnostic-to-patch-to-test loop.

Acceptance commands:

```bash
cargo test -p ori-compiler patch
cargo run -p ori -- patch check --json examples/agent_patch.json
```

---

### M10 — Package manager and manifests

**Status:** manifest schema exists. Package manager not implemented.

Build:

- `ori.toml` parser and validator.
- Lockfile format.
- Dependency resolver.
- Registry protocol.
- Local path dependencies.
- Version constraints.
- Package capability declarations.
- Build-script capability restrictions.
- Dependency capability diff.
- SBOM generation.
- Provenance verification.

Commands to implement:

```bash
ori package check --json
ori add <package>
ori update
ori audit --json
ori vendor
ori publish --dry-run
ori sbom --json
ori provenance verify --json
```

Tests required:

- Manifest schema tests.
- Resolver tests.
- Lockfile reproducibility tests.
- Capability diff tests.
- Build script denial tests.
- Audit output schema tests.

Acceptance commands:

```bash
cargo test package
cargo run -p ori -- package check --json
cargo run -p ori -- audit --json
```

---

### M11 — Standard distribution

**Status:** documented only.

Build in layers:

1. `core`: types, result, option, iterators, strings, bytes, collections base.
2. `std`: JSON, paths, filesystem, env, process, time, logging, HTTP, crypto, regex, config, validation.
3. `app`: service/router/middleware/auth/session/forms/UI/state/design/testing.
4. `platform`: web, Wasm, iOS, Android, desktop, edge, GPU/tensor.
5. `labs`: autodiff, agent, robotics, embedded, experimental ML.

Required policy:

- Standard modules must be tree-shakable.
- Standard modules must declare effects/capabilities.
- Standard modules must expose agent capsules.
- No standard module may depend on undeclared ambient authority.

Tests required:

- API conformance tests for each module.
- Capability tests.
- Serialization tests.
- No-dependency baseline tests.
- Bundle-size tests for small programs.

Acceptance commands:

```bash
cargo test stdlib
cargo run -p ori -- build --dev examples/hello.ori
```

---

### M12 — Backend framework

**Status:** documented only.

Build:

- `service` declarations.
- Typed routes.
- Path parameter parsing.
- Request/response body typing.
- Middleware.
- Auth policies.
- Validation integration.
- OpenAPI generation.
- Typed clients.
- Observability spans.

Tests required:

- Route parser tests.
- Typed path parameter tests.
- Request body validation tests.
- Middleware order tests.
- OpenAPI golden tests.
- Security policy tests.

Acceptance commands:

```bash
cargo test backend
cargo run -p ori -- openapi --json examples/fullstack/users.ori
```

---

### M13 — API, data, and database framework

**Status:** documented only.

Build:

- SQL query DSL.
- Schema/migration DSL.
- Query result shape checking.
- Migration graph.
- OpenAPI import.
- GraphQL import.
- gRPC or schema-based RPC support.
- Typed API clients.

Tests required:

- Query type checking tests.
- Migration ordering tests.
- Migration rollback tests where supported.
- OpenAPI import golden tests.
- GraphQL import golden tests.
- Generated client tests.

Acceptance commands:

```bash
cargo test data
cargo run -p ori -- schema import openapi examples/contracts/openapi.yaml --module vendor.example
```

---

### M14 — UI framework and design system

**Status:** documented only.

Build:

- Typed `view` declarations.
- View tree IR.
- HTML/web adapter.
- Design tokens.
- Accessibility checks.
- Route validation.
- State model.
- Form validation.
- Component capsules.
- Snapshot render tests.

Tests required:

- View parser tests.
- Invalid route diagnostics.
- Accessibility diagnostics.
- Design-token violation tests.
- Snapshot rendering tests.
- Bundle tree-shaking tests.

Acceptance commands:

```bash
cargo test ui
cargo run -p ori -- build --target web examples/fullstack/users.ori
```

---

### M15 — WebAssembly and mobile targets

**Status:** documented only.

Build:

- Wasm component output.
- WIT/interface generation or equivalent typed component contract.
- Web adapter.
- Mobile manifest generation.
- Mobile permission/capability checking.
- Platform-specific escape hatches with explicit capabilities.

Tests required:

- Wasm smoke tests.
- Component interface golden tests.
- Mobile permission manifest tests.
- Platform capability denial tests.

Acceptance commands:

```bash
cargo test wasm
cargo run -p ori -- build --target wasm-component examples/hello.ori
cargo run -p ori -- build --target mobile examples/fullstack/users.ori
```

---

### M16 — LSP and editor tooling

**Status:** not implemented.

Build:

- `ori lsp` server.
- Completion.
- Hover.
- Go to definition.
- Find references.
- Rename.
- Semantic tokens.
- Code actions from diagnostics/Patch IR.
- Test discovery.
- Effect/capability visualization.
- Agent capsule inspection.

Tests required:

- LSP protocol unit tests.
- Completion golden tests.
- Rename safety tests.
- Code action tests.
- Diagnostics consistency between CLI and LSP.

Acceptance commands:

```bash
cargo test lsp
cargo run -p ori -- lsp --stdio
```

---

### M17 — Documentation and migration system

**Status:** documentation scaffold exists. Generated docs not implemented.

Build:

- Generated human docs.
- Generated agent docs.
- Symbol documentation pages.
- Effect/capability docs.
- API docs.
- Edition migration tool.
- Migration patches.

Tests required:

- Docs golden tests.
- Agent docs budget tests.
- Migration fixture tests.
- Backward compatibility tests for older edition samples.

Acceptance commands:

```bash
cargo test docs
cargo run -p ori -- docs --format agent --budget 8000
cargo run -p ori -- migrate --from 2027.1 --to 2028.1 --dry-run --json
```

---

### M18 — Security and supply chain

**Status:** policy documented. Enforcement mostly not implemented.

Build:

- Capability enforcement.
- Package signing plan.
- SBOM generation.
- Provenance verification.
- Registry trust policy.
- Dependency audit.
- Unsafe-code audit report.
- Security-sensitive change manifests.
- Secret handling rules.

Tests required:

- Capability bypass tests.
- Build-script sandbox tests.
- Lockfile tamper tests.
- SBOM schema tests.
- Provenance failure tests.
- Unsafe surface report tests.

Acceptance commands:

```bash
cargo test security
cargo run -p ori -- audit --json
cargo run -p ori -- sbom --json
cargo run -p ori -- provenance verify --json
```

---

### M19 — Performance, release, and benchmark discipline

**Status:** not implemented.

Build:

- Build-time benchmarks.
- Runtime microbenchmarks.
- Agent-token benchmarks.
- Bundle-size benchmarks.
- Regression thresholds.
- Benchmark dashboard or JSON output.

Required metrics:

- Cold check latency.
- Warm check latency.
- Incremental edit latency.
- Release build latency.
- Binary/Wasm bundle size.
- Tokens per accepted patch.
- Diagnostic-to-fix success rate.
- Patch rejection rate.
- Regression rate after agent patch.

Tests required:

- Criterion or equivalent benchmark suite once dependency policy permits.
- JSON benchmark output schema.
- CI threshold tests for non-flaky metrics.

Acceptance commands:

```bash
cargo bench
cargo run -p ori -- bench agent --json
```

---

## Required quality gates

### Local static gate

Use when Rust is unavailable or when validating an archive shape:

```bash
python3 scripts/validate_all.py --static-only
```

Checks:

- Required root files exist and are non-empty.
- Required directories exist.
- JSON schemas parse.
- JSON examples parse.
- JSONL golden fixtures parse.
- Schema instances validate when Python `jsonschema` is installed.
- Shell scripts and Git hooks use strict mode and pass `bash -n`.
- Git hooks are executable.
- Rust production source has no `unwrap()`, `expect()`, `panic!`, `todo!`, `unimplemented!`, `dbg!`, or unsafe Rust constructs.
- Workspace dependencies remain within the approved bootstrap set.

### Pre-commit gate

Installed by:

```bash
./scripts/install_hooks.sh
```

Runs:

```bash
python3 scripts/validate_all.py --pre-commit
```

Required checks:

- Everything in the static gate.
- `cargo fmt --all --check`.
- `cargo check --workspace --all-targets`.

### Pre-push / full gate

Runs:

```bash
python3 scripts/validate_all.py --full
```

Equivalent Make target:

```bash
make quality-gate
```

Required checks:

- Everything in the static gate.
- `cargo fmt --all --check`.
- `cargo check --workspace --all-targets`.
- `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace`.
- CLI contract smoke tests:
  - `ori doctor`
  - `ori check --json`
  - `ori agent map --json`
  - `ori agent explain --json`
  - `ori capsule --json`
  - `ori patch check --json`

### CI gate

CI must remain at least as strict as:

```bash
python3 scripts/validate_all.py --full
```

Do not weaken CI to make a patch pass. Fix the patch.

---

## Test and validation matrix

| Area | Required tests | Required fixtures | Required schema coverage |
|---|---|---|---|
| Lexer | Token unit tests for every token class | valid and invalid `.ori` files | diagnostics |
| Parser | CST/AST golden tests, recovery tests | syntax fixtures | diagnostics, patch candidates |
| Formatter | Snapshot tests, idempotence tests | formatting fixtures | none |
| Resolver | imports, aliases, visibility, cycles | module graph fixtures | diagnostics, agent map |
| Type checker | inference, generics, variants, records, protocols | type fixtures | diagnostics |
| Effects | effect propagation, unknown effects, capabilities | capability fixtures | capability manifest |
| Borrow checker | move/use, mutable borrow, arenas, shared ownership | memory fixtures | diagnostics |
| MIR/HIR | lowering, constant eval | IR golden files | none |
| Codegen | dev run, release build, Wasm output | executable fixtures | build report |
| Incremental | invalidation, cache recovery, affected tests | edit sequences | build report, agent tests |
| Diagnostics | JSONL validity, stable IDs, fix hints | golden diagnostics | diagnostic schema |
| Patch IR | validation, apply, dry-run, explain | patch fixtures | patch and patch-check schemas |
| Agent ABI | maps, symbols, capsules, symbol cards, budgets | agent golden files | all agent schemas |
| Package manager | manifest, lockfile, resolver, audit | package fixtures | manifest, capability, SBOM schemas |
| Backend | typed routes, middleware, OpenAPI | service fixtures | OpenAPI output, diagnostics |
| Database | query shape checking, migrations | SQL/migration fixtures | migration report |
| UI | view tree, accessibility, design tokens | UI fixtures | diagnostics, UI manifest |
| Mobile | permission manifests, adapters | mobile fixtures | capability manifest |
| Wasm | component output, interface contracts | Wasm fixtures | component manifest |
| Security | denied ambient authority, unsafe reports | attack fixtures | audit, capability, SBOM |
| LSP | completion, hover, rename, code actions | LSP transcript fixtures | LSP-compatible diagnostics |
| Docs | generated docs, agent docs budgets | docs fixtures | docs manifest |
| Benchmarks | build latency, runtime, agent token cost | benchmark fixtures | benchmark JSON |

---

## Required schema contracts

Existing schemas must remain valid and versioned:

- `schemas/diagnostic.schema.json`
- `schemas/patch.schema.json`
- `schemas/patch-check.schema.json`
- `schemas/capsule.schema.json`
- `schemas/agent-map.schema.json`
- `schemas/symbol-card.schema.json`
- `schemas/manifest.schema.json`
- `schemas/change.schema.json`
- `schemas/capability.schema.json`

Schemas to add as functionality matures:

- `schemas/agent-symbol-list.schema.json`
- `schemas/agent-tests.schema.json`
- `schemas/build-report.schema.json`
- `schemas/doctor.schema.json`
- `schemas/lockfile.schema.json`
- `schemas/sbom.schema.json`
- `schemas/audit-report.schema.json`
- `schemas/provenance.schema.json`
- `schemas/openapi-report.schema.json`
- `schemas/ui-manifest.schema.json`
- `schemas/mobile-manifest.schema.json`
- `schemas/wasm-component.schema.json`
- `schemas/benchmark.schema.json`
- `schemas/lsp-code-action.schema.json`

Rules for every schema:

1. Use Draft 2020-12.
2. Include `title`, `type`, `required`, and `additionalProperties` policy.
3. Include `schema` field in emitted JSON objects when the object is a public contract.
4. Add at least one valid example.
5. Add at least one invalid example where useful.
6. Wire schema-instance validation into `scripts/validate_all.py`.
7. Add CLI or unit tests proving emitted output validates against the schema.

---

## Code quality control hooks and guardrails

The following hooks are installed in this continuation pack:

```text
.githooks/pre-commit
.githooks/pre-push
scripts/install_hooks.sh
scripts/validate_all.py
scripts/check_json_contracts.sh
```

Required installation:

```bash
./scripts/install_hooks.sh
```

Hook policy:

- Pre-commit blocks malformed contracts, broken static guardrails, non-executable hooks, formatting drift, and Rust type-check failures.
- Pre-push blocks warnings, failing tests, broken CLI contract outputs, and all static issues.
- CI must run the same full gate.

Production Rust guardrails:

- No `unwrap()`.
- No `expect()`.
- No `panic!`.
- No `todo!`.
- No `unimplemented!`.
- No `dbg!`.
- No unsafe Rust constructs in `crates/*/src` without an explicit approved exception.
- No new workspace dependency without approval recorded in `MEMORY.md` and `CHANGELOG.md`.

Public contract guardrails:

- Do not hand-build diagnostic, capsule, patch, agent-map, symbol-card, or manifest JSON.
- Use typed serializable structs.
- Preserve schema version fields.
- Do not remove fields from public schemas without migration notes and compatibility tests.
- Do not change diagnostic IDs unless a migration note exists.
- Do not change CLI JSON output shape without updating schemas, examples, tests, docs, and changelog.

Agent guardrails:

- Prefer `ori agent explain` and symbol cards over reading whole files.
- Prefer Patch IR over broad rewrites.
- Update `TASKS.md` when completing or adding tasks.
- Update `MEMORY.md` for architectural decisions.
- Update `CHANGELOG.md` for user-visible behavior.
- Never add a dependency to avoid implementing a simple local primitive.
- Never bypass a failing test by deleting the test unless the spec changed and the spec change is documented.

---

## Agent implementation playbook

For every development task:

1. Read root context files.
2. Run the relevant validation gate before edits.
3. Identify the narrowest module to change.
4. Add or update tests first when behavior is clear.
5. Implement the smallest coherent change.
6. Run formatter.
7. Run targeted tests.
8. Run full gate before finalizing.
9. Update docs and tracking files.
10. Summarize exactly what changed and which gates passed.

Recommended command loop:

```bash
python3 scripts/validate_all.py --static-only
cargo fmt --all --check
cargo test --workspace
# edit
cargo fmt --all
cargo test --workspace
python3 scripts/validate_all.py --full
```

For parser/compiler work:

```bash
cargo test -p ori-compiler
cargo run -p ori -- check --json examples/hello.ori
cargo run -p ori -- check --json examples/bad_null.ori || true
```

For agent contract work:

```bash
cargo test -p ori-agent
cargo run -p ori -- agent map --budget 2000 --json examples/fullstack/users.ori
cargo run -p ori -- agent explain sym:store.users.fetch_user --json examples/fullstack/users.ori
python3 scripts/validate_all.py --contracts-only
```

For Patch IR work:

```bash
cargo test -p ori-compiler patch
cargo run -p ori -- patch check --json examples/agent_patch.json
python3 scripts/validate_all.py --contracts-only
```

---

## First implementation tickets to execute

Execute in order unless a senior maintainer changes priority.

### Ticket 1 — Replace declaration parser with CST parser skeleton

Deliver:

- `cst.rs`
- parser events or green-tree-like representation
- error recovery
- tests for module/import/function/type declarations

Done when:

- existing CLI outputs remain compatible
- parser golden tests pass
- static and full gates pass

### Ticket 2 — Implement real formatter from CST

Deliver:

- idempotent `ori fmt`
- formatting tests
- no semantic changes from formatting

Done when:

```bash
cargo test -p ori-compiler formatter
```

passes.

### Ticket 3 — Add resolver module

Deliver:

- `resolver.rs`
- symbol table
- import graph
- duplicate/cycle diagnostics
- agent map dependencies use resolver output

### Ticket 4 — Add type AST and type parser

Deliver:

- type expression parser
- function signature type AST
- record/variant/newtype structures
- generic parameter support

### Ticket 5 — Implement baseline type checker

Deliver:

- expression typing for literals, variables, calls, records, match
- explicit public API return type checks
- mismatch diagnostics

### Ticket 6 — Implement exhaustive match checking

Deliver:

- variant coverage analysis
- missing-arm diagnostics
- Patch IR suggestion for missing arms

### Ticket 7 — Expand effect checker

Deliver:

- effect propagation
- capability requirements
- capability manifest output

### Ticket 8 — Implement Patch IR apply/dry-run

Deliver:

- stable node target resolution
- structural modifications
- dry-run output schema
- tests proving no broad rewrite required

### Ticket 9 — Add `ori agent symbols --changed`

Deliver:

- changed symbol detection from file hash or diff input
- schema and tests

### Ticket 10 — Add package manifest parser

Deliver:

- `ori package check --json`
- manifest schema validation
- capability declaration checks

### Ticket 11 — Implement affected-test graph prototype

Deliver:

- test discovery
- symbol-to-test mapping
- `ori test --changed --json` prototype

### Ticket 12 — Implement HIR lowering

Deliver:

- typed HIR structures
- lowering tests
- stable debug/golden representation

### Ticket 13 — Implement MIR skeleton

Deliver:

- basic block structure
- instructions
- function lowering
- MIR golden tests

### Ticket 14 — Implement dev interpreter or bytecode prototype

Deliver:

- `ori run examples/hello.ori`
- expression/function execution smoke tests

### Ticket 15 — Add Wasm backend design spike

Deliver:

- documented backend strategy
- minimal interface contract
- no premature commitment to backend dependency without review

### Ticket 16 — Add backend route DSL parser

Deliver:

- parse `service` declarations
- typed route metadata
- route diagnostics

### Ticket 17 — Add OpenAPI generation prototype

Deliver:

- route-to-schema conversion
- golden OpenAPI output tests

### Ticket 18 — Add UI view parser prototype

Deliver:

- parse `view` declarations
- view tree IR
- route/design-token diagnostics

### Ticket 19 — Add LSP skeleton

Deliver:

- `ori lsp --stdio`
- diagnostics parity with CLI
- basic hover for symbols

### Ticket 20 — Add benchmark JSON contract

Deliver:

- `ori bench --json`
- schema
- smoke benchmark fixtures

---

## Definition of done for any feature

A feature is not done until all are true:

- Specification updated.
- Implementation added.
- Unit tests added.
- CLI behavior tested if applicable.
- JSON schema updated if public output changed.
- Golden fixtures updated if diagnostics or agent output changed.
- Agent-facing summary/capsule behavior considered.
- `TASKS.md` updated.
- `MEMORY.md` updated for architectural decisions.
- `CHANGELOG.md` updated for user-visible changes.
- `python3 scripts/validate_all.py --full` passes on a Rust-capable machine.

---

## Release readiness criteria

### Prototype release

Required:

- Real parser.
- Formatter.
- Resolver.
- Baseline type checker.
- JSON diagnostics.
- Capsules.
- Agent map and symbol cards.
- Patch check.
- Full quality gate.

### Alpha release

Required:

- Type checker with records, variants, generics, `Option`, `Result`.
- Exhaustive match.
- Effect declarations.
- Package manifest validation.
- Dev execution backend.
- `ori test`.
- LSP diagnostics.
- Agent Patch IR dry-run.

### Beta release

Required:

- Borrow/ownership checker.
- Capability enforcement.
- Incremental check.
- Patch apply.
- Backend framework prototype.
- Standard distribution core modules.
- Wasm prototype.
- Security audit report.

### Production release

Required:

- Stable language edition.
- Stable schema contracts.
- Reproducible builds.
- Package manager and lockfile.
- SBOM/provenance.
- Mature standard distribution.
- Full-stack framework support.
- Performance benchmarks.
- Security review.
- Conformance suite.

---

## Non-goals for the next implementation phase

Do not spend early cycles on:

- Advanced neural-network training framework.
- Full mobile runtime.
- Highly optimized release backend.
- Macro system.
- Large third-party ecosystem.
- Perfect Rust-like borrow checking before parser/type checker maturity.

The first defensible wedge is:

> safe, typed, agent-native full-stack web/backend development with fast check/repair loops.

---

## Known risks

| Risk | Mitigation |
|---|---|
| Language scope is too broad | Land compiler frontend, type checker, diagnostics, and Agent ABI before frameworks. |
| AI agents rewrite too much | Use Patch IR, symbol cards, context budgets, and hooks. |
| Schema drift breaks agents | Version schemas and validate emitted JSON in CI. |
| Build speed goals are unproven | Add incremental benchmarks early. |
| Dependency creep bloats bootstrap | Enforce dependency policy in `validate_all.py`. |
| Safety model becomes too complex | MVP should reject unsupported unsafe/concurrent cases with clear diagnostics. |
| Standard distribution becomes unmaintainable | Layer `core`, `std`, `app`, `platform`, `labs`; stabilize in that order. |
| Docs fall behind implementation | Definition of done requires spec/docs updates. |

---

## Final instruction to future agents

Work like a compiler maintainer, not a demo generator.

Prefer a small correct compiler feature with tests, schemas, and diagnostics over a large impressive-looking feature that weakens contracts. The public product is the edit-check-repair loop. Protect it.
