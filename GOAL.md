# Orison Goal v2 — From Bootstrap to a New, Unique Programming Language

> **Document role.** This is the authoritative end-to-end specification of
> what Orison must become to ship as a first-class production programming
> language. It supersedes any earlier `GOAL.md`. Every claim is grounded in
> what is shipping today (verifiable via `cargo test --workspace` =
> 476 passing / 0 failing, `python3 scripts/validate_all.py --full` =
> validation passed) and in what remains, with explicit acceptance criteria
> per remaining milestone.

---

## Table of contents

1. [Mission](#1-mission)
2. [What makes Orison unique](#2-what-makes-orison-unique)
3. [Non-negotiable invariants](#3-non-negotiable-invariants-preserved-forever)
4. [Product wedges, ordered](#4-product-wedges-ordered)
5. [Where we are today (honest baseline)](#5-where-we-are-today-honest-baseline)
6. [Remaining milestones M21–M51](#6-remaining-milestones-m21m51)
7. [Success metrics](#7-success-metrics-numerical-targets)
8. [Non-goals](#8-non-goals-explicit)
9. [Risks](#9-risks)
10. [Definition of "production ready"](#10-definition-of-production-ready)
11. [Cross-references](#11-cross-references)

---

## 1. Mission

Build a memory-safe, statically typed, compiled full-stack application
language whose **edit-check-repair loop is materially cheaper than any
existing language for both humans and AI agents** — measured in
wall-clock latency, in context-token consumption, in iteration count to
a passing test, and in regressions per merged change.

The wedge is the loop. Every design decision asks: *does this make the
next edit faster, safer, or cheaper to attempt?* If the answer is no
across all three axes, the decision is wrong.

---

## 2. What makes Orison unique

Orison is **not "Rust with simpler syntax"** and not "another web
framework wearing a language costume." Five design propositions make it
distinct from every shipping language as of 2026. Each is already
expressed in the bootstrap and must be preserved as the system matures.

### 2.1 Structural Patch IR with stable node identifiers

Every other language ships *text* as the canonical change unit (diffs,
patches, edits). Orison ships **stable structural node IDs** that an
agent (or refactoring tool) targets by identity, not by line number.
Today's bootstrap already encodes IDs as
`node:<module>.<kind>.<name>.<discriminant>` derived from a salted
FNV-1a fingerprint of `(parent_id, kind, name, sibling_index,
signature)`. They survive whitespace edits, comment changes, and
unrelated edits to other parts of the same file.

This unlocks workflows no text-based language can match:
- An LLM emits `{"op": "insert_match_arm", "target": "<id>", ...}` and
  the compiler accepts/rejects it in **~1.5 µs at p50** before disk is
  touched.
- Partial-apply semantics let cross-file patches succeed for the ops
  whose targets are live and skip-with-`P1010` for the stale ones —
  not "merge conflict, abandon."
- The same `id` round-trips through capsules, agent maps, symbol
  cards, doctor reports, and code-action `data` fields, so every
  upstream consumer points at the same object.

**Bootstrap status:** shipping. `ori patch check / apply / dry-run /
explain` all live, with 17 tests in `patch.rs` + `patch_apply.rs`.

### 2.2 Compiler-native agent context budgeting

`ori agent map --budget N --json <file>` returns at most N bytes of
schema-versioned symbol-table JSON, truncated deterministically.
Verified across budgets 200 / 500 / 1000 / 2000 / 4000 — and `truncated`
flips at the right threshold every time. No competing language has
this as a first-class CLI operation; it's the difference between an
agent burning tokens reading whole files and an agent loading exactly
the surface it needs.

The same pattern is shipping for `ori docs --format agent --budget
1500` (budgeted markdown) and `ori capsule --json` (per-module
semantic summary). The contract is: the compiler is the source of
truth for "what matters about this module in N tokens."

**Bootstrap status:** shipping. Budget enforcement is a regression
test in `crates/ori-cli/tests/cli_smoke.rs::agent_map_budget_respected`.

### 2.3 Capability-secured effects as the package-level contract

Every other language treats effects as either invisible (Python,
Go, TypeScript) or fully type-system-encoded (Haskell, Koka, Effekt).
Orison takes a third path: **effects are first-class on functions, and
the package boundary is the capability contract**.

```toml
# ori.toml
[capabilities]
declared = ["http", "db.read", "db.write"]
```

```ori
fn post_checkout(cart: Cart) -> Result[Order, CheckoutError] uses http, db.write
```

The compiler enforces both layers statically: any function transitively
calling a `db.write` callee without declaring `db.write` fails to
compile (`E0420` with a `change_signature` Patch IR fix); any
declared capability not actually used by any symbol surfaces as an
`AUD0002` info finding; any used capability not in the package policy
fails the audit with `AUD0001` error severity.

**Bootstrap status:** shipping for static enforcement.
Runtime gating is **M48** on this roadmap.

### 2.4 Universal JSON envelope per CLI command

Every Orison CLI command emits a JSON object conforming to a
schema-versioned contract under `schemas/`. **34 schemas ship today.**
`ori doctor` enumerates the full list. The contract is: any agent or
tool that consumes Orison output never has to grep, parse, or guess.

This is not "tooling" — it's a language-level guarantee. The schema
contract is a public API and follows semantic versioning
(`ori.diagnostic.v1` will never silently change shape; `v2` is a new
contract). The bootstrap already enforces this via:
- A `doctor_report_lists_every_shipped_schema` drift test.
- `schemas/*.schema.json` validated to Draft 2020-12 by the static gate.
- A 19-case conformance suite round-tripping live CLI output against
  committed golden snapshots.

No other shipping production language treats "every CLI command's
output is a versioned wire contract" as a first-class commitment.

### 2.5 Full-stack from one source, with effects gating each target

Orison is the only language designed from day one to compile the
**same source** to native binary, WebAssembly component, web bundle,
and mobile native — with the compiler-emitted manifest at each target
gating which capabilities the deployable receives. A function whose
effects include `auth` cannot mount in a context that hasn't requested
the `auth` capability; a view whose props reference a record from
`demo_store.domain` is type-checked against the same record the API
and database modules use; a mobile build's `permissions` array is
derived deterministically from the `uses` clauses in the source.

Today the manifests ship; the actual codegen for native + wasm
multi-function + mobile native UI is staged across M24 / M25 / M32 /
M33.

---

## 3. Non-negotiable invariants (preserved forever)

These hold across every future change. Any PR that breaks one of them
must either restore the invariant or be rejected; they are not subject
to convenience.

### 3.1 No null. No exceptions.

`null` → `E0100`. `throw` → `E0101`. Absence is `Option[T]`. Failure
is `Result[T, E]`. There is no plan to ever relax this; it predates
every other language design decision.

### 3.2 No ambient capabilities

A function or package that wants to read the filesystem must declare
`fs.read`. Implicit `open()` is not a feature. The capability lattice
is the security model; if a capability ever gains an implicit
acquisition path, the security model is broken.

### 3.3 Schemas are public APIs

A shipped `schemas/*.schema.json` file is a permanent contract. We
add `v2` next to `v1`, we do not edit `v1` in place. Field removals
require a `MEMORY.md`-equivalent decision plus a deprecation period.

### 3.4 Anything the compiler knows is available to tools through
stable structured output

Diagnostics, capsules, patches, agent maps, symbol cards, manifests,
capabilities, SBOMs, audits, provenance, benchmarks, doctor, run
reports, build reports — all are JSON-serialised typed structs, never
hand-built strings. New compiler knowledge gets a schema before it
gets a feature.

### 3.5 No `.unwrap()`, `.expect()`, `panic!`, `todo!`, `unimplemented!`,
`dbg!`, or `unsafe` in `crates/*/src/**/*.rs`

Enforced by `scripts/validate_all.py`. Tests included. The only
sanctioned escape hatch for tests is `assert!(false, "msg")` with
`#[allow(clippy::assertions_on_constants)]`.

### 3.6 Dependency policy is `serde + serde_json` only (in the bootstrap)

Adding a third-party Rust dependency requires a `CHANGELOG.md` entry
and a paragraph of rationale. Native AOT codegen (M24) will require
the first exception (LLVM or Cranelift); that exception must be
explicitly documented as the first crack and not the precedent for
subsequent additions.

### 3.7 Every diagnostic is actionable

Every diagnostic carries: stable ID, severity, message, span,
expected/found vectors where applicable, `docs:` reference, and an
`agent_summary` line. Anything less is a regression.

### 3.8 The edit-check-repair loop is sub-100 µs at p50

Today: warm check ~22 µs, patch validation ~1.5 µs, patch apply
~20 µs, capsule generation ~31 µs. Composite ≈ 77 µs at p50. If a
PR raises any of these above the 10% regression band documented in
`BENCHMARKS.md` §8, it must either restore the budget or carry an
explicit decision.

### 3.9 Determinism over expressivity

Two consecutive runs on the same input produce byte-identical output
for every shipping JSON envelope. Wherever a choice exists between a
more expressive abstraction and a more deterministic one, choose
deterministic. `BTreeMap` not `HashMap`. Sorted vectors not ordered
sets. No timestamps in golden output. No wall-clock in
identifier-derivation.

### 3.10 Honest scope

The matrix in `README.md` distinguishing "Shipping" from "Not yet" is
load-bearing. No external communication may describe a "Not yet"
subsystem as working. Every external doc must reflect the actual
state of the repo at the commit that ships the doc.

---

## 4. Product wedges, ordered

Orison serves five wedge use cases. They are explicitly ordered: the
first wedge is the one we will *not* compromise to expand the second.

### Wedge 1 — Agent-native full-stack web/backend

The bootstrap is purpose-built for this. The acceptance test is the
`examples/demo_store/` storefront, which already exercises every
shipping subsystem. M21–M27 on this roadmap complete this wedge to
production grade.

### Wedge 2 — WebAssembly-target backend services

Once the wasm component encoder produces real multi-function modules
(M25), Orison's "effects → capabilities → permissions" pipeline gives
it a uniquely tight story for serverless / edge deploy targets.

### Wedge 3 — Cross-platform full-stack mobile

After M32–M33 land the native UI bindings, the same `examples/blog/`
source compiles to iOS, Android, and web simultaneously, with the
mobile manifest's permission list derived from `uses` clauses
automatically.

### Wedge 4 — Native CLI / systems-adjacent tools

Native AOT codegen (M24) opens this. The ownership model + capability
discipline give it a Rust-class safety story without Rust's
ergonomics tax.

### Wedge 5 — Embedded / GPU / specialised compute

Explicitly **deferred** to platform layers, never the primary wedge.
The `stdlib/labs/` layer is where these incubate.

---

## 5. Where we are today (honest baseline)

Shipping, tested, schema-versioned:

- 5 crates, ~32k LOC, 476 tests passing, 0 failing.
- 34 JSON contract schemas.
- 30+ CLI subcommands.
- 27 stdlib modules (declarations only — no runtime implementations).
- 6 example apps.
- Item parser + body parser (literals, vars, calls, blocks, if, match,
  return, try, lambda, record, tuple, construct).
- Multi-module resolver, signature-level type checker, expression-level
  type inference for the body parser's expression set.
- Effect propagation through call graph (`E0420` with Patch IR fix).
- Borrow checker prototype (B0010–B0050, signature-level).
- Exhaustive match check (`E0540` with `insert_match_arm` fix).
- Const folding pass.
- HIR/MIR scaffolds + tree-walking executing interpreter (R0001–R0005).
- Cooperative async scheduler (single-threaded, A0001–A0003).
- Hand-rolled wasm bytecode encoder (39-byte validating hello-module).
- Textual LLVM-IR-style codegen scaffold.
- Patch IR validation + apply + dry-run + explain.
- CST-preserving formatter (idempotent).
- OpenAPI 3.1 extraction, UI manifest, design-token enforcement,
  mobile manifest, wasm component manifest, capability manifest.
- SQL query shape check + migration toposort.
- Package manager: manifest, lockfile, SBOM, audit, provenance,
  local registry stub (publish/fetch/list/yank).
- GraphQL SDL importer + gRPC proto3 importer.
- LSP server: hover, completion, rename, code actions, workspace
  symbols, document symbols, definition, references.
- Docs generator (human + agent-budgeted) + edition migration tool.
- Safe macro preprocessor (allow-list-gated `${ENV}` / `@orison/X`).
- Coverage estimator, query engine (per-symbol fingerprints),
  incremental cache.
- Benchmark harness with 32 suites / 40 metrics.

Not yet shipping (this document is the roadmap for closing each gap):

- Binary operators, string interpolation, raw strings, multiline strings.
- Full bidirectional type inference inside arbitrary expression bodies.
- Generic instantiation by usage, protocol resolution, default impls,
  coherence, associated types.
- Region-inference borrow checker.
- Real native AOT codegen + optimisation passes + linker.
- Real wasm multi-function modules + memory + imports + WIT.
- M:N async runtime + async I/O + work-stealing scheduler.
- Real stdlib implementations (everything currently a declaration).
- Real backend dispatcher, middleware system, auth/session runtime.
- Real UI render pipeline.
- Real mobile native UI bindings (UIKit, Jetpack Compose).
- Real desktop UI bindings.
- Cryptographic registry signing (Sigstore / GPG).
- Distributed registry protocol (HTTPS).
- SAT-style version solver.
- Build script sandboxing.
- LSP semantic tokens, inlay hints, code lens, refactorings beyond rename.
- Debug Adapter Protocol implementation.
- TreeSitter grammar.
- Model-in-the-loop benchmark harness.
- Self-hosting.

The remainder of this document is the milestone plan for the "not yet"
list.

---

## 6. Remaining milestones M21–M51

Earlier milestones (M0–M19) are documented in
`docs/ROADMAP.md` and the historical waves in `CHANGELOG.md`. The
milestones here extend that plan.

Each milestone has:
- **Why** — what wedge or invariant it serves.
- **Deliverables** — concrete artefacts.
- **Acceptance criteria** — testable when "done."
- **Order constraint** — what must land before it.

### M21 — Complete the body parser

**Why.** Without binary operators and string interpolation, body-level
type inference (M22) cannot do useful work, the interpreter (M27) can
only run literal-return programs, and codegen (M24) has nothing to
lower beyond `return Unit`.

**Deliverables.**
- Binary operators: `+`, `-`, `*`, `/`, `%`, `==`, `!=`, `<`, `<=`,
  `>`, `>=`, `&&`, `||`, `??` (null-coalesce on Option).
- Unary operators: `-`, `!`.
- Method-call syntax `recv.method(args)` parsing into
  `Expr::MethodCall { recv, name, args }`.
- String interpolation `"hello {name}"` parsing into
  `Expr::InterpString { parts: Vec<StringPart> }`.
- Raw strings `r"..."` and `r#"..."#` for embedded quotes.
- Multiline strings with leading-whitespace stripping.
- Numeric literals with `_` separators, hex `0x`, binary `0b`, octal `0o`.
- Doc comments `///` parsed as `Stmt::DocComment` attached to the
  next item.
- Trailing commas in argument lists, record literals, variant
  payloads, type generic lists.

**Acceptance criteria.**
- Every grammar production above has at least one golden fixture under
  `tests/golden/body/`.
- The body parser test count grows from 11 to ≥ 25.
- `cargo test --workspace` = green; no regression in the existing 476.
- Parsing `examples/demo_store/src/cart.ori` (currently signature-only)
  recovers all function bodies through the new parser.

**Order.** Blocks M22, M27, M24.

### M22 — Full bidirectional type inference (Hindley-Milner + effects)

**Why.** Wedge 1 requires that an agent can write
`fn handle(req: Request) -> Response: return req |> validate |> route |> render`
and have the compiler infer every intermediate type. Today the type
checker only validates signatures.

**Deliverables.**
- Bidirectional inference: each expression is checked against an
  expected type (synthesis or inference mode).
- Unification with occurs check.
- Let polymorphism (generalise after let bindings).
- Generic function instantiation by use site.
- Method resolution against protocols.
- Type narrowing in pattern match arms.
- Refine `Option` / `Result` through `if let`, `match`, and `?`.
- Effect inference: a function's effect set is the union of its body
  callees' effects plus its own primitive effects; the declared
  `uses` clause is checked against the inferred set.
- Effect inference produces the same `E0420` and Patch IR fix the
  current call-graph propagator does, but driven from the body parser
  instead of the call-graph approximation.
- Type errors carry `expected` vs `found` with the inferred types
  rendered in surface syntax.

**Acceptance criteria.**
- Inference round-trips every function in `examples/demo_store/`.
- A `tests/golden/inference/` directory with ≥ 20 fixtures, one per
  inference rule, each with the expected post-inference type
  annotation as JSON.
- Conformance tests assert that running `ori check` on the demo
  produces zero new diagnostics versus today.
- A bench suite `type_infer_bodies_latency` lands with p50 < 50 µs
  on the medium fixture (currently TBD in `BENCHMARKS.md` §5.3).

**Order.** Blocked by M21. Blocks M27, M30.

### M23 — Region-inference borrow checker

**Why.** The current borrow checker (B0010–B0050) is signature-level
only. To match Rust-class memory safety, it must reason about
expression-body flows: move after use, mutable borrow exclusivity at
the use site, drop semantics at scope exit.

**Deliverables.**
- Move analysis on `let` bindings, function returns, struct
  construction, variant payloads.
- Mutable borrow exclusivity at use sites (B0060).
- Use-after-move detection (B0070).
- Drop ordering at block exit (B0080).
- Lifetime parameters on functions and types (`fn first<'a>(xs: &'a List[T]) -> &'a T`).
- Region inference for the common case (no explicit lifetimes needed
  for paths that don't escape).
- Safe-wrapper contracts for `Shared[T]` / `Weak[T]` enforced at the
  borrow checker, not just at signature.

**Acceptance criteria.**
- Negative test corpus per new diagnostic ID.
- A specification document `docs/language/MEMORY_MODEL.md` is updated
  with the formal rules.
- The demo storefront compiles without any borrow errors.
- A synthetic test fixture deliberately exercising
  use-after-move / double-mut / arena-escape produces the expected
  diagnostic for each rule.

**Order.** Blocked by M21, M22.

### M24 — Real native AOT codegen

**Why.** Wedge 4 (native CLI / systems tools) requires it. The
current `ori build --target llvm-text` emits a textual scaffold but
no executable.

**Deliverables.**
- A real native backend. **First decision point:** LLVM via `inkwell`,
  or Cranelift via `cranelift-codegen`. Recommendation: Cranelift
  first (smaller dep, faster compile, simpler ABI), LLVM as M24.5
  for production optimisation.
- This is the **first sanctioned exception** to the bootstrap
  dependency policy (`MEMORY.md` D002). The decision lands as a
  `MEMORY.md` D017 entry.
- MIR → native lowering for the body parser's expression set.
- Optimisation passes: constant prop, DCE, simple inlining.
- Register allocation (Cranelift handles this).
- Linker integration (lld or system ld).
- ELF (Linux), Mach-O (macOS), PE (Windows) outputs.
- `ori build --target release <file>` produces an executable binary.

**Acceptance criteria.**
- `ori build --target release examples/hello.ori` produces a binary
  that exits 0.
- `ori build --target release examples/demo_store/src/main.ori` produces
  a binary < 5 MB.
- An `examples/native_hello/` example app demonstrating the workflow.
- `cargo test --workspace` green on Linux, macOS, Windows in CI.
- ABI stability test: a binary built with compiler version N can be
  re-linked against a library built with version N+1 within the same
  minor series.

**Order.** Blocked by M21, M22. The biggest single milestone on this
roadmap.

### M25 — Real wasm component v1

**Why.** Wedge 2 requires multi-function wasm modules with imports,
memory, and a WIT interface contract — not just a 39-byte hello.

**Deliverables.**
- MIR → wasm lowering for the full body parser expression set.
- Memory section (1 page initial, growable).
- Function table for indirect calls.
- Imports section (host imports for stdlib hooks).
- Component model wrapping (preview2 compatible).
- WIT (`*.wit`) generation from Orison source — every public symbol
  emits a corresponding WIT type/function.
- `ori build --target wasm-component <file>` produces a `.wasm` that
  passes `wasm-validate` and a `.wit` that passes the standard component
  validator.

**Acceptance criteria.**
- `examples/demo_store/src/api.ori` compiles to a multi-function wasm
  component < 100 KB.
- The emitted `.wit` matches a committed golden fixture.
- Round-trip: `ori build → wasm-tools validate → wasmtime run` exits
  with the expected return value.
- Performance: wasm-execution of `fn fib(n: Int) -> Int` matches
  hand-written wasm within 2×.

**Order.** Blocked by M21, M22.

### M26 — M:N async runtime

**Why.** Wedge 1 (web/backend) and Wedge 2 (serverless) need real
concurrency. The current cooperative scheduler runs single-threaded
in-process; it cannot drive a request-handling loop.

**Deliverables.**
- Work-stealing scheduler (Tokio-style, but designed natively).
- Per-thread local task queues + global queue.
- Async I/O via epoll (Linux), kqueue (macOS), IOCP (Windows).
- `await` keyword wired through HIR/MIR to suspend frames.
- Cancellation tokens.
- Bounded + unbounded channels (`std.channels`).
- `Mutex[T]`, `RwLock[T]`, atomics.
- Stack management for spawned tasks.
- Panic isolation (a panicking task does not abort the runtime).
- `select!` / `join!` / `try_join!` combinators.

**Acceptance criteria.**
- A new `examples/async_chat/` app demonstrates 1k concurrent
  connections over websocket.
- A bench suite `async_io_throughput` reports ≥ 100k requests/sec on
  commodity hardware for the trivial echo server.
- Cross-platform CI matrix (Linux, macOS, Windows) all green.
- No deadlocks in a 24-hour soak test of the demo storefront under
  synthetic load.

**Order.** Blocked by M22, M24. Required for M30.

### M27 — Real standard distribution v1

**Why.** Today, every `stdlib/*.ori` module is a declaration. Wedges
1–4 all require actual implementations.

**Deliverables (per layer).**

`core` (foundational, no I/O):
- Real `Option`/`Result` combinators (`map`, `and_then`, `or_else`,
  `unwrap_or`, `is_some`/`is_none`/`is_ok`/`is_err`).
- Real `Iter[T]` with `map`, `filter`, `fold`, `take`, `skip`,
  `collect_list`, `count`, `chain`, `zip`, `enumerate`.
- Real `List[T]`, `Map[K, V]`, `Set[T]`, `Pair[A, B]`.
- Real `Str` operations (`split`, `join`, `replace`, `trim`,
  `to_lower`, `to_upper`, `contains`, `starts_with`, `ends_with`,
  `chars`, `bytes`).
- Real numeric helpers (`abs`, `min`, `max`, `clamp`, `pow`,
  `safe_div`, `safe_mul`, `parse_int`, `parse_float`).

`std` (typical production needs, hooks into runtime/OS):
- Real `std.json` (parse + serialize, ≥ 1 GB/s on a synthetic
  benchmark).
- Real `std.http` (HTTP/1.1 + HTTP/2, TLS via rustls or platform
  primitives).
- Real `std.websocket` (RFC 6455).
- Real `std.sql` (connection pool, parameterised queries, prepared
  statement cache).
- Real `std.queue` (in-memory + redis adapter).
- Real `std.mail` (SMTP, signed DKIM).
- Real `std.fs` (async file I/O).
- Real `std.process` (subprocess spawn + pipes).
- Real `std.time` (UTC + timezone via tzdb).
- Real `std.crypto` (X25519, Ed25519, ChaCha20-Poly1305, AES-GCM,
  SHA-256/512, KDF). Constant-time where it matters.
- Real `std.regex` (RE2-style, no catastrophic backtracking).
- Real `std.url` (RFC 3986).
- Real `std.config` (env + .ori-config + TOML).
- Real `std.validation` (composable validators with structured
  error paths).
- Real `std.logging` (structured, span-aware, JSON output).
- Real `std.tasks` (Future + select_first + with_timeout backed by M26
  scheduler).
- Real `std.cache` (LRU + TTL + size-bounded).

`app` (framework integration — depends on M30 and M31):
- Real `app.services` runtime — see M30.
- Real `app.views` runtime — see M31.
- Real `app.auth` — cookie + JWT + session.

`platform`:
- Real `platform.web` (DOM bindings for the wasm target).
- Real `platform.mobile` (notifications, camera, sensors).

`labs`:
- Incubating APIs without stability promise.

**Acceptance criteria.**
- Every stdlib module has at least one integration test that exercises
  the public surface against a deterministic fixture.
- API conformance tests assert backward-compatible behaviour across
  compiler versions.
- Module-level bench suites land in the harness (`std_json_parse_latency`,
  `std_http_request_latency`, etc.) with thresholds documented in
  `BENCHMARKS.md`.
- The demo storefront switches from declaration-only stdlib to real
  stdlib with zero source changes (proves the contracts shipped today
  are stable).

**Order.** Blocked by M21, M22, M24, M26.

### M28 — Real backend framework v1

**Why.** Wedge 1's acceptance test is "an Orison developer ships a
production REST API in one source tree." Today the compiler extracts
OpenAPI from `service` declarations but no request dispatcher exists.

**Deliverables.**
- Real router (radix tree, parameterised routes).
- Real request body parsing (JSON, form, multipart).
- Real response writing.
- Real middleware stack with deterministic ordering.
- Cookie + JWT + session middleware.
- CSRF protection.
- Rate limiting hooks.
- Auth policy hooks.
- Generated typed client (`ori openapi --emit-client typescript`,
  `... rust`, `... python`).
- Observability spans (OpenTelemetry export hook).
- Health-check endpoints by default.

**Acceptance criteria.**
- `examples/demo_store/src/api.ori` boots as a real HTTP server that
  serves the routes documented in its OpenAPI report.
- An end-to-end test (`crates/ori-bench/tests/e2e_demo.rs`) drives the
  server with a real HTTP client and asserts contract conformance.
- Latency: p50 < 1 ms for a no-op GET on commodity hardware.

**Order.** Blocked by M22, M26, M27.

### M29 — Real UI framework v1

**Why.** Wedge 1's full-stack promise requires the same source tree
that ships the API also ships the UI.

**Deliverables.**
- View tree IR (typed component graph).
- State management primitives (`Signal[T]`, `Computed[T]`, `Effect`).
- Event loop integration (M26 scheduler).
- Form binding + validation flows (integrates `std.validation`).
- Design token enforcement at render time (extends today's static
  `design check`).
- Accessibility audit (ARIA roles, contrast, focus order).
- HTML adapter for the web target.
- Snapshot rendering tests.
- Tree-shaking for unreferenced components.

**Acceptance criteria.**
- `examples/demo_store/src/ui.ori` boots in a browser as a real
  single-page app reading from the M28 API.
- Lighthouse-style score ≥ 90 for accessibility / performance on the
  demo.
- An `examples/counter/` app builds to a < 50 KB gzipped bundle.

**Order.** Blocked by M25, M26, M27, M28.

### M30 — Mobile + desktop targets

**Why.** Wedge 3. The bootstrap already emits the mobile manifest
correctly; the runtime + bindings are missing.

**Deliverables (mobile).**
- iOS adapter via UIKit (or SwiftUI bridge).
- Android adapter via Jetpack Compose (or View bridge).
- App store packaging (`ori build --target ios-archive`,
  `ori build --target android-aab`).
- Runtime capability gating that mirrors `mobile manifest` static gate.

**Deliverables (desktop).**
- macOS adapter (Cocoa or SwiftUI bridge).
- Windows adapter (Win32 / WinUI).
- Linux adapter (GTK or Qt — decide via RFC).
- Notarisation / code-signing integration.

**Acceptance criteria.**
- `examples/blog/` builds and launches on iOS simulator, Android
  emulator, macOS, Windows, Linux from one source tree.
- Permission denial at runtime (`fs.write` not granted) surfaces as
  the same `RuntimeError` shape on every platform.

**Order.** Blocked by M24, M25, M29.

### M31 — Cryptographic registry + version solver

**Why.** Today's lockfile checksum is FNV-1a and the registry is a
local-filesystem stub. Production-grade package management requires
real cryptographic integrity and real version resolution.

**Deliverables.**
- Cryptographic signing: Sigstore-compatible signatures + transparency
  log (Rekor-style).
- SBOM signature matching SLSA Level 3 attestation.
- HTTPS registry protocol (specified as `docs/compiler/REGISTRY_PROTO.md`).
- Real SAT-style version solver (Cargo's PubGrub or equivalent).
- Build-script sandboxing (no ambient FS / network).
- Mirror / vendor support (`ori vendor --to <dir>`).
- Workspace projects (`[workspace] members = ["a", "b"]`).
- Feature flags, optional + dev + build dependencies.
- Provenance attestations included in published artefacts.

**Acceptance criteria.**
- A reference registry server (`crates/ori-registry/`) runs the protocol.
- Published artefacts include a verifiable Sigstore signature.
- Lockfile tamper test (existing) extended to assert signature
  verification.
- Resolver tests cover the canonical PubGrub edge cases.

**Order.** Blocked by M27.

### M32 — Editor v1 (LSP completeness)

**Why.** Today's LSP ships 8 method handlers. Production editor support
needs the full spec surface.

**Deliverables.**
- `textDocument/semanticTokens/full` + `range` — full syntax highlighting.
- `textDocument/inlayHint` — inferred types, parameter names, lifetime
  hints.
- `textDocument/codeLens` — "run test", "run main", "view docs",
  "view capability impact".
- `textDocument/foldingRange`.
- `textDocument/selectionRange`.
- `textDocument/prepareRename` — pre-validates rename targets.
- `textDocument/typeDefinition`, `implementation`.
- `textDocument/callHierarchy/*`.
- Code actions beyond rename: extract function, extract variable,
  inline variable, move to module, generate match arm,
  generate test stub.
- DAP (Debug Adapter Protocol) implementation in `crates/ori-dap/`.
- VS Code extension (`extensions/vscode/`).
- TextMate grammar fallback (`extensions/textmate/orison.tmLanguage.json`).
- TreeSitter grammar (`extensions/tree-sitter/grammar.js`).

**Acceptance criteria.**
- The official VS Code extension shows: syntax highlighting, inlay
  hints, hover, completion, rename, code actions, references,
  definition, debug breakpoints.
- A second editor integration (Helix or Neovim) ships via the same
  LSP binary.
- LSP compliance tested against the official LSP spec test suite.

**Order.** Blocked by M22.

### M33 — Agent ABI v2 — Model-in-the-loop telemetry

**Why.** The wedge claim "lower context-token consumption" needs
direct measurement, not anecdote.

**Deliverables.**
- A reference agent harness in `crates/ori-bench-agent/` that drives a
  model (configurable provider) through the demo storefront tasks.
- Metrics:
  - `tokens_per_accepted_patch`
  - `patches_accepted_first_try` (fraction)
  - `regression_rate_per_patch` (fraction)
  - `iterations_to_green` (avg)
  - `tokens_per_completed_task` (avg)
  - `wall_clock_per_task`
- Task corpus: 20 canonical demo-storefront fixes (each with a Patch
  IR ground-truth and a passing-test acceptance).
- Differential diagnostics: `ori agent diagnose --since <prev-sha>`
  returns only what changed since the previous compile.
- Streaming diagnostics: an LSP-style notification stream from
  `ori agent diagnose --watch`.
- Cross-language agent ABI: client libraries in Python and TypeScript
  that round-trip the JSON envelopes.

**Acceptance criteria.**
- The harness reports the headline numbers on at least three models.
- An external committable `BENCHMARKS_AGENTS.md` ships the results.
- The numbers improve materially over a baseline of "agent reads
  whole files."

**Order.** Blocked by M22, M28. The wedge differentiator.

### M34 — Conformance + CI matrix

**Why.** Production language status requires evidence the language
does what it says everywhere.

**Deliverables.**
- A golden fixture per diagnostic ID (the list from
  `docs/language/REFERENCE.md`).
- A negative-test corpus per diagnostic.
- Per-grammar-production parser fixture under `tests/golden/parser/`.
- Per-CLI-subcommand integration test in
  `crates/ori-cli/tests/cli_smoke.rs` (current: 20 of 30+).
- CI matrix: Linux × macOS × Windows × {Rust stable, beta} × {Orison
  N-1, N, N+1 edition compat samples}.
- Cross-version ABI tests for binary + wasm outputs.
- Per-stdlib-module conformance suite.
- Benchmark regression budget enforced as a CI gate (≥ 2× p50 = block).

**Acceptance criteria.**
- All 50+ diagnostic IDs have a fixture.
- CI completes in < 30 min for the full matrix.
- Benchmark regression budgets surface as CI status checks.

**Order.** Parallel to M21–M33.

### M35 — Security v1 — runtime capability enforcement

**Why.** Today the capability model is static-only. A production
language must enforce capabilities at the runtime boundary too.

**Deliverables.**
- Capability tokens passed implicitly through the call graph; runtime
  rejects an attempted `fs.write` from a function whose call path
  doesn't carry the `fs.write` token.
- Sandbox for build scripts (no ambient FS / network, scoped temp
  directory, time limit).
- TLS verification with certificate pinning option.
- Constant-time crypto primitives in `std.crypto`.
- ASLR / PIE / stack canaries for native binaries via M24 toolchain.
- Audit log of capability acquisitions in debug builds.
- Threat-model documentation per subsystem.

**Acceptance criteria.**
- A test that attempts unauthorised `fs.write` from a `uses fs.read`
  function fails at runtime with `R0010 capability denied`.
- A build-script sandbox test asserts that a script trying to read
  outside its sandbox fails.
- TLS pinning test asserts behaviour against a deliberately-mismatched
  certificate.

**Order.** Blocked by M24, M26.

### M36 — Self-hosting (stage1 / stage2)

**Why.** A language that cannot compile itself stays dependent on its
host. Self-hosting validates the language's expressiveness and
forces the stdlib + framework to be production-quality.

**Deliverables.**
- The compiler rewritten in Orison, building via the Rust bootstrap
  (stage0) → Orison-built compiler (stage1) → stage1 compiles itself
  to produce stage2.
- Stage2 output must be byte-identical to running stage1 on the same
  source (modulo deterministic timestamps).
- The Rust bootstrap (`crates/ori-compiler`) remains in the repo as a
  reference + fallback, but ceases to be the canonical compiler.

**Acceptance criteria.**
- `make bootstrap-stage2` succeeds and produces an executable that
  passes the full conformance suite.
- Stage1 and stage2 produce identical wasm/native artefacts for the
  conformance corpus.

**Order.** Blocked by M21–M27. The capstone milestone.

### M37 — Public ecosystem

**Why.** A language without a community is a language without a
future.

**Deliverables.**
- Public package registry (Sigstore-signed).
- Interactive playground (wasm-compiled `ori` running in-browser).
- Tutorial series (beginner → advanced) under `docs/tutorial/`.
- Cookbook (idiomatic patterns) under `docs/cookbook/`.
- Migration guides from Rust, Go, TypeScript, Python under
  `docs/migration/`.
- Performance + security + framework cookbooks.
- Public CI badges + status page.
- Discord/Matrix/forum.
- Code of Conduct.
- RFC process (`docs/rfcs/`).
- Governance model (BDFL → TC → SIGs progression).
- Conference talks / blog post template.

**Acceptance criteria.**
- The playground compiles and runs hello world end-to-end in the
  browser.
- The tutorial walks a new user from `cargo install ori` to a
  deployed demo storefront in < 1 hour.
- Three external contributors merge non-trivial PRs.

**Order.** Parallel to M21–M36.

---

## 7. Success metrics (numerical targets)

These are the numbers that define "production grade." Any milestone
above is only complete when its contribution to the relevant target
is shipping and measured.

### 7.1 Compile time
- Cold check on hello: **< 50 µs** at p50. (Today: ~4 µs ✓.)
- Warm check on medium fixture: **< 50 µs** at p50. (Today: ~22 µs ✓.)
- Cold check on 100-symbol module: **< 1 ms** at p50.
- Cold check on 10k-symbol crate: **< 100 ms** at p50.
- Cold check on 1M-LOC workspace: **< 30 s** at p50.
- Incremental edit (single line, single symbol): **< 5 µs** at p50.

### 7.2 Runtime
- Native binary cold start: **< 1 ms** for hello.
- Wasm component cold start: **< 2 ms** for hello.
- HTTP request → response (no-op): **< 1 ms** at p50.
- Interpreter throughput: **≥ 100k ops/sec** for arithmetic.
- Async task spawn: **≥ 1M ops/sec**.

### 7.3 Agent ABI (the wedge)
- `agent map --budget 2000`: **≥ 95%** of relevant symbols fit on the
  demo storefront's medium fixture.
- `tokens_per_symbol_density`: **< 50 tokens** per symbol at default
  budget.
- `patches_accepted_first_try`: **≥ 70%** on the canonical 20-fix
  corpus across three model providers (M33).
- `iterations_to_green`: **≤ 2.0** average across the canonical
  corpus.
- `regression_rate_per_patch`: **≤ 5%**.

### 7.4 Memory safety
- Use-after-move detection: **100%** on the curated negative corpus.
- Double-mutable-borrow detection: **100%** on the curated negative
  corpus.
- Zero `unsafe` in `crates/*/src/**/*.rs` — already enforced by the
  unsafe_surface_report test.

### 7.5 Deployable size
- Native demo storefront binary: **< 5 MB**.
- Wasm demo storefront component: **< 100 KB** gzipped.
- Web counter app bundle: **< 50 KB** gzipped.

### 7.6 Build size
- Compiler self-hosted binary: **< 20 MB**.
- Stdlib (compiled, all layers): **< 5 MB**.

### 7.7 CI
- Full quality gate: **< 5 min**.
- Cross-platform matrix: **< 30 min**.

### 7.8 Coverage
- Workspace test coverage: **≥ 80%** line + branch.
- Every diagnostic ID has a golden fixture: **100%**.
- Every CLI subcommand has a smoke test: **100%**.

### 7.9 Determinism
- Two identical-input runs produce byte-identical output: **100%** of
  shipping JSON envelopes. (Already enforced by 3 explicit tests; to
  become a property sweep.)

---

## 8. Non-goals (explicit)

These are *deliberate exclusions*. Rejecting them is part of what
makes Orison Orison.

### 8.1 Garbage collection

The ownership model is the alternative. GC is incompatible with the
sub-100 µs edit-check-repair budget and with capability-aware
runtime gating.

### 8.2 Implicit ambient capabilities

A `Reader::open(path)` that silently asks the OS for `fs.read` will
never ship. Capabilities are declared.

### 8.3 Exceptions / unwinding control flow

`Result` is the failure shape. Stack unwinding is allowed for panics
in `unsafe`-free Rust at the bootstrap layer; user code in Orison
never raises.

### 8.4 Reflection / dynamic dispatch by default

Method resolution is static. Trait objects (dyn-style erased
dispatch) may ship as a future opt-in but never as the default.

### 8.5 Macros as Turing-complete metaprogramming

The current preprocessor (`${ENV}` / `@orison/X` substitution) is the
ceiling. We will not add procedural macros that can execute arbitrary
Rust at compile time. Compile-time evaluation is restricted to
`const fn` style pure functions over the language's own type system.

### 8.6 NPM-style microdependency ecosystem

The stdlib is intentionally large. Packages of < 100 lines that do
one thing are explicitly discouraged. The package registry will
include policy hooks for minimum-quality / minimum-maturity bands.

### 8.7 Backwards compatibility with another language's runtime

No "this compiles to JS" / "this is Python-on-the-JVM." Orison's
codegen targets are native (M24), wasm component (M25), and mobile
native (M30). The browser is reached via wasm, not via JS.

### 8.8 Single-vendor lock-in

The language specification (`docs/language/SPECIFICATION.md`), the
schemas (`schemas/*`), and the protocol contracts are public and
forkable. The bootstrap reference compiler is Apache-2.0. The package
registry protocol is open; running your own registry is a first-class
deploy.

### 8.9 Solving cross-language interop in the language itself

FFI exists (M24 native codegen exposes a C ABI). Beyond that,
cross-language integration is the job of importers (`schema import
graphql / grpc`) and adapters, not language features.

### 8.10 Self-modifying code at runtime

The Patch IR exists for build-time / agent-time changes. A running
Orison program does not patch itself in memory.

---

## 9. Risks

### 9.1 The agent wedge depends on LLM behaviour we don't control

If LLMs continue improving at reading whole files cheaply (e.g.,
1M-context becomes free), the "budget-aware capsule" wedge weakens.
**Mitigation:** lean into the *patch* and *capability* wedges, which
are model-independent.

### 9.2 Native codegen quality is years of work

LLVM/Cranelift integration is well understood but tuning it for
release-quality output (matching Rust within 2×) is real engineering.
**Mitigation:** the bootstrap textual IR stays as a fallback;
Cranelift-first gives us 80% of the value at 20% of the cost; LLVM
follows.

### 9.3 Self-hosting (M36) might never reach byte-identical stages

This has bitten other languages (Rust took ~10 years to reach
trustworthy stage2 determinism). **Mitigation:** declare a "stage1
= production" milestone separately from "stage2 = byte-identical"
and ship the former first.

### 9.4 The stdlib is enormous

M27 alone is 18 modules of real implementations. **Mitigation:** ship
per-module behind opt-in feature flags; the demo storefront only
needs the modules it imports.

### 9.5 The mobile native UI story is platform-political

Apple and Google move targets every year. **Mitigation:** target
**one** UI primitive per platform (UIKit on iOS, Compose on Android)
and treat new platform UIs as additive.

### 9.6 The community might not show up

A language without external contributors stays a research project.
**Mitigation:** M37 prioritises tutorial, playground, and migration
guides — the three things that bring outside users in.

### 9.7 Dependency-policy erosion

The first sanctioned dep (LLVM/Cranelift, M24) is the easiest. The
second is harder to justify, the third easier than the second, and so
on. **Mitigation:** every new dep requires an explicit `MEMORY.md`
decision entry and a budget impact assessment in `BENCHMARKS.md`.

---

## 10. Definition of "production ready"

Orison reaches **production-ready** status when *all* of these are true:

- [ ] M21 (body parser completion) — shipping.
- [ ] M22 (full bidirectional inference) — shipping.
- [ ] M23 (region-inference borrow checker) — shipping.
- [ ] M24 (native AOT) — shipping; demo storefront produces a < 5 MB
      binary that exits 0.
- [ ] M25 (multi-function wasm component) — shipping.
- [ ] M26 (M:N async runtime) — shipping.
- [ ] M27 (real stdlib) — at least `core` + `std.{json, http, sql,
      validation, logging, time, crypto}` shipping.
- [ ] M28 (real backend dispatcher) — shipping; demo storefront boots
      as a real HTTP server.
- [ ] M29 (real UI render pipeline) — shipping; demo storefront UI
      loads in a browser.
- [ ] M31 (cryptographic registry + version solver) — shipping.
- [ ] M32 (LSP completeness + VS Code extension) — shipping.
- [ ] M33 (agent ABI v2 + measured wedge numbers) — shipping with at
      least three model providers.
- [ ] M34 (conformance suite + cross-platform CI) — shipping; CI matrix
      green on Linux / macOS / Windows.
- [ ] M35 (runtime capability enforcement) — shipping.
- [ ] All numerical targets in §7 met or exceeded.
- [ ] At least 100 external contributors and 1k repositories in the
      public package registry.
- [ ] No `unsafe` in the bootstrap; one sanctioned dep exception (codegen).
- [ ] A 1.0 stability commitment shipping in a `STABILITY.md` document.

M30 (mobile + desktop), M36 (self-hosting), and M37 (full ecosystem
build-out) are post-1.0 milestones.

**Production-ready does not mean "feature-complete with Rust."** It
means: the wedge use cases (full-stack web/backend, wasm-target
backend, agent-native iteration) are first-class, every contract is
stable, every safety invariant is enforced at runtime as well as
statically, and the numerical targets are met.

---

## 11. Cross-references

- [`README.md`](./README.md) — public landing page; shipping/not-yet matrix.
- [`docs/ROADMAP.md`](./docs/ROADMAP.md) — milestone delta (companion to this doc).
- [`BENCHMARKS.md`](./BENCHMARKS.md) — measured performance + planned suites + regression policy.
- [`SECURITY.md`](./SECURITY.md) — threat model + capability lifecycle.
- [`CONTRIBUTING.md`](./CONTRIBUTING.md) — developer workflow.
- [`CHANGELOG.md`](./CHANGELOG.md) — historical waves of the bootstrap.
- [`docs/language/REFERENCE.md`](./docs/language/REFERENCE.md) — current language reference.
- [`docs/language/SPECIFICATION.md`](./docs/language/SPECIFICATION.md) — intended language specification.
- [`docs/compiler/ARCHITECTURE.md`](./docs/compiler/ARCHITECTURE.md) — compiler architecture.
- [`schemas/`](./schemas) — every shipping JSON contract.
- `crates/ori-cli/tests/cli_smoke.rs` — 20-case CLI conformance.
- `crates/ori-compiler/src/bench.rs` — 32 benchmark suites / 40 metrics.

---

## 12. Authoring note

This document is **the truth** about Orison's direction. If something
shipping today contradicts it, that ship is a mistake. If something on
the roadmap conflicts with §3 (Non-negotiable invariants), the
roadmap entry is the one that changes.

Any PR that materially alters the wedge ordering, the success metrics,
the non-goals, or the definition of production-ready must update this
document in the same change.
