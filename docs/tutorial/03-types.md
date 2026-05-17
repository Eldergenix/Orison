# Chapter 03: Types

**What you'll build.** A small domain module that demonstrates the four type
forms the bootstrap parser recognises: **newtypes**, **records**, **variants**,
and the built-in `Option` / `Result` parameterised types. You will see how
`E0100` rules out `null`, how `E0101` rules out exception-style throws, and how
the exhaustiveness checker emits `E0540` when a match misses a variant arm.

**Time:** ~10 minutes.

## 1. Start a new module

Create a working directory and `types_demo.ori`:

```bash
mkdir -p ~/orison-tutorial/types && cd ~/orison-tutorial/types
```

Initial source:

```ori
module shop.domain

fn main() -> Unit:
  return Unit
```

Sanity check:

```bash
ori check --json types_demo.ori
echo "exit=$?"
```

Empty stdout, exit 0. You now have a blank canvas.

## 2. Newtypes keep distinct domains apart

A *newtype* wraps an existing type without inheriting any of its operations.
In Orison the syntax is `type <Name> wraps <Underlying>`:

```ori
module shop.domain

type ProductId wraps Str
type OrderId   wraps Str
type Email     wraps Str

fn main() -> Unit:
  return Unit
```

Check:

```bash
ori check --json types_demo.ori; echo "exit=$?"
```

```
exit=0
```

`ori capsule --json types_demo.ori | jq '.exports[] | select(.kind=="type")'`
shows three distinct type exports:

```json
{ "id": "sym:shop.domain.ProductId", "kind": "type", "name": "ProductId",
  "signature": "type ProductId wraps Str", "effects": [], "calls": [], "tests": [],
  "summary": "type `ProductId` declared in this module." }
{ "id": "sym:shop.domain.OrderId",  "kind": "type", "name": "OrderId",
  "signature": "type OrderId wraps Str",   "effects": [], "calls": [], "tests": [],
  "summary": "type `OrderId` declared in this module." }
{ "id": "sym:shop.domain.Email",    "kind": "type", "name": "Email",
  "signature": "type Email wraps Str",     "effects": [], "calls": [], "tests": [],
  "summary": "type `Email` declared in this module." }
```

A function that takes a `ProductId` can never silently accept an `OrderId` — the
type checker rejects the cross-domain confusion (`B0020` in the borrow / type
families) and the agent ABI surfaces both `id`s independently so refactors
target exactly one.

## 3. Records are nominal

Add a small record:

```ori
module shop.domain

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

fn main() -> Unit:
  return Unit
```

Check:

```bash
ori check --json types_demo.ori; echo "exit=$?"
```

```
exit=0
```

Records introduce one symbol per type. Field names live inside the signature
string and are surfaced by `ori capsule`. There is no inheritance, no `null`
fields, and no "default" mutation — every field must be supplied when you
construct the record.

## 4. Variants model algebraic data

Variants are pipe-separated constructors with optional payloads:

```ori
module shop.domain

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

type OrderStatus =
  | Pending
  | Paid
  | Fulfilled
  | Cancelled(reason: Str)

fn main() -> Unit:
  return Unit
```

Check it (still clean):

```bash
ori check --json types_demo.ori; echo "exit=$?"
```

```
exit=0
```

The signature reported by `ori capsule --json` for the variant is the header
line `"type OrderStatus ="`; the constructors themselves are recovered by the
exhaustiveness checker when it needs them (see step 6 below).

## 5. `Option` and `Result` replace `null` and `throw`

Orison has no `null` and no `throw`. The compiler enforces this lexically: the
words trigger `E0100` and `E0101` respectively when they appear outside string
literals or comments.

Add this intentionally broken function temporarily:

```ori
fn fetch_user() -> Str:
  let candidate = null
  return candidate
```

Check:

```bash
ori check --json types_demo.ori
```

```json
{
  "schema":  "ori.diagnostic.v1",
  "id":      "E0100",
  "level":   "error",
  "message": "`null` is not part of Orison; use Option[T]",
  "span":    { "file": "types_demo.ori",
               "start": { "line": 27, "column": 19 },
               "end":   { "line": 27, "column": 23 } },
  "expected": ["Option[T]", "None", "Some(value)"],
  "found":    ["null"],
  "fixes": [
    { "kind": "replace_null",
      "description": "Replace `null` with `None` or an explicit Option value.",
      "confidence": 0.82 }
  ],
  "agent": { "summary": "Replace null with Option semantics.",
             "minimal_context": [],
             "docs": ["doc:types.option"] }
}
```

Exit code 1. Now try the throw variant instead:

```ori
fn fetch_user() -> Str:
  throw "missing"
```

```bash
ori check --json types_demo.ori
```

```json
{
  "schema":  "ori.diagnostic.v1",
  "id":      "E0101",
  "level":   "error",
  "message": "exceptions are not part of Orison; return Result[T, E]",
  "span":    { "file": "types_demo.ori",
               "start": { "line": 27, "column": 3 },
               "end":   { "line": 27, "column": 8 } },
  "expected": ["Result[T, E]", "Err(value)"],
  "found":    ["throw"],
  "fixes": [
    { "kind": "replace_throw",
      "description": "Return `Err(...)` from a Result-returning function.",
      "confidence": 0.76 }
  ],
  "agent": { "summary": "Replace exception-style control flow with Result.",
             "minimal_context": [],
             "docs": ["doc:errors.result"] }
}
```

The fix is to switch to `Result[T, E]`. Replace the function with the canonical
shape:

```ori
type LookupError =
  | NotFound
  | InvalidEmail(input: Str)

fn fetch_user(email: Email) -> Result[Email, LookupError]:
  return Err(NotFound)
```

```bash
ori check --json types_demo.ori; echo "exit=$?"
```

```
exit=0
```

`Result[T, E]` is total: every caller must handle both the `Ok` and the `Err`
arm. `Option[T]` is a sub-case (`Some(value)` / `None`) for absence without an
explicit error reason. Both types live in
[`stdlib/core/option.ori`](../../stdlib/core/option.ori) and
[`stdlib/core/result.ori`](../../stdlib/core/result.ori).

## 6. Non-exhaustive `match` is `E0540`

The exhaustiveness checker lives in
[`crates/ori-compiler/src/exhaustive.rs`](../../crates/ori-compiler/src/exhaustive.rs)
and walks every match expression in a parsed body. It refuses to consider a
match exhaustive when:

- the scrutinee is a variant declared in the current module, and
- at least one declared constructor is missing from the arm list, and
- there is no wildcard `_` arm at the end.

Write a small file that exercises this path. Save the following as
`color_demo.ori`:

```ori
module shop.color

type Color = | Red | Green | Blue

fn handle(c: Color) -> Int:
  match c | Red => 1 | Green => 2
```

The body uses the bootstrap's inline match syntax (`match scrutinee | pat => expr | ...`).
This is the form the exhaustiveness checker recognises today; the indented
multi-line match form is being added in wave 2 of the body parser (see
[`docs/language/REFERENCE.md`](../language/REFERENCE.md)).

The current bootstrap CLI `ori check` does not yet route the exhaustiveness
pass into its diagnostic pipeline — that wiring lands as part of M37b. You can
still observe the same `E0540` diagnostic directly through the library: it is
exercised by `cargo test -p ori-compiler exhaustive::tests`, which runs the
checker against the same source you just wrote and asserts the diagnostic
shape below.

```bash
cargo test -p ori-compiler exhaustive::tests::missing_arm_emits_e0540 -- --nocapture 2>&1 | tail -10
```

The diagnostic envelope the test asserts on, in the canonical
`ori.diagnostic.v1` shape, is:

```json
{
  "schema":  "ori.diagnostic.v1",
  "id":      "E0540",
  "level":   "error",
  "message": "match is not exhaustive: missing arm `Blue`",
  "symbol":  { "id": "sym:shop.color.handle" },
  "expected": ["Red", "Green", "Blue"],
  "found":    ["Blue"],
  "fixes": [
    {
      "kind":        "insert_match_arm",
      "description": "Add a match arm for `Blue`.",
      "confidence":  0.9,
      "patch": {
        "schema":  "ori.patch.v1",
        "intent":  "Add missing variant arm `Blue` to match in `handle`",
        "operations": [
          { "op": "insert_match_arm",
            "target":  "sym:shop.color.handle",
            "pattern": "Blue",
            "body":    "Blue" }
        ],
        "tests":   { "run": ["cargo test -p ori-compiler exhaustive"] }
      }
    }
  ],
  "agent": {
    "summary": "Add an arm for the missing variant or use a wildcard `_` arm.",
    "docs":    ["doc:match.exhaustiveness"]
  }
}
```

Two things to take away from this envelope:

1. The diagnostic owns a structured fix payload. Agents can apply that Patch IR
   directly without reparsing the prose. You'll work with Patch IR end-to-end in
   [chapter 08](./08-patches-and-agents.md).
2. The fix targets the function symbol id `sym:shop.color.handle`, which is
   stable across whitespace edits. Even if you reformat the file the patch
   still resolves.

To make the match exhaustive, add the missing arm (or a wildcard):

```ori
module shop.color

type Color = | Red | Green | Blue

fn handle(c: Color) -> Int:
  match c | Red => 1 | Green => 2 | Blue => 3
```

`ori check --json color_demo.ori` is now silent and exits 0. A wildcard arm
also satisfies the checker:

```ori
fn handle(c: Color) -> Int:
  match c | Red => 1 | _ => 0
```

(That said, prefer real arms over wildcards: the variants explicitly tell the
compiler that you considered the new case when someone adds a `Yellow` later.)

## 7. The full module

Save the final `types_demo.ori`:

```ori
module shop.domain

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

type OrderStatus =
  | Pending
  | Paid
  | Fulfilled
  | Cancelled(reason: Str)

type LookupError =
  | NotFound
  | InvalidEmail(input: Str)

fn fetch_user(email: Email) -> Result[Email, LookupError]:
  return Err(NotFound)

fn main() -> Unit:
  return Unit
```

Verify:

```bash
ori check --json types_demo.ori; echo "exit=$?"
```

```
exit=0
```

And confirm the capsule has all six type exports plus the two functions:

```bash
ori capsule --json types_demo.ori | jq '.exports | length'
```

```
8
```

## 8. Optional: agent map of the same module

```bash
ori agent map --budget 2000 --json types_demo.ori | jq '.symbols[].id'
```

You should see something like:

```
"mod:shop.domain"
"sym:shop.domain.ProductId"
"sym:shop.domain.OrderId"
"sym:shop.domain.Email"
"sym:shop.domain.Money"
"sym:shop.domain.Product"
"sym:shop.domain.OrderStatus"
"sym:shop.domain.LookupError"
"sym:shop.domain.fetch_user"
"sym:shop.domain.main"
```

Symbols are sorted alphabetically within each kind so the budget-bounded view
is stable across runs.

## Common errors

| Diagnostic | Cause | Fix |
|------------|-------|-----|
| `E0100` — `null` is not part of Orison | Literal `null` appeared outside a string or comment. | Switch the variable to `Option[T]`. Use `None` for absence. |
| `E0101` — exceptions are not part of Orison | Literal `throw` keyword appeared. | Change the return type to `Result[T, E]` and return `Err(...)`. |
| `E0540` — match is not exhaustive | A variant constructor is missing from the arm list with no wildcard. | Add the missing arm or end with a `_` wildcard arm. The structured fix attached to the diagnostic is an `insert_match_arm` Patch IR op. |
| `W0501` — unknown type | A type name that is not a primitive, a declared symbol, or a permitted generic (`Option`, `Result`, `List`, `Pair`, `Fn`, `Iter`, `Query`, `Map`, `Set`) was used. | Import or declare the type, or fix the typo. |
| `W0510` — `Result` / `Option` without generic arguments | You wrote `Result` or `Option` without `[T]` / `[T, E]`. | Always parameterise: `Option[Product]`, `Result[User, LookupError]`. |
| `W0301` — public function without an explicit return type | A function omitted `-> T`. | Be explicit even when the type is `Unit`: `fn boot() -> Unit:`. |

## Recap

- The four type forms the bootstrap recognises are **newtype**
  (`type X wraps Y`), **record** (`type X = { f: T, ... }`), **variant**
  (`type X = | A | B(field: T)`), and the parameterised built-ins
  `Option[T]` / `Result[T, E]`.
- `null` and `throw` are syntactic errors (`E0100` / `E0101`) outside of
  strings and comments. Use `Option[T]` and `Result[T, E]`.
- The exhaustiveness checker emits `E0540` with an `insert_match_arm` Patch IR
  fix when a variant arm is missing and no wildcard `_` arm exists.
- Newtypes give you distinct identifier domains for free; a `ProductId` cannot
  silently flow into an `OrderId`.
- Every type and function shows up as a stable `sym:<module>.<name>` in
  `ori capsule` and `ori agent map`; those ids are what every patch and
  refactor targets.

## Next

Continue with [chapter 04: Effects and capabilities](./04-effects.md). You will
declare effects on functions, write an `ori.toml` capability policy, and watch
`E0410` / `E0420` flag a mismatch.

For the authoritative list of every type form and every diagnostic family see
[`docs/language/REFERENCE.md`](../language/REFERENCE.md). For the long-form
description of the type system roadmap see
[`docs/language/TYPE_SYSTEM.md`](../language/TYPE_SYSTEM.md).
