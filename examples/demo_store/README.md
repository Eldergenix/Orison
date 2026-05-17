# Demo Storefront

The canonical end-to-end Orison demo application. It is intentionally
small but exercises every major language subsystem: typed domain data,
typed HTTP services, typed UI views, database queries and migrations,
explicit effects and capabilities, tests, and agent-facing JSON
contracts (Patch IR and change manifests).

The demo is also a compiler acceptance target: every command listed in
the "Acceptance commands" section below must succeed at the current
bootstrap stage with no `E`-level diagnostics on source files.

## Files

```
examples/demo_store/
  README.md                                — this file
  GOAL.md                                  — phase-by-phase goals and mapping
  ori.toml                                 — package manifest (declared capabilities, deps, scripts)
  src/
    domain.ori                             — newtypes, records, variants, smart constructors
    catalog.ori                            — typed queries, CatalogError, add_product_search migration
    cart.ori                               — pure cart operations and CartError
    api.ori                                — Storefront service: GET/POST routes, CheckoutError
    ui.ori                                 — typed views: list, detail, cart, checkout, admin
    main.ori                               — boot() entry point with declared effects
  tests/
    store_smoke.ori                        — smoke tests referenced by Patch IR `tests.run`
  contracts/
    agent_patch_add_product_search.json    — Patch IR adding search end-to-end
    change_manifest_checkout.json          — change manifest for the checkout route
```

## Quick start

From the workspace root:

```
cargo run -p ori -- check   --json examples/demo_store/src/domain.ori
cargo run -p ori -- check   --json examples/demo_store/src/api.ori
cargo run -p ori -- capsule --json examples/demo_store/src/api.ori
cargo run -p ori -- agent map --budget 3000 --json examples/demo_store/src/api.ori
cargo run -p ori -- patch check --json examples/demo_store/contracts/agent_patch_add_product_search.json
python3 scripts/validate_all.py --static-only
```

## Acceptance commands (Stage 1 — scaffold-compatible showcase)

These must all succeed at the bootstrap stage:

| Command | Expected outcome |
| --- | --- |
| `cargo run -p ori -- check --json examples/demo_store/src/domain.ori` | Parses; no `E`-level diagnostics. |
| `cargo run -p ori -- check --json examples/demo_store/src/api.ori`    | Parses; no `E`-level diagnostics. |
| `cargo run -p ori -- capsule --json examples/demo_store/src/api.ori`  | Emits a semantic capsule for the Storefront service. |
| `cargo run -p ori -- agent map --budget 3000 --json examples/demo_store/src/api.ori` | Emits an agent map within budget. |
| `cargo run -p ori -- patch check --json examples/demo_store/contracts/agent_patch_add_product_search.json` | Returns `valid: true`. |
| `python3 scripts/validate_all.py --static-only` | Passes. |

## Quality invariants

- Every `.ori` source begins with `module demo_store.<name>`.
- No source uses `null` or `throw`; absence and failure are modelled with
  `Option[T]` and `Result[T, E]`.
- Every public function declares an explicit `-> T` return type.
- All declared effects are drawn from the compiler's known set
  (`db.read`, `db.write`, `http`, `ui`, `auth`, `fs.read`, `time`,
  `crypto`, ...).
- Both JSON contracts validate against `schemas/patch.schema.json` and
  `schemas/change.schema.json` and pass `ori patch check`.
- `tests.run` in the Patch IR points at real test symbol ids in
  `tests/store_smoke.ori`.

See `GOAL.md` for the mapping between demo features and Orison's
compiler phases.
