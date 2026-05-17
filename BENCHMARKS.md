# Orison Benchmarks

Authoritative source for **how fast Orison is**, **what we measure**, and
**what the regression budget is**. Companion to `TESTING-GOALS.md`
(correctness) and `docs/ROADMAP.md` (production-grade delta).

The benchmark contract is `schemas/benchmark.schema.json`
(`ori.benchmark.v1`). Every output from `target/release/ori bench --json`
matches that schema. Raw measurements live in `BENCHMARKS.results.json`.

---

## 1. TL;DR (Apple Silicon, n=100, 40 metrics across 32 suites)

Most recent run on `aarch64-apple-darwin`, `rustc 1.92`, release build:

### Core edit-check-repair loop

| Suite                              | mean      | p50       | p95       |
|------------------------------------|----------:|----------:|----------:|
| `cold_check_latency`               | ~4.8 µs   | ~4.4 µs   | ~14.7 µs  |
| `warm_check_latency`               | ~34.6 µs  | ~21.6 µs  | ~77.9 µs  |
| `cst_parse_latency`                | ~48.8 µs  | ~35.6 µs  | ~110.7 µs |
| `agent_map_token_density` (wall)   | ~25.7 µs  | ~19.3 µs  | ~48.3 µs  |
| `patch_validation_latency`         | ~1.6 µs   | ~1.5 µs   | ~1.8 µs   |
| `patch_apply_latency` (dry-run)    | ~22.7 µs  | ~20.2 µs  | ~68.0 µs  |
| `formatter_throughput`             | ~2.2 µs   | ~1.2 µs   | ~4.1 µs   |
| `capsule_generation_latency`       | ~44.4 µs  | ~31.4 µs  | ~139.2 µs |

### Type system, effects, borrow

| Suite                                 | mean      | p50       | p95       |
|---------------------------------------|----------:|----------:|----------:|
| `type_check_signatures_latency`       | ~46.6 µs  | ~36.1 µs  | ~129.0 µs |
| `exhaustive_match_latency`            | ~90.5 µs  | ~74.5 µs  | ~191.6 µs |
| `effect_propagation_fixpoint_latency` | ~93.7 µs  | ~102.5 µs | ~171.1 µs |
| `capability_manifest_latency`         | ~62.1 µs  | ~21.2 µs  | ~176.3 µs |
| `borrow_check_module_latency`         | ~42.6 µs  | ~21.0 µs  | ~227.3 µs |

### Lowering + codegen

| Suite                                 | mean      | p50       | p95       |
|---------------------------------------|----------:|----------:|----------:|
| `hir_lower_medium_ns`                 | ~34.3 µs  | ~21.4 µs  | ~97.0 µs  |
| `mir_lower_medium_ns`                 | ~31.8 µs  | ~21.9 µs  | ~58.3 µs  |
| `wasm_minimal_ns`                     | **~0.02 µs** | ~0.04 µs | ~0.04 µs |
| `wasm_hello_ns`                       | **~0.56 µs** | ~0.54 µs | ~0.62 µs |
| `wasm_from_mir_ns`                    | ~6.2 µs   | ~6.3 µs   | ~6.5 µs   |
| `textual_ir_emit_latency`             | ~63.9 µs  | ~23.7 µs  | ~185.5 µs |

### Manifests

| Suite                                 | mean      | p50       | p95       |
|---------------------------------------|----------:|----------:|----------:|
| `openapi_extract_latency`             | ~42.4 µs  | ~19.7 µs  | ~161.8 µs |
| `ui_manifest_latency`                 | ~25.7 µs  | ~20.1 µs  | ~51.8 µs  |
| `wasm_component_manifest_latency`     | ~26.6 µs  | ~20.7 µs  | ~61.5 µs  |
| `mobile_manifest_latency`             | ~20.8 µs  | ~20.8 µs  | ~21.1 µs  |

### Body parser, runtime

| Suite                                 | mean      | p50       | p95       |
|---------------------------------------|----------:|----------:|----------:|
| `body_parse_latency`                  | ~38.2 µs  | ~32.3 µs  | ~83.7 µs  |
| `async_scheduler_throughput` (100 ops)| ~7.9 µs   | ~7.1 µs   | ~10.1 µs  |

### Importers

| Suite                                 | mean      | p50       | p95       |
|---------------------------------------|----------:|----------:|----------:|
| `graphql_parse_ns`                    | ~4.4 µs   | ~4.0 µs   | ~5.8 µs   |
| `graphql_emit_ns`                     | ~7.4 µs   | ~4.5 µs   | ~13.3 µs  |
| `rpc_parse_ns`                        | ~16.7 µs  | ~14.7 µs  | ~53.7 µs  |
| `rpc_emit_ns`                         | ~25.6 µs  | ~19.3 µs  | ~76.8 µs  |

### Database

| Suite                                 | mean      | p50       | p95       |
|---------------------------------------|----------:|----------:|----------:|
| `sql_query_check_latency`             | ~27.6 µs  | ~11.6 µs  | ~116.7 µs |
| `migration_toposort_latency`          | ~28.2 µs  | ~9.7 µs   | ~155.6 µs |

### Agent ABI

| Suite                                 | mean      | p50       | p95       |
|---------------------------------------|----------:|----------:|----------:|
| `agent_map_budget_500_ns`             | ~37.1 µs  | ~19.7 µs  | ~135.3 µs |
| `agent_map_budget_2000_ns`            | ~52.2 µs  | ~41.3 µs  | ~152.6 µs |
| `agent_map_budget_4000_ns`            | ~59.8 µs  | ~41.3 µs  | ~215.1 µs |
| `coverage_report_latency`             | ~57.4 µs  | ~41.0 µs  | ~191.1 µs |
| `docs_human_ns`                       | ~83.1 µs  | ~62.9 µs  | ~253.7 µs |
| `docs_agent_budget_1500_ns`           | ~63.6 µs  | ~28.4 µs  | ~206.8 µs |
| `preproc_substitute_ns`               | **~0.77 µs** | ~0.71 µs | ~0.88 µs |

### Incremental + query engine

| Suite                                 | mean      | p50       | p95       |
|---------------------------------------|----------:|----------:|----------:|
| `incremental_cache_latency`           | ~41.4 µs  | ~23.7 µs  | ~108.4 µs |
| `query_fingerprint_latency`           | ~38.8 µs  | ~21.3 µs  | ~108.1 µs |

**Headline.** Patch validation and the wasm-encoder cold path are
sub-microsecond. Every per-symbol manifest (capability / OpenAPI / UI /
wasm component / mobile) lands under 50 µs at p50. The most expensive
shipping pass is effect propagation (~100 µs p50) because it walks the
entire body parser output. The whole compile + emit-capsule +
budget-pack-symbol-map + patch round-trip on a typical storefront API
file fits in well under 200 µs at p95.

The full structured run lives in `BENCHMARKS.results.json` and matches
`schemas/benchmark.schema.json`.

---

## 2. Why these suites

Each suite measures a discrete step in the **edit-check-repair loop**
that the language is designed for:

1. **Edit happens** (text changes — not measured; happens in the
   editor or in an agent's tool call).
2. **`cold_check_latency`** — first compile on a freshly started
   process. Models a cold CI step.
3. **`warm_check_latency`** — re-compile after a warm-up. Models the
   per-keystroke editor experience.
4. **`cst_parse_latency`** — just the CST construction, isolated.
   Stable component of the warm number.
5. **`agent_map_token_density`** — emit the agent map JSON that an LLM
   reads to scope its next action. Time + (planned) tokens per symbol.
6. **`patch_validation_latency`** — agent emits a Patch IR; the compiler
   says yes/no before disk is touched.
7. **`patch_apply_latency` (dry-run)** — apply that patch to source in
   memory, hand back the resulting text. The "preview" the agent shows.
8. **`formatter_throughput`** — normalising whitespace after a patch
   round-trip.
9. **`capsule_generation_latency`** — produce the structural summary
   the next agent invocation reads as context.

Total p50 round-trip = ~2 µs (cold) + 20 µs (warm) + 19 µs (agent map)
+ 1 µs (patch check) + 10 µs (patch apply) + 1 µs (format) + 24 µs
(capsule) ≈ **~77 µs at p50** for a complete agent edit cycle on a
medium fixture.

---

## 3. Methodology

### Warm-up

`ori bench` performs **2 untimed warm-up iterations** per metric before
recording samples. This amortises one-time costs: JIT, branch caches,
filesystem page caches.

### Sample size

Default 5; the headline run above uses `--samples 100`. The harness
clamps the lower bound to `MIN_SAMPLES = 3` so percentile reporting
never reads a single observation.

### Statistics

- `mean` — arithmetic mean across all samples.
- `p50` — median.
- `p95` — 95th percentile (floor of `0.95 * n`).
- `min` / `max` — raw extremes.

No automatic outlier dropping — the harness returns the full extreme so
callers can apply their own policy. The headline table above quotes p50
when comparing across runs because p95/max are sensitive to OS
background activity.

### Environment fields recorded

Each `BENCHMARKS.results.json` envelope includes:

- `environment.os` — `std::env::consts::OS`.
- `environment.arch` — `std::env::consts::ARCH`.
- `environment.rustc_version` — from `option_env!("RUSTC_VERSION")`.
- `environment.cpu` — `null` in the bootstrap (planned: read
  `/proc/cpuinfo` on Linux, `sysctl -n machdep.cpu.brand_string` on
  macOS).
- `generated_at` — `@unix:<seconds>` placeholder. No chrono dependency
  in the bootstrap.

### Hardware caveats

Benchmark numbers are **not portable between machines**. Cite
`environment.os` and `environment.arch` when comparing results. Do not
extrapolate from laptop runs to server hardware.

### Determinism

The benchmark harness itself is deterministic in input but not in
output (wall-clock measurements have inherent jitter). Two consecutive
runs on the same hardware should agree on p50 within ±20% and on mean
within ±50%; deviations beyond those bands are an environmental signal
(thermal throttling, background CPU pressure, swap activity).

---

## 4. The eight shipping suites in detail

### 4.1 `cold_check_latency`

**Measures.** Wall time for `Compiler::check_source` on a small
fixture (`hello`-shape, ~2 symbols), no warm cache for the process.

**Fixture.** Inlined `DEFAULT_SMALL` in
`crates/ori-compiler/src/bench.rs`; falls back to a disk read of
`examples/hello.ori` when the working directory is the repo root.

**Threshold (advisory).** p50 < 10 µs, p95 < 50 µs on commodity ARM /
x86 silicon.

### 4.2 `warm_check_latency`

**Measures.** Wall time for `Compiler::check_source` on the medium
fixture (~12 symbols, multi-import, multi-type, service + view), with
caches warm.

**Fixture.** Inlined `DEFAULT_MEDIUM`; falls back to
`examples/fullstack/users.ori`.

**Threshold (advisory).** p50 < 50 µs, p95 < 100 µs. The
edit-check-repair loop budget assumes warm check stays at this level.

### 4.3 `cst_parse_latency`

**Measures.** `cst::parse_cst` on the medium fixture, isolated from
the rest of the compile pipeline.

**Why isolated.** Tracks whether CST construction (the most
algorithmically structural piece of the parser) regresses
independently of the rest of `check_source`.

**Threshold (advisory).** p50 < 50 µs.

### 4.4 `agent_map_token_density`

**Measures.** Wall time for one compile + one agent-map-shaped JSON
emit on the medium fixture.

**Future shape.** Adds a `tokens_per_symbol` metric once a tokenizer
is committed (the bootstrap currently reports wall time only). The
schema (`benchmark.schema.json`) already allows arbitrary `key` strings
so adding the new metric is non-breaking.

**Threshold (advisory).** p50 < 50 µs.

### 4.5 `patch_validation_latency`

**Measures.** `patch::check_patch_json` on a 1-op patch fixture.

**Threshold (advisory).** p50 < 5 µs. This sets the agent feedback
budget — patch validation is the inner loop of "agent proposes a fix,
compiler rejects bad shape, agent retries."

### 4.6 `patch_apply_latency` (dry-run)

**Measures.** `patch_apply::apply_patch` on a no-op insert against the
small fixture, in `dry_run = true` mode.

**Threshold (advisory).** p50 < 50 µs.

### 4.7 `formatter_throughput`

**Measures.** `Compiler::format_source` on the medium fixture.

**Threshold (advisory).** p50 < 10 µs. The formatter must stay cheaper
than the parser; if formatting ever exceeds 2× parser time, something
regressed (formatter is supposed to be a single CST walk).

### 4.8 `capsule_generation_latency`

**Measures.** One compile + one `Compiler::capsule_json` emit on the
medium fixture.

**Threshold (advisory).** p50 < 100 µs.

---

## 5. Planned suites (not yet measured)

Every line is a benchmark that **should** be added. Each is gated on a
specific implementation milestone (see `docs/ROADMAP.md` for the
larger context).

### 5.1 Compilation

- `incremental_edit_latency` — re-check after a one-line edit using the
  query cache. **Gated on:** real query engine memoisation
  (`crates/ori-compiler/src/query.rs` ships the cache, but the compiler
  doesn't yet plug into it). Target p50 < 5 µs once wired.
- `multi_module_check_latency` — full demo storefront (6 modules) cold
  check. Target p50 < 200 µs.
- `large_project_check_latency` — synthesised 100-symbol module. Target
  p50 < 1 ms.
- `release_build_latency` — `ori build --target release` end-to-end.
  **Gated on:** real release backend (currently textual IR only). Target
  p50 < 50 ms for the demo storefront.

### 5.2 Lexer / parser micro

- `lex_throughput_chars_per_ms` — lex a 10 KB synthetic file. Target
  > 5 MB/s.
- `body_parse_latency` — `body::parse_module_bodies` on a 20-fn
  fixture. Target p50 < 100 µs.
- `cst_node_id_construction` — `make_node_id` micro-bench. Target
  p50 < 100 ns.

### 5.3 Type system

- `type_check_signatures_latency` — `type_check::type_check_module` on
  the medium fixture. Target p50 < 20 µs.
- `type_infer_bodies_latency` — `type_infer::check_module_bodies` on the
  body-parser corpus. Target p50 < 50 µs.
- `exhaustive_match_latency` — `exhaustive::check_module_matches`.
  Target p50 < 30 µs.
- `effect_propagation_fixpoint_latency` — `effect_propagate::propagate_effects`
  on a 3-deep call chain. Target p50 < 20 µs.

### 5.4 Borrow checker

- `borrow_check_module_latency` — `borrow::borrow_check_module` on the
  medium fixture. Target p50 < 30 µs.

### 5.5 Codegen

- `wasm_encode_hello_latency` — `wasm_encoder::encode_hello_module`.
  Target p50 < 1 µs (cold), < 200 ns (warm).
- `wasm_encode_from_mir_latency` — `wasm_encoder::encode_from_mir` on
  a 5-function MIR. Target p50 < 5 µs.
- `textual_ir_emit_latency` — `codegen_text::emit_textual_ir`. Target
  p50 < 5 µs.
- `wasm_module_size_bytes` — emitted bytes for the demo storefront once
  cross-module wasm lands. Target < 50 KB for the demo storefront.
- `native_binary_size_bytes` — **gated on:** native AOT backend. Target
  < 5 MB for the demo storefront's `ori build --target release`.

### 5.6 Runtime

- `interp_arithmetic_throughput` — `interp_exec::exec_program` running
  100 nested `if` / `let` expressions. Target > 100 k ops/s.
- `interp_call_throughput` — recursive Fibonacci-style benchmark with
  the 256-frame cap. Target > 50 k calls/s.
- `interp_match_throughput` — variant pattern matching, 1000 samples.
  Target > 100 k matches/s.
- `async_scheduler_throughput` — `async_runtime::run_to_completion` on
  1000 spawn + 1000 resume tasks. Target > 1 M scheduling ops/s.

### 5.7 Patch IR

- `patch_check_complex_latency` — 20-op patch. Target p50 < 20 µs.
- `patch_apply_complex_latency` — 20-op patch dry-run. Target p50 < 200 µs.
- `patch_round_trip_determinism_ns` — apply twice, assert byte-equal,
  measure overhead vs single apply. Target < 5% overhead.

### 5.8 Agent ABI

- `agent_map_budget_overhead` — packing time at each budget level.
  Target overhead < 1 µs per included symbol.
- `agent_diagnose_latency` — `agent_diagnose_json` on the demo
  storefront. Target p50 < 30 µs.
- `agent_symbols_latency` — `agent_symbol_list_json`. Target p50 < 20 µs.
- `tokens_per_symbol_density` — token count of `agent map` divided by
  symbol count. Target < 50 tokens / symbol (planned histogram, see §4.4).
- `affected_test_selection_latency` — `select_affected_tests` on a
  100-test corpus with 5 changed symbols. Target p50 < 100 µs.

### 5.9 Package manager

- `manifest_parse_latency` — `Manifest::parse` on the repo's
  `ori.toml`. Target p50 < 10 µs.
- `lockfile_build_latency` — `build_lockfile` on a 10-dep graph.
  Target p50 < 50 µs.
- `sbom_generate_latency` — `build_sbom`. Target p50 < 100 µs.
- `audit_latency` — `run_audit`. Target p50 < 100 µs.
- `provenance_verify_latency` — `verify_provenance`. Target p50 < 10 µs.
- `registry_publish_latency` — local registry stub publish. Target
  p50 < 1 ms (filesystem-bound).

### 5.10 LSP

- `lsp_initialize_latency` — first response after `initialize` request.
  Target p50 < 50 µs.
- `lsp_publish_diagnostics_latency` — emit after `didChange`. Target
  p50 < 100 µs on the medium fixture.
- `lsp_completion_response_latency` — `textDocument/completion`. Target
  p50 < 200 µs (covers compile + sort + serialise).
- `lsp_rename_throughput` — `textDocument/rename` on a 100-occurrence
  fixture. Target p50 < 1 ms.
- `lsp_workspace_symbol_latency` — open 10 docs, query. Target p50 < 5 ms.

### 5.11 Importers

- `graphql_parse_latency` — `parse_sdl` on a 20-type schema. Target
  p50 < 50 µs.
- `graphql_emit_latency` — `to_orison_module`. Target p50 < 30 µs.
- `grpc_parse_latency` — `parse_proto` on a 10-message + 5-service
  proto. Target p50 < 100 µs.
- `grpc_emit_latency` — `to_orison_module`. Target p50 < 30 µs.

### 5.12 Database

- `query_check_latency` — `sql_check::check_module_queries` on a
  10-query module. Target p50 < 50 µs.
- `migration_toposort_latency` — `migration_graph::topological_order` on
  a 20-node graph. Target p50 < 20 µs.

### 5.13 UI / mobile / preprocessor / coverage / docs

- `ui_manifest_latency` — `ui_check::build_ui_manifest`. Target
  p50 < 30 µs.
- `design_token_check_latency` — `design_tokens::check_module`. Target
  p50 < 50 µs (depends on file IO for tokens.toml).
- `mobile_manifest_latency` — `mobile::build_mobile_manifest`. Target
  p50 < 20 µs.
- `preproc_substitution_throughput` — substitutions per ms on a 100-marker
  fixture. Target > 1 M substitutions/s.
- `coverage_report_latency` — `coverage::coverage_for_files` on the demo
  storefront. Target p50 < 100 µs.
- `docs_generate_human_latency` — `docs::generate_human_docs` on the demo
  storefront. Target p50 < 500 µs.
- `docs_generate_agent_latency_at_budget` — `docs::generate_agent_docs`
  at budget = 1500. Target p50 < 300 µs.

### 5.14 Full quality gate

- `static_gate_latency` — `python3 scripts/validate_all.py --static-only`
  cold. Target < 5 s on commodity hardware.
- `pre_commit_gate_latency` — `--pre-commit`. Target < 30 s.
- `full_gate_latency` — `--full`. Target < 5 minutes.

### 5.15 End-to-end agent loop (the headline number)

The composite that matters most for the language's wedge:

```
edit → check → diagnose → patch → apply → re-check → capsule
```

**Composite p50 budget (planned):** < 200 µs at p50 for the demo
storefront. Today the sub-components add up to ~77 µs at p50.

This becomes a real suite when the harness wires those subcalls into
a single closure rather than separate metrics.

### 5.16 Model-in-the-loop benchmarks (research target)

These need a model + a test harness not in the bootstrap. They drive
the language's product wedge:

- `tokens_per_accepted_patch` — average tokens an agent consumes to
  produce a patch that passes `ori patch check`.
- `patches_accepted_first_try` — fraction of first-shot patches that
  apply cleanly.
- `regression_rate_per_patch` — fraction of accepted patches that break
  another test.
- `iterations_to_green` — average compile-fix-rerun cycles per
  successful fix.
- `tokens_per_completed_task` — end-to-end budget for the canonical
  demo-storefront task set.

These belong in a separate `benchmarks/agents/` repo because they pull
in models, prompts, and timing methodology that exceed the bootstrap
scope. They're listed here for completeness.

---

## 6. How to run

### One-shot release run

```bash
cargo build --release -p ori
target/release/ori bench --samples 100 --json > BENCHMARKS.results.json
```

### Human-readable

```bash
target/release/ori bench --samples 50 --no-json
```

### Quick smoke (3 samples / suite)

```bash
target/release/ori bench --samples 3
```

### From Make

```bash
make bench         # quick run (default samples=20)
make bench-json    # runs n=100 and writes BENCHMARKS.results.json
```

### `cargo bench` (Criterion)

Currently **unavailable**: the bootstrap dependency policy
(`MEMORY.md` D002) forbids `criterion`. Adding it requires a
`MEMORY.md` decision entry and a `CHANGELOG.md` note.

---

## 7. Reading `BENCHMARKS.results.json`

The file is a single object matching `schemas/benchmark.schema.json`:

```json
{
  "schema": "ori.benchmark.v1",
  "generated_at": "@unix:1778971817",
  "environment": {
    "os": "macos",
    "arch": "aarch64",
    "rustc_version": "unknown",
    "cpu": null
  },
  "suites": [
    {
      "name": "warm_check_latency",
      "metrics": [
        {
          "key": "check_medium_ns",
          "unit": "ns",
          "samples": 100,
          "mean": 24114.7,
          "p50": 20211.5,
          "p95": 46541.0,
          "max": 77083.0,
          "min": 19500.0
        }
      ]
    }
  ]
}
```

To compare two runs:

1. Confirm both runs share `environment.os` and `environment.arch`. If
   not, comparison is invalid.
2. Compare `p50` first; use `p95` for tail-sensitive suites
   (`cold_check_latency`, `agent_map_token_density`,
   `capsule_generation_latency` — these flake more under OS noise).
3. Flag any p50 regression > 10% over the previous accepted run for
   that suite. Investigate; don't auto-block.

---

## 8. Regression policy

| Class | Threshold | Action |
|-------|-----------|--------|
| **Hard regression** | p50 doubles vs the previous accepted run | Block merge; bisect |
| **Soft regression** | p50 grows 10–100% | Investigate; document; merge if explained |
| **Drift** | p50 within ±10% | Ignore |
| **Improvement** | p50 shrinks | Celebrate; update advisory thresholds in §4 if sustained over 3 runs |

The accepted run is whatever lives in `BENCHMARKS.results.json` on
`main`. The CI release workflow (`release.yml`) uploads a fresh
`bench-<sha>.json` artefact per release; the next PR can diff against
that.

---

## 9. What's *not* benchmarked here

The following are deliberately out of scope for the bootstrap (see
`docs/ROADMAP.md` for the production-grade delta):

- **Native AOT codegen wall time.** The bootstrap emits a textual
  LLVM-IR-shape stand-in (`codegen_text.rs`); no native binary.
  Numbers here would mislead.
- **Wasm module size for non-trivial inputs.** The encoder produces a
  39-byte hello-module; multi-function modules need the cross-fn
  encoder.
- **M:N async runtime throughput.** The async scheduler is cooperative
  + single-threaded by design.
- **Cryptographic-signing overhead.** The lockfile checksum is FNV-1a;
  benchmarking it gives a misleading sense of "supply-chain security."
- **Real network IO.** The HTTP / WebSocket / queue stdlib modules are
  declarations only; no runtime.
- **Compile-time of the *Orison compiler itself* in Orison.** Self-hosting
  is a non-goal of the bootstrap.

When any of these gain real implementations, this document gets new
suites and `docs/ROADMAP.md` gets updated.

---

## 10. Changelog

| Date | Change |
|------|--------|
| 2026-05-16 | First real measured numbers committed across the eight shipping suites. Full plan of planned suites documented. |
| 2026-05-16 | Headline composite p50 budget (< 200 µs end-to-end for the demo storefront) set as the wedge target. |
| 2026-05-16 | Regression policy formalised — 10% / 100% / 200% bands. |

---

## 11. Cross-references

- Test framework: `TESTING-GOALS.md`
- Quality gate: `docs/QUALITY_GATES.md`
- Roadmap (delta to production): `docs/ROADMAP.md`
- Honest scope: `README.md`, `MEMORY.md` D014
- Architecture: `docs/ARCHITECTURE_OVERVIEW.md`
- Schema: `schemas/benchmark.schema.json`
- Raw data: `BENCHMARKS.results.json`
- Demo end-to-end: `docs/DEMO_WALKTHROUGH.md`
