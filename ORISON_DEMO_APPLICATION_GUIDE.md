# Demo Application Buildout Guide

This guide defines the canonical demo application for Orison: **Demo Storefront**.

The demo is not throwaway sample code. It is the product showcase and an end-to-end compiler acceptance target. Every major language subsystem should eventually be demonstrated here: domain modeling, validation, typed APIs, database queries, backend services, UI views, design tokens, capabilities, tests, diagnostics, semantic capsules, and agent patch workflows.

---

## Demo goals

The demo must show developers what Orison enables:

1. A readable full-stack application in one coherent language.
2. Static types for domain data, API contracts, database query outputs, routes, and UI props.
3. Explicit effects and capabilities for database, HTTP, auth, UI, and outbound network use.
4. Schema-generated API and client contracts.
5. Compiler-generated JSON diagnostics, capsules, symbol cards, and agent maps.
6. Small, structural agent patches instead of whole-file rewrites.
7. A fast edit-check-repair loop for both humans and AI agents.
8. Production-oriented guardrails: validation, tests, design consistency, accessibility, capability manifests, and supply-chain metadata.

---

## Demo app concept

**Demo Storefront** is a small commerce application.

It includes:

- product catalog;
- cart operations;
- checkout flow;
- order creation;
- admin product management surface;
- typed backend API;
- typed UI views;
- database queries and migrations;
- auth-protected admin route;
- design tokens;
- agent patch examples.

The current scaffold stores the demo under:

```text
examples/demo_store/
├── README.md
├── GOAL.md
├── ori.toml
├── src/
│   ├── domain.ori
│   ├── catalog.ori
│   ├── cart.ori
│   ├── api.ori
│   ├── ui.ori
│   └── main.ori
├── tests/
│   └── store_smoke.ori
└── contracts/
    ├── agent_patch_add_product_search.json
    └── change_manifest_checkout.json
```

---

## Required buildout stages

### Stage 1 — Scaffold-compatible showcase

Status: included in this archive.

Purpose: keep demo files parseable by the current bootstrap parser enough to emit capsules, maps, symbol cards, and diagnostics.

Required commands:

```bash
cargo run -p ori -- check --json examples/demo_store/src/domain.ori
cargo run -p ori -- check --json examples/demo_store/src/api.ori
cargo run -p ori -- capsule --json examples/demo_store/src/api.ori
cargo run -p ori -- agent map --budget 3000 --json examples/demo_store/src/api.ori
cargo run -p ori -- patch check --json examples/demo_store/contracts/agent_patch_add_product_search.json
```

Acceptance:

- No source file uses forbidden runtime constructs such as `null` or exception-style control flow.
- Every `.ori` file begins with a `module` declaration.
- Public examples use explicit return types.
- Demo Patch IR and change manifests validate against schemas.
- `scripts/validate_all.py --static-only` passes.

### Stage 2 — Real parser acceptance target

When the CST/AST frontend lands, the demo must be promoted into parser golden coverage.

Add tests for:

- records;
- variants;
- newtypes;
- services;
- views;
- queries;
- migrations;
- capabilities;
- function effects;
- nested UI blocks;
- route declarations;
- comments and formatting.

Acceptance commands:

```bash
cargo test -p ori-compiler parser_demo_store
cargo run -p ori -- check --json examples/demo_store/src/domain.ori
cargo run -p ori -- check --json examples/demo_store/src/api.ori
```

### Stage 3 — Type-system acceptance target

When the type checker lands, the demo must prove that common full-stack mistakes are caught.

Required checks:

- invalid `ProductId` and `OrderId` mixing is rejected;
- unvalidated `Str` cannot be passed as `Email`;
- route parameters match handler signatures;
- query result shapes match declared return types;
- UI props match view signatures;
- `Result` values are handled;
- match expressions over variants are exhaustive;
- public functions have explicit return types.

Add negative fixtures under:

```text
tests/golden/demo_store/type_errors/
```

Each negative fixture must have a matching JSONL diagnostic fixture.

### Stage 4 — Effects and capability acceptance target

When effect checking lands, the demo must enforce external authority.

Required checks:

- catalog reads require `db.read`;
- checkout writes require `db.write`;
- API services require `http`;
- UI views require `ui`;
- payment simulation or notification examples require declared capabilities;
- undeclared effects fail with actionable JSON diagnostics;
- capability manifests are generated from the demo.

Acceptance commands:

```bash
ori check examples/demo_store --json
ori capabilities emit examples/demo_store --json
ori agent map examples/demo_store --budget 8000 --json
```

### Stage 5 — Backend and API acceptance target

When backend services are implemented, the demo must generate API contracts.

Required generated artifacts:

- OpenAPI document;
- typed client surface;
- route manifest;
- validation schema for request bodies;
- operation-level effect/capability manifest;
- service tests.

Acceptance commands:

```bash
ori build examples/demo_store --dev
ori api emit examples/demo_store --format openapi --out build/demo_store/openapi.json
ori test examples/demo_store --filter api
```

### Stage 6 — Database acceptance target

When query and migration support lands, the demo must include validated persistence.

Required features:

- migrations for products, carts, orders, and order lines;
- typed SQL queries;
- query output shape checking;
- migration ordering;
- rollback metadata where possible;
- seed data for local development.

Acceptance commands:

```bash
ori db migrate examples/demo_store --check
ori db seed examples/demo_store --dry-run
ori test examples/demo_store --filter db
```

### Stage 7 — UI acceptance target

When UI support lands, the demo must include real developer-facing UI examples.

Required features:

- product list;
- product detail;
- cart summary;
- checkout form;
- admin product editor;
- design token use;
- accessibility diagnostics;
- invalid route detection;
- loading and error states.

Acceptance commands:

```bash
ori ui check examples/demo_store --json
ori build examples/demo_store --target wasm-component
ori test examples/demo_store --filter ui
```

### Stage 8 — Agentic workflow acceptance target

When Patch IR application and affected-test graphs land, the demo must prove low-context AI development.

Required scenarios:

1. Agent adds product search.
2. Agent adds product discount badge.
3. Agent adds admin-only product archive route.
4. Agent fixes an exhaustive match diagnostic.
5. Agent handles a new checkout error variant.
6. Agent updates UI after a route change without reading unrelated files.

Acceptance commands:

```bash
ori agent map examples/demo_store --budget 4000 --json
ori patch check examples/demo_store/contracts/agent_patch_add_product_search.json --json
ori patch apply examples/demo_store/contracts/agent_patch_add_product_search.json
ori test examples/demo_store --changed --json
```

Measure:

- token budget used;
- number of files read;
- patch size;
- diagnostics produced;
- tests selected;
- tests passed;
- regressions introduced.

---

## Demo quality rules

Any future change to the demo must preserve these rules:

- No pseudo-code comments that pretend unsupported features are implemented.
- No hidden dependency on third-party packages unless explicitly added to the package manifest.
- No raw string routes once route typing is implemented.
- No hardcoded UI colors once design-token checks are implemented.
- No public API change without a test and change manifest.
- No new effect without capability-policy updates.
- No agent patch file without `tests.run` in Patch IR.
- No generated contract without a JSON schema or snapshot test.

---

## Required validation hooks

The repository validation gate must keep demo checks wired in.

Current static hook requirements:

- `examples/demo_store/README.md` exists;
- `examples/demo_store/GOAL.md` exists;
- `examples/demo_store/ori.toml` is valid TOML;
- every `examples/demo_store/src/*.ori` file starts with `module`;
- demo patch and change manifest JSON parse;
- demo patch and change manifest validate against public schemas when `jsonschema` is available;
- `docs/examples/DEMO_APPLICATION.md` exists and contains stage guidance.

Full future hook requirements:

- `ori check examples/demo_store --json`;
- `ori test examples/demo_store --changed --json`;
- `ori capsule examples/demo_store --json`;
- `ori agent map examples/demo_store --budget 8000 --json`;
- generated API/UI/database artifacts validate against schemas and snapshots.

---

## Developer narrative for the demo

The demo should tell this story:

1. Define business data as types, not loose maps.
2. Encode absence and failure with `Option` and `Result`.
3. Add backend routes whose path parameters and responses are statically checked.
4. Add database queries whose result shapes are known to the compiler.
5. Render UI views with typed props and design tokens.
6. Declare all effects so security and agent context are explicit.
7. Let the compiler emit exactly the context an AI agent needs.
8. Apply a small structural patch and run only affected tests.
9. Build for native development and Wasm/web delivery.

When Orison is mature, a developer should be able to read the demo and understand the language’s value without reading the compiler source.
