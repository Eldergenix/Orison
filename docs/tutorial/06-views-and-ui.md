# Chapter 06: Views and UI

**What you'll build.** A small UI module with four typed views: a product list,
a product detail, a checkout form, and an admin editor. You will see how the
`view` keyword declares a typed UI component, how prop types are taken straight
from the record shapes in your domain module, and how the compiler reports
accessibility findings in the `ori.ui_manifest.v1` envelope.

**Time:** ~5 minutes.

## 1. The shape of a view

A view is declared with the `view` keyword, a typed prop list, a return type
(typically `Html`), and an effect set (typically just `ui`):

```ori
view <Name>(<prop1>: <T1>, <prop2>: <T2>) -> <Html|Element|...> uses <effect1>, ...:
  <body using a small element DSL>
```

The body is the bootstrap's UI element DSL. The parser is intentionally
permissive: it recognises element tags (`card`, `list`, `form`, `heading`,
`text`, `button`, `text_input`, ...) and their named arguments. The bootstrap
does **not** type-check inside the body yet — that's M37c — but it does extract
the prop list and the accessibility findings.

## 2. Reuse the domain module

Continue from the directory you created in chapter 05 (or set one up):

```bash
cd ~/orison-tutorial/services
```

You should already have `domain.ori`, `catalog.ori`, and `api.ori` from
chapter 05. If not, copy them out of
[`examples/demo_store/src/`](../../examples/demo_store/src) — the file shapes
are identical.

## 3. Add `ui.ori`

```ori
module store.ui

import store.domain

view ProductList(products: List[Product]) -> Html uses ui:
  list:
    for product in products:
      heading(level: 3, text: product.name)
      text(product.sku)

view ProductDetail(product: Product) -> Html uses ui:
  card:
    heading(level: 2, text: product.name)
    text(product.sku)

view CheckoutForm(cart: Cart) -> Html uses ui:
  form:
    heading(level: 2, text: "Checkout")
    text(cart.customer.value)

view AdminProductEditor(product: Product) -> Html uses ui, auth:
  card:
    heading(level: 2, text: "Edit product")
    text(product.sku)
```

Check:

```bash
ori check --json ui.ori; echo "exit=$?"
```

```
exit=0
```

Capsule, scoped to the view kind:

```bash
ori capsule --json ui.ori | jq '.exports[] | select(.kind=="view") | {id, signature, effects}'
```

```json
{ "id": "sym:store.ui.ProductList",
  "signature": "view ProductList(products: List[Product]) -> Html uses ui",
  "effects":   ["ui"] }
{ "id": "sym:store.ui.ProductDetail",
  "signature": "view ProductDetail(product: Product) -> Html uses ui",
  "effects":   ["ui"] }
{ "id": "sym:store.ui.CheckoutForm",
  "signature": "view CheckoutForm(cart: Cart) -> Html uses ui",
  "effects":   ["ui"] }
{ "id": "sym:store.ui.AdminProductEditor",
  "signature": "view AdminProductEditor(product: Product) -> Html uses ui, auth",
  "effects":   ["ui", "auth"] }
```

Note that `AdminProductEditor` carries `auth` alongside `ui`. The capability
manifest will report `auth` as a separate effect, and any caller that mounts
this view must hold that capability.

## 4. The `ori.ui_manifest.v1` envelope

```bash
ori ui --json ui.ori | jq .
```

```json
{
  "schema": "ori.ui_manifest.v1",
  "views": [
    {
      "symbol": "sym:store.ui.ProductList",
      "name":   "ProductList",
      "route":  null,
      "props":  [{ "name": "products", "type": "List[Product]" }],
      "tokens_used":            [],
      "accessibility_findings": []
    },
    {
      "symbol": "sym:store.ui.ProductDetail",
      "name":   "ProductDetail",
      "route":  null,
      "props":  [{ "name": "product", "type": "Product" }],
      "tokens_used":            [],
      "accessibility_findings": []
    },
    {
      "symbol": "sym:store.ui.CheckoutForm",
      "name":   "CheckoutForm",
      "route":  null,
      "props":  [{ "name": "cart", "type": "Cart" }],
      "tokens_used":            [],
      "accessibility_findings": [
        {
          "severity": "info",
          "message":  "form view `CheckoutForm` should expose a `submit_label` prop for screen readers"
        }
      ]
    },
    {
      "symbol": "sym:store.ui.AdminProductEditor",
      "name":   "AdminProductEditor",
      "route":  null,
      "props":  [{ "name": "product", "type": "Product" }],
      "tokens_used":            [],
      "accessibility_findings": []
    }
  ]
}
```

Field summary:

| Field                          | Meaning                                                                |
|--------------------------------|------------------------------------------------------------------------|
| `views[].symbol`               | Stable symbol id of the view declaration.                              |
| `views[].name`                 | The view name as written in source.                                    |
| `views[].route`                | Path string if the view is routed (always `null` in the bootstrap).    |
| `views[].props`                | Ordered prop list with type annotations.                               |
| `views[].tokens_used`          | Design-token references (see [`ori design check`](../../README.md#cli-tour)). |
| `views[].accessibility_findings` | Per-view findings emitted by the a11y heuristics.                   |

## 5. Accessibility findings

The bootstrap currently emits two heuristic findings:

| Heuristic                                 | Trigger                                       | Severity |
|-------------------------------------------|-----------------------------------------------|----------|
| `submit_label` missing on `form` views    | `form:` body present but no `submit_label` prop on the view | `info`   |
| `alt` missing on visual `card`/`image` views | A view containing an `image(...)` element with no `alt` arg | `warning` |

The findings are advisory: they do not change the `ori check` exit code. They
are surfaced separately so an a11y CI gate can be opinionated without forcing
every developer to fail the main pipeline.

You can verify the `submit_label` heuristic on `CheckoutForm` above — it
appears in the envelope as:

```json
{
  "severity": "info",
  "message":  "form view `CheckoutForm` should expose a `submit_label` prop for screen readers"
}
```

Add the prop to silence the finding:

```ori
view CheckoutForm(cart: Cart, submit_label: Str) -> Html uses ui:
  form:
    heading(level: 2, text: "Checkout")
    text(cart.customer.value)
```

Re-run:

```bash
ori ui --json ui.ori | jq '.views[] | select(.name=="CheckoutForm").accessibility_findings'
```

```
[]
```

## 6. The element DSL

The body grammar inside a view is the bootstrap's element DSL. It is
intentionally minimal:

| Form               | Meaning                                                                 |
|--------------------|-------------------------------------------------------------------------|
| `<tag>:`           | Open a container element. Children are indented two spaces below.       |
| `<tag>(args...)`   | Self-closing element with named or positional arguments.                |
| `for <x> in <expr>:` | Loop construct over a list-typed prop. Children are rendered per-item. |
| `if <expr>:`       | Conditional branch.                                                     |
| String literals    | Always passed as `text` arguments.                                      |

The DSL is *not* JSX-like and *not* HTML. The compiler does not currently
validate that an element tag is in a known set — element-name typos surface
only at the framework boundary (e.g. when a JavaScript runtime renders the
view). M37c adds element-name validation.

## 7. The full UI envelope joins back to the domain

The view prop types reference `Product`, `Cart`, and other records declared in
`store.domain`. The compiler resolves them through the `import store.domain`
header; the UI envelope's `props` field surfaces the *type string* exactly as
written. Downstream tools (the LSP, the docs generator, the agent capsule)
share that resolution.

A useful one-liner: list every view together with every record type it
depends on, in shell:

```bash
ori ui --json ui.ori | jq -r '.views[] | "\(.name) <- " + (.props | map(.type) | join(", "))'
```

```
ProductList <- List[Product]
ProductDetail <- Product
CheckoutForm <- Cart
AdminProductEditor <- Product
```

That output is friendly enough to paste into a PR description.

## 8. Views with effects beyond `ui`

The `AdminProductEditor` view declares `ui, auth`. The capability manifest
reflects this — and crucially, the route OpenAPI envelope will too. If you add
a route that mounts an admin view, the route's `effects` field must include
`auth`:

```bash
ori capability --json ui.ori | jq .
```

```json
{
  "schema": "ori.capability.v1",
  "module": "store.ui",
  "effects": [
    { "name": "auth", "uses": ["sym:store.ui.AdminProductEditor"] },
    { "name": "ui",   "uses": ["sym:store.ui.AdminProductEditor",
                                 "sym:store.ui.CheckoutForm",
                                 "sym:store.ui.ProductDetail",
                                 "sym:store.ui.ProductList"] }
  ],
  "policy": {
    "declared":   [],
    "undeclared": ["auth", "ui"],
    "unused":     []
  }
}
```

Set a tighter policy and you'll see `auth` flagged when it is missing from the
package:

```bash
ori capability --policy "ui" --json ui.ori | jq .policy
```

```json
{ "declared": ["ui"], "undeclared": ["auth"], "unused": [] }
```

## Common errors

| Diagnostic | Cause | Fix |
|------------|-------|-----|
| `W0301` — public function without return type | A view forgot `-> Html`. | Always annotate `-> Html` even if it feels redundant. |
| `W0501` — unknown type | The prop type isn't visible (missing import, typo). | Import the module that declares the type, or fix the typo. |
| `E0220` — unknown import | `import` references a module that does not exist. | Add the file or remove the import. |
| `accessibility_findings[].severity = "info"` | A form view has no `submit_label` prop. | Add `submit_label: Str` to the view's prop list. |
| `accessibility_findings[].severity = "warning"` | An image element has no `alt` argument. | Pass `alt: "..."` to every `image(...)` element. |
| `W0401` — unknown effect | A view declared `uses something_not_in_the_list`. | Use a known effect or declare a real capability. `ui` is the canonical view effect. |

## Recap

- A view is `view Name(props...) -> Html uses ...:` followed by an indented
  element-DSL body. Props become a typed prop list; effects become part of the
  view's capability budget.
- `ori ui --json` returns the `ori.ui_manifest.v1` envelope: one entry per view,
  with the symbol id, the prop list, design-token references, and accessibility
  findings.
- Accessibility findings are advisory and shipped per-view; they do not change
  the `ori check` exit code but a CI gate can read them straight from the
  envelope.
- A view's effects show up in the capability manifest exactly like a function's.
  `auth`-tagged views require the package to declare `auth`.
- The element DSL is minimal in the bootstrap; element-name validation lands
  in M37c.

## Next

Continue with [chapter 07: Queries and migrations](./07-queries-and-migrations.md).
You will declare typed SQL queries, watch `Q0010` and `Q0020` flag bad column
types and conflicting shapes, and topologically sort a migration graph.

For the long-form UI roadmap see
[`docs/frameworks/UI.md`](../frameworks/UI.md); for the design-token contract
see [`ori design check`](../../README.md#cli-tour) and
[`schemas/design-tokens-report.schema.json`](../../schemas/design-tokens-report.schema.json).
