# Orison

> A memory-safe, statically typed, compiled full-stack application language with
> a small readable syntax, capability-secured effects, JSON-first diagnostics,
> structural patching, and a compiler-native Agent Context ABI.

Orison is the language for the **edit-check-repair loop**. Whether you're a
human in an editor or an LLM agent shelling out tools, every step of the loop
is sub-100 µs at p50, every diagnostic is machine-readable, and every change
can be a structural patch instead of a whole-file rewrite.

**Status:** bootstrap + alpha. **456 passing tests, 0 failing.** The full
quality gate (`python3 scripts/validate_all.py --full`) is green. See
[`docs/ROADMAP.md`](./docs/ROADMAP.md) for the explicit delta to production
grade.

[![Tests](https://img.shields.io/badge/tests-456%20%2F%200%20failing-success)](./CONTRIBUTING.md)
[![Schemas](https://img.shields.io/badge/JSON%20schemas-34-blue)](./schemas)
[![Stdlib](https://img.shields.io/badge/stdlib%20modules-27-blue)](./stdlib)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](./LICENSE)

---

## Table of contents

- [Why Orison](#why-orison)
- [Install](#install)
- [Hello world](#hello-world)
- [Language tour](#language-tour)
  - [Modules](#modules)
  - [Types](#types) — newtypes, records, variants, Option, Result
  - [Functions and effects](#functions-and-effects)
  - [Services and routes](#services-and-routes)
  - [Views](#views) — typed UI
  - [Queries and migrations](#queries-and-migrations) — typed SQL
- [CLI tour](#cli-tour) — 30+ subcommands
- [Benchmarks](#benchmarks)
- [Developing with Orison](#developing-with-orison)
  - [Project layout](#project-layout)
  - [The edit-check-repair loop](#the-edit-check-repair-loop)
  - [Agent-native features](#agent-native-features)
  - [Editor integration](#editor-integration) — LSP
- [Standard distribution](#standard-distribution)
- [Example apps](#example-apps)
- [Architecture](#architecture)
- [Capability model](#capability-model)
- [Honest scope](#honest-scope)
- [Contributing](#contributing)
  - [Governance](#governance)
  - [Learning Orison](#learning-orison)
- [License](#license)

---

## Why Orison

Five design promises, each backed by code shipping today:

1. **Type-safe by default.** No null, no exceptions. `Option[T]` and
   `Result[T, E]` are the only nullable / failure shapes. Newtypes prevent
   silent cross-domain confusion (you can't pass a `ProductId` where an
   `OrderId` is expected). Exhaustive match checking refuses to compile a
   `match` that misses a variant arm.

2. **Capability-secured effects.** Every function declares the effects it
   needs (`uses http, db.read`); every package declares the capabilities it
   permits (`declared = ["http", "db.read"]`). The compiler enforces both
   statically and emits a `ori.capability.v1` manifest. Effects propagate
   through call chains, so a function calling a `db.write` callee without
   declaring `db.write` fails to compile (`E0420` with a Patch IR fix
   suggestion).

3. **JSON diagnostics, schemas, contracts.** Every CLI envelope conforms to a
   versioned schema under [`schemas/`](./schemas). 34 contracts ship today.
   Agents can query the compiler instead of inferring project state from raw
   text.

4. **Structural patches, not whole-file rewrites.** Patch IR
   (`ori.patch.v1`) lets agents propose changes against **stable node IDs**
   that survive whitespace and unrelated edits. `ori patch dry-run` previews
   the result before disk is touched. Partial apply: stale-target ops are
   skipped per-op (`P1010`), structural failures abort the whole patch.

5. **Sub-100 µs round trip.** Cold check ~2 µs, warm check ~20 µs, patch
   validation ~0.7 µs, patch apply ~10 µs (Apple Silicon, n=100). The whole
   edit-check-repair-cycle composite is ~77 µs at p50. See
   [Benchmarks](#benchmarks).

---

## Install

Prerequisites: **Rust 1.92** (pinned in `rust-toolchain.toml`).

```bash
git clone https://github.com/Eldergenix/Orison.git
cd Orison
./scripts/install_hooks.sh          # pre-commit + pre-push gates
cargo build --release -p ori        # builds the `ori` CLI
alias ori=$PWD/target/release/ori   # for the rest of this README
```

Verify:

```bash
ori doctor
# → {"schema":"ori.doctor.v1","status":"ok","compiler":"bootstrap",
#    "language":"Orison","version":"0.1.1","rust_toolchain":"…",
#    "checks":[…],"schema_versions":{ … 34 contracts … }}
```

---

## Hello world

```ori
// hello.ori
module hello

fn main() -> Unit:
  return Unit
```

```bash
ori check --json hello.ori          # → exit 0, no diagnostics
ori run hello.ori                   # → status: ok / entry: main / value: Unit
```

A slightly meatier hello:

```ori
module greeter

fn greet(name: Str) -> Str:
  return name

fn main() -> Unit uses log:
  return Unit
```

```bash
ori capsule --json greeter.ori | jq .
# {
#   "schema": "ori.capsule.v1",
#   "module": "greeter",
#   "exports": [
#     {"id": "sym:greeter.greet", "kind": "function",
#      "signature": "fn greet(name: Str) -> Str", "effects": []},
#     {"id": "sym:greeter.main",  "kind": "function",
#      "signature": "fn main() -> Unit uses log", "effects": ["log"]}
#   ],
#   …
# }
```

---

## Language tour

The examples below are excerpted from the canonical
[`examples/demo_store/`](./examples/demo_store) storefront app. The
authoritative language reference lives in
[`docs/language/REFERENCE.md`](./docs/language/REFERENCE.md).

### Modules

Every `.ori` file starts with `module <dotted.name>`. Imports use the same
dotted form:

```ori
module demo_store.api

import demo_store.domain
import demo_store.catalog
import demo_store.cart
```

Missing module declaration → `E0001`. Trailing dot → `E0002`. Unknown import →
`E0220`. Circular import → `E0230`.

### Types

**Newtypes** keep distinct domains from mixing:

```ori
type ProductId wraps Str
type OrderId   wraps Str
type Email     wraps Str
```

A function that takes `ProductId` can never silently accept an `OrderId`,
even though both wrap `Str` — that's `B0020` *newtype confusion*, with a
`change_signature` Patch IR fix suggested.

**Records** are nominal:

```ori
type Money = {
  currency: Str,
  amount_minor: Int
}

type Product = {
  id: ProductId,
  sku: Str,
  name: Str,
  price: Money,
  stock: Int
}
```

**Variants** with optional payloads model algebraic data:

```ori
type OrderStatus =
  | Pending
  | Paid
  | Fulfilled
  | Cancelled(reason: Str)
```

Match against a variant without covering every arm → `E0540` *non-exhaustive
match*, with an `insert_match_arm` Patch IR fix.

**Option / Result are first class.** There is no `null`, no `throw`. `null` →
`E0100`. `throw` → `E0101`. Use `None` / `Some(v)` and `Err(e)` / `Ok(v)`.

```ori
fn fetch_product(id: ProductId) -> Result[Product, CatalogError] uses db.read:
  return Err(NotFound)
```

### Functions and effects

Functions declare their **effects** with `uses`:

```ori
fn get_products() -> List[Product] uses http, db.read:
  return list_active()
```

Known effects: `fs.read`, `fs.write`, `net.inbound`, `net.outbound`,
`db.read`, `db.write`, `env.read`, `process.spawn`, `crypto`, `time`,
`random`, `ui`, `gpu`, `http`, `auth`, `mail.send`, plus user-declared
`capability` names (must start uppercase).

If a function transitively calls a callee whose effects exceed its own
declared set, the compiler fails with `E0420` and suggests the missing effect
via a Patch IR `change_signature` op.

### Services and routes

A `service` block groups routes. Each route is a function whose effects
include `http`:

```ori
service Storefront uses http, db.read, db.write

fn get_products() -> List[Product] uses http, db.read:
  return list_active()

fn get_product(id: ProductId) -> Result[Product, CatalogError] uses http, db.read:
  return fetch_product(id)

fn post_checkout(cart: Cart) -> Result[Order, CheckoutError] uses http, db.write:
  return Err(PaymentFailed)
```

The compiler emits OpenAPI 3.1 directly from the source:

```bash
ori openapi --json demo_store/src/api.ori
# {
#   "schema": "ori.openapi_report.v1",
#   "openapi_version": "3.1.0",
#   "services": ["Storefront"],
#   "routes": [
#     {"method": "GET",  "path": "/products", "response_type": "List[Product]", …},
#     {"method": "GET",  "path": "/product",  "response_type": "Result[Product, CatalogError]", …},
#     {"method": "POST", "path": "/checkout", "response_type": "Result[Order, CheckoutError]", …}
#   ]
# }
```

### Views

UI components are typed against the same record shapes the API uses:

```ori
view ProductDetail(product: Product) -> Html uses ui:
  card:
    heading(level: 2, text: product.name)
    text(product.sku)

view AdminProductEditor(product: Product) -> Html uses ui, auth:
  card:
    heading(level: 2, text: "Edit product")
    text(product.sku)
```

The `auth` effect on `AdminProductEditor` makes it impossible to mount that
view from a context that hasn't requested the `auth` capability.

`ori ui --json` extracts a UI manifest with accessibility findings
(missing `alt` on visual views, missing `submit_label` on form views, etc.).

### Queries and migrations

```ori
fn fetch_product(id: ProductId) -> Result[Product, CatalogError] uses db.read:
  return Err(NotFound)

migration add_product_search:
  up   "CREATE INDEX products_name_trgm ON products USING gin (name gin_trgm_ops)"
  down "DROP INDEX IF EXISTS products_name_trgm"
```

`ori db check --json` validates the query shapes (`Q0010` unknown column
type, `Q0020` duplicate query with conflicting shape) and topologically
orders the migration graph (`MigrationError::Cycle` on cycles).

---

## CLI tour

Orison is one binary, 30+ subcommands, every command emits a JSON envelope
matching a schema. The full surface from `ori --help`:

| Subcommand | Schema | Purpose |
|------------|--------|---------|
| `ori check [--json] <file>` | `ori.diagnostic.v1` | Parse + type/effect check |
| `ori fmt <file>` | — | CST-preserving formatter |
| `ori capsule --json <file>` | `ori.capsule.v1` | Per-module semantic capsule |
| `ori agent map --budget N --json <file>` | `ori.agent_map.v1` | Budget-bounded symbol map |
| `ori agent explain <sym> --json <file>` | `ori.symbol_card.v1` | Single-symbol detail |
| `ori agent symbols [--changed] --json <file>` | `ori.agent_symbol_list.v1` | Symbol enumeration |
| `ori agent diagnose --json <file>` | `ori.agent_diagnose.v1` | Status + top repair candidates |
| `ori agent tests --affected --json <root>` | `ori.agent_tests.v1` | Per-file test selection |
| `ori agent changed --json <root>` | `ori.agent_changed.v1` | Per-symbol fingerprint diff |
| `ori patch check --json <patch>` | `ori.patch_check.v1` | Validate Patch IR shape |
| `ori patch apply [--dry-run] <patch> <src>` | `ori.patch_apply.v1` | Apply Patch IR with stable IDs |
| `ori patch dry-run --json <patch> <src>` | `ori.patch_apply.v1` | Preview without writing disk |
| `ori patch explain --json <patch>` | `ori.patch_explain.v1` | Intent + op count summary |
| `ori run [--entry <name>] <file>` | `ori.run.v1` | Tree-walking interpreter |
| `ori build [--target dev|release|wasm-component|llvm-text|mobile]` | `ori.build_report.v1` | Build report + artefacts |
| `ori bench [--samples N] --json` | `ori.benchmark.v1` | Self-benchmarks |
| `ori openapi --json <file>` | `ori.openapi_report.v1` | OpenAPI 3.1 extraction |
| `ori ui --json <file>` | `ori.ui_manifest.v1` | UI manifest + a11y findings |
| `ori wasm --json <file>` | `ori.wasm_component.v1` | Wasm component manifest |
| `ori capability [--policy] --json <file>` | `ori.capability.v1` | Effect → capability diff |
| `ori test [--changed] --json <root>` | `ori.agent_tests.v1` | Test discovery |
| `ori coverage --json <root>` | `ori.coverage_report.v1` | Per-symbol test coverage |
| `ori docs --format human|agent --budget N` | `ori.docs.v1` | Doc generator |
| `ori migrate --from X --to Y --dry-run` | `ori.migration_report.v1` | Edition migration plan |
| `ori db check --json <file>` | `ori.db_check.v1` | Query + migration validation |
| `ori schema import graphql <sdl> --module` | `ori.graphql_import.v1` | SDL → Orison module |
| `ori schema import grpc <proto> --module` | `ori.rpc_import.v1` | proto3 → Orison module |
| `ori preprocess --const k=v <file>` | `ori.preprocess.v1` | Safe `${ENV}` / `@orison/X` substitution |
| `ori design check --tokens <toml> <file>` | `ori.design_tokens_report.v1` | Design-token enforcement |
| `ori package check --json` | `ori.package_check.v1` | Manifest + lockfile validation |
| `ori audit --json` | `ori.audit_report.v1` | Capability + dep audit |
| `ori sbom [--format ori-native|spdx|cyclonedx]` | `ori.sbom.v1` | Software bill of materials |
| `ori provenance verify <file>` | `ori.provenance.v1` | Provenance check |
| `ori publish --registry <path> --tarball <file>` | `ori.publish_receipt.v1` | Publish to local registry |
| `ori fetch --registry <path> <name>@<v>` | — | Fetch from local registry |
| `ori registry list --registry <path>` | `ori.registry_list.v1` | List registry contents |
| `ori registry yank <name>@<v> --reason` | — | Yank a published version |
| `ori lsp --stdio` | LSP | Language server (initialize, hover, completion, rename, code actions, workspace/symbol, definition, references) |
| `ori doctor [--json]` | `ori.doctor.v1` | Health report + advertised schema versions |

---

## Benchmarks

Real measured numbers on Apple Silicon (`aarch64-apple-darwin`, `rustc 1.92`,
n=100):

| Suite                              | mean     | p50      | p95      |
|------------------------------------|---------:|---------:|---------:|
| `cold_check_latency`               | **4.0 µs**   | 2.2 µs   | 6.4 µs   |
| `warm_check_latency`               | **24.1 µs**  | 20.2 µs  | 46.5 µs  |
| `cst_parse_latency`                | 28.2 µs  | 26.0 µs  | 37.1 µs  |
| `agent_map_token_density` (wall)   | 29.1 µs  | 19.7 µs  | 75.1 µs  |
| `patch_validation_latency`         | **1.7 µs**   | 1.5 µs   | 1.6 µs   |
| `patch_apply_latency` (dry-run)    | **11.2 µs**  | 9.8 µs   | 21.3 µs  |
| `formatter_throughput`             | 1.9 µs   | 1.3 µs   | 1.8 µs   |
| `capsule_generation_latency`       | 31.5 µs  | 23.8 µs  | 95.5 µs  |

**Composite edit-check-repair round trip:** ~77 µs at p50.

That breaks down as: warm check (~20 µs) + agent map (~20 µs) + patch
validate (~1 µs) + patch apply (~10 µs) + format (~1 µs) + capsule (~24 µs)
= **~76 µs**, plus the cold first hit (~2 µs). All measurements deterministic
across runs; methodology, planned suites, and regression bands documented in
[`BENCHMARKS.md`](./BENCHMARKS.md). Raw data in `BENCHMARKS.results.json`.

Generate fresh numbers yourself:

```bash
ori bench --samples 100 --json > BENCHMARKS.results.json
```

---

## Developing with Orison

### Project layout

```
your_project/
├── ori.toml                 # package manifest + declared capabilities
├── src/
│   ├── domain.ori           # types (records, variants, newtypes)
│   ├── storage.ori          # typed queries + migrations
│   ├── api.ori              # service + routes (HTTP)
│   ├── ui.ori               # views (UI)
│   └── main.ori             # boot entrypoint
├── tests/
│   └── smoke.ori            # test functions
└── contracts/               # Patch IR + change manifests (agent-authored)
```

`ori.toml`:

```toml
[package]
name = "my_app"
version = "0.1.0"
edition = "2027.1"
description = "What this does"
license = "Apache-2.0"

[capabilities]
declared = ["http", "db.read", "db.write", "auth"]

[scripts]
check = "ori check --json src/api.ori"
test  = "ori test --json"
```

### The edit-check-repair loop

The day-to-day developer workflow:

```bash
ori check --json src/api.ori           # 20 µs warm — fast feedback
ori capsule --json src/api.ori | jq .  # what does this module export?
ori openapi --json src/api.ori         # what's the HTTP surface?
ori capability --policy "$POLICY" \    # which effects exceed our policy?
  --json src/api.ori
ori test --json                        # discover + run tests
ori bench --samples 30 --no-json       # quick perf check
ori coverage --json .                  # per-function coverage
ori docs --format agent --budget 1500  # generate agent-budgeted markdown
ori build --target dev --json src/api.ori
ori build --target wasm-component --json src/api.ori   # writes .wasm
```

For an LLM agent loop:

```bash
# 1. Get oriented (low context)
ori agent map --budget 2000 --json src/api.ori    # ≤ 2 KB symbol table
ori agent diagnose --json src/api.ori             # status + repair candidates

# 2. Agent proposes a Patch IR
ori patch check --json proposed_patch.json        # validate shape
ori patch dry-run --json proposed_patch.json \    # preview result
  src/api.ori

# 3. Apply + verify
ori patch apply --json proposed_patch.json src/api.ori
ori check --json src/api.ori
ori test --json
```

### Agent-native features

Three features make Orison materially cheaper for AI iteration:

**1. Stable node IDs.** `ori capsule` and the CST attach IDs of the form
`node:<module>.<kind>.<name>.<discriminant>` derived from structural
fingerprint (parent + kind + name + sibling-index + signature hash). They
survive whitespace edits, comment edits, and unrelated edits to other parts
of the same file. Two `fn dup` siblings get different discriminants.

**2. Structural Patch IR.** Instead of diffing text, an agent emits:

```json
{
  "schema": "ori.patch.v1",
  "intent": "Add product search to catalog",
  "operations": [
    {
      "op": "insert_node",
      "target": "sym:demo_store.catalog.list_active",
      "position": "after",
      "text": "fn search(query: Str) -> Result[List[Product], CatalogError] uses db.read:\n  return Ok([])\n"
    },
    {
      "op": "insert_match_arm",
      "target": "sym:demo_store.catalog.CatalogError",
      "pattern": "SearchUnavailable",
      "body": "SearchUnavailable"
    }
  ],
  "tests": { "run": ["sym:demo_store.tests.store_smoke.test_search"] }
}
```

`ori patch check` validates the shape (~0.7 µs at p50). `ori patch dry-run`
applies it in memory and returns the resulting source. Stale-target ops
(`P1010`) are skipped per-op so cross-file patches do partial-apply
correctly; structural failures (`P1000`–`P1003`) abort the whole patch.

**3. Budgeted context packing.** `ori agent map --budget N` returns at most
N bytes of symbol-table JSON. The compiler is honest about what fits:

```bash
ori agent map --budget 200  --json src/api.ori   # 2 symbols, truncated=true
ori agent map --budget 500  --json src/api.ori   # 4 symbols, truncated=true
ori agent map --budget 1000 --json src/api.ori   # 6 symbols, truncated=false
```

`ori agent explain <sym>` returns a single-symbol card for follow-up;
`ori agent diagnose` returns status + top repair candidates from diagnostic
fix attachments; `ori agent tests --affected --changed-name <name>`
returns the per-file set of tests that touch a given identifier.

### Editor integration

Orison ships a hand-rolled LSP server (no third-party deps):

```bash
ori lsp --stdio
```

Implements:
- `initialize` / `initialized` / `shutdown` / `exit`
- `textDocument/didOpen` / `didChange` / `didClose`
- `textDocument/publishDiagnostics` — parity with `ori check --json`
- `textDocument/hover` — markdown with symbol id, signature, effects
- `textDocument/completion` — module exports + 20 keywords, sorted
- `textDocument/rename` — string-literal-aware `WorkspaceEdit`
- `textDocument/codeAction` — quickfixes sourced from diagnostic `fixes`
- `workspace/symbol` — substring case-insensitive, capped at 100
- `textDocument/documentSymbol`
- `textDocument/definition`
- `textDocument/references`

Wire it into VS Code / Helix / Neovim like any LSP server.

---

## Standard distribution

27 modules across five layers, all parse-clean today:

```
stdlib/core/{option, result, iter, string, bytes, list, numeric}.ori
stdlib/std/{json, http, validation, logging, config, time, sql,
           queue, mail, websocket, process, tasks, cache, url}.ori
stdlib/app/{services, views, auth}.ori
stdlib/platform/{web, mobile}.ori
stdlib/labs/experimental.ori
```

Layer purpose:

- **`core`** — language primitives and zero-dep utilities.
- **`std`** — typical production-app needs (JSON, HTTP, DB, validation, logging).
- **`app`** — framework integration (services, views, auth).
- **`platform`** — target-specific shims (web DOM, mobile native).
- **`labs`** — incubating APIs without stability guarantees.

See [`stdlib/README.md`](./stdlib/README.md) and
[`docs/stdlib/STANDARD_DISTRIBUTION.md`](./docs/stdlib/STANDARD_DISTRIBUTION.md).

---

## Example apps

Six first-party demos, all parse with zero errors and zero warnings,
end-to-end through every CLI command:

| App | Focus | Files |
|-----|-------|-------|
| [`demo_store/`](./examples/demo_store) | Canonical full-stack storefront with cart, checkout, admin | 6 modules + tests + 2 Patch IR contracts |
| [`todo_app/`](./examples/todo_app) | Minimal CRUD over a typed domain | 4 modules + tests |
| [`blog/`](./examples/blog) | Auth-gated routes + status variants | 3 modules |
| [`chat/`](./examples/chat) | Websocket + queue + variant payloads | 2 modules |
| [`counter/`](./examples/counter) | Single-view minimal UI | 1 module |
| [`feed_aggregator/`](./examples/feed_aggregator) | Periodic worker over HTTP + queue | 3 modules |

Each has a README with the exact CLI commands to exercise it end-to-end.

---

## Architecture

Five crates, ~32,000 LOC, dependency-light (only `serde` + `serde_json`):

```
                    ┌─────────────────────────┐
                    │       ori (CLI)         │  30+ subcommands
                    └────────────┬────────────┘
        ┌──────────────┬─────────┼────────────┬──────────────┐
        ▼              ▼         ▼            ▼              ▼
   ┌─────────┐   ┌─────────┐ ┌───────┐  ┌─────────┐    ┌──────────┐
   │ori-compiler│ │ori-agent│ │ori-lsp│  │ori-pkg  │    │           │
   │           │ │         │ │       │  │         │    │ schemas/  │
   │ Lexer     │ │ Capsule │ │ Hover │  │ Manifest│    │ 34 stable │
   │ Parser    │ │ Map     │ │ Compl │  │ Lockfile│    │ contracts │
   │ CST+IDs   │ │ Symbol  │ │ Rename│  │ SBOM    │    └───────────┘
   │ Resolver  │ │ Diagnose│ │ CodeAc│  │ Audit   │
   │ TypeCheck │ │ Doctor  │ │ Defn  │  │ Provenance
   │ TypeInfer │ └─────────┘ │ Refs  │  │ Registry│
   │ Effects   │             └───────┘  └─────────┘
   │ Borrow    │
   │ Patch IR  │
   │ HIR/MIR   │
   │ Interp    │
   │ AsyncRT   │
   │ WasmEnc   │
   │ Codegen   │
   │ Formatter │
   │ DocsGen   │
   │ Migrate   │
   │ Coverage  │
   │ Query     │
   │ Bench     │
   │ Importers │
   │ Preproc   │
   └───────────┘
```

Detailed architecture: [`docs/compiler/ARCHITECTURE.md`](./docs/compiler/ARCHITECTURE.md).

---

## Capability model

Every effect is named, declared, and propagated. The package boundary is the
trust boundary:

```
[ori.toml [capabilities].declared]
        │
        ├──► [function `uses` clauses]            ← surface enforced
        │
        ├──► [call-graph propagation E0420]       ← transitive enforced
        │
        ├──► [audit AUD0001 / AUD0002]            ← dependency diff
        │
        ├──► [SBOM ori.sbom.v1]                   ← provenance trail
        │
        └──► [mobile MOB0001 permission map]      ← target-specific
```

Threat model + capability lifecycle in [`SECURITY.md`](./SECURITY.md).

---

## Honest scope

What the bootstrap **does** ship (alpha-grade, tested, schema-versioned):

- Lexer + error-tolerant CST with stable node IDs
- Item parser + body parser (literals, vars, calls, blocks, if, match,
  return, try, lambda, record, tuple, construct)
- Multi-module resolver, signature-level type checker, expression-level
  type inference
- Effect propagation through call graph with `change_signature` repair hints
- Borrow checker prototype (B0010–B0050)
- Exhaustive match check, constant folding
- HIR/MIR + tree-walking executing interpreter
- Cooperative async scheduler
- Hand-rolled WebAssembly bytecode encoder (39-byte hello-module),
  textual LLVM-IR-style codegen
- Patch IR validation + apply + dry-run + explain
- CST-preserving formatter
- OpenAPI 3.1 extraction, UI manifest, design-token enforcement, mobile
  manifest, wasm component manifest, capability manifest
- SQL query shape check + migration toposort
- Package manager: manifest + lockfile + SBOM + audit + provenance +
  local registry stub
- GraphQL SDL importer + gRPC proto3 importer
- LSP server with hover, completion, rename, code actions, workspace
  symbols, document symbols, go-to-def, references
- Doc generator + edition migration tool
- Safe macro preprocessor
- 34 schema contracts, 27 stdlib modules, 6 example apps

What's **not yet** production-grade (years of work; see
[`docs/ROADMAP.md`](./docs/ROADMAP.md)):

- Full HM-style inference inside arbitrary expression bodies
- Region-inference borrow checker
- Optimising native AOT codegen (LLVM/Cranelift) — bootstrap ships
  textual IR scaffold only
- M:N async runtime (bootstrap scheduler is single-threaded cooperative)
- Cryptographic registry signing (bootstrap checksum is FNV-1a)
- Real HTTP/WebSocket/queue runtime (modules are declarations only)
- Self-hosting

This list is enforced by `MEMORY.md`-style discipline: subsystems on the
"not yet" list must not be described as working beyond their stated shape
in any documentation, blog post, or agent prompt.

---

## Contributing

See [`CONTRIBUTING.md`](./CONTRIBUTING.md). The fast version:

```bash
./scripts/install_hooks.sh
cargo fmt --all
cargo test --workspace
python3 scripts/validate_all.py --full   # must end with "validation passed"
```

PR checklist, source guardrails (no `.unwrap()` / `.expect()` / `panic!` in
production), and JSON contract rules are all in `CONTRIBUTING.md`. Security
issues: see [`SECURITY.md`](./SECURITY.md).

### Governance

- [`GOVERNANCE.md`](./GOVERNANCE.md) — decision process, roles, voting.
- [`CODE_OF_CONDUCT.md`](./CODE_OF_CONDUCT.md) — community standards
  and reporting process.
- [`MAINTAINERS.md`](./MAINTAINERS.md) — current maintainer team.
- [`STABILITY.md`](./STABILITY.md) — compatibility tiers + version
  policy + schema lifecycle.
- [`docs/rfcs/PROCESS.md`](./docs/rfcs/PROCESS.md) — RFC process for
  tier-1 / tier-2 changes.

### Learning Orison

The full tutorial series lives at
[`docs/tutorial/`](./docs/tutorial/). Start with
[`01-install.md`](./docs/tutorial/01-install.md) and follow the order in
[`docs/tutorial/README.md`](./docs/tutorial/README.md). The
[`docs/tutorial/CHEATSHEET.md`](./docs/tutorial/CHEATSHEET.md) is a one-
page reference of every CLI subcommand, diagnostic prefix, and keyword.

## License

[Apache-2.0](./LICENSE).
