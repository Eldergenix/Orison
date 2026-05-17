# Chapter 10: Shipping the demo storefront

**What you'll build.** A complete walkthrough of every source file in
[`examples/demo_store/src/`](../../examples/demo_store/src), exercised through
every CLI envelope you have learned so far. By the end you will have run
`check`, `capsule`, `openapi`, `ui`, `capability`, `wasm`, `run`, and `build`
against the right module for each command, then dry-run the canonical agent
patch end-to-end.

**Time:** ~10 minutes.

## 1. Orient yourself

The demo storefront is the canonical end-to-end app. The directory tree:

```bash
ls -F examples/demo_store/
# README.md      contracts/   ori.toml   src/         tests/      tokens.toml
```

```bash
ls examples/demo_store/src/
# api.ori   cart.ori   catalog.ori   domain.ori   main.ori   ui.ori
```

Six modules, ordered roughly by dependency depth:

| File          | Module                  | Role                                                    |
|---------------|-------------------------|---------------------------------------------------------|
| `domain.ori`  | `demo_store.domain`     | Newtypes, records, variants. No imports, no effects.    |
| `catalog.ori` | `demo_store.catalog`    | Catalog reads + the `add_product_search` migration.     |
| `cart.ori`    | `demo_store.cart`       | Pure cart operations and `CartError`.                   |
| `api.ori`     | `demo_store.api`        | `Storefront` service: three HTTP routes + `CheckoutError`. |
| `ui.ori`      | `demo_store.ui`         | Five typed views including an `auth`-tagged admin view. |
| `main.ori`    | `demo_store.main`       | `boot()` entry point with declared effects.             |

Two contracts:

```bash
ls examples/demo_store/contracts/
# agent_patch_add_product_search.json   change_manifest_checkout.json
```

One smoke-test file:

```bash
ls examples/demo_store/tests/
# store_smoke.ori
```

## 2. `domain.ori` — types only

```bash
ori check --json examples/demo_store/src/domain.ori; echo "exit=$?"
```

```
exit=0
```

```bash
ori capsule --json examples/demo_store/src/domain.ori | jq '{module, exports: (.exports | length)}'
```

```json
{ "module": "demo_store.domain", "exports": 9 }
```

The nine exports: `ProductId`, `OrderId`, `Email`, `Money`, `Product`,
`CartLine`, `Cart`, `OrderStatus`, `Order`, plus the smart constructors
`money_zero` and `cart_empty`. The capability manifest is empty because the
module declares no effects:

```bash
ori capability --json examples/demo_store/src/domain.ori | jq '.effects'
```

```json
[]
```

`ori openapi` and `ori ui` both return an empty list of routes / views — this
module has neither.

## 3. `catalog.ori` — queries and a migration

```bash
ori check --json examples/demo_store/src/catalog.ori; echo "exit=$?"
```

```
exit=0
```

```bash
ori capsule --json examples/demo_store/src/catalog.ori | jq '.exports[] | {id, kind, effects}'
```

```json
{ "id": "sym:demo_store.catalog.CatalogError",      "kind": "type",      "effects": [] }
{ "id": "sym:demo_store.catalog.add_product_search","kind": "migration", "effects": [] }
{ "id": "sym:demo_store.catalog.fetch_product",     "kind": "function",  "effects": ["db.read"] }
{ "id": "sym:demo_store.catalog.list_active",       "kind": "function",  "effects": ["db.read"] }
```

`ori db check` runs the SQL query checker and the migration toposort:

```bash
ori db check --json examples/demo_store/src/catalog.ori | jq .
```

```json
{
  "schema": "ori.db_check.v1",
  "module": "demo_store.catalog",
  "queries":    { "diagnostics": [] },
  "migrations": { "schema": "ori.migration_graph.v1",
                  "ordered": ["add_product_search"],
                  "cycles":  [] }
}
```

```bash
ori capability --json examples/demo_store/src/catalog.ori | jq '.effects'
```

```json
[
  { "name": "db.read", "uses": ["sym:demo_store.catalog.fetch_product",
                                "sym:demo_store.catalog.list_active"] }
]
```

```bash
ori wasm --json examples/demo_store/src/catalog.ori \
  | jq '{schema, module, world, exports: (.exports | length)}'
```

```json
{
  "schema":  "ori.wasm_component.v1",
  "module":  "demo_store.catalog",
  "world":   "demo_store-catalog-world",
  "exports": 3
}
```

## 4. `cart.ori` — pure transformations

```bash
ori check --json examples/demo_store/src/cart.ori; echo "exit=$?"
```

```
exit=0
```

```bash
ori capsule --json examples/demo_store/src/cart.ori | jq '{module, exports: (.exports | length)}'
```

```json
{ "module": "demo_store.cart", "exports": 5 }
```

`cart.ori` is intentionally pure: no effects declared. The capability manifest
confirms this:

```bash
ori capability --json examples/demo_store/src/cart.ori | jq '.effects'
```

```json
[]
```

`ori openapi --json` on a pure module returns no routes (the file has no
`get_` / `post_` / `put_` / etc. prefixed function); `ori ui` returns no views.

## 5. `api.ori` — the HTTP surface

```bash
ori check --json examples/demo_store/src/api.ori; echo "exit=$?"
```

```
exit=0
```

```bash
ori capsule --json examples/demo_store/src/api.ori | jq '{module, exports: (.exports | length)}'
```

```json
{ "module": "demo_store.api", "exports": 5 }
```

`ori openapi --json` returns the three routes derived from the
`get_products`, `get_product`, and `post_checkout` functions:

```bash
ori openapi --json examples/demo_store/src/api.ori | jq '.routes[] | {method, path, effects}'
```

```json
{ "method": "GET",  "path": "/products", "effects": ["http", "db.read"] }
{ "method": "GET",  "path": "/product",  "effects": ["http", "db.read"] }
{ "method": "POST", "path": "/checkout", "effects": ["http", "db.write"] }
```

```bash
ori capability --policy "http,db.read,db.write" --json examples/demo_store/src/api.ori | jq '.policy'
```

```json
{ "declared": ["http", "db.read", "db.write"], "undeclared": [], "unused": [] }
```

```bash
ori wasm --json examples/demo_store/src/api.ori | jq '{module, world, exports: (.exports | length), imports: (.imports | length)}'
```

```json
{
  "module":  "demo_store.api",
  "world":   "demo_store-api-world",
  "exports": 4,
  "imports": 3
}
```

The imports are the three sibling modules (`demo_store.domain`,
`demo_store.catalog`, `demo_store.cart`); the four exports are the service +
the three route handlers.

## 6. `ui.ori` — typed views

```bash
ori check --json examples/demo_store/src/ui.ori; echo "exit=$?"
```

```
exit=0
```

```bash
ori capsule --json examples/demo_store/src/ui.ori | jq '{module, exports: (.exports | length)}'
```

```json
{ "module": "demo_store.ui", "exports": 5 }
```

```bash
ori ui --json examples/demo_store/src/ui.ori | jq '.views[] | {name, effects: .props | map(.name)}'
```

```json
{ "name": "ProductList",         "effects": ["products"] }
{ "name": "ProductDetail",       "effects": ["product"] }
{ "name": "CartSummary",         "effects": ["cart"] }
{ "name": "CheckoutForm",        "effects": ["cart"] }
{ "name": "AdminProductEditor",  "effects": ["product"] }
```

The `CheckoutForm` view emits one accessibility finding:

```bash
ori ui --json examples/demo_store/src/ui.ori \
  | jq '.views[] | select(.accessibility_findings | length > 0) | {name, accessibility_findings}'
```

```json
{
  "name": "CheckoutForm",
  "accessibility_findings": [
    {
      "severity": "info",
      "message":  "form view `CheckoutForm` should expose a `submit_label` prop for screen readers"
    }
  ]
}
```

The capability manifest surfaces both `ui` and `auth` because
`AdminProductEditor` declares `uses ui, auth`:

```bash
ori capability --json examples/demo_store/src/ui.ori | jq '.effects'
```

```json
[
  { "name": "auth", "uses": ["sym:demo_store.ui.AdminProductEditor"] },
  { "name": "ui",   "uses": ["sym:demo_store.ui.AdminProductEditor",
                              "sym:demo_store.ui.CartSummary",
                              "sym:demo_store.ui.CheckoutForm",
                              "sym:demo_store.ui.ProductDetail",
                              "sym:demo_store.ui.ProductList"] }
]
```

## 7. `main.ori` — boot

```bash
ori check --json examples/demo_store/src/main.ori; echo "exit=$?"
```

```
exit=0
```

```bash
ori capsule --json examples/demo_store/src/main.ori | jq '.exports'
```

```json
[
  {
    "id":        "sym:demo_store.main.boot",
    "kind":      "function",
    "name":      "boot",
    "signature": "fn boot() -> Unit uses http, ui, db.read, db.write",
    "effects":   ["http", "ui", "db.read", "db.write"],
    "calls":     [],
    "tests":     [],
    "summary":   "function `boot` declared in this module."
  }
]
```

```bash
ori run --json examples/demo_store/src/main.ori
```

```json
{ "entry": "boot", "module": "demo_store.main", "schema": "ori.run.v1",
  "status": "ok", "value": "Unit" }
```

The interpreter selects `boot` as the entry function automatically (it is the
sole exported function). `Unit` is the canonical "no value to report" result.

## 8. Build it

`ori build --target dev` produces a development build report:

```bash
ori build --target dev --json examples/demo_store/src/main.ori
```

```json
{ "schema":          "ori.build_report.v1",
  "package":         "demo_store.main",
  "target":          "dev",
  "units_compiled":  1,
  "cached_units":    0,
  "duration_ms":     0,
  "errors":          0,
  "warnings":        0,
  "emit_warnings":   [],
  "outputs":         [] }
```

`ori build --target wasm-component` emits a single 37-byte Wasm-component
stub for the api module:

```bash
ori build --target wasm-component --json examples/demo_store/src/api.ori
```

```json
{ "schema":          "ori.build_report.v1",
  "package":         "demo_store.api",
  "target":          "wasm-component",
  "units_compiled":  1,
  "cached_units":    0,
  "duration_ms":     0,
  "errors":          0,
  "warnings":        0,
  "emit_warnings":   [],
  "outputs": [
    { "byte_count": 37, "kind": "wasm-component",
      "path": "examples/demo_store/src/api.ori.wasm" }
  ] }
```

Clean up the artefact when you are done:

```bash
rm -f examples/demo_store/src/api.ori.wasm
```

## 9. The whole module set in one shell

```bash
for f in examples/demo_store/src/*.ori; do
  echo "=== $f ==="
  ori check    --json "$f" > /dev/null && echo "check    ok"
  ori capsule  --json "$f" > /dev/null && echo "capsule  ok"
  ori capability --json "$f" > /dev/null && echo "capability ok"
done
```

Expected:

```
=== examples/demo_store/src/api.ori ===
check    ok
capsule  ok
capability ok
=== examples/demo_store/src/cart.ori ===
check    ok
capsule  ok
capability ok
=== examples/demo_store/src/catalog.ori ===
check    ok
capsule  ok
capability ok
=== examples/demo_store/src/domain.ori ===
check    ok
capsule  ok
capability ok
=== examples/demo_store/src/main.ori ===
check    ok
capsule  ok
capability ok
=== examples/demo_store/src/ui.ori ===
check    ok
capsule  ok
capability ok
```

## 10. Package check, audit, sbom

`ori package check` reads `ori.toml`:

```bash
ori package check --json examples/demo_store | jq '{lockfile: (.lockfile.packages | length), diagnostics: (.diagnostics | length)}'
```

```json
{ "lockfile": 5, "diagnostics": 0 }
```

`ori audit --json` runs the dependency + capability audit:

```bash
ori audit --json examples/demo_store | jq '{summary, findings: (.findings | length)}'
```

```json
{
  "summary":  { "pass": 2, "warn": 0, "fail": 0 },
  "findings": 2
}
```

The two findings are `AUD0002` info-level notes about declared-but-unused
capabilities (`db.read`, `db.write`) — the bootstrap dependencies are stubs
and do not yet pull in the capabilities the package declares.

```bash
ori sbom --json examples/demo_store | jq '{schema, components: (.components | length)}'
```

```json
{ "schema": "ori.sbom.v1", "components": 5 }
```

## 11. Dry-run the canonical agent patch

The shipped patch adds product search across `catalog.ori`, `ui.ori`, and the
test runner. First validate it:

```bash
ori patch check --json examples/demo_store/contracts/agent_patch_add_product_search.json
```

```json
{ "schema": "ori.patch_check.v1", "valid": true, "diagnostics": [] }
```

Now dry-run against each affected file in turn. The Patch IR carries three
operations — one targets `catalog.ori`, two target `ui.ori`. When you dry-run
against `catalog.ori` alone, the ui-targeting op gracefully skips with
`P1010`:

```bash
ori patch dry-run --json \
  examples/demo_store/contracts/agent_patch_add_product_search.json \
  examples/demo_store/src/catalog.ori \
  | jq '{applied, operations_attempted, operations_applied,
         diagnostics: (.diagnostics | map(.id))}'
```

```json
{
  "applied":              true,
  "operations_attempted": 3,
  "operations_applied":   2,
  "diagnostics":          ["P1010"]
}
```

Two of three ops land. The third op (referencing `sym:demo_store.ui.ProductList`)
is per-op stale and surfaces as `P1010`. The exit code is 0.

`ori patch explain` returns a one-line summary suitable for a PR description:

```bash
ori patch explain --json examples/demo_store/contracts/agent_patch_add_product_search.json | jq .
```

```json
{
  "schema":          "ori.patch_explain.v1",
  "intent":          "Add product search to the catalog module, extend CatalogError with a SearchUnavailable arm, and insert a search box into the ProductList view.",
  "operation_count": 3,
  "advice":          "Run `ori patch dry-run` to preview the resulting source before applying."
}
```

## 12. The whole end-to-end gate in one shell

A typical CI gate runs every command in series against every file. The
shortened form, suitable for a smoke check:

```bash
set -euo pipefail
for f in examples/demo_store/src/*.ori; do
  ori check    --json "$f" > /dev/null
  ori capsule  --json "$f" > /dev/null
  ori capability --json "$f" > /dev/null
done
ori openapi  --json examples/demo_store/src/api.ori    > /dev/null
ori ui       --json examples/demo_store/src/ui.ori     > /dev/null
ori wasm     --json examples/demo_store/src/api.ori    > /dev/null
ori run      --json examples/demo_store/src/main.ori   > /dev/null
ori build --target dev --json examples/demo_store/src/main.ori > /dev/null
ori patch check  --json examples/demo_store/contracts/agent_patch_add_product_search.json > /dev/null
ori patch dry-run --json \
  examples/demo_store/contracts/agent_patch_add_product_search.json \
  examples/demo_store/src/catalog.ori > /dev/null
echo "demo_store: all gates pass"
```

Exit code 0 on success. Drop any one line, change a single character in a
source file in a way that introduces a diagnostic, or rename a target id in
the patch, and the gate fails.

## Common errors

| Symptom | Likely cause | Fix |
|--------|--------------|-----|
| `ori openapi` on `cart.ori` returns no routes | The file has no `get_` / `post_` / ... prefixed functions. | Use `api.ori` for the route surface; pure modules legitimately have no routes. |
| `ori ui` on `domain.ori` returns no views | The file has no `view` declarations. | Use `ui.ori` for the view surface. |
| `ori run` on `cart.ori` returns `status: "error"` because no `boot` / `main` exists | The interpreter could not select a default entry. | Pass `--entry <name>` explicitly, or run the `main.ori` module. |
| `P1010` from `ori patch dry-run` against one file | Per-op stale id; the patch targets a node in a different file. | Expected for cross-file patches. The other ops still land. To verify the missing ops, run the patch against the other file. |
| `ori build --target wasm-component` leaves a `.ori.wasm` next to the source | Build artefacts are written next to the input. | Delete with `rm -f <path>.wasm`, or build to a temporary directory. |
| `AUD0002` warnings on the demo storefront | Declared-but-unused capabilities in `ori.toml`. | Info-level; expected today because the dependencies are stubs. |

## Recap

- The demo storefront is six modules, two contracts, one test file, and an
  `ori.toml`. Every module is exercised through the right subset of the CLI
  surface; pure modules legitimately have no routes / views.
- `ori openapi`, `ori ui`, and `ori wasm` all derive from the same symbol
  table — they will never disagree about effects or types.
- `ori capability --policy ...` is the CI-friendly gate for the capability
  budget; setting the right policy makes `policy.undeclared` empty.
- `ori patch dry-run` against a single file partial-applies cross-file ops;
  `P1010` per-op skips never abort the whole patch.
- The full end-to-end gate fits in one shell loop and is the smoke check
  every contributor should run before opening a pull request.

## Next

You have finished the tutorial.

For the language reference see [`docs/language/REFERENCE.md`](../language/REFERENCE.md);
for the long-form architecture see
[`docs/compiler/ARCHITECTURE.md`](../compiler/ARCHITECTURE.md); for the next
items on the alpha roadmap see [`docs/ROADMAP.md`](../ROADMAP.md). The
single-page cheatsheet is at [`CHEATSHEET.md`](./CHEATSHEET.md).

When you are ready to contribute, read [`CONTRIBUTING.md`](../../CONTRIBUTING.md)
and run `python3.13 scripts/validate_all.py --full` before opening a pull
request.
