# Demo Application — Acceptance Commands

This page is the short-form companion to `ORISON_DEMO_APPLICATION_GUIDE.md`. It lists the
per-stage acceptance commands and the file map of the canonical demo, **Demo Storefront**.

The long-form guide remains authoritative. When this page disagrees with that guide, the
guide wins.

See also: `examples/demo_store/README.md` (in-tree narrative for the demo itself; created as
part of the Stage 1 build-out).

## File map

The demo lives entirely under `examples/demo_store/`:

```text
examples/demo_store/
├── README.md                                           # Demo narrative and run instructions
├── GOAL.md                                             # Product framing for the demo
├── ori.toml                                            # Package manifest (validated)
├── src/
│   ├── domain.ori                                      # Newtypes, records, variants, money
│   ├── catalog.ori                                     # Product catalog reads
│   ├── cart.ori                                        # Cart operations
│   ├── api.ori                                         # Typed backend routes
│   ├── ui.ori                                          # Typed UI views
│   └── main.ori                                        # Application entry
├── tests/
│   └── store_smoke.ori                                 # End-to-end smoke test
└── contracts/
    ├── agent_patch_add_product_search.json             # Patch IR fixture
    └── change_manifest_checkout.json                   # Change manifest fixture
```

Stage 1 status of these files is tracked in `CHANGELOG.md`; not every file in this map
exists today. The canonical guide marks each file as required by Stage 1.

## Stage 1 — Scaffold-compatible showcase

Status: source files must parse under the bootstrap parser and emit valid capsules, maps,
symbol cards, and diagnostics. No real type or effect checking yet.

```bash
cargo run -p ori -- check --json examples/demo_store/src/domain.ori
cargo run -p ori -- check --json examples/demo_store/src/api.ori
cargo run -p ori -- capsule --json examples/demo_store/src/api.ori
cargo run -p ori -- agent map --budget 3000 --json examples/demo_store/src/api.ori
cargo run -p ori -- patch check --json examples/demo_store/contracts/agent_patch_add_product_search.json
```

Acceptance:

- Every `.ori` file in `examples/demo_store/src/` begins with `module`.
- No source file uses `null` or exception-style control flow.
- Public functions declare explicit return types.
- The patch JSON validates under `schemas/patch.schema.json`.
- `python3 scripts/validate_all.py --static-only` passes.

## Stage 2 — Real parser acceptance target

Gated on M1 (error-tolerant CST/AST) landing in `ori-compiler`.

```bash
cargo test -p ori-compiler parser_demo_store
cargo run -p ori -- check --json examples/demo_store/src/domain.ori
cargo run -p ori -- check --json examples/demo_store/src/api.ori
```

Required parser coverage: records, variants, newtypes, services, views, queries, migrations,
capabilities, function effects, nested UI blocks, route declarations, comments, formatting.

## Stage 3 — Type-system acceptance target

Gated on M3 (type checker) landing.

Required checks the demo must enforce:

- Mixing `ProductId` and `OrderId` is rejected.
- Passing an unvalidated `Str` where `Email` is expected is rejected.
- Route parameters must match handler signatures.
- Query result shapes must match declared return types.
- `Result` values must be handled.
- Match expressions over variants must be exhaustive.
- Public functions must declare explicit return types.

Negative fixtures live under:

```text
tests/golden/demo_store/type_errors/
```

with one JSONL diagnostic fixture per negative `.ori` fixture.

## Stage 4 — Effects and capabilities acceptance target

Gated on M4 (effect/capability checker).

```bash
ori check examples/demo_store --json
ori capabilities emit examples/demo_store --json
ori agent map examples/demo_store --budget 8000 --json
```

Demo enforcement:

- Catalog reads require `db.read`.
- Checkout writes require `db.write`.
- API services require `http`.
- UI views require `ui`.
- Undeclared effects fail with actionable JSON diagnostics.
- Capability manifests are generated from the demo.

## Stage 5 — Backend and API acceptance target

Gated on M12 (backend framework).

```bash
ori build examples/demo_store --dev
ori api emit examples/demo_store --format openapi --out build/demo_store/openapi.json
ori test examples/demo_store --filter api
```

Required artifacts: OpenAPI document, typed client surface, route manifest, request-body
validation schema, operation-level effect/capability manifest, service tests.

## Stage 6 — Database acceptance target

Gated on M13 (data/database framework).

```bash
ori db migrate examples/demo_store --check
ori db seed examples/demo_store --dry-run
ori test examples/demo_store --filter db
```

Required: migrations for products, carts, orders, and order lines; typed SQL queries;
result-shape checking; migration ordering; rollback metadata where possible; seed data.

## Stage 7 — UI acceptance target

Gated on M14 (UI framework).

```bash
ori ui check examples/demo_store --json
ori build examples/demo_store --target wasm-component
ori test examples/demo_store --filter ui
```

Required: product list, product detail, cart summary, checkout form, admin product editor,
design-token use, accessibility diagnostics, invalid-route detection, loading/error states.

## Stage 8 — Agentic workflow acceptance target

Gated on M8/M9 (agent ABI plus Patch IR apply) and M7 (affected-test graph).

```bash
ori agent map examples/demo_store --budget 4000 --json
ori patch check examples/demo_store/contracts/agent_patch_add_product_search.json --json
ori patch apply examples/demo_store/contracts/agent_patch_add_product_search.json
ori test examples/demo_store --changed --json
```

Required scenarios (each must be reproducible from the demo): agent adds product search;
agent adds product discount badge; agent adds admin-only product archive route; agent fixes
an exhaustive match diagnostic; agent handles a new checkout error variant; agent updates UI
after a route change without reading unrelated files.

Per-scenario measurements to record: token budget used, number of files read, patch size,
diagnostics produced, tests selected, tests passed, regressions introduced. These feed the
`agent_map_token_density` and (future) patch-success benchmarks in `BENCHMARKS.md`.

## Validation hook expectations

The repository validation gate must keep demo checks wired in. Current static hooks
require:

- `examples/demo_store/README.md` exists.
- `examples/demo_store/GOAL.md` exists.
- `examples/demo_store/ori.toml` is valid TOML.
- Every `examples/demo_store/src/*.ori` file starts with `module`.
- Demo patch and change-manifest JSON parse.
- Demo patch and change-manifest validate against public schemas when `jsonschema` is
  available.
- This file (`docs/examples/DEMO_APPLICATION.md`) exists and contains stage guidance.

Future (post-Stage-5) hooks add `ori check examples/demo_store --json`, `ori test
examples/demo_store --changed --json`, `ori capsule examples/demo_store --json`, and
`ori agent map examples/demo_store --budget 8000 --json`.
