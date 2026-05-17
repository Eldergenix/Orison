# Chapter 05: Functions and services

**What you'll build.** A `Storefront` service with three HTTP routes — one
`GET /products`, one `GET /product`, one `POST /checkout` — and the
machine-readable OpenAPI 3.1 description the compiler emits straight from the
source. You will see how the `service` keyword groups routes under a single
capability budget, how the compiler infers request/response types from the
route function signatures, and how to inspect the resulting `ori.openapi_report.v1`
envelope.

**Time:** ~5 minutes.

## 1. The shape of a function

A function in Orison looks like this:

```ori
fn <name>(<arg1>: <T1>, <arg2>: <T2>) -> <ReturnType> uses <effect1>, <effect2>:
  <body>
```

Pieces:

| Position           | Required? | Meaning                                                                |
|--------------------|-----------|------------------------------------------------------------------------|
| `fn <name>`        | yes       | Introduces a function symbol.                                          |
| `(args)`           | yes       | Zero or more typed positional parameters.                              |
| `-> <T>`           | yes (public functions) | Return type. Omitting it on a public function emits `W0301`.  |
| `uses ...`         | optional  | Comma-separated effect list. Absent ≡ pure.                            |
| `:` and body       | yes       | Colon opens the body; body is indented two spaces below.               |

There is no `pub` or `private` keyword yet — every top-level function is
exported unless it is inside a future `private` module section.

## 2. Start the api module

```bash
mkdir -p ~/orison-tutorial/services && cd ~/orison-tutorial/services
```

You will reuse the domain types from chapter 03 as a separate module so the
api file stays focused. First, `domain.ori`:

```ori
module store.domain

type ProductId wraps Str
type OrderId   wraps Str
type Email     wraps Str

type Money = {
  currency:     Str,
  amount_minor: Int
}

type Product = {
  id:    ProductId,
  sku:   Str,
  name:  Str,
  price: Money,
  stock: Int
}

type CartLine = {
  product_id: ProductId,
  qty:        Int
}

type Cart = {
  customer: Email,
  lines:    List[CartLine]
}

type OrderStatus =
  | Pending
  | Paid
  | Fulfilled
  | Cancelled(reason: Str)

type Order = {
  id:       OrderId,
  customer: Email,
  total:    Money,
  status:   OrderStatus
}

fn money_zero() -> Money:
  return Money { currency: "USD", amount_minor: 0 }
```

Sanity check:

```bash
ori check --json domain.ori; echo "exit=$?"
```

```
exit=0
```

## 3. Add the catalog module

`catalog.ori`:

```ori
module store.catalog

import store.domain

type CatalogError =
  | NotFound
  | Unavailable

fn fetch_product(id: ProductId) -> Result[Product, CatalogError] uses db.read:
  return Err(NotFound)

fn list_active() -> List[Product] uses db.read:
  return []
```

Check:

```bash
ori check --json catalog.ori; echo "exit=$?"
```

```
exit=0
```

## 4. Declare the service

`api.ori` is the new piece. A `service` block declares the trust boundary;
every route function inside the same module that uses `http` is a route on the
service:

```ori
module store.api

import store.domain
import store.catalog

type CheckoutError =
  | PaymentFailed
  | InventoryConflict

service Storefront uses http, db.read, db.write

fn get_products() -> List[Product] uses http, db.read:
  return list_active()

fn get_product(id: ProductId) -> Result[Product, CatalogError] uses http, db.read:
  return fetch_product(id)

fn post_checkout(cart: Cart) -> Result[Order, CheckoutError] uses http, db.write:
  return Err(PaymentFailed)
```

The `service Storefront` line declares the service's *effect budget*: every
route function inside this module must use a subset of `{http, db.read, db.write}`.
You can see this when you inspect the capsule:

```bash
ori check --json api.ori; echo "exit=$?"
```

```
exit=0
```

```bash
ori capsule --json api.ori | jq '.exports[] | {id, kind, effects}'
```

```json
{ "id": "sym:store.api.CheckoutError",  "kind": "type",     "effects": [] }
{ "id": "sym:store.api.Storefront",     "kind": "service",  "effects": ["http", "db.read", "db.write"] }
{ "id": "sym:store.api.get_product",    "kind": "function", "effects": ["http", "db.read"] }
{ "id": "sym:store.api.get_products",   "kind": "function", "effects": ["http", "db.read"] }
{ "id": "sym:store.api.post_checkout",  "kind": "function", "effects": ["http", "db.write"] }
```

## 5. The HTTP route convention

The bootstrap derives HTTP routes from function names by a small set of rules:

| Function name prefix | HTTP method |
|----------------------|-------------|
| `get_<rest>`         | `GET /<rest>`    |
| `post_<rest>`        | `POST /<rest>`   |
| `put_<rest>`         | `PUT /<rest>`    |
| `delete_<rest>`      | `DELETE /<rest>` |
| `patch_<rest>`       | `PATCH /<rest>`  |
| Anything else        | Not a route (still a normal exported function) |

`<rest>` is the suffix in `kebab-case`. Function parameters become path
parameters (in the order they appear); the return type becomes the response
shape; the function's effect set becomes the route's effect budget.

## 6. Generate OpenAPI 3.1

The `ori openapi` subcommand extracts the routes from the parsed module and
emits a structured `ori.openapi_report.v1` envelope. Run it on `api.ori`:

```bash
ori openapi --json api.ori | jq .
```

```json
{
  "schema":          "ori.openapi_report.v1",
  "openapi_version": "3.1.0",
  "services":        ["Storefront"],
  "routes": [
    {
      "method":            "GET",
      "path":              "/products",
      "handler_symbol":    "sym:store.api.get_products",
      "params":            [],
      "request_body_type": null,
      "response_type":     "List[Product]",
      "effects":           ["http", "db.read"]
    },
    {
      "method":            "GET",
      "path":              "/product",
      "handler_symbol":    "sym:store.api.get_product",
      "params": [
        { "name": "id", "in": "path", "type": "ProductId", "required": true }
      ],
      "request_body_type": null,
      "response_type":     "Result[Product, CatalogError]",
      "effects":           ["http", "db.read"]
    },
    {
      "method":            "POST",
      "path":              "/checkout",
      "handler_symbol":    "sym:store.api.post_checkout",
      "params": [
        { "name": "cart", "in": "path", "type": "Cart", "required": true }
      ],
      "request_body_type": null,
      "response_type":     "Result[Order, CheckoutError]",
      "effects":           ["http", "db.write"]
    }
  ]
}
```

Notes on the envelope:

- `services` lists every `service` keyword observed in the file. Routes are
  joined to a service by being defined in the same module.
- `handler_symbol` is the stable id of the implementing function — Patch IR ops
  target this id when an agent wants to rewrite the route.
- `params[].in` is currently always `"path"` for the bootstrap. JSON body
  binding and query strings land in M37c with the body parser.
- `effects` is the *route* effect set, identical to the function's effect
  declaration. CI gates can iterate `routes[].effects` to enforce per-endpoint
  capability policies.
- `request_body_type` is `null` today; the next milestone (M37c) sets it to
  the type of a parameter explicitly annotated `body`.

## 7. The capability manifest agrees

`ori capability --json api.ori` enumerates the same effects, grouped per
effect and per declaring symbol:

```bash
ori capability --json api.ori | jq .
```

```json
{
  "schema": "ori.capability.v1",
  "module": "store.api",
  "effects": [
    { "name": "db.read",  "uses": ["sym:store.api.Storefront",
                                     "sym:store.api.get_product",
                                     "sym:store.api.get_products"] },
    { "name": "db.write", "uses": ["sym:store.api.Storefront",
                                     "sym:store.api.post_checkout"] },
    { "name": "http",     "uses": ["sym:store.api.Storefront",
                                     "sym:store.api.get_product",
                                     "sym:store.api.get_products",
                                     "sym:store.api.post_checkout"] }
  ],
  "policy": {
    "declared":   [],
    "undeclared": ["db.read", "db.write", "http"],
    "unused":     []
  }
}
```

Set the policy and the undeclared list collapses:

```bash
ori capability --policy "http,db.read,db.write" --json api.ori | jq .policy
```

```json
{ "declared": ["http", "db.read", "db.write"], "undeclared": [], "unused": [] }
```

`ori.openapi_report.v1` and `ori.capability.v1` are derived from the same
symbol table — they will never disagree about which effects a route has.

## 8. Agent map of the api module

`ori agent map` is your one-stop view for an LLM context. On the api module:

```bash
ori agent map --budget 2000 --json api.ori | jq '{symbols: .symbols | length, imports}'
```

```json
{
  "symbols": 6,
  "imports": ["store.catalog", "store.domain"]
}
```

The imports list is part of every agent envelope — useful for prompt-building
scripts that want to pull the same context for upstream modules. Chapter 08
walks through the budget knob in detail.

## 9. A note on `service` blocks vs free functions

The bootstrap accepts `service` as a header-only declaration:

```ori
service Storefront uses http, db.read, db.write
```

There is no opening brace and no nested routes today — every function defined
later in the same module that uses `http` is a route on the most recent
service. The richer "service block with nested routes" syntax is M38a.

This means a small file can have at most one `service` declaration. If you
need two services in one process, split them across two modules.

## Common errors

| Diagnostic | Cause | Fix |
|------------|-------|-----|
| `W0301` — public function without return type | You wrote `fn name()` and stopped, with no `-> T`. | Always specify a return type, even `-> Unit`. |
| `W0501` — unknown type | A parameter or return references a name the compiler does not know. | Import the module that declares the type, or fix the typo. |
| `W0510` — `Result` / `Option` without generic arguments | You wrote `Result` instead of `Result[T, E]`. | Parameterise every Result and Option. |
| `E0220` — unknown import | An `import store.foo` references a module that does not exist on disk. | Either add the file or remove the import. |
| `E0410` — effect not in package policy | A route uses an effect not in `[capabilities].declared`. | Add the effect to the policy or remove it from the route. |

## Recap

- Functions are defined with `fn name(args) -> T uses e1, e2: <body>`. The
  `uses` clause is optional; absent means pure.
- A `service` block declares the trust boundary for the routes that follow it
  in the same module. Routes inherit the service's capability budget.
- The bootstrap derives HTTP routes from function name prefixes (`get_`, `post_`,
  ...). Parameters become path params; the return type becomes the response.
- `ori openapi --json` emits the `ori.openapi_report.v1` envelope directly from
  the parsed source. There is no separate annotation language.
- `ori capability --json` and `ori openapi --json` are derived from the same
  symbol table; they will never disagree.

## Next

Continue with [chapter 06: Views and UI](./06-views-and-ui.md). You will
declare typed views, set prop types against the same record shapes the routes
return, and inspect the accessibility findings the compiler emits in the
`ori.ui_manifest.v1` envelope.

For the language reference on functions and signatures see
[`docs/language/REFERENCE.md`](../language/REFERENCE.md); for the long-form
roadmap on services see
[`docs/frameworks/BACKEND.md`](../frameworks/BACKEND.md).
