# Demo Storefront — Goals

## What this demo demonstrates

The Demo Storefront is a single, coherent full-stack application written
in Orison. It is shaped to surface the value of every Orison subsystem
in roughly two hundred lines of source:

- **Typed domain modelling.** `ProductId`, `OrderId`, and `Email` are
  newtypes; `Money`, `Product`, `Cart`, `CartLine`, and `Order` are
  records; `OrderStatus`, `CatalogError`, `CartError`, and
  `CheckoutError` are variants. Mixing identifier types or forgetting an
  error arm will be a compile-time error once the type system lands.
- **Pure cart algebra.** `add_line`, `remove_line`, `total`, and
  `validate` are pure functions over `Cart`. They take no effects, so
  the same code can run in tests, on the server, and inside UI views.
- **Typed HTTP service.** `service Storefront` declares three routes
  (`get_products`, `get_product`, `post_checkout`). Each route carries
  an explicit `uses ...` effect set and a total return type. There is
  no untyped JSON in the public surface.
- **Typed UI views.** `ProductList`, `ProductDetail`, `CartSummary`,
  `CheckoutForm`, and `AdminProductEditor` accept typed props that
  match the domain records the API serves. `AdminProductEditor` is the
  only view that adds the `auth` effect, making protected surfaces
  visible in the manifest.
- **Database queries and migrations.** `catalog.ori` declares typed
  queries (`fetch_product`, `list_active`) and a real migration
  (`add_product_search`) so the database story is co-located with the
  code that depends on it.
- **Explicit effects and capabilities.** Every function that reaches
  outside its arguments declares the exact effects it needs. The
  package manifest (`ori.toml`) declares the corresponding
  capabilities. Discrepancies are diagnostics, not runtime surprises.
- **Stable test identifiers.** `tests/store_smoke.ori` defines tests
  that the Patch IR `tests.run` field references by symbol id. This is
  the contract that lets an agent run only affected tests after a
  small structural patch.
- **Agent-native contracts.** `contracts/agent_patch_add_product_search.json`
  is a real Patch IR document and
  `contracts/change_manifest_checkout.json` is a real change manifest.
  Both validate against the public schemas under `schemas/`.

## Phase-by-phase mapping

Each Orison compiler phase maps onto specific demo features. As the
compiler advances, the demo is the acceptance target that proves the
phase is real.

### Phase 1 — Lexer, parser, module/symbol graph

- `module demo_store.<name>` headers, imports, `fn`, `type`, `service`,
  `view`, `migration`, and `uses` clauses must all tokenise and parse.
- The bootstrap parser already indexes the demo's symbols and emits
  capsules and agent maps over them.

### Phase 2 — Diagnostics and JSON contracts

- Every demo source produces machine-readable diagnostics under the
  shared diagnostic schema.
- Patch IR and change manifest documents under `contracts/` validate
  against `schemas/patch.schema.json` and `schemas/change.schema.json`
  and pass `ori patch check`.

### Phase 3 — Type system

- Newtypes (`ProductId`, `OrderId`, `Email`) cannot be silently
  swapped or constructed from raw strings.
- `Result[T, E]` from `fetch_product`, `validate`, and `post_checkout`
  must be handled at every call site.
- `match` over `OrderStatus`, `CatalogError`, `CartError`, and
  `CheckoutError` must be exhaustive.
- Public functions require explicit `-> T` return types.

### Phase 4 — Effects and capabilities

- `db.read`, `db.write`, `http`, `ui`, and `auth` must be declared on
  every function that uses them.
- The package manifest's `[capabilities] declared` field must cover
  every effect transitively reachable from `boot`.
- `AdminProductEditor` requires `auth`, separating admin surfaces from
  the public storefront at type level.

### Phase 5 — Backend and API contracts

- `service Storefront` is the source of truth for an OpenAPI document
  and a typed client surface.
- Route shapes (path params, request bodies, responses) are checked
  against the domain types defined in `domain.ori`.

### Phase 6 — Database

- The `add_product_search` migration is the first real migration in
  the demo. Migration ordering and rollback metadata will be enforced
  here.
- `fetch_product` and `list_active` are typed queries whose result
  shapes must match `Product`.

### Phase 7 — UI

- `view` declarations under `ui.ori` are the acceptance target for
  typed props, design-token use, accessibility diagnostics, and
  loading/error states.

### Phase 8 — Agentic workflow

- `contracts/agent_patch_add_product_search.json` is a small,
  structural patch that adds product search end-to-end without
  rewriting the affected files.
- `tests.run` points at real test symbol ids so the affected-test
  graph can drive selective re-runs.
- `contracts/change_manifest_checkout.json` documents the
  capability-relevant addition of `post_checkout` and the tests that
  validate it.

## Non-goals

This file is intentionally **not** a tutorial on Orison syntax. It is
the contract between the demo and the compiler: any future change to
the compiler that breaks one of these goals is, by definition, a
regression and must be either fixed or explicitly recorded as a
breaking change in `CHANGELOG.md`.
