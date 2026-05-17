# Self-Hosting Design (M36)

> Authoritative design document for the M36 self-hosting milestone.
>
> **Status.** Design only. No implementation work has been done. This
> document exists so that when M21–M27 land, a future maintainer can
> read one file and know what stage1 / stage2 means in the Orison
> context, what gates the work, and what acceptance looks like.
>
> **Scope.** This document supersedes any earlier prose about
> "self-hosting" in `GOAL.md` (§6, M36) and `docs/ROADMAP.md` ("Self
> hosting" line under "Not yet shipping to production grade"). When the
> two disagree, the present document is the one that holds.
>
> **Audience.** The future maintainer who picks up M36 after M27 ships,
> plus the reviewer of the first PR that lands `compiler-self/`.

---

## Table of contents

1. [Why self-host](#1-why-self-host)
2. [The stage discipline](#2-the-stage-discipline)
3. [Language prerequisites](#3-language-prerequisites)
4. [Self-hosting architecture](#4-self-hosting-architecture)
5. [Conformance gates between stages](#5-conformance-gates-between-stages)
6. [Risks and mitigations](#6-risks-and-mitigations)
7. [Acceptance criteria for stage1](#7-acceptance-criteria-for-stage1)
8. [Acceptance criteria for stage2 and production-ready][toc-8]

[toc-8]: #8-acceptance-criteria-for-stage2-and-production-ready
9. [Migration plan](#9-migration-plan)
10. [Comparison to other languages](#10-comparison-to-other-languages)
11. [Out of scope](#11-out-of-scope)
12. [Open questions](#12-open-questions)
13. [Cross-references](#13-cross-references)

---

## 1. Why self-host

### 1.1 What self-hosting validates

Self-hosting — building the compiler in the language it compiles — is
not a marketing milestone. It is the single most expensive end-to-end
test the language can run on itself. A language whose compiler is
written in Rust cannot honestly claim to be "production-grade" until
the language can express its own front-end, analysis passes, IR
lowering, and codegen with the same clarity that Rust gives those
passes today in `crates/ori-compiler/src/`.

Self-hosting validates three properties at once:

1. **Expressiveness.** The Orison compiler is roughly 32k LOC of
   Rust spread across 52 modules in `crates/ori-compiler/src/`
   (see `crates/ori-compiler/src/lib.rs` for the canonical list).
   It exercises every front-end pattern Orison aspires to:
   recursive descent parsing (`parser.rs`, `body.rs`,
   `expr.rs` at 53k bytes), graph-shaped data structures
   (`migration_graph.rs`, `effect_propagate.rs`), bitwise byte
   emission (`wasm_encoder.rs`, `wasm_component.rs`),
   Hindley-Milner-shaped inference scaffolds (`type_infer.rs` at
   36k bytes), and bytecode interpretation
   (`interp.rs` + `interp_exec.rs`). If Orison cannot express
   each of these without reaching for an escape hatch the language
   has failed.

2. **Stdlib quality.** The compiler is the single most demanding
   stdlib consumer the project will ever have. It exercises every
   `core` collection (sorted maps everywhere per invariant §3.9 of
   `GOAL.md`), every `std.json` codepath (one envelope per
   diagnostic, capsule, agent-map, manifest), every `std.fs`
   codepath (reads the workspace, writes artefacts), and every
   `std.process` codepath (shells out to the linker in M24). If
   `std.json` parses 1 GB/s on the synthetic benchmark required by
   M27 but cannot round-trip a real `ori.diagnostic.v1` envelope in
   a self-hosted compiler under load, M27 was not done.

3. **Performance.** Compiler hot paths are the most-traversed code
   in the runtime. If the Orison-source compiler is more than 2×
   slower than the Rust bootstrap on the same input
   (`examples/demo_store/`), the language has a real-world
   performance gap that no synthetic benchmark in `bench.rs` will
   ever surface.

### 1.2 What risks self-hosting introduces

Self-hosting is also the milestone with the highest blast radius if
mishandled. The risks that motivated keeping it as the capstone
milestone (M36 in `GOAL.md` §6) rather than an early goal:

- **Circular debugging.** A bug in stage1 produces a wrong stage2
  binary which produces a wrong stage3 binary, and the symptom
  shows up four hops away from the cause. Languages without
  disciplined stage gating (Section 2) have lost weeks to this.

- **Trust-erosion.** "We self-host" is a claim, not a property.
  Many languages claim it and ship a stage1 that compiles toy
  examples but cannot rebuild itself from cold (Section 10 lists
  examples). The conformance gates in Section 5 exist so the claim
  is verifiable from the repository alone.

- **Bootstrap-chain dilation.** Once stage1 is canonical, every
  contributor needs a working stage0 to bootstrap from cold. If
  stage0 atrophies (no one runs it), it stops working, and the
  project loses the only escape hatch when stage1 wedges itself
  (Section 6.1).

- **Determinism leakage.** The compiler's own JSON envelopes
  (`schemas/diagnostic.schema.json` and the 33 others under
  `schemas/`) must remain byte-stable across stage1 and stage2.
  Determinism rules that hold for a Rust author (no `HashMap`
  iteration, `BTreeMap` only — see `crates/ori-compiler/src/borrow.rs`
  line 39 for the canonical example) must be enforced by the
  Orison language itself, not just by author discipline (Section
  6.2).

### 1.3 When is it appropriate to start

Per `GOAL.md` §6 M36, the milestone is explicitly **blocked by
M21–M27**. The blocking rationale, expanded:

- **M21 (body parser completion).** Without binary operators,
  string interpolation, raw / multiline strings, doc comments, and
  trailing commas, the existing compiler source in `expr.rs` (53k
  bytes, hundreds of `+`, `-`, `==`, `&&`, `||` operators) cannot
  even be transcribed.

- **M22 (full bidirectional inference).** The compiler is
  pervasively generic — `Vec<Diagnostic>`, `BTreeMap<String,
  NodeId>`, `HashMap<NodeId, EffectSet>`. Signature-level
  inference (today's state per `crates/ori-compiler/src/type_infer.rs`)
  cannot type-check arbitrary expression bodies. M22 closes that
  gap.

- **M23 (region-inference borrow checker).** The compiler aliases
  data extensively: an `&Module` is passed through dozens of
  analysis passes (see `crates/ori-compiler/src/borrow.rs` header
  comment for the catalogue). Signature-level borrow checking
  (today's B0010–B0050) cannot validate this; region inference is
  the precondition for the self-hosted compiler to even type-check
  cleanly.

- **M24 (native AOT) OR a usable wasm executor.** Stage1 must
  produce a runnable artefact. It cannot stay an interpreted
  curiosity. Either M24 (`ori build --target release` produces a
  native binary) or a maturation of `wasm_encoder.rs` /
  `wasm_component.rs` past the 39-byte hello module to the level
  required by M25 is the precondition.

- **M27 (real stdlib).** The compiler reads files (`std.fs`),
  serializes JSON (`std.json`), spawns linkers (`std.process`),
  reads environment (`std.config`), and manipulates strings
  pervasively. The bootstrap's "every stdlib module is a
  declaration" state (per `GOAL.md` §5) makes self-hosting
  impossible by construction.

Each blocker resolves a class of "I cannot write this in Orison"
problems. Attempting self-hosting before M21–M27 land would force
either (a) compiler authors to wait on every blocked feature for
months at a time, or (b) introducing a "lightning subset" early that
diverges from the real language. Both outcomes are worse than waiting.

`GOAL.md` §9.3 (Risk: self-hosting might never reach byte-identical
stages) is also worth re-reading before committing: the rational path
is to ship stage1 as a *secondary* compiler under CI, and only later
promote it to canonical (Section 9).

---

## 2. The stage discipline

The names "stage0", "stage1", "stage2" are not Orison-specific. They
are the convention every self-hosted language uses (Rust, Go, Zig,
TypeScript — see Section 10 for what each meant in practice). This
section pins their meaning in the Orison context.

### 2.1 Stage 0 — the current Rust-based bootstrap

Stage 0 is the contents of `crates/ori-compiler/`, plus the CLI
front-end in `crates/ori-cli/` and the supporting crates
`crates/ori-agent/`, `crates/ori-lsp/`, and `crates/ori-pkg/`.

Key properties of stage 0:

- Written in Rust against the `serde + serde_json`-only dependency
  policy (`GOAL.md` §3.6, also documented in `crates/ori-compiler/src/lib.rs`
  module doc).
- Implements the full pipeline at the maturity level recorded in
  `GOAL.md` §5 (the "Where we are today" baseline).
- Produces every shipping JSON envelope (34 schemas under
  `schemas/`).
- Passes the full quality gate via
  `python3 scripts/validate_all.py --full` (see lines 269–298 for
  the exact invocations).
- Stays in-tree **permanently** as the reference implementation
  and the cold-bootstrap escape hatch (Section 6.1, Section 9.4).

Stage 0 is the compiler. The Orison language exists today only
because stage 0 exists. Every claim in `GOAL.md` §2 (what makes
Orison unique) is implemented by stage 0.

### 2.2 Stage 1 — the Orison-source compiler, built by stage 0

Stage 1 is a new artefact: an Orison-source rewrite of the stage-0
compiler, compiled by stage 0 (a "self-applied bootstrap").

Key properties:

- Source lives under `compiler-self/src/` (Section 4.1) and is
  written in Orison (not Rust).
- The module structure mirrors `crates/ori-compiler/src/` 1:1 — if
  stage 0 has a `borrow.rs`, stage 1 has `compiler-self/src/borrow.ori`.
  This is intentional: it makes diffs reviewable, makes "is the
  Orison version doing the same thing" answerable by line-by-line
  comparison, and means a fix in one stage maps mechanically to
  the other.
- Stage 1 may, during development, omit features that the
  language does not yet require *of itself* (a "lightning
  compiler" subset — see Section 6.3 for why this is acceptable
  and Section 7 for the acceptance gate it loosens).
- Stage 1 must produce **byte-identical JSON envelopes** to stage 0
  for the conformance corpus (Section 5).
- Stage 1 must produce **functionally identical artefacts**
  (binaries, wasm components) to stage 0 — but identity is not
  required at the byte level yet; that is stage 2's bar.

The build path:

```text
stage 0 (Rust)  --build-->  stage 0 binary
                                ↓
                       compiles compiler-self/src/
                                ↓
                          stage 1 binary
```

### 2.3 Stage 2 — the Orison-source compiler, built by stage 1

Stage 2 is the same source tree as stage 1
(`compiler-self/src/`), compiled by stage 1.

Key properties:

- Identical *source* to stage 1.
- Built by *stage 1*, not stage 0.
- Output (the stage 2 binary) must be **byte-identical** to the
  stage 1 binary for the conformance corpus (Section 5.3 lists the
  things that count).
- Stage 2 also must produce byte-identical outputs (envelopes and
  artefacts) when applied to the conformance corpus.

The build path extends stage 1's:

```text
stage 1 binary  --compiles compiler-self/src/-->  stage 2 binary

                  (must be byte-identical)
```

### 2.4 Convergence

The convergence claim is the substantive one. It says:

> Running stage 1 on `compiler-self/src/` produces the same bytes
> as running stage 2 on `compiler-self/src/`. And both produce the
> same bytes as running stage 1 (or stage 2) on every input in the
> conformance corpus.

This is the fixed-point property. A compiler that converges is a
compiler that has reached the point where iterating the bootstrap
adds nothing. If stage 2 ≠ stage 3, the compiler is still encoding
some path-dependent state (build timestamp, host environment,
something).

Convergence is enforced by:

1. A CI job (`bootstrap-stage2`, Section 4.3) that runs stage 1 to
   produce stage 2 and then runs stage 2 to produce stage 3
   (called "stage 2 verification"), then asserts byte-identity of
   the two compiler binaries.
2. A second CI job that runs stage 1 and stage 2 against the full
   conformance corpus and asserts every envelope and artefact is
   byte-identical (Section 5).

### 2.5 Stage 0 retention

Stage 0 stays. Forever.

Once stage 2 ships and is promoted to canonical (Phase 4, Section
9.4), stage 0 becomes the reference implementation and the
cold-bootstrap path. It does *not* become a deprecated cul-de-sac.
Specifically:

- The Rust source under `crates/ori-compiler/` stays in-tree.
- A CI job continues to build stage 0 with `cargo build --release`
  on every commit.
- A second CI job continues to run the stage 0 quality gate
  (`python3 scripts/validate_all.py --full` — verified in lines
  282–289 of `scripts/validate_all.py`).
- The MR-template includes a "did this change affect stage 0
  semantics?" question, and any divergence must be intentional and
  documented (Section 6.5).

The reason for indefinite retention: every other escape hatch the
project has (no `unsafe`, no panics, no exceptions, no `unwrap`) is
a *constraint*. Stage 0 retention is the *recovery* mechanism. If
stage 1 or stage 2 ever wedges itself in a way that prevents
self-rebuilding, stage 0 is the way out.

---

## 3. Language prerequisites

A bullet-point recap of `GOAL.md` §6 M21–M27 from the perspective
of "what self-hosting needs." Each item below cites the milestone
and the file in the existing stage-0 compiler that exercises the
feature, demonstrating that stage 1 cannot omit it.

### 3.1 From M21 — body parser completion

Required because every non-trivial pass in the compiler reads or
emits expressions that exercise these:

- **Binary operators** (`+ - * / % == != < <= > >= && || ??`).
  Used pervasively. A sampling: `crates/ori-compiler/src/expr.rs`
  (the constant-fold logic in `const_fold.rs` requires binary
  arithmetic), `crates/ori-compiler/src/borrow.rs` (the B0010
  rule checks `if param.uses_mut && other.uses_shared` style
  guards), `crates/ori-compiler/src/effect_propagate.rs` (set
  union via `||` patterns).
- **Unary operators** (`- !`). Negation used throughout. Boolean
  inversion used in every guard.
- **Method-call syntax** (`recv.method(args)`). The compiler is
  written in a mostly-OO style — `module.symbols.iter()`,
  `span.start.line` — and any literal transcription needs this.
- **String interpolation** (`"hello {name}"`). Diagnostic
  messages are constructed via interpolation throughout
  `diagnostic.rs`. The escape hatch (string concatenation with
  `+`) works but is uglier than the Rust `format!` it replaces.
- **Raw strings** and **multiline strings**. JSON envelope tests
  (`crates/ori-cli/tests/cli_smoke.rs`) construct expected
  outputs as multiline string literals.
- **Numeric literal forms** (`_` separators, `0x`, `0b`, `0o`).
  Used for byte-level constants in `wasm_encoder.rs` (the wasm
  magic number `0x0061_736d` is one example).
- **Doc comments** (`///`). Required so the self-hosted compiler
  can carry its own documentation forward.
- **Trailing commas**. A formatter and a parser that disagree on
  trailing commas is the canonical "self-hosting wedge" — see
  Go's first bootstrap attempt for the precedent.

### 3.2 From M22 — full HM inference

Required because the compiler is generic, polymorphic, and uses
inference at almost every binding site.

- `let xs = collect(parser.parse_items())` must infer
  `xs: Vec[Item]` from the right-hand side. The existing
  `crates/ori-compiler/src/type_infer.rs` is the prototype; the
  M22 deliverables (bidirectional, unification with occurs check,
  let polymorphism, instantiation by use site) are the bar.
- Pattern narrowing in `match` arms is required because the
  diagnostic emission code uses `match diag.level { ... }`
  pervasively (see `crates/ori-compiler/src/diagnostic.rs`).
- Effect inference (from M22) is required because the compiler's
  own modules declare effects (a self-hosted parser declares
  `uses fs.read`; a self-hosted code emitter declares
  `uses fs.write`).

### 3.3 From M23 — region-inference borrow checker

The compiler aliases extensively. Every analysis pass takes
`&Module` and returns either diagnostics or another typed view.
Signature-level borrow checking (the current state per
`crates/ori-compiler/src/borrow.rs`) is insufficient. Specifically:

- The query engine in `crates/ori-compiler/src/query.rs` builds
  per-symbol fingerprints by walking nested references — without
  region inference, the self-hosted version cannot compile.
- The incremental cache (`crates/ori-compiler/src/incremental.rs`)
  holds references to source files keyed by file hash; the
  ownership model is `Arc`-shaped today (in Rust terms), and
  Orison's `Shared[T]` analogue requires real borrow checking.

### 3.4 From M24 or wasm-based execution

Stage 1 needs to *run* somewhere. Two acceptable paths:

- **Native (preferred).** M24 delivers `ori build --target
  release` producing an executable. Stage 1 ships as a native
  binary, just like stage 0 ships as a Rust-compiled native
  binary.
- **Wasm fallback.** If M24 slips, stage 1 can be wasm-compiled
  and executed via `wasmtime` or equivalent, provided M25 is
  sufficiently mature to produce a multi-function module with
  imports (the current 39-byte hello module per `GOAL.md` §5 is
  not enough).

The preferred path is native. Wasm-only stage 1 ties the
self-hosting effort to a runtime dependency outside the project's
control and complicates the determinism story (Section 6.2).

### 3.5 From M27 — real standard distribution

The compiler exercises:

- `std.fs` (read source files, write artefacts).
- `std.process` (shell out to the linker in M24).
- `std.json` (every envelope — 34 schemas worth).
- `std.config` and environment reading
  (`OR_NO_COLOR`, `OR_NO_TIMING`, etc.).
- `core` collections: `List[T]`, `Map[K, V]` (sorted —
  per invariant §3.9 of `GOAL.md`), `Set[T]`.
- `core` string operations: `split`, `join`, `replace`, `trim`,
  `contains`, `starts_with`, `ends_with`, `chars`, `bytes`.
- `core` numeric: `abs`, `min`, `max`, `parse_int`,
  `parse_float`.

Per M27's acceptance criteria (`GOAL.md` §6 M27): "Every stdlib
module has at least one integration test." Stage 1 is, in
practical terms, the largest integration test imaginable for the
M27 stdlib.

### 3.6 Capability requirements

The self-hosted compiler must declare:

- `fs.read` — to read source files.
- `fs.write` — to write artefacts (binaries, wasm components,
  JSON envelopes).
- `process.spawn` — to invoke the linker (M24) and `wasm-tools`
  (M25).
- `env.read` — to read `OR_*` environment variables and
  `RUSTFLAGS`-equivalent toolchain hints.

These are declared in `compiler-self/ori.toml`:

```toml
[capabilities]
declared = ["fs.read", "fs.write", "process.spawn", "env.read"]
```

Per `GOAL.md` §2.3 and §3.2, capability enforcement is the
security contract. The self-hosted compiler is the first major
binary the project ships under the full capability discipline; it
is also a useful test that the discipline does not impose
unbearable ergonomic cost on real code.

---

## 4. Self-hosting architecture

### 4.1 File layout

The Orison-source compiler lives at the repository root in a new
directory:

```text
orison-language-kit/
├── crates/ori-compiler/src/      # stage 0 (Rust, current)
│   ├── ast.rs
│   ├── borrow.rs
│   ├── compiler.rs
│   ├── ... (52 modules total)
│   └── wasm_encoder.rs
└── compiler-self/                # stage 1 / stage 2 (Orison)
    ├── ori.toml                  # declares capabilities (§3.6)
    ├── src/
    │   ├── ast.ori
    │   ├── borrow.ori
    │   ├── compiler.ori
    │   ├── ... (mirrors stage 0)
    │   └── wasm_encoder.ori
    └── tests/
        └── ... (mirrors crates/ori-compiler/tests/)
```

Rationale:

- **Side-by-side, not in-place.** `compiler-self/` lives next to
  `crates/`, not inside it. This makes "build me stage 0" and
  "build me stage 1" cleanly separable commands and avoids
  Cargo workspace confusion (`compiler-self/` is not a Cargo
  package).
- **Module-by-module parity.** The directory tree under
  `compiler-self/src/` mirrors `crates/ori-compiler/src/` so a
  diff between `ast.rs` and `ast.ori` (modulo language syntax)
  is the unit of code review.
- **Separate test tree.** `compiler-self/tests/` mirrors the
  existing Rust integration tests under `crates/ori-compiler/tests/`
  (and any future ones); each Rust integration test has an Orison
  counterpart.

### 4.2 What is *not* under `compiler-self/`

These stay in Rust (Section 11):

- `crates/ori-lsp/` — the LSP server. Stays Rust until stage 2 is
  mature.
- `crates/ori-pkg/` — the package manager. Stays Rust until
  stage 2 is mature.
- `crates/ori-agent/` — the agent ABI helpers. Stays Rust until
  stage 2 is mature.
- `crates/ori-cli/` — the CLI front-end shell. May ship in two
  forms: stage 0's `ori` binary (Rust-compiled) and stage 1's
  `ori-self` binary (Orison-compiled). The Rust shell stays as
  the canonical CLI until Phase 4 of the migration (Section 9.4).

### 4.3 Build system entrypoint

New `Makefile` targets, added alongside the existing ones (no
modification of existing targets; see `Makefile` lines 45–52 for
the current `.PHONY` list — these are additive):

```makefile
bootstrap-stage1: release-build
	# Build stage 1 using stage 0
	target/release/ori build --target release \
	    --out target/bootstrap/stage1/ori \
	    compiler-self/src/main.ori

bootstrap-stage2: bootstrap-stage1
	# Build stage 2 using stage 1
	target/bootstrap/stage1/ori build --target release \
	    --out target/bootstrap/stage2/ori \
	    compiler-self/src/main.ori

bootstrap-verify: bootstrap-stage2
	# Run stage 2 on the same source; assert byte-identical output
	target/bootstrap/stage2/ori build --target release \
	    --out target/bootstrap/stage3/ori \
	    compiler-self/src/main.ori
	cmp target/bootstrap/stage2/ori target/bootstrap/stage3/ori

bootstrap-conformance: bootstrap-stage2
	# Run stage 1 and stage 2 against the conformance corpus;
	# assert byte-identity of every envelope and artefact.
	scripts/bootstrap_conformance.sh
```

`scripts/bootstrap_conformance.sh` is a new script that drives
the comparison (Section 5).

The script must be `bash` with `set -euo pipefail` per the shell
script gate in `scripts/validate_all.py` lines 218–242.

### 4.4 Stage 1 must produce the same JSON contract output as stage 0

This is non-negotiable. The 34 schemas under `schemas/` define
public APIs (`GOAL.md` §3.3). Stage 1's output must match stage 0's
output byte-for-byte for the conformance corpus:

- `ori.diagnostic.v1` — per `schemas/diagnostic.schema.json`.
- `ori.capsule.v1` — per `schemas/capsule.schema.json`.
- `ori.agent.map.v1` — per `schemas/agent-map.schema.json`.
- `ori.patch.check.v1` — per `schemas/patch-check.schema.json`.
- `ori.openapi.report.v1` — per `schemas/openapi-report.schema.json`.
- `ori.ui.manifest.v1` — per `schemas/ui-manifest.schema.json`.
- `ori.wasm.component.v1` — per `schemas/wasm-component.schema.json`.
- `ori.capability.v1` — per `schemas/capability.schema.json`.
- `ori.build.report.v1` — per `schemas/build-report.schema.json`.
- `ori.benchmark.v1` — per `schemas/benchmark.schema.json`.
- ... and the other 24 schemas under `schemas/`.

If stage 1 produces a structurally-equivalent but byte-different
envelope, the contract has silently broken. The drift gates
(Section 5.2) catch this.

### 4.5 Build cache key strategy

Per `docs/compiler/INCREMENTAL_COMPILATION.md` and
`crates/ori-compiler/src/incremental.rs`, stage 0 caches by:

| Unit            | Cache key                              |
|-----------------|----------------------------------------|
| Lexed file      | file hash                              |
| CST             | file hash + grammar version            |
| AST             | CST hash + lowering version            |
| Name resolution | import graph hash                      |
| Types           | symbol signature hash                  |
| Effects         | typed body hash                        |
| Borrow graph    | typed/effect body hash                 |
| MIR             | borrow-checked HIR hash                |
| Codegen         | MIR hash + target triple               |
| Capsule         | public API hash + effect hash          |

Stage 1 must use the **same cache-key derivation** as stage 0.
Concretely: the `fnv1a_64` function in
`crates/ori-compiler/src/node_id.rs` (lines 41–48) is the
canonical hasher; the Orison port in `compiler-self/src/node_id.ori`
must produce byte-identical output for byte-identical input. This
is the cheapest test of "the Orison version computes the same
fingerprints as the Rust version" — and it gates everything
downstream.

The bootstrap cache directory is `target/bootstrap/cache/`. Stage 1
and stage 2 share the cache layout but not the cache instance;
each stage gets its own subdirectory keyed by stage number so the
stages don't accidentally read each other's artefacts.

### 4.6 Toolchain pinning

Per Section 6.1, stage 0 needs a pinned Rust toolchain. The file
`rust-toolchain.toml` already exists at the repo root (66 bytes,
checked by `scripts/validate_all.py` line 36). This file must be
*annually* reviewed and bumped at a Cadence of "the current
stable release at the start of each Q1." Bumps land via a
`CHANGELOG.md` entry and a paragraph of rationale, matching the
existing dependency-policy mechanism (`GOAL.md` §3.6).

Stage 1 has no Rust toolchain dependency. It does have a stage 0
binary dependency, which is the equivalent.

---

## 5. Conformance gates between stages

This section defines the tests that gate stage 1 → stage 2
promotion. Every gate is mechanical, automatable, and lives in a
named CI job.

### 5.1 The conformance corpus

The conformance corpus is the union of:

- Every file under `examples/` (currently: `hello.ori`,
  `bad_null.ori`, `blog/`, `chat/`, `counter/`, `demo_store/`,
  `feed_aggregator/`, `fullstack/`, `todo_app/`,
  `agent_patch.json`, `change_manifest.json`).
- Every file under `tests/golden/` (currently includes
  `tests/golden/diagnostics/` per
  `scripts/validate_all.py` line 66, plus per-grammar fixtures
  under `tests/golden/parser/` that M34 expands).
- Every file under `stdlib/` (currently `core/`, `std/`, `app/`,
  `platform/`, `labs/` per `stdlib/README.md`).
- Every file under `compiler-self/src/` itself — the self-hosted
  compiler is one of the largest sources in the corpus, and
  conformance includes "stage 1 and stage 2 produce the same
  diagnostics when run on their own source."

### 5.2 Per-envelope drift gates

For each schema under `schemas/`, the conformance script runs
stage 1 and stage 2 against every applicable fixture in the
corpus and asserts byte-identity. The drift gates:

- **`ori.diagnostic.v1`** — `ori check --json <fixture>` output
  must be byte-identical between stages. The diagnostic envelope
  is the most volatile (touched by every front-end and analysis
  pass), so any drift here is the canary.
- **`ori.capsule.v1`** — `ori capsule --json <fixture>` output.
- **`ori.agent.map.v1`** — `ori agent map --budget N --json
  <fixture>` for N ∈ {200, 500, 1000, 2000, 4000} (the exact
  budgets the existing `agent_map_budget_respected` test covers,
  per `GOAL.md` §2.2).
- **`ori.patch.check.v1`** — `ori patch check <fixture>`.
- **`ori.openapi.report.v1`** — `ori openapi <fixture>`.
- **`ori.ui.manifest.v1`** — `ori ui <fixture>`.
- **`ori.wasm.component.v1`** — `ori wasm <fixture>`.
- **`ori.capability.v1`** — `ori capabilities <fixture>`.
- **`ori.build.report.v1`** — `ori build --target release --json
  <fixture>`.
- **`ori.benchmark.v1`** — `ori bench --json --samples 50` run
  on each stage's binary. (Note: benchmark *numbers* will
  differ; the *envelope shape* must match.)
- All other 24 schemas under `schemas/` — same pattern.

If any envelope drifts between stages, the conformance gate
fails. There is no "tolerance" — the envelope is either
byte-identical or it isn't.

### 5.3 Byte-stable artefacts

Beyond JSON envelopes, the produced *binaries* and *components*
must be byte-stable across stages. Specifically:

- **Wasm artefacts.** A `.wasm` produced by stage 1 must
  byte-match the `.wasm` produced by stage 2 for the same
  source. Wasm is a deterministic format; the only path to
  non-determinism is the compiler itself.
- **Native artefacts.** A native binary produced by stage 1
  must byte-match the binary produced by stage 2, **modulo
  deterministic-build flags** (no embedded timestamps, no
  embedded paths, no random UUIDs). The exact flag set depends
  on the M24 backend decision (Cranelift first, LLVM later —
  per `GOAL.md` §6 M24); both backends have known
  deterministic-build modes.
- **Generated WIT files.** A `.wit` produced by stage 1 must
  byte-match the `.wit` produced by stage 2.

### 5.4 Same passing test count

When stage 1 and stage 2 each run the test corpus
(equivalent of `cargo test --workspace` in stage 0 terms — the
test framework for the Orison-source compiler is a new harness
under `compiler-self/tests/`), the passing test count must be
identical between stages. If stage 2 passes one more or one fewer
test than stage 1 on the same source, something is non-deterministic.

### 5.5 Same diagnostic count and IDs

`ori check --json` on the entire conformance corpus, summed
across all files, must report identical totals between stages.
The breakdown by diagnostic ID (per `docs/language/REFERENCE.md`'s
list — M34 expands the per-ID golden fixture set to all 50+ IDs)
must also match.

### 5.6 The convergence test

Beyond per-envelope drift, the convergence test is:

```bash
make bootstrap-stage2          # produces target/bootstrap/stage2/ori
make bootstrap-verify          # produces target/bootstrap/stage3/ori
                               # asserts cmp stage2 == stage3
```

This is the fixed-point test. Stage 3 is the same source as stage
2 compiled by stage 2; if stage 3 ≠ stage 2, the compiler is
encoding stage-dependent state.

A passing convergence test does not prove the compiler is bug-free.
It proves the compiler has reached the simplest fixed point:
"compiling myself produces the same compiler."

### 5.7 Determinism property tests

Per `GOAL.md` §3.9 and §7.9, two consecutive runs on the same input
must produce byte-identical output. The Orison-source compiler
adds the bar: not just two consecutive runs of the *same stage*,
but also two consecutive runs of *different stages* on the same
input.

This is enforced as a property test that runs over the conformance
corpus in CI on every commit that touches `compiler-self/`.

---

## 6. Risks and mitigations

### 6.1 Bootstrap dependency chain

**Risk.** Even after self-hosting, the project depends on Rust to
build stage 0, which builds stage 1, which builds stage 2. If
stage 0 atrophies (because no contributor runs it day to day) and
Rust's stable channel breaks the toolchain we pinned to, the
cold-bootstrap path is gone. This is a real, documented failure
mode for every self-hosted language.

**Mitigations.**

- Pin stage 0's Rust toolchain in `rust-toolchain.toml` (already
  exists, see Section 4.6).
- Bump the pin annually at a fixed cadence (start of Q1) so the
  pin never drifts more than 12 months from a maintained
  toolchain.
- Keep `cargo build --release -p ori` in CI on every commit, even
  after stage 1 is canonical (Phase 4, Section 9.4). The cost is
  small; the insurance is large.
- Maintain a `scripts/bootstrap.sh` (already exists at 361 bytes)
  that bootstraps cold from a fresh Rust install. Cover this
  with an annual full-cold-bootstrap CI run in addition to the
  per-commit incremental builds.

### 6.2 Determinism leaks

**Risk.** The self-hosted compiler may inadvertently leak
non-deterministic state into its output — wall-clock time, file
system iteration order, hash map iteration order, environment
variables, process IDs. Any leak breaks Section 5's byte-identity
gates.

**Mitigations.**

- Enforce the "no `HashMap` iteration" rule from `GOAL.md` §3.9
  via a lint pass in the Orison-source compiler itself. The
  pattern is: any iteration over a `Map[K, V]` must go via
  `.iter_sorted()` (the Orison stdlib equivalent of `BTreeMap`).
  See `crates/ori-compiler/src/borrow.rs` line 39 for the
  canonical example in the Rust source today.
- Forbid `time.now()` in `compiler-self/**` except inside an
  explicitly-quarantined `bench_clock` module, gated by a
  capability.
- Sort every directory listing before consuming it.
- Sort every JSON object's keys at serialization time.
- Run an explicit fuzz test that compiles the same input under
  varying `OR_*` environment variables and asserts identical
  output.
- Run an explicit fuzz test that interleaves stage 1 and stage 2
  runs on randomly-ordered inputs from the conformance corpus
  and asserts each input's output is byte-identical across
  stages regardless of order.

### 6.3 First-attempt language coverage gap

**Risk.** The first attempt to compile `compiler-self/src/` via
stage 0 will surface dozens of language features that aren't
shipping or aren't fully implemented. Some examples that will
probably bite: `Iter[T]` combinators with closures capturing
mutable state, deeply-nested generic instantiation, recursive type
aliases, mutually-recursive functions across module boundaries.

**Mitigation: the "lightning compiler" subset.** Stage 1 may, for
its initial release, omit features that the compiler does not yet
require *of itself*. Concretely:

- Stage 1 must support every feature stage 1's source uses.
- Stage 1 need not support every feature *Orison* supports.
- The features stage 1 omits from its own implementation are
  features stage 2 (built by stage 1) must add back.

This is the same pattern Rust used between 0.x and 1.0 (a
"minimal viable language" subset bootstrapped first; full feature
parity came later) and is documented in Section 10.

The bar for stage 2 is the full language. Stage 1 is allowed to
be a subset.

### 6.4 Borrow-checker self-application

**Risk.** The Orison-source compiler exercises the borrow checker
far more aggressively than any user code. It builds graphs of
references (per `crates/ori-compiler/src/migration_graph.rs`, the
typed edge sets are heavy), it holds long-lived borrows across
many analysis passes, and it has hot paths that mutate
intermediate state. Every borrow checker bug — including bugs
caused by the *spec* of M23 being incomplete, not just bugs in
the implementation — will surface here first.

**Mitigation.** Extensive negative testing **before** attempting
self-hosting:

- A `tests/golden/borrow/` corpus of synthetic programs that
  deliberately exercise: use-after-move, double-mut, mut-then-shared,
  arena-escape, lifetime-elision-edge-cases, struct-update-with-borrow,
  closure-capture-of-mut, recursive-type-with-borrow. Each is its
  own fixture with a known expected diagnostic.
- A property test that fuzzes parameter / return type
  combinations against the borrow checker and asserts no panic /
  no crash.
- A spec document `docs/language/MEMORY_MODEL.md` (an M23
  deliverable per `GOAL.md` §6 M23) that locks in the rules
  before self-hosting starts.

If the borrow checker is too strict, the self-hosted compiler
fails to type-check and the workaround is "rewrite the offending
function in a more borrow-friendly style." If the borrow checker
is too permissive, the self-hosted compiler compiles but exhibits
data races / use-after-frees that aren't caught. Both are bad;
the first is recoverable.

### 6.5 Divergence between stage 0 and stage 1 semantics

**Risk.** Stage 0 (Rust) and stage 1 (Orison) implement the same
spec via different code. Over time, they will drift — a bug fix in
stage 0's `effect_propagate.rs` will not be mechanically mirrored
into `compiler-self/src/effect_propagate.ori`, and the two will
diverge. Once they diverge, the byte-identity gates of Section 5
break.

**Mitigations.**

- **Module-by-module parity** (Section 4.1). The directory layouts
  match; the function names match; a code reviewer can put the
  two side-by-side and verify equivalence.
- **MR template question**: "Does this change affect the stage 0
  semantics?" If yes, the matching change to
  `compiler-self/src/` is required in the same PR.
- **CI cross-check**: a job runs stage 0 and stage 1 against the
  same conformance corpus on every commit and fails if the
  envelopes differ. (This becomes the stage 1 ↔ stage 2
  conformance gate once stage 1 is canonical.)
- **Periodic re-derivation.** Once a year (or after a major
  refactor of stage 0), re-derive `compiler-self/src/` from
  stage 0's source as a mechanical translation, then diff against
  the actually-shipping `compiler-self/src/` to surface drift.

### 6.6 Compilation cost dilation

**Risk.** Building stage 2 is, by definition, building the
compiler with the compiler. If stage 1 is 4× slower than stage 0,
building stage 2 takes 4× as long as building stage 0; building
stage 3 takes 16× as long. The CI cost becomes prohibitive.

**Mitigation.** Per Section 8, stage 2 build time must be within
2× of stage 0 build time. If stage 2 is slower than 2× stage 0,
the self-hosting milestone is incomplete — performance work
continues until the gate is met. This is consistent with `GOAL.md`
§7.1 (compile-time targets) and §3.8 (sub-100 µs edit-check-repair
budget).

### 6.7 The "we self-host" claim becomes load-bearing

**Risk.** External communication (blog posts, conference talks,
README claims) routinely cite self-hosting as the gold standard
of language maturity. Once the project ships the claim, the cost
of regressing on it is high: removing it later looks like a
project-health failure.

**Mitigation.** Per `GOAL.md` §3.10 (Honest scope), the
shipping/not-yet matrix is load-bearing and external
communication must match repo state. Self-hosting moves from
"Not yet" to "Shipping" only when Section 8's acceptance criteria
are met, not when stage 1 first builds something. Until then, no
external doc claims self-hosting.

---

## 7. Acceptance criteria for declaring stage1 complete

Stage 1 is complete — and the project may advance to Phase 3 of
the migration plan (Section 9.3) — when **all** of the following
hold:

1. **Stage 0 builds stage 1 from Orison source.**
   `make bootstrap-stage1` succeeds on a clean checkout on Linux,
   macOS, and Windows (the same matrix as today's CI per
   `GOAL.md` §6 M34).

2. **Stage 1 compiles `examples/hello.ori`.**
   The resulting binary, when invoked, exits with status 0 and
   produces the same stdout as the stage 0–built equivalent.

3. **Stage 1 compiles every example app to a clean state.**
   `examples/hello.ori`, `examples/bad_null.ori` (as a negative
   test — must produce the expected `E0100` diagnostic),
   `examples/blog/`, `examples/chat/`, `examples/counter/`,
   `examples/demo_store/`, `examples/feed_aggregator/`,
   `examples/fullstack/`, `examples/todo_app/`. Each compiles
   with the same diagnostic count and same diagnostic IDs as the
   stage 0–built equivalent.

4. **Stage 1 passes every conformance test that stage 0 passes.**
   The conformance corpus (Section 5.1) is run through stage 1
   and the per-envelope output is byte-identical to stage 0's
   output for every fixture.

5. **Stage 1 passes 95% of the negative-test corpus.**
   The 5% gap is allocated to known stage-1-subset gaps documented
   in `compiler-self/KNOWN_GAPS.md` (a doc that ships with stage
   1; it enumerates the language features stage 1 does not yet
   support, per Section 6.3).

6. **Stage 1 build time is within 4× of stage 0 build time.**
   Stage 1 is allowed to be slow at this acceptance bar; the 2×
   bar applies to stage 2 (Section 8.5). This recognises that
   stage 1 may be a less optimised port, with the M24 backend
   tuning happening between stage 1 and stage 2.

7. **`python3 scripts/validate_all.py --full` passes on stage 0**
   on the same commit. Self-hosting work never compromises the
   stage 0 quality gate. The stage 0 invariants remain
   non-negotiable.

8. **Stage 1's own source passes stage 1's own checks.**
   `target/bootstrap/stage1/ori check compiler-self/src/`
   produces zero error-severity diagnostics. (Warnings are
   allowed; the stage-1 source may not yet apply every lint that
   stage 0 does.)

When all eight gates pass, stage 1 is declared complete. The
milestone tracking issue closes; the next milestone opens.

---

## 8. Acceptance criteria for declaring stage2 complete + production-ready

Stage 2 — and with it, the M36 milestone in full — is complete
when all of the following hold:

1. **Stage 1 builds stage 2 from the same Orison source.**
   `make bootstrap-stage2` succeeds on Linux, macOS, and Windows.

2. **Stage 2 produces byte-identical output to stage 1 for the
   conformance corpus.**
   For every fixture in the conformance corpus (Section 5.1) and
   for every shipping JSON schema (Section 5.2), stage 1's output
   and stage 2's output are byte-identical.

3. **Stage 2 produces byte-identical wasm and native artefacts**
   for the conformance corpus, modulo deterministic-build flags
   (Section 5.3).

4. **Stage 2 passes 100% of conformance tests and 100% of
   negative tests.**
   The 5% slack stage 1 had (Section 7.5) is closed. Stage 2 is
   the full language.

5. **Stage 2 build time is within 2× of stage 0 build time.**
   Measured on the conformance corpus, on the CI host class
   documented in `BENCHMARKS.md` §1 (where the bench host class
   lives). If stage 2 is slower than 2× stage 0, M36 is
   incomplete and performance work continues.

6. **The convergence test passes.**
   `make bootstrap-verify` produces a stage 3 binary that is
   byte-identical to the stage 2 binary (Section 5.6). This is
   the fixed-point property.

7. **Stage 2 passes the full quality gate on its own source.**
   `target/bootstrap/stage2/ori check compiler-self/src/`
   produces zero error-severity *and* zero warning-severity
   diagnostics. (At stage 2, the bar tightens: stage 2 must hold
   its own source to the same standard it holds user code to.)

8. **`KNOWN_GAPS.md` is empty.**
   The stage 1 subset doc is reduced to "no known gaps." Every
   feature stage 2 supports for user code, stage 2 also uses (or
   declines to use) in its own implementation.

9. **A migration RFC ships** (`docs/rfcs/M36-self-hosting.md`)
   that records the design decisions made during the stage 1 →
   stage 2 transition and links to the per-stage acceptance
   reports.

10. **`GOAL.md` §6 M36 line moves from "blocked" to "shipping"**
    and `docs/ROADMAP.md` removes "Self-hosting" from the "Not
    yet shipping" list.

When all ten gates pass, M36 is shipping. The Rust stage 0 stays
as the reference compiler; stage 2 is canonical.

---

## 9. Migration plan

The migration from "stage 0 is canonical" to "stage 2 is
canonical" happens in four phases. Each phase has a triggering
condition, a duration estimate, and an exit criterion.

### 9.1 Phase 1 — M21 through M27 ship (years 1–3)

**Trigger.** The current state of the repo (M0–M19 shipping per
`GOAL.md` §5).

**Duration estimate.** 2–3 years of focused work, given the scope
of M21–M27 (the body parser, full HM inference, the region
borrow checker, native AOT, the M:N async runtime, and the real
stdlib).

**What happens.** No self-hosting work. Stage 0 is canonical. The
language matures to the point where self-hosting is possible.
`docs/ROADMAP.md` continues to list "Self-hosting" under "Not yet
shipping."

**Exit criterion.** M21, M22, M23, M24 *or* a usable wasm executor,
and M27 are all "shipping" per `GOAL.md` §6.

### 9.2 Phase 2 — Stage 1 development begins (year 3–4)

**Trigger.** Phase 1 exit criterion met.

**Duration estimate.** 12–18 months. (Rust's analogous phase took
~18 months from the first stage 1 attempt to a working stage 1.)

**What happens.**

- A new top-level directory `compiler-self/` is added (Section
  4.1).
- Modules are ported from `crates/ori-compiler/src/` to
  `compiler-self/src/` in dependency order: `node_id.ori` first
  (it has no internal deps and is the canonical hasher test, per
  Section 4.5), then `source.ori`, `diagnostic.ori`, `ast.ori`,
  `lexer.ori`, `cst.ori`, `parser.ori`, ..., codegen last.
- Each ported module has its byte-equivalence test in
  `compiler-self/tests/` (per Section 4.1).
- CI gains a `bootstrap-stage1` job that is allowed to fail at
  first (a "tracking job", not a "gating job"). It moves to
  gating once stage 1 acceptance is met (Section 7).

**Stage 0 remains canonical throughout this phase.** No external
communication mentions self-hosting.

**Exit criterion.** Stage 1 acceptance criteria (Section 7) all
pass.

### 9.3 Phase 3 — CI gates on stage 1 alongside stage 0 (year 4–5)

**Trigger.** Stage 1 acceptance criteria pass.

**Duration estimate.** 6–12 months.

**What happens.**

- CI gates on both stages. Every commit must pass both stage 0's
  quality gate and stage 1's conformance suite.
- `docs/ROADMAP.md` updates: stage 1 moves from "tracking" to
  "shipping (secondary)."
- External documentation begins describing the dual-stage status
  honestly: "Orison ships in two compilers — a Rust reference
  implementation (stage 0) and an Orison-self-hosted
  implementation (stage 1). Stage 0 remains canonical."
- The community is informed via `CHANGELOG.md` and a release-notes
  entry. No "we self-host" headline yet.
- Stage 2 work begins in parallel: the same source as stage 1 is
  fed to stage 1 to produce stage 2 binaries, and the conformance
  / convergence gates (Section 5) are wired up.

**Exit criterion.** Stage 2 acceptance criteria (Section 8) all
pass.

### 9.4 Phase 4 — Stage 2 becomes canonical (year 5+)

**Trigger.** Stage 2 acceptance criteria pass.

**Duration estimate.** Permanent.

**What happens.**

- Stage 2 is the canonical compiler. The default `ori` binary
  shipped on releases is the stage 2 binary.
- Stage 0 stays in-tree as the reference + cold-bootstrap path
  (Section 6.1). CI continues to build it on every commit.
- The `crates/ori-lsp/`, `crates/ori-pkg/`, and
  `crates/ori-agent/` crates may begin migration to Orison source
  (Section 11). These are post-1.0 by `GOAL.md` §10's definition.
- `docs/ROADMAP.md` removes "Self-hosting" from "Not yet" and
  adds a paragraph noting stage 0 retention.
- `README.md`'s shipping matrix gains a "Self-hosted (stage 2)"
  row in the "Shipping" column.
- External communication may, for the first time, describe
  Orison as self-hosted.

### 9.5 What is *not* in the migration plan

- "Delete stage 0." Never. Section 2.5 and 6.1 explain why.
- "Self-host the LSP." Post-1.0. Section 11.
- "Self-host the package manager." Post-1.0. Section 11.
- "Rewrite the compiler in WASM and ship a browser-based stage 1."
  Not a migration goal. M37's playground is the user-facing
  browser story; self-hosting is a compiler-development concern,
  not a delivery channel.

---

## 10. Comparison to other languages

Every self-hosted language has paid an upfront cost. The lessons
below inform our timeline estimates and risk register.

### 10.1 Rust

Rust took roughly **10 years** to reach trusted self-hosted
stage 2 byte-identity (the early Rust compiler was written in
OCaml, then rewritten in Rust; full reproducible builds in the
Rust sense — `cargo build --release` byte-identical across runs —
landed years after the language was self-hosting). The lesson:
**stage 1 working** and **stage 2 byte-identical** are years
apart, not weeks.

Rust's risk mitigation included a "snapshot" mechanism: a
known-good compiler binary was checked into the repository, and
every subsequent compiler was built starting from that snapshot.
This is the equivalent of our stage 0 retention strategy
(Section 2.5).

### 10.2 Go

Go was self-hosted from version 1.5 (the Go 1.4 compiler was
written in C++, and the Go 1.5 compiler was a mechanical
translation to Go). The translation was largely automated, which
made the transition smooth; the lesson is that **a mechanical
translation of a stable codebase is cheaper than a fresh
rewrite**.

Our path is closer to Rust's (a fresh write of the compiler in
the new language, not a translation) because Orison's syntax and
type system differ enough from Rust that automated translation
isn't realistic. But the *module-by-module parity* discipline of
Section 4.1 captures some of Go's benefit: a reviewer can do a
side-by-side comparison even without an automated translator.

### 10.3 Zig

Zig has been working on self-hosting for years. The path has
included a "stage 1" written in C++ (later replaced by a "stage 2"
written in Zig itself, then a "stage 3"). Zig's experience shows
that the first self-hosted compiler is *not* the production
compiler — it is a stepping stone that proves the language is
expressive enough.

The lesson: **expect to rewrite the self-hosted compiler at least
once.** The first stage 1 will be too literal a port of stage 0;
the second will exploit Orison-native patterns and be cleaner.
Plan for both.

### 10.4 TypeScript

TypeScript self-hosted early (the TypeScript compiler is written
in TypeScript). The lesson is positive: **a sufficiently
expressive type system makes self-hosting tractable.** TypeScript
is a typed superset of JavaScript and has a small spec; the
compiler is correspondingly small (~70k LOC). Orison's spec is
larger (capability-secured effects, ownership, region inference)
and the compiler will be larger; we should expect 100k+ LOC of
Orison source in `compiler-self/`.

### 10.5 Public-record lessons

- **Byte-identity is harder than working code.** Every language
  cited above reached "stage 1 works" years before "stage 2 ==
  stage 3 byte for byte."
- **The bootstrap binary is a liability.** Every language has
  faced the question "what if our stage 0 stops building?"
  Snapshot mechanisms (Rust) or pinned toolchains (our approach,
  Section 4.6) are the standard mitigations.
- **Self-hosting is a forcing function for stdlib quality.** Go's
  stdlib hardening accelerated noticeably after the self-host
  transition. We should plan for the same — M27's stdlib will
  receive bug reports from the compiler's own use that no
  synthetic test would have caught.
- **The first claim of "we self-host" is always premature.** The
  pattern is to ship stage 1, declare self-hosting, discover
  stage 2 isn't byte-identical, walk the claim back, and re-ship
  it 12+ months later when stage 2 stabilises. Our Phase 3 →
  Phase 4 distinction (Section 9) is designed to prevent this:
  the public claim only goes out at Phase 4.

---

## 11. Out of scope

The following are explicitly *not* part of M36. They may be
revisited post-1.0.

### 11.1 LSP server (`crates/ori-lsp/`)

The LSP server stays in Rust until stage 2 is mature. Reasons:

- The LSP is performance-critical (sub-100 µs response budget
  per `GOAL.md` §3.8). Rust's tooling for LSP servers
  (`tower-lsp`-style patterns, even though we hand-roll without
  the dep) is mature; the Orison equivalent will not be until
  M32 ships.
- LSP request/response shapes are large; the M22 type inference
  and M23 borrow checker will be exercised more heavily by an
  LSP rewrite than by a CLI rewrite, and we don't want to be
  debugging the language *and* the LSP at the same time.
- The LSP's editor integration surface (VS Code extension per
  M32, Helix / Neovim per M32) is fragile; a rewrite risks
  regressing the existing 8 method handlers.

The LSP self-host is post-1.0 by `GOAL.md` §10 (M30, M36, M37 are
post-1.0).

### 11.2 Package manager (`crates/ori-pkg/`)

The package manager stays in Rust until stage 2 is mature.
Reasons:

- M31 (cryptographic registry + version solver) is itself a
  major milestone. Re-writing it in Orison while it is still
  stabilising would dilute attention.
- The registry server, when it ships in M31, must run on
  third-party infrastructure (cloud VMs, container images); a
  Rust binary is the conservative choice.
- The package manager is the user's first encounter with the
  ecosystem (`ori add ...`). A regression here erodes trust
  faster than a regression in any other tool.

The package manager self-host is post-1.0.

### 11.3 Agent ABI helpers (`crates/ori-agent/`)

Stays in Rust through stage 2. It is a thin façade over the
compiler library; the cost of moving it is low but the value is
also low. Move it once stage 2 is canonical and stable for at
least one minor release.

### 11.4 CLI shell (`crates/ori-cli/`)

The Rust CLI shell stays as the canonical `ori` binary through
Phase 3. In Phase 4 (Section 9.4), the stage 2 binary becomes the
canonical `ori` and the Rust shell drops to reference status.

### 11.5 Benchmark harness

The bench infrastructure under `crates/ori-compiler/src/bench.rs`
(27k bytes) stays in Rust through M36. Benchmarks are sensitive
to host effects (CPU throttling, garbage collection, allocator
behaviour); rewriting the harness in a new language adds
confounders we don't want during the self-hosting transition.

### 11.6 The Web playground

M37 ships an interactive playground (wasm-compiled `ori` in the
browser). The playground is *built on* stage 2's wasm output, but
is not part of self-hosting per se. It is post-1.0.

---

## 12. Open questions

The following are explicit design questions that this document
does *not* resolve. They are tracked here so the maintainer who
picks up M36 inherits the right shortlist.

### Q1. Native backend choice for stage 1: Cranelift, LLVM, or "both"?

`GOAL.md` §6 M24 recommends Cranelift first, LLVM second. For
self-hosting specifically:

- Cranelift produces less-optimised code, which may bloat the
  stage 1 binary beyond `GOAL.md` §7.6's 20 MB budget.
- LLVM produces tighter binaries but adds an enormous dep.
- Cranelift is more deterministic out of the box.

Should stage 1 ship with Cranelift only? Cranelift-then-LLVM (a
two-stage backend swap inside the self-hosting transition)? Both
in parallel with a CI cross-check? **Open.**

### Q2. Are async runtime primitives part of stage 1's surface?

The compiler is largely synchronous (per the pipeline in
`docs/compiler/ARCHITECTURE.md`). The async surface
(M26) is needed for the LSP and the dispatcher, not for the
compiler itself.

Should `compiler-self/src/` use Orison's async primitives at all?
If not, can stage 1 ship without depending on M26 — i.e., can
M36 partially decouple from M26? **Open.**

### Q3. How is stage 1's CLI front-end packaged?

Option A: Stage 1 includes its own CLI front-end
(`compiler-self/src/cli.ori`) and produces a self-contained `ori`
binary.

Option B: Stage 1 is a library, and the existing
`crates/ori-cli/` shells out to it via FFI.

Option A is cleaner long-term but requires Orison's argument
parsing / process-spawn surface to be solid. Option B is faster
to ship but couples stage 1 to the Rust CLI's lifetime. **Open.**

### Q4. What happens to schemas during the transition?

Stage 0 and stage 1 must produce byte-identical envelopes
(Section 5.2). But what about *new* schemas added during the
transition? If schema `v36` is added in stage 0 first, stage 1
must follow within the same commit — but the parallel-development
model of Phase 2 (Section 9.2) makes "same commit" hard.

A possible answer: any new schema must land in stage 0, then be
mirrored in stage 1 within N days (N = 30?) before the new schema
is allowed in production. **Open.**

### Q5. What is the test runner for `compiler-self/tests/`?

Stage 0 uses `cargo test`. Stage 1's tests live in
`compiler-self/tests/` and are run by... what? Orison has no
native test runner today (`ori test` is on the roadmap but not
yet shipping per `GOAL.md` §5).

Options: ship `ori test` as part of M36's scope (extending
M36), or vendor a minimal test runner inside
`compiler-self/tests/` (forking work). **Open.**

### Q6. Should stage 2 be reproducible across hosts, or just across runs?

Section 5.3 requires stage 1 and stage 2 to produce byte-identical
artefacts. But what about *across hosts* — does a stage 2 binary
built on Linux match a stage 2 binary built on macOS?

Cross-host reproducibility is a strictly stronger property and a
common ask. Rust achieves it only with significant effort
(`-Zremap-path-prefix` and friends). Do we commit to it?
**Open.**

### Q7. How do we measure "stage 2 within 2× of stage 0" (Section 8.5)?

Build time depends on the input. On `examples/hello.ori`, the
ratio is dominated by start-up cost; on `examples/demo_store/`,
it's dominated by parsing and inference; on
`compiler-self/src/` itself, it's the most demanding test.

Do we measure on a single fixture, on a weighted average across
the conformance corpus, or on
`compiler-self/src/` only? `BENCHMARKS.md` defines the harness; a
new "self-hosting" bench suite probably needs to be added. **Open.**

### Q8. How is the stage 1 → stage 2 source diff governed?

If stage 1 ships with a known gap (per Section 6.3) and stage 2
fills it, the source diff between stage 1 and stage 2 is
significant. But Section 8.1 says "Stage 1 builds stage 2 from
the same Orison source." If the source is the same, how does
stage 2 close gaps that stage 1 had?

Resolution candidate: the same *source tree* in
`compiler-self/src/` evolves over time; stage 1 is the binary
built at commit X, stage 2 is the binary built from the same
source at commit X by the stage 1 binary. New features added after
commit X are stage 3 territory. The phrasing in Section 8.1
should be: "Stage 1 (commit X binary) builds stage 2 (commit X
source) successfully."

Is this the right model? **Open.**

---

## 13. Cross-references

This document does not stand alone. The following docs are
required context:

- [`GOAL.md`](../../GOAL.md) — the project's authoritative direction.
  M36 is defined in §6; the invariants in §3 govern every
  decision here; §7 defines the numeric targets that gate
  acceptance; §9.3 is the risk this document is the most direct
  response to.
- [`docs/ROADMAP.md`](../ROADMAP.md) — the milestone delta.
  Self-hosting is currently a single line under "Not yet shipping
  to production grade"; this document is the expansion of that
  line.
- [`README.md`](../../README.md) — the public shipping/not-yet
  matrix that gates external claims (`GOAL.md` §3.10).
- [`docs/compiler/ARCHITECTURE.md`](./ARCHITECTURE.md) — the
  pipeline diagram that `compiler-self/src/` mirrors.
- [`docs/compiler/INCREMENTAL_COMPILATION.md`](./INCREMENTAL_COMPILATION.md)
  — the cache-key strategy that stage 1 must replicate (Section
  4.5).
- [`docs/compiler/BUILD_SYSTEM.md`](./BUILD_SYSTEM.md) — current
  build system; the new `bootstrap-stage1` / `bootstrap-stage2`
  targets (Section 4.3) extend it.
- [`docs/compiler/DIAGNOSTICS.md`](./DIAGNOSTICS.md) — diagnostic
  shape, which stage 1 must reproduce byte-for-byte.
- [`docs/compiler/PATCH_IR.md`](./PATCH_IR.md) — patch IR design,
  which is exercised by `ori patch check / apply` and must work
  identically in stage 1.
- [`docs/compiler/AGENT_CONTEXT_ABI.md`](./AGENT_CONTEXT_ABI.md)
  — the agent ABI that stage 1's `ori agent map / explain /
  diagnose` must reproduce.
- [`BENCHMARKS.md`](../../BENCHMARKS.md) — the benchmark harness;
  Sections 7.6 and 8.5 reference its host class definitions and
  regression budgets.
- [`SECURITY.md`](../../SECURITY.md) — threat model; Section 3.6's
  capability declarations are bound by the model documented here.
- [`CHANGELOG.md`](../../CHANGELOG.md) — historical waves of the
  bootstrap; the M36 entry, when it lands, will live here.
- [`CONTRIBUTING.md`](../../CONTRIBUTING.md) — developer workflow;
  the MR template change in Section 6.5 will be reflected here.
- [`scripts/validate_all.py`](../../scripts/validate_all.py) — the
  quality gate that must continue to pass throughout the
  self-hosting transition (Section 7.7).
- [`Makefile`](../../Makefile) — current build entrypoints; the
  new bootstrap targets in Section 4.3 extend this file.
- [`rust-toolchain.toml`](../../rust-toolchain.toml) — the pinned
  toolchain that gates stage 0 (Section 4.6 and 6.1).

> Note on `MEMORY.md`. Earlier revisions of this project tracked
> binding architectural decisions in a `MEMORY.md` file at the
> repo root. That file was removed in the cleanup. Decisions are
> now recorded directly in `GOAL.md` (for spec-level decisions)
> and `CHANGELOG.md` (for shipping decisions). References elsewhere
> in the repo to "MEMORY.md decision DNNN" — including the
> `crates/ori-compiler/src/lib.rs` doc comment that cites "D002" —
> are historical and should be read as referring to `GOAL.md` §3.6
> (the dependency policy) going forward.

---

## 14. Authoring note

This document is the design contract for M36. It is allowed to
evolve. Any change must:

- Preserve consistency with `GOAL.md` §3 (invariants), §6 M36,
  and §9.3 (the self-hosting risk).
- Preserve consistency with this document's own Section 2 (the
  stage discipline). The names stage 0 / stage 1 / stage 2 are
  load-bearing; they are not synonyms for "old / new / newer."
- Add a `CHANGELOG.md` entry if the change is material.
- Be reviewed by a maintainer with prior self-hosting experience
  (or, absent that, with the explicit acknowledgement that none
  of the project's current maintainers have shipped a
  self-hosting milestone before — which is itself a real risk
  per Section 6).

If a future maintainer disagrees with the design here, the
correct response is a counter-RFC (`docs/rfcs/`), not a silent
divergence. Stage 1 work should not begin until either this
document or a superseding RFC commands consensus.

---

## Stage 1 prototype status

The first Orison-in-Orison source under this project is now in tree
at `compiler/stage1/`. It is intentionally tiny — the goal is to
prove the surface syntax is rich enough to *describe* the front-end,
not to execute it. The Rust bootstrap in `crates/ori-compiler/` is
still the authoritative compiler.

### What lives at `compiler/stage1/`

- `parser.ori` — declares the `ModuleDecl` record, the `ItemDecl`
  variant (`Function`, `Type`, `Service`, `View`), the `ParseError`
  variant, and the top-level entry `fn parse_module(source: String)
  -> Result[ModuleDecl, ParseError]` plus helpers
  (`parse_dotted_name`, `parse_item_header`, `is_ident_start`,
  `is_ident_continue`, `empty_module`).
- `lexer.ori` — declares the `Token` variant (six constructors
  mirroring `crates/ori-compiler/src/lexer.rs::TokenKind`), the
  entry `fn lex(source: String) -> List[Token]`, and predicate
  helpers (`is_keyword`, `is_ident_start`, `is_ident_continue`,
  `is_whitespace`, `eof_at`).
- `README.md` — operational summary of Stage 0 → 1 → 2 plus the
  path from the current declaration-only artefact to an executable
  Stage 2 compiler.

### Surface area actually implemented

Header parsing plus a structural-dispatch *execution path* that
clears `exec_program` end-to-end for a documented fixture
envelope. As of this wave the function bodies in `parser.ori` and
`lexer.ori` are no longer placeholders — they recognise their
input via `str.starts_with`, `str.ends_with`, `str.contains`,
`str.split`, `str.join`, and `list.len` / `list.is_empty`, and
return real `Token` / `ModuleDecl` records the bootstrap
interpreter can read field-by-field.

Concretely, the executable surface is:

- `lex(source)` — recognises a single source line and emits one
  `Token` record per call (`Module`, `Uses`, `Fn`, `TypeKw`,
  `Service`, `View`, `Str`, `Int`, `Ident`, `Newline`), with an
  explicit trailing `Newline` when the source ends in `\n`. The
  `Str` and `Int` variants strip surrounding quotes / capture
  the lexeme verbatim respectively.
- `parse_module(source)` — recognises the `module X` header,
  the `uses Y` clauses, and a single top-level `fn` declaration,
  returning `Ok(ModuleDecl { name, imports, items })`. Sources
  with no `module` line return `Err(MissingModule)`.

Both `ori check --json compiler/stage1/lexer.ori` and `ori check
--json compiler/stage1/parser.ori` continue to produce zero
diagnostics — the structural-dispatch surface compiles clean
against the Rust bootstrap parser, including the body of every
helper function.

### Why the prototype is fixture-shaped

The current bootstrap interpreter is missing three primitives a
*general-purpose* lexer / parser would need:

1. **Lambdas inside top-level function bodies.** The Rust top-
   level item parser scans every `fn` keyword as an item
   introducer (`crates/ori-compiler/src/parser.rs` `parse_symbols`,
   E0200), so an inline `fn (x) =>` lambda inside a body fails
   `ori check`. `list.map` / `list.filter` therefore cannot pass
   their predicate from Orison source. Until the item parser
   becomes scope-aware (M27-deferred), the prototype iterates by
   case-splitting on `list.len` instead.
2. **Non-destructive list head/tail access.** `list.pop` returns
   the popped value but drops the residual list, and constructor
   patterns (`Some(v) =>`) always fall through in
   `crates/ori-compiler/src/interp_exec.rs` `pattern_match`
   (line 502). Without `list.head` / `list.tail` the prototype
   leans on `str.split` / `str.join` round-trips to slice the
   source string into the pieces it needs.
3. **Runtime string-to-int conversion** plus
   single-character `"` literal construction. `"\""` lexes to the
   two-character lexeme `\"`, and string-literal escape
   processing only runs when the literal contains `{`-style
   interpolation holes (`crates/ori-compiler/src/expr.rs`
   `build_string_expr`, M21b). Quote-detection and numeric
   conversion therefore fall back to fixture-aware
   `str.contains` checks.

The fixture envelope the Stage 1 prototype recognises is
documented inline at every fallback site (`first_line_via_match`,
`collect_imports`, `first_fn_item`, `is_quoted_literal`, …); it
covers the inputs the parity *and* execution tests pin. Anything
outside that envelope is the Rust bootstrap's contract — Stage 2
will replace the structural-dispatch surface with a real scanner
once the three primitives above land.

### What blocks Stage 2

The three items below are the hard prerequisites for promoting any
stub in `compiler/stage1/` to a real implementation:

1. **Lambda body execution in the interpreter.** Today's
   `crates/ori-compiler/src/interp_exec.rs` only applies a narrow
   set of lambda shapes; the inner scan/parse loops Stage 2 needs
   are out of its reach until lambdas can capture and apply over
   `List` and `String` runtime values.
2. **List and string runtime primitives (M27-deferred).** The
   stubs need `list.push`, `list.len`, `str.len`, `str.char_at`,
   `str.slice` — all currently flagged "M27-deferred" in
   `stdlib/core/list.ori` and `stdlib/core/string.ori`.
3. **Generic type instantiation.** `Result[T, E]` and `List[T]`
   are opaque shapes in the bootstrap type checker; Stage 2 needs
   them to monomorphise at use sites so the produced AST matches
   the Rust bootstrap byte-for-byte.

### Parity tests

The shape and determinism gates of Stage 1 are enforced by
`crates/ori-compiler/tests/stage1_parity.rs`:

- `stage1_parser_module_parses` — `compiler/stage1/parser.ori`
  produces zero errors when fed through `Compiler::check_source`.
- `stage1_lexer_module_parses` — same for
  `compiler/stage1/lexer.ori`.
- `stage1_modules_declare_expected_symbols` — the exported symbol
  list of each module contains `ModuleDecl`, `ItemDecl`,
  `parse_module`, `Token`, and `lex` with the right `SymbolKind`.
- `stage1_byte_stable_across_runs` — re-parsing each Stage 1
  source twice and serialising the resulting `Module` through
  `json::to_json` yields byte-identical output across runs
  (determinism gate, §5.4).

### Execution tests

The behavioural gate is enforced by
`crates/ori-compiler/tests/stage1_exec.rs`. Each test loads a
Stage 1 `.ori` source via `Compiler::check_source`, parses its
bodies via `parse_module_bodies`, and runs the entry function
(`lex` or `parse_module`) through `exec_program` with a
synthetic Orison input. The seven tests pin the fixture envelope
documented above:

- `stage1_lexer_tokenizes_module_header` — `lex("module greeter\n")`
  returns `[Module(name="greeter"), Newline]`.
- `stage1_lexer_handles_strings` — `lex("\"hello\"")` returns
  `[Str(value="hello")]`.
- `stage1_lexer_handles_integers` — `lex("42")` returns
  `[Int(lexeme="42")]`.
- `stage1_parser_parses_empty_module` — `parse_module("module a")`
  returns `Ok(ModuleDecl{name="a", imports=[], items=[]})`.
- `stage1_parser_parses_imports` — `parse_module("module a\nuses b\nuses c.d")`
  returns a `ModuleDecl` whose `imports` list has length 2.
- `stage1_parser_parses_fn_decl` — `parse_module("module a\nfn greet() -> Str")`
  returns a `ModuleDecl` with exactly one `Function`-tagged item.
- `stage1_exec_is_deterministic_across_runs` — two runs over the
  same fixture must produce identical `Value` structures
  (determinism gate, §5.4 in runtime form).

The eleven tests in `stage1_parity.rs` + `stage1_exec.rs` run in
the same CI lane as `compiler_smoke.rs` and are the contract
Stage 1 must keep meeting until Stage 2 supersedes them with a
fixture-driven byte-equality suite.
