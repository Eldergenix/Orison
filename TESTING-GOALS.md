# Orison Testing Goals

Comprehensive testing framework for the Orison toolchain. This document is
the authoritative source for **what must be tested**, **how**, and **why**.
It complements `BENCHMARKS.md` (performance) and
`ORISON_AGENT_DEVELOPMENT_HANDOFF.md` (milestone plan).

> **Status snapshot (2026-05-16).** Workspace currently ships
> **456 passing tests, 0 failing** across 5 crates
> (ori-compiler 374, ori-pkg 45, ori-lsp 29, ori-agent 8, ori-cli 0). Full
> quality gate (`python3 scripts/validate_all.py --full`) is green. Targets
> below are the framework the project commits to filling out — every
> "should" is a future test the next maintainer should add.

---

## 1. Philosophy

The bootstrap follows these testing rules without exception:

1. **Every diagnostic ID has a golden fixture.** A diagnostic without a
   committed `.ori` source + `.expected.jsonl` pair under `tests/golden/`
   is treated as a missing test, not an existing diagnostic.
2. **Every public JSON contract is shape-validated.** Both
   `schemas/*.schema.json` (statically) and the live CLI output
   (dynamically, via `conformance.rs`) must agree.
3. **Every bug becomes a regression test.** A diagnostic was missed, a
   flake surfaced, an off-by-one slipped — the fix lands with a test that
   would have caught it. No exceptions.
4. **No `.unwrap()` / `.expect()` / `panic!` / `todo!` / `unimplemented!` /
   `dbg!` in production source.** Enforced by
   `scripts/validate_all.py` — any test that needs to fail with context
   uses `assert!(false, "...")` with `#[allow(clippy::assertions_on_constants)]`
   (see `MEMORY.md` D016).
5. **Tests must be deterministic.** Hard-coded temp dirs are forbidden;
   scratch directories use `pid + nanos` namespacing (see
   `crates/ori-pkg/tests/audit_capability_diff.rs` and
   `resolver_cycle.rs` for the pattern).
6. **Tests run on a quiet, default-config machine.** No global state, no
   environment variables read outside an explicit allow-list, no
   filesystem dependencies outside the workspace.
7. **Test names read like specifications.** `it_works` is not a test
   name. `parses_section_and_array`, `nested_matches_are_walked`,
   `unsafe_diagnostic_includes_patch_fix` are.

---

## 2. Test categories

The Orison test suite is organised across **eight categories**. Every
subsystem must cover at least the first three; subsystems exposing public
schemas or invariants must cover the rest.

| # | Category | Lives in | When required |
|---|----------|----------|---------------|
| 1 | **Unit** | `crates/<crate>/src/**/*.rs` `#[cfg(test)] mod tests` | Every public function in a module |
| 2 | **Integration** | `crates/<crate>/tests/*.rs` | Every public CLI subcommand and crate boundary |
| 3 | **Golden** | `tests/golden/**` | Every diagnostic ID, every CLI envelope |
| 4 | **Conformance** | `crates/ori-compiler/tests/conformance.rs` | Every shipped contract has a real-CLI roundtrip |
| 5 | **Property** | `#[cfg(test)] mod tests` | Idempotence, determinism, monotonicity invariants |
| 6 | **Security** | `crates/{ori-compiler,ori-pkg}/tests/{capability_bypass,lockfile_tamper,sbom_schema,provenance_failure,unsafe_surface_report}.rs` | Every capability + provenance + tamper rule |
| 7 | **Regression** | Anywhere | Every fixed bug |
| 8 | **Fuzz** | `crates/<crate>/tests/fuzz_*.rs` (planned) | Lexer, parser, patch, importers — TBD |

Categories 1–7 are all populated today. Category 8 (fuzz) is planned —
see §15.

---

## 3. Subsystem coverage matrix

Each row is a subsystem. Each column is a category. `✓` = covered today;
`P` = planned (target test count in parentheses); blank = N/A.

| Subsystem | Unit | Integration | Golden | Conformance | Property | Security | Regression | Fuzz |
|-----------|:----:|:-----------:|:------:|:-----------:|:--------:|:--------:|:----------:|:----:|
| **lexer** | ✓ | ✓ | ✓ | ✓ | P(2) | — | ✓ | P(1) |
| **parser (item)** | ✓ | ✓ | ✓ | ✓ | P(3) | — | ✓ | P(1) |
| **parser (body/expr)** | ✓ | ✓ | P(6) | P(4) | P(2) | — | ✓ | P(1) |
| **CST + node IDs** | ✓ | — | P(2) | P(1) | ✓ | — | ✓ | — |
| **formatter** | ✓ | ✓ | P(2) | ✓ | ✓ | — | ✓ | — |
| **resolver** | ✓ | ✓ | ✓ | ✓ | P(1) | — | ✓ | — |
| **type checker** | ✓ | ✓ | ✓ | ✓ | P(2) | — | ✓ | — |
| **type inference** | ✓ | — | P(4) | P(2) | ✓ | — | — | — |
| **effect check** | ✓ | ✓ | ✓ | ✓ | P(1) | ✓ | ✓ | — |
| **effect propagation** | ✓ | — | P(3) | P(1) | ✓ | ✓ | — | — |
| **borrow checker** | ✓ | — | P(5) | P(1) | ✓ | — | — | — |
| **exhaustive match** | ✓ | — | P(2) | P(1) | ✓ | — | — | — |
| **const folding** | ✓ | — | — | — | ✓ | — | — | — |
| **HIR/MIR** | ✓ | — | P(3) | P(1) | ✓ | — | — | — |
| **interpreter (effect-only)** | ✓ | ✓ | — | — | ✓ | — | — | — |
| **interpreter (executing)** | ✓ | ✓ | P(4) | P(1) | ✓ | — | ✓ | — |
| **async runtime** | ✓ | — | — | P(1) | ✓ | — | ✓ | — |
| **wasm encoder** | ✓ | ✓ | — | ✓ | ✓ | — | — | P(1) |
| **textual codegen** | ✓ | — | — | — | ✓ | — | — | — |
| **patch IR check** | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ | P(1) |
| **patch IR apply** | ✓ | ✓ | P(4) | ✓ | ✓ | — | ✓ | — |
| **agent map** | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ | — |
| **agent diagnose** | ✓ | — | P(2) | P(1) | — | — | — | — |
| **agent symbols** | ✓ | — | P(2) | P(1) | ✓ | — | — | — |
| **agent tests (affected)** | ✓ | — | P(2) | P(1) | ✓ | — | — | — |
| **agent changed (query)** | ✓ | — | — | P(1) | ✓ | — | — | — |
| **doctor** | ✓ | ✓ | — | ✓ | ✓ | — | ✓ | — |
| **bench harness** | ✓ | ✓ | — | — | ✓ | — | — | — |
| **openapi** | ✓ | ✓ | ✓ | ✓ | — | — | ✓ | — |
| **ui manifest** | ✓ | ✓ | ✓ | ✓ | — | — | — | — |
| **wasm component manifest** | ✓ | ✓ | ✓ | ✓ | — | — | — | — |
| **capability manifest** | ✓ | ✓ | ✓ | ✓ | — | ✓ | ✓ | — |
| **mobile manifest** | ✓ | ✓ | — | P(1) | — | — | — | — |
| **design tokens** | ✓ | — | — | P(1) | ✓ | — | — | — |
| **coverage** | ✓ | — | — | P(1) | ✓ | — | — | — |
| **graphql import** | ✓ | ✓ | — | ✓ | ✓ | — | ✓ | — |
| **gRPC import** | ✓ | ✓ | — | ✓ | ✓ | — | ✓ | — |
| **SQL DSL** | ✓ | — | — | P(1) | ✓ | — | ✓ | — |
| **migration graph** | ✓ | — | — | P(1) | ✓ | — | — | — |
| **preprocessor** | ✓ | ✓ | — | P(1) | ✓ | ✓ | — | — |
| **docs generator** | ✓ | ✓ | — | P(1) | ✓ | — | — | — |
| **migrate** | ✓ | ✓ | — | P(1) | ✓ | — | — | — |
| **incremental cache** | ✓ | — | — | — | ✓ | — | — | — |
| **package manifest** | ✓ | ✓ | — | ✓ | ✓ | — | ✓ | — |
| **lockfile** | ✓ | ✓ | — | ✓ | ✓ | ✓ | ✓ | — |
| **resolver (pkg)** | ✓ | ✓ | — | ✓ | ✓ | — | ✓ | — |
| **SBOM** | ✓ | ✓ | — | ✓ | ✓ | ✓ | — | — |
| **audit** | ✓ | ✓ | — | ✓ | ✓ | ✓ | — | — |
| **provenance verify** | ✓ | ✓ | — | ✓ | — | ✓ | — | — |
| **local registry stub** | ✓ | — | — | P(1) | ✓ | — | — | — |
| **LSP server** | ✓ | ✓ | — | ✓ | ✓ | — | ✓ | — |
| **schema-on-disk** | — | — | — | ✓ | — | — | ✓ | — |
| **CLI subcommand matrix** | — | P(30) | — | ✓ | — | — | ✓ | — |
| **Quality gate (validate_all.py)** | — | ✓ | — | ✓ | — | — | ✓ | — |

`P` count is the **minimum** the framework asks for. Adding more is
welcome; reducing requires a `MEMORY.md` decision.

---

## 4. Subsystem test goals (detailed)

### 4.1 Lexer (`crates/ori-compiler/src/lexer.rs`)

**Goal.** Every token class round-trips correctly; comments and string
literals never confuse the runtime-hazard detectors.

**Unit tests.** One per token kind: `Ident`, `Keyword`, `Number`,
`String`, `Symbol`, `Eof`. Edge cases: leading underscore identifiers,
identifiers with trailing digits, multi-line strings, escaped quotes,
hex/binary literals (once supported).

**Golden.** `tests/golden/parser/hello.ori` plus an `.expected.jsonl`
file that lists every token for the input.

**Property.** `lex(s)` is idempotent under round-trip:
`lex(s).iter().map(|t| t.lexeme).join("") + whitespace == s`.

**Regression.** Once flaked: `null` inside a string literal must not
trigger `E0100`. Once flaked: `throw` inside a `//` comment must not
trigger `E0101`. Both covered by
`emits_null_diagnostic_outside_strings_and_comments` in
`crates/ori-compiler/tests/compiler_smoke.rs`.

### 4.2 Parser — item level (`crates/ori-compiler/src/parser.rs`)

**Goal.** Every top-level item the bootstrap grammar recognises produces
a `Symbol` with stable id, signature, and effects.

**Golden.** `tests/golden/parser/*.ori` already covers `hello`,
`full_signatures`, `variants`, `records`, `services`, `views`. Add:
`migrations.ori`, `queries.ori`, `actors.ori`, `capabilities.ori`,
`mixed_imports.ori`, `error_recovery.ori`.

**Property.** Parsing the same source twice yields equal `Module`
values. Parsing then re-formatting then re-parsing yields equal
modules (`format ∘ parse ∘ format ∘ parse = format ∘ parse`).

### 4.3 Parser — body level (`crates/ori-compiler/src/expr.rs`, `body.rs`)

**Goal.** Every `Expr` variant survives parse → AST round-trip.

**Golden plan.** `tests/golden/body/{literals,vars,calls,blocks,if,
match,return,try,construct,record,tuple,lambda,recovery}.ori` with the
expected JSON-serialised expression tree.

**Known gaps.** Binary operators, match guards, multiline strings,
string interpolation are documented in `docs/language/REFERENCE.md`
under "What's *not* yet in this reference".

### 4.4 CST + stable node IDs (`crates/ori-compiler/src/cst.rs`,
`node_id.rs`)

**Goal.** Node IDs survive unrelated edits.

**Property tests required.**
- `id(fn f after no edit) == id(fn f after blank line added)`
- `id(fn f) != id(fn f after signature change)`
- `id(fn dup) != id(fn dup again)` — siblings get distinct discriminants.
- `id` set is monotonic across whitespace-only edits.

All present today in `crates/ori-compiler/src/cst.rs::tests`.

### 4.5 Formatter (`crates/ori-compiler/src/formatter.rs`)

**Goal.** Idempotent and semantics-preserving.

**Tests.** `formats_are_idempotent`, `collapses_double_blank_lines`,
`preserves_comments_verbatim`, `does_not_touch_string_contents`,
`collapses_multispace_inside_item_lines`. Add a snapshot test per
example app: `examples/demo_store/src/api.ori | format` must match a
committed `tests/golden/format/demo_store_api.expected`.

### 4.6 Resolver (`crates/ori-compiler/src/resolver.rs`)

**Goal.** Multi-module symbol table with namespace separation, cycle
detection, and unresolved-import diagnostics.

**Tests.** `detects_duplicate_function_across_distinct_ids` (E0211),
`flags_unresolved_import` (E0220), `detects_module_import_cycle`
(E0230), `allows_standard_distribution_imports`,
`allows_value_and_type_with_same_name`. Add: 3-module cycle, 5-module
cycle, self-cycle, re-export of a private symbol (visibility), import
aliasing once aliases land.

### 4.7 Type checker (`crates/ori-compiler/src/type_check.rs`)

**Goal.** Signature-level type validation against builtins, declared
types, and permitted generics.

**Tests.** `accepts_known_builtin_signature`,
`accepts_option_with_argument`, `accepts_result_with_two_args`,
`flags_unknown_type` (W0501), `flags_bare_result` (W0510),
`allows_user_declared_type`. Add: nested generics
(`Result[List[Option[T]], CartError]`), record field types, variant
payload types.

### 4.8 Type inference (`crates/ori-compiler/src/type_infer.rs`)

**Goal.** Expression-level inference for the body parser's expression set.

**Tests already shipping.** 22 cases including `int_literal_infers_int`,
`var_lookup_uses_env`, `if_with_mismatched_branches_emits_w0541`,
`try_on_result_returns_payload`, `unknown_does_not_pollute_concrete_branch`.

**Plan.** Binary operators (once parsed), generic instantiation by
usage, protocol bound resolution.

### 4.9 Effect check + propagation (`crates/ori-compiler/src/effect_check.rs`,
`effect_propagate.rs`)

**Goal.** Static enforcement of declared package capabilities;
propagation through the call graph.

**Tests.** `capability_manifest_groups_symbols_by_effect`,
`policy_diff_reports_undeclared_effect`,
`diagnostics_flag_undeclared_effect` (E0410), `diagnostics_quiet_when_policy_empty`.
Plus 14 propagation tests including
`two_function_chain_propagates_db_read`,
`direct_cycle_terminates_and_propagates`,
`diagnostic_e0420_includes_patch_and_docs`.

### 4.10 Borrow checker (`crates/ori-compiler/src/borrow.rs`)

**Goal.** Signature-level ownership rules.

**Tests already shipping.** 11 cases covering B0010 (double `&mut`),
B0011 (mixed `&`/`&mut`), B0020 (newtype confusion), B0030 (`Shared` +
write), B0040 (`unsafe`), B0050 (dangling-borrow heuristic), plus
`unsafe_diagnostic_includes_patch_fix`, `borrow_check_is_idempotent`.

**Plan.** Body-level move-after-use, region inference (years out — see
`docs/ROADMAP.md`).

### 4.11 Exhaustive match + const folding (`crates/ori-compiler/src/exhaustive.rs`,
`const_fold.rs`)

**Goal.** Match coverage analysis + literal folding.

**Tests.** `missing_arm_emits_e0540`, `redundant_arm_emits_w0541`,
`wildcard_makes_match_exhaustive`, `nested_matches_are_walked`,
`payload_bearing_arms_count_toward_coverage`. Const fold: 15 tests
including idempotence.

### 4.12 HIR / MIR / interpreter (`hir.rs`, `mir.rs`, `interp.rs`,
`interp_exec.rs`)

**Goal.** Lowering correctness and runtime semantics.

**Tests.** Hir: `lowers_function_with_params_and_return`,
`strips_uses_from_return_type`. Mir: `lowers_single_function_to_mir`.
Interp (effect-only): `run_records_observed_effects_from_entry`,
`run_falls_back_to_boot_when_main_absent`. Interp (executing): 16 cases
including `integer_literal_main_returns_it`,
`function_calling_another_function`, `try_unwraps_ok_and_propagates_err`,
`recursion_cap_returns_r0005`.

### 4.13 Async runtime (`crates/ori-compiler/src/async_runtime.rs`)

**Goal.** Cooperative scheduler with deadlock + leak detection.

**Tests.** 11 cases including `pending_then_resume_completes`,
`a0001_overflow_diagnostic`, `a0002_deadlock_report`,
`a0003_future_leak_report`, `ids_are_monotonic_and_not_reusable`,
`stress_thousand_spawn_thousand_resume`.

### 4.14 Wasm encoder + codegen (`wasm_encoder.rs`, `codegen_text.rs`)

**Goal.** Byte-deterministic binary output that re-decodes correctly.

**Tests.** 19 encoder cases including round-trip decoding of every
section, LEB128 boundary cases, `hello_module_is_byte_deterministic`.
Add (planned): structured fuzz over random `MirModule` inputs that
respect type discipline.

### 4.15 Patch IR (`patch.rs`, `patch_apply.rs`)

**Goal.** Validation rejects every malformed shape; apply respects
stable node IDs.

**Tests.** `patch_checker_accepts_structural_patch`,
`patch_checker_rejects_unknown_operations`. Apply tests:
`insert_node_adds_line_after_target`, `stale_target_yields_p1010`,
`add_import_inserts_after_module`,
`rename_symbol_renames_only_identifiers`,
`rename_does_not_touch_string_contents`,
`unsupported_op_is_reported`,
`partial_apply_returns_after_when_one_op_works`. Add (planned): every
op in `KNOWN_OPERATIONS` (14) gets an apply test.

### 4.16 Agent ABI

`agent_map`: `flags_unresolved_import`. Add per-budget tests
(200/500/1000/2000/4000 — already verified live; should become a
property test).

`agent_diagnose`, `agent_symbols`, `agent_tests`, `agent_changed`:
each has 1+ test today; add golden fixtures + conformance tests.

### 4.17 Package manager (`crates/ori-pkg/`)

**Tests.** Manifest roundtrip + errors (6), lockfile determinism (2),
resolver cycle (1), audit capability diff (1), capability bypass (3),
lockfile tamper (2), SBOM schema (2), provenance failure (5). Plus
registry stub: 11 cases including
`init_is_idempotent`,
`publish_then_fetch_round_trips`,
`yank_then_fetch_returns_yanked`,
`yank_reason_strips_control_chars`.

### 4.18 LSP server (`crates/ori-lsp/`)

**Tests.** 29 across codec roundtrip (8), diagnostic translation (3),
initialize flow (5), completion flow (2), rename flow (2),
workspace_symbol_flow (4), document_symbol_flow (1), definition_flow
(2), references_flow (2).

**Goal.** Every advertised capability has a test that actually
exercises it.

### 4.19 Importers (`graphql_import.rs`, `rpc_import.rs`)

**Tests.** GraphQL: 13 cases including nullable types, list types,
deterministic output, generated-Orison-parses-clean. gRPC: 14 cases
including `oneof` rejection, zero field number rejection, server/client
streaming, deterministic output, generated-Orison-parses-clean.

### 4.20 Conformance suite (`crates/ori-compiler/tests/conformance.rs`)

**Goal.** Every public CLI envelope's shape matches the committed
fixture.

**Status.** 19 cases today, covering: parser fixtures, diagnostic
fixtures, capsule snapshot, agent_map snapshot, openapi snapshot, ui
snapshot, wasm snapshot, capability snapshot.

**Re-bless.** `ORI_CONFORMANCE_BLESS=1 cargo test -p ori-compiler --test conformance`
regenerates `*.expected.*` files; only invoke after intentional
behaviour changes.

### 4.21 Security audit suite

| File | Tests | Asserts |
|------|------:|---------|
| `crates/ori-pkg/tests/capability_bypass.rs` | 3 | AUD0001 fires on missing capability; AUD0002 info on unused; report is byte-stable across runs |
| `crates/ori-pkg/tests/lockfile_tamper.rs` | 2 | Rebuilt lockfile is byte-equal; tampered checksum is detected |
| `crates/ori-pkg/tests/sbom_schema.rs` | 2 | Generated SBOM matches shape; shape walker rejects synthetic violations |
| `crates/ori-pkg/tests/provenance_failure.rs` | 5 | Missing/unrecognised signature rejected with notes |
| `crates/ori-compiler/tests/unsafe_surface_report.rs` | 2 | Workspace has **zero** `unsafe fn / impl / trait / {` in `crates/*/src/` |
| `crates/ori-compiler/tests/capability_runtime_denial.rs` | 3 | `unsafe` denied via policy; unknown capability → W0401; empty policy does not silently deny |

### 4.22 Quality gate (`scripts/validate_all.py`)

**Static gate (`--static-only`).** Required-file existence, JSON
contract parse, JSONL fixture parse, schema-instance validation when
`jsonschema` is installed, shell hook strict-mode check, Rust source
guardrails (no `.unwrap()`/`.expect()`/`panic!`/`todo!`/`unimplemented!`/`dbg!`/`unsafe`),
workspace dependency allow-list (only `serde` + `serde_json`).

**Pre-commit gate (`--pre-commit`).** Static + `cargo fmt --all --check`
+ `cargo check --workspace --all-targets`.

**Pre-push / full gate (`--full`).** Pre-commit + `cargo clippy
--workspace --all-targets -- -D warnings` + `cargo test --workspace` +
six CLI contract smoke commands.

---

## 5. Test repository layout

```
crates/<crate>/src/**/*.rs       # Unit tests in #[cfg(test)] mod
crates/<crate>/tests/*.rs        # Integration tests (one binary each)
tests/golden/                    # Cross-crate golden fixtures
  parser/        # parseable .ori files exercising grammar
  diagnostics/   # .ori source + .expected.jsonl per diagnostic
  capsule/       # *.expected.json snapshots of ori capsule
  agent_map/     # *.expected.json snapshots of ori agent map
  openapi/       # *.expected.json snapshots of ori openapi
  ui/            # *.expected.json snapshots of ori ui
  wasm/          # *.expected.json snapshots of ori wasm
  capability/    # *.expected.json snapshots of ori capability
  format/        # (planned) *.expected snapshots of ori fmt
  body/          # (planned) Expr tree snapshots
schemas/                         # Draft 2020-12 contract files
docs/QUALITY_GATES.md            # Validation pyramid
```

---

## 6. How to run the tests

### Fast feedback loop

```bash
cargo test -p ori-compiler --lib <module>::tests::<name>   # Single test
cargo test -p ori-compiler                                 # One crate
cargo test --workspace                                     # Full suite
```

### Pre-commit gate locally

```bash
python3 scripts/validate_all.py --pre-commit
```

### Full gate (matches CI)

```bash
python3 scripts/validate_all.py --full
# or
make gate-full
```

### Single-threaded (for filesystem-touching integration tests)

```bash
cargo test --workspace -- --test-threads=1
```

### Re-bless conformance snapshots after intentional changes

```bash
ORI_CONFORMANCE_BLESS=1 cargo test -p ori-compiler --test conformance
```

### Run a single example app end-to-end

```bash
ORI=target/release/ori
$ORI check --json examples/demo_store/src/api.ori
$ORI capsule --json examples/demo_store/src/api.ori
$ORI agent map --budget 2000 --json examples/demo_store/src/api.ori
$ORI agent diagnose --json examples/demo_store/src/api.ori
$ORI openapi --json examples/demo_store/src/api.ori
$ORI ui --json examples/demo_store/src/ui.ori
$ORI wasm --json examples/demo_store/src/api.ori
$ORI capability --policy "http,db.read,db.write" --json examples/demo_store/src/api.ori
$ORI patch dry-run --json \
  examples/demo_store/contracts/agent_patch_add_product_search.json \
  examples/demo_store/src/catalog.ori
$ORI db check --json examples/demo_store/src/catalog.ori
$ORI coverage --json examples/demo_store
$ORI docs --format agent --budget 1500 examples/demo_store/src
$ORI run examples/demo_store/src/main.ori
```

All commands return exit 0 on success and emit a JSON envelope
conforming to a schema under `schemas/`.

---

## 7. Test coverage policy

### Per-change rules

1. **A new public function** must land with at least one unit test
   asserting its return value on a representative input.
2. **A new diagnostic ID** must land with: a unit test that emits it, a
   golden source file under `tests/golden/diagnostics/`, and an
   `.expected.jsonl` row.
3. **A new schema** must land with: a `schemas/<name>.schema.json` file
   (Draft 2020-12 + `examples` block), a matching emitter in the
   compiler/agent/pkg, a conformance test that round-trips the live
   output, and a one-line entry in the doctor report's `schema_versions`
   map.
4. **A new CLI subcommand** must land with: help text, exit code 0 on
   success, exit code 1 on user error, exit code 2 on invalid args, and
   at least one CLI-level smoke test.
5. **A new bug fix** must land with a regression test that fails on the
   pre-fix tree.

### Per-release rules

1. The conformance suite must be re-blessed only when behaviour
   intentionally changes, and the bless must be in the same PR as the
   behaviour change.
2. No PR may reduce the workspace test count without an explicit
   `MEMORY.md` decision.
3. No PR may bypass `python3 scripts/validate_all.py --full`.

---

## 8. Determinism contract

Every shipping JSON envelope is **byte-deterministic** for the same
input on the same compiler version:

- `ori check --json`, `ori capsule --json`, `ori agent map --json`,
  `ori openapi --json`, `ori ui --json`, `ori wasm --json`,
  `ori capability --json`, `ori sbom --json`, `ori audit --json`,
  `ori doctor --json` — all use `BTreeMap` / sorted `Vec` for any
  collection that becomes JSON.
- The bench harness's `wasm_encoder::encode_*` returns byte-stable
  output across runs (verified by `hello_module_is_byte_deterministic`).
- The lockfile builder is byte-stable for the same manifest + path
  (`crates/ori-pkg/tests/lockfile_deterministic.rs`).
- The migration plan is byte-stable for the same edition pair.

Tests:
- Existing: 3 explicit `is_deterministic` / `is_byte_stable` cases.
- Planned: a property test sweeping every JSON-emitting CLI subcommand,
  running it twice on a fixed fixture, and asserting byte equality.

---

## 9. Drift prevention

These tests exist specifically to catch drift between subsystems:

| Drift test | Watches |
|------------|---------|
| `doctor_report_lists_every_shipped_schema` | `schemas/*.schema.json` vs in-code doctor list |
| `unsafe_surface_report::workspace_has_zero_unsafe_surface` | every `crates/*/src/**/*.rs` |
| `conformance` (19 cases) | live CLI output vs `tests/golden/*/*.expected.*` |
| `validate_all.py --contracts-only` | every `schemas/*.schema.json` parses + conforms to Draft 2020-12 |
| `validate_all.py` Rust guardrails | no `.unwrap()` / `.expect()` / `panic!` etc. in production sources |
| `validate_all.py` workspace deps | only `serde` + `serde_json` allowed |

Adding new "should never drift" invariants → add a drift test, not
documentation. Drift documentation rots; drift tests don't.

---

## 10. Flake policy

If a test flakes:

1. **Stop the line.** Investigate before merging anything else.
2. **Reproduce in isolation.** `cargo test -p <crate> --test <bin> <name>
   -- --test-threads=1`.
3. **Common cause: shared filesystem state.** Use the `pid + nanos`
   namespacing pattern — see
   `crates/ori-pkg/tests/audit_capability_diff.rs:8` for the
   reference implementation:

   ```rust
   let nanos = std::time::SystemTime::now()
       .duration_since(std::time::UNIX_EPOCH)
       .map(|d| d.as_nanos())
       .unwrap_or(0);
   let tmp = std::env::temp_dir().join(format!(
       "ori_pkg_audit_{}_{}",
       std::process::id(),
       nanos
   ));
   ```

4. **Common cause: non-deterministic iteration.** Replace `HashMap` /
   `HashSet` with `BTreeMap` / `BTreeSet`. Fixed at least 4 sites in
   the bootstrap (see `REJECTIONS_REPORT.md`).
5. **Once fixed, add a regression test** that would have failed under
   parallel scheduling. Mark with the issue number in a comment.

---

## 11. Negative-test corpus (planned)

For every diagnostic ID, a `tests/golden/diagnostics/<id>.ori` file
should trigger exactly that diagnostic. The corpus today covers:
E0001, E0002, E0003, E0100, E0101, E0200, E0201, W0301, W0401, W0501,
W0510. The full set of diagnostic ID prefixes is documented in
`docs/language/REFERENCE.md`; the gaps to fill:

- E0211 (resolver duplicate)
- E0220 (resolver unresolved import)
- E0230 (resolver cycle)
- E0410 (effect undeclared)
- E0420 (effect propagation)
- E0540 (exhaustive match missing arm)
- E1100–E1199 (body parser errors)
- B0010–B0050 (borrow rules)
- P0000–P1010 (Patch IR)
- Q0010, Q0020 (SQL)
- D0010, D0020 (design tokens)
- MOB0001–MOB0003 (mobile)
- PRE0010–PRE0030 (preprocessor)
- A0001–A0003 (async)
- R0001–R0005 (runtime)
- AUD0001, AUD0002 (audit)
- PROTO_E_* (gRPC import)

Each missing fixture is a planned PR.

---

## 12. CLI surface coverage

Every `ori` subcommand must have at least:

1. A help-text assertion (exit 0, contains command name).
2. A JSON-mode assertion (exit 0, output is parseable JSON).
3. A missing-arg assertion (exit 2, helpful error to stderr).
4. A bad-arg assertion (exit 2, helpful error).

Subcommands shipping today (30):

`check`, `fmt`, `capsule`, `agent map`, `agent explain`, `agent symbols`,
`agent diagnose`, `agent tests`, `agent changed`, `patch check`,
`patch apply`, `patch dry-run`, `patch explain`, `lsp`, `package check`,
`audit`, `sbom`, `provenance verify`, `run`, `build`, `bench`, `openapi`,
`ui`, `wasm`, `capability`, `test`, `docs`, `migrate`, `db check`,
`coverage`, `schema import graphql`, `schema import grpc`, `preprocess`,
`publish`, `fetch`, `registry list`, `registry yank`, `design check`,
`doctor`.

Status: smoke-tested manually end-to-end. **Planned:** a single
`crates/ori-cli/tests/cli_smoke.rs` integration binary that exercises
each subcommand on a fixture and asserts the four rules above.

---

## 13. Property test catalogue

| Property | Subject | Status |
|----------|---------|--------|
| `format(format(s)) == format(s)` | formatter | ✓ |
| `parse(s)` is pure | parser | ✓ |
| `lex(s)` is pure | lexer | ✓ |
| `node_id(x) == node_id(x)` for identical input | CST | ✓ |
| `unrelated edit ⇒ node_id unchanged` | CST | ✓ |
| `bench output ordering deterministic` | bench | ✓ |
| `wasm bytes deterministic` | wasm_encoder | ✓ |
| `capsule JSON byte-stable` | capsule | ✓ |
| `agent_map JSON byte-stable` | agent_map | ✓ |
| `lockfile byte-stable` | ori-pkg | ✓ |
| `audit report byte-stable` | ori-pkg | ✓ |
| `migration plan byte-stable` | migrate | ✓ |
| `preprocess(preprocess(x)) == preprocess(x)` for cache | preproc | ✓ |
| `const_fold(const_fold(x)) == const_fold(x)` | const_fold | ✓ |
| Apply patch then check ⇒ no new errors | patch_apply | Planned |
| Resolver fixpoint is monotone | resolver | Planned |
| Effect propagation fixpoint is monotone | effect_propagate | ✓ |

---

## 14. Performance test linkage

Every functional test that runs in < 100 ms can also be timed. The
bench harness (`ori bench`, `crates/ori-compiler/src/bench.rs`)
includes per-suite latency measurements; see `BENCHMARKS.md` for the
methodology and the current numbers.

Performance regressions trigger a follow-up test if and only if the
regression points at a correctness bug. Otherwise they're tracked in
`BENCHMARKS.md`.

---

## 15. Fuzz testing (planned)

The bootstrap policy (`MEMORY.md` D002) forbids third-party deps
including `arbitrary` / `proptest` / `cargo-fuzz`. Once a `MEMORY.md`
decision allows them, the planned harnesses are:

| Target | Input shape | Invariants |
|--------|-------------|------------|
| `lex` | arbitrary UTF-8 bytes | never panics; output token spans sum to input length |
| `parse_source` | arbitrary UTF-8 | never panics; produces a `Module` |
| `check_patch_json` | arbitrary JSON | never panics; produces a `PatchCheckResult` |
| `parse_proto` | arbitrary text | never panics; either `Ok(file)` or `Err(ProtoError)` |
| `parse_sdl` | arbitrary text | never panics; either `Ok(schema)` or `Err(GraphqlParseError)` |
| `apply_patch` | arbitrary CST + Patch IR | never produces a structurally-broken `after` text |
| `encode_from_mir` | arbitrary `MirModule` | output decodes via the in-tree wasm decoder |

Until then, the unit + integration test corpus is the safety net.

---

## 16. Acceptance criteria for "done"

A subsystem is **done** under this framework when, in addition to its
implementation:

- [ ] Every public function has a unit test.
- [ ] Every diagnostic ID has a golden fixture.
- [ ] Every public JSON contract has a schema in `schemas/`.
- [ ] Every CLI subcommand has the four-rule smoke test.
- [ ] Every drift between subsystems has a drift test (§9).
- [ ] The full quality gate (`python3 scripts/validate_all.py --full`)
      is green.
- [ ] Performance is recorded in `BENCHMARKS.md` with a real measured
      number (not a TBD).
- [ ] Any limitations are documented in
      `docs/language/REFERENCE.md` or `docs/ROADMAP.md`.

Subsystems on the matrix above with column-cell `P` are **not done**
until those `P` tests land.

---

## 17. Cross-references

- Quality gate commands: `docs/QUALITY_GATES.md`
- Performance: `BENCHMARKS.md`
- Roadmap (delta to production): `docs/ROADMAP.md`
- Per-milestone plan: `ORISON_AGENT_DEVELOPMENT_HANDOFF.md`
- Honest scope: `README.md`, `MEMORY.md` D014
- Security model: `docs/SECURITY_MODEL.md`
- Demo end-to-end: `docs/DEMO_WALKTHROUGH.md`
- Language reference: `docs/language/REFERENCE.md`
- Architecture map: `docs/ARCHITECTURE_OVERVIEW.md`
- CI: `docs/CI.md`
- Contributing: `docs/CONTRIBUTING.md`
