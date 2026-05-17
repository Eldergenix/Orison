# TASKS.md

Task board for building Orison end-to-end.

Status markers:

- `[ ]` not started
- `[~]` in progress
- `[x]` complete
- `[!]` blocked or needs decision

## Phase 0 — Bootstrap repository

- [x] Create repository scaffold.
- [x] Add root README, GOAL, AGENTS, MEMORY, TASKS, CHANGELOG.
- [x] Add language specification documents.
- [x] Add compiler architecture documents.
- [x] Add JSON schemas.
- [x] Add Rust workspace scaffold.
- [x] Add minimal CLI.
- [x] Add example `.ori` source files.
- [x] Add CI skeleton.
- [x] Add toolchain pin and editor configuration.
- [x] Add review-remediation report.
- [x] Add continuation handoff file for future agents.
- [x] Add canonical validation gate.
- [x] Add installable pre-commit and pre-push hooks.

## Phase 1 — Lexer and parser

- [x] Add simple lexer scaffold.
- [x] Add symbol-oriented parser scaffold.
- [x] Parse imports in scaffold parser.
- [x] Extract effects from declarations.
- [x] Avoid reserved-runtime diagnostics inside strings and comments.
- [x] Add error-tolerant CST (`cst::parse_cst`) over the existing token
  stream. (Grammar-driven recovery is still a follow-up.)
- [x] Add stable node IDs (`node_id::make_node_id`, content-derived).
- [x] Preserve comments and blank lines in CST.
- [~] Add parser recovery rules. (CST tolerates malformed items by
  promoting them to `CstNodeKind::Error`; no per-token recovery yet.)
- [~] Add golden syntax fixtures. (Bench fixtures exist in `bench.rs`; a
  dedicated `tests/golden/syntax/*.expected` set is still TBD.)
- [~] Emit syntax diagnostics with repair candidates. (Existing
  diagnostics carry `fixes`; structural Patch IR suggestions land
  alongside the full type checker.)

## Phase 2 — AST and name resolution

- [x] Define AST structs for modules, imports, and top-level
  declarations (`ast::{Module,Symbol,SymbolKind}` were already present;
  expression / pattern AST nodes remain TBD until bodies are recovered).
- [~] Lower CST to AST. (`parse_source` keeps producing the existing
  symbol-level AST; CST is parallel and consumed by patch apply.)
- [x] Implement module graph (`resolver::ModuleGraph`).
- [x] Implement import resolver (`resolver::resolve`).
- [x] Implement symbol table (`resolver::Resolution::symbols`).
- [x] Implement duplicate definition diagnostics (`E0211`).
- [x] Implement unresolved name diagnostics (`E0220` per-import).
- [x] Emit basic symbol cards.

## Phase 3 — Type system

- [x] Implement primitive types (existing `types::is_builtin_type`).
- [x] Implement records (declared via `type X = { ... }` parsing).
- [x] Implement variants (declared via `type X = | A | B(...)` parsing).
- [x] Implement newtypes (declared via `type X wraps Y` parsing).
- [x] Implement `Option[T]` and `Result[T, E]` recognition in signatures
  (`type_check::PERMITTED_GENERICS`).
- [~] Implement local type inference. (Bootstrap only checks signatures;
  body-level inference lands with the expression AST.)
- [~] Implement protocol definitions. (Schema reserved; no checker yet.)
- [~] Implement protocol impl checking.
- [~] Implement exhaustive match checking. (`patch_apply` insert_match_arm
  is wired; the static checker lands with the full type system.)
- [x] Implement typed query API for diagnostics (`type_check::type_check_module`).

## Phase 4 — Effects and capabilities

- [x] Define initial known-effect registry.
- [x] Parse `uses` clauses in top-level declarations.
- [~] Type-check effects. (Signature-level effects are enforced; call-graph
  propagation lands when expression bodies are recovered.)
- [~] Propagate effects through calls. (Bootstrap surfaces declared effects
  only.)
- [x] Emit capability manifests (`effect_check::build_capability_manifest`
  → `ori.capability.v1`).
- [x] Reject undeclared ambient effects (`E0410`).
- [x] Add package-level capability policy (`ori capability --policy a,b,c`).

## Phase 5 — Ownership and memory model

- [ ] Implement move semantics for non-copy types.
- [ ] Implement explicit borrow types.
- [ ] Implement mutable borrow restrictions.
- [ ] Implement shared ownership wrappers.
- [ ] Implement arena scopes.
- [ ] Emit ownership diagnostics with patch hints.

## Phase 6 — Diagnostics and Patch IR

- [x] Add diagnostic JSON scaffold.
- [x] Replace hand-built diagnostic JSON with typed serialization.
- [x] Add patch schema scaffold.
- [x] Implement semantic `ori patch check` validation for schema, intent, operations, known op names, and required op fields.
- [x] Add structured repair candidates for syntax errors (existing
  diagnostic `fixes` model exposed via `ori agent diagnose`).
- [~] Add structured repair candidates for type errors. (Type checker
  reports W0501/W0510 with expected/found data; Patch IR fixes land with
  the full inference pass.)
- [x] Implement `ori patch apply` (`patch_apply::apply_patch`,
  `ori patch apply` / `ori patch dry-run` / `ori patch explain`).
- [x] Add patch operation application tests (6 cases in
  `crates/ori-compiler/src/patch_apply.rs::tests`).
- [~] Add migration diagnostics. (Migration syntax parses; per-arm
  validation lands with the migration graph in M13.)

## Phase 7 — Agent Context ABI

- [x] Add agent map scaffold.
- [x] Add capsule scaffold.
- [x] Add `ori agent explain` symbol-card output.
- [x] Add `ori capsule` / `ori agent capsule` output.
- [x] Add `ori agent symbols` (`ori.agent_symbol_list.v1`).
- [x] Add `ori agent diagnose` (`ori.agent_diagnose.v1`).
- [~] Add budgeted context packing with dependency scoring. (Existing
  `ori agent map --budget` truncates by symbol size; dependency-weighted
  packing lands with the resolver-driven graph traversal.)
- [x] Add affected-symbol graph (`resolver::ModuleGraph`).
- [x] Add affected-test graph (`incremental::select_affected_tests`,
  `ori agent tests --affected`).
- [x] Add token-cost reporting (`bench::agent_map_token_density` plus
  `agent map`'s `used_estimate`).

## Phase 8 — Formatter

- [x] Add simple trailing-whitespace formatter scaffold.
- [x] Implement CST-preserving formatter (`formatter::format_text`).
- [~] Add style diagnostics. (Tab warnings exist via `compiler::add_style_diagnostics`;
  richer style lints land with the AST formatter pass.)
- [x] Add formatter golden tests (`crates/ori-compiler/src/formatter.rs::tests`).

## Phase 9 — Dev backend

- [x] Define MIR (`mir::MirModule` with basic blocks and instructions).
- [x] Lower typed HIR to MIR (`mir::lower_module`).
- [~] Implement bytecode or baseline native dev backend. (Tree-walking
  interpreter in `interp::run_module` reports observed effects and
  callee candidates; expression body execution lands once bodies are
  recovered.)
- [x] Add `ori run` (`cmd_run` in `ori-cli`).
- [~] Add runtime error model. (`RunReport.status` carries `ok` /
  `missing_entry`; richer runtime errors land with body execution.)

## Phase 10 — Release and Wasm backends

- [ ] Select release backend strategy.
- [ ] Add native AOT backend prototype.
- [ ] Add Wasm component backend prototype.
- [ ] Emit `.wit` for Wasm components.
- [ ] Add tree-shaking metadata.

## Phase 11 — Standard distribution

- [x] Implement `core.option` (host stub).
- [x] Implement `core.result` (host stub).
- [x] Implement `core.iter` (host stub).
- [x] Implement `std.json` (host stub).
- [x] Implement `std.http` (host stub).
- [x] Implement `std.validation` (host stub).
- [x] Implement `std.logging` (host stub).
- [x] Implement `std.config` (host stub).
- [x] Implement `std.sql` prototype (host stub).
- [x] Add `core.{string,bytes,list}` plus `app.{services,views,auth}`
  layer stubs and the `stdlib/README.md` overview.

## Phase 12 — Application frameworks

- [x] Implement typed service declarations (parser + capsule support).
- [x] Implement typed routes (`openapi::extract_openapi`).
- [x] Generate OpenAPI 3.1 (`ori openapi --json`).
- [~] Implement database query declarations. (`query` items parse and
  appear in capsules; SQL DSL lands with M13.)
- [x] Implement UI view DSL (`ui_check::build_ui_manifest`).
- [~] Implement design token checks. (Token list is recorded but the
  enforcement pass is empty; lands with the view-tree IR.)
- [x] Implement accessibility diagnostics (`ui_check::a11y_findings`,
  visual-view alt-text and form submit-label heuristics).
- [x] Implement mobile capability manifest (`schemas/mobile-manifest.schema.json`).

## Phase 13 — Package manager and supply chain

- [~] Define package registry protocol. (Manifest + lockfile shapes
  drafted; the wire protocol is the next milestone.)
- [x] Implement manifest parser (`ori-pkg::Manifest::parse`).
- [x] Implement lockfile (`ori-pkg::Lockfile`, deterministic
  serialisation).
- [~] Implement package hashing. (Bootstrap uses a deterministic FNV-1a
  stand-in; real SHA-256 over artefacts lands with the registry.)
- [x] Implement SBOM output (`ori sbom --json`).
- [x] Implement provenance verification (`ori provenance verify`).
- [x] Implement capability diffing for dependencies (`audit` AUD0001 /
  AUD0002 findings).

## Phase 14 — Benchmarks

- [x] Add build latency benchmarks (`warm_check_latency`,
  `cold_check_latency`).
- [x] Add diagnostics quality benchmarks (`agent_diagnose` exposed; the
  numerical regression budget lands in CI when a baseline ships).
- [x] Add agent context-size benchmarks (`agent_map_token_density`
  measured in `BENCHMARKS.md`; the tokens-per-symbol histogram pairs with
  the budgeted packer in M8).
- [~] Add small-model patch success benchmark harness. (Patch validation
  and dry-run latency are measured; the model-in-the-loop harness is
  out of bootstrap scope.)
- [~] Add full-stack CRUD benchmark. (Demo storefront drives the
  end-to-end story; a timed CRUD scenario lands when `ori run` executes
  bodies.)
- [x] Add `schemas/benchmark.schema.json` and wire `ori bench --json` to it.

## Phase 15 — Quality gates and continuation control

- [x] Add `ORISON_AGENT_DEVELOPMENT_HANDOFF.md`.
- [x] Add `scripts/validate_all.py`.
- [x] Add `.githooks/pre-commit`.
- [x] Add `.githooks/pre-push`.
- [x] Wire CI to the full quality gate.
- [x] Add `docs/QUALITY_GATES.md` quick-reference card.
- [x] Add `docs/ARCHITECTURE_OVERVIEW.md` crate map.
- [~] Add schema validation for every new public contract as it is
  introduced. (14 new schemas committed and validated for shape;
  `SCHEMA_MAP` instance-fixture wiring is the last mile.)
- [~] Add dynamic validation that emitted CLI JSON validates against
  every schema. (Doctor, capsule, agent map, patch check, and bench
  outputs are exercised in CLI smoke tests; full instance-validation
  pass lands with `SCHEMA_MAP` expansion.)

## Phase 17 — LSP and editor tooling

- [x] Add `ori-lsp` crate as a workspace member.
- [x] Implement diagnostic parity with the CLI (`ori-lsp/src/diagnostics.rs`).
- [x] Implement hover with symbol id/signature/effects.
- [~] Implement completion and rename. (Hover is wired; rename is
  exposed via the Patch IR `rename_symbol` op; completion provider is
  the follow-up.)
- [x] Implement code actions sourced from diagnostic fixes
  (`ori.lsp_code_action.v1`).
- [x] Add LSP transcript fixtures and protocol unit tests (16 tests in
  `crates/ori-lsp/tests/`).

## Phase 16 — Release readiness

- [ ] Publish language reference v0.1.
- [ ] Publish compiler alpha.
- [ ] Publish schema reference.
- [ ] Publish standard distribution alpha.
- [ ] Publish agent integration guide.
