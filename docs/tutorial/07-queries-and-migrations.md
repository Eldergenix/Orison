# Chapter 07: Queries and migrations

**What you'll build.** A small `queries.ori` module that declares two typed SQL
queries and two migrations. You will see the structured `ori.db_check.v1`
envelope, watch `Q0010` flag an unknown column type, and watch the migration
graph topologically sort `after` dependencies and report cycles.

**Time:** ~5 minutes.

## 1. The shape of a query

A `query` declaration is structurally similar to a function: it has a name, a
typed parameter list, a record return type (one row of the result set), and an
SQL body. The bootstrap recognises:

```ori
query <name>(<arg1>: <T1>) -> {<col1>: <T1>, <col2>: <T2>}
  "<SQL string with $1, $2, ... placeholders>"
```

The arrow record literal `{<col>: <Type>, ...}` is the *shape* the SQL
statement is expected to produce. The compiler does not yet validate the SQL
itself against the shape — that lands in M37d when the SQL parser ships — but
it does extract the shape and run two cheap correctness checks:

- `Q0010` — a declared column type is neither a built-in, nor a type declared
  in the same module, nor one of the permitted generic constructors
  (`Option`, `Result`, `List`, `Pair`, `Fn`, `Iter`, `Query`, `Map`, `Set`).
- `Q0020` — two queries share a name but declare different shapes. The
  bootstrap surface parser deduplicates queries by `(module, name)`, so this
  diagnostic surfaces today only through library-level construction (the
  cross-module case); the wiring becomes user-reachable in M37d.

## 2. Set up

Create a working directory and a module file:

```bash
mkdir -p ~/orison-tutorial/queries && cd ~/orison-tutorial/queries
```

`queries.ori`:

```ori
module store.queries

type ProductId wraps Str

query find_product_by_sku(sku: Str) -> {id: ProductId, sku: Str, name: Str}
  "SELECT id, sku, name FROM products WHERE sku = $1"

query list_active_products() -> {id: ProductId, sku: Str, name: Str, stock: Int}
  "SELECT id, sku, name, stock FROM products WHERE active = true"

migration create_products_table:
  up   "CREATE TABLE products (id text primary key, sku text unique, name text, stock int, active bool)"
  down "DROP TABLE IF EXISTS products"

migration add_product_search_index after create_products_table:
  up   "CREATE INDEX products_name_trgm ON products USING gin (name gin_trgm_ops)"
  down "DROP INDEX IF EXISTS products_name_trgm"
```

Notes:

- The `query` body is a string literal indented two spaces below the signature.
- The `migration` block has `up` and `down` strings on separate indented lines.
- The `after <other-migration>` clause on a migration declares a dependency
  used by the topological sort. Omit it on the first migration.
- `ProductId` is declared *locally* in this module so the query column types
  resolve. Imported types do not currently participate in `Q0010` because the
  query checker only sees same-module symbols.

Check it:

```bash
ori check --json queries.ori; echo "exit=$?"
```

```
exit=0
```

## 3. The `ori.db_check.v1` envelope

```bash
ori db check --json queries.ori | jq .
```

```json
{
  "schema": "ori.db_check.v1",
  "module": "store.queries",
  "queries": {
    "diagnostics": []
  },
  "migrations": {
    "schema":  "ori.migration_graph.v1",
    "ordered": ["create_products_table", "add_product_search_index"],
    "cycles":  []
  }
}
```

Field summary:

| Field                    | Meaning                                                                   |
|--------------------------|---------------------------------------------------------------------------|
| `module`                 | The dotted module name.                                                   |
| `queries.diagnostics`    | Per-query findings (`Q0010` / `Q0020`). Empty list means all queries pass.|
| `migrations.ordered`     | Topologically sorted apply order. Stable across runs.                     |
| `migrations.cycles`      | Non-empty if the dependency graph has cycles; the planner refuses to order. |

## 4. Trigger `Q0010` with an unknown column type

Edit `queries.ori` so one column references a type that is not declared, not a
builtin, and not in the permitted-generics list:

```ori
query find_product_role(id: Str) -> {id: ProductId, role: Mystery}
  "SELECT id, role FROM products WHERE id = $1"
```

Re-run the database check:

```bash
ori db check --json queries.ori | jq '.queries.diagnostics'
```

```json
[
  {
    "schema":  "ori.diagnostic.v1",
    "id":      "Q0010",
    "level":   "warning",
    "message": "query `find_product_role` column `role` references unknown type `Mystery`",
    "span":    { "file": "queries.ori", "start": { "line": ..., "column": 1 },
                                         "end":   { "line": ..., "column": ... } },
    "symbol":  { "id": "sym:store.queries.find_product_role" },
    "expected": [
      "a builtin, a type declared in `store.queries`, or a permitted generic (Option, Result, List, Pair, Fn, Iter, Query, Map, Set)"
    ],
    "found":   ["Mystery"],
    "fixes":   [],
    "agent":   { "summary": "Declare the referenced type or import it before using it in a query column.",
                 "minimal_context": [],
                 "docs":            ["doc:db.queries"] }
  }
]
```

`Q0010` is a **warning**: it does not change the exit code. Fix it by:

1. Declaring the type in the module: `type Mystery wraps Str`.
2. Or changing the column to a known type: `role: Str`.

Revert the file before moving on.

## 5. The migration graph topologically sorts `after` dependencies

The `migrations.ordered` list above already showed the dependency working —
`add_product_search_index` ran after `create_products_table`. Verify the
contract by reversing the source order:

```ori
migration add_product_search_index after create_products_table:
  up   "CREATE INDEX products_name_trgm ON products USING gin (name gin_trgm_ops)"
  down "DROP INDEX IF EXISTS products_name_trgm"

migration create_products_table:
  up   "CREATE TABLE products (id text primary key, sku text unique, name text, stock int, active bool)"
  down "DROP TABLE IF EXISTS products"
```

Re-run:

```bash
ori db check --json queries.ori | jq '.migrations.ordered'
```

```json
["create_products_table", "add_product_search_index"]
```

Source order does not matter to the planner — only the `after` clauses do.
This is the property a real migration runner depends on. The planner breaks
ties by id so the order is reproducible across machines.

## 6. Cycles surface explicitly

Cycles abort the planner. Write a tiny file that demonstrates the contract:

```bash
cat > cycle.ori <<'EOF'
module store.cycle

migration a after b:
  up   "A_UP"
  down "A_DOWN"

migration b after a:
  up   "B_UP"
  down "B_DOWN"
EOF
```

Check:

```bash
ori db check --json cycle.ori | jq .migrations
```

```json
{
  "schema":  "ori.migration_graph.v1",
  "ordered": [],
  "cycles":  [["a", "b"]]
}
```

`ordered` is empty when a cycle is present — the planner refuses to guess. The
exit code is non-zero. Repair by removing one of the `after` clauses (or
restructuring the dependency).

```bash
ori db check --json cycle.ori; echo "exit=$?"
```

```
exit=1
```

## 7. Multiple modules

Real apps split migrations across modules. The bootstrap accepts a directory
as the `<file>` argument to `ori db check` when running through automation, but
the per-file CLI takes one module at a time. The contract is that every
migration id is *globally* unique across modules — the planner does not have
a multi-module mode yet (M37e). For now, prefix each migration id with the
module it belongs to:

```ori
migration store_create_products_table:
  ...
migration store_add_product_search_index after store_create_products_table:
  ...
```

## 8. Inspect the capsule

Queries and migrations are both symbol kinds. The capsule lists them in the
same shape as functions:

```bash
ori capsule --json queries.ori | jq '.exports[] | {id, kind}'
```

```json
{ "id": "sym:store.queries.ProductId",                  "kind": "type" }
{ "id": "sym:store.queries.find_product_by_sku",        "kind": "query" }
{ "id": "sym:store.queries.list_active_products",       "kind": "query" }
{ "id": "sym:store.queries.create_products_table",      "kind": "migration" }
{ "id": "sym:store.queries.add_product_search_index",   "kind": "migration" }
```

Symbol ids are stable: a Patch IR op can target a single query or a single
migration without referencing line numbers.

## 9. Effects in queries

Queries do not yet declare effects at the signature site; the runtime treats
every `query` as requiring `db.read` and every `query!` (the future
mutation variant, M38b) as requiring `db.write`. Until that variant lands you
will model writes as functions that wrap an SQL string:

```ori
fn insert_product(product: Product) -> Unit uses db.write:
  return Unit
```

That function then shows up in the capability manifest under `db.write` and is
auditable via `ori capability --policy ...` exactly like every other route.

## Common errors

| Diagnostic | Cause | Fix |
|------------|-------|-----|
| `Q0010` — query column references an unknown type | A column in the return record uses a type that is not a builtin, locally declared, or a permitted generic. | Declare the type in this module, switch to a builtin, or change the column type. |
| `Q0020` — duplicate query with conflicting shape | Two queries share a name but differ on column types. Today this is surface-unreachable; the contract still lives in the library for cross-module construction. | Rename one of the queries, or align their shapes. |
| `migrations.cycles` non-empty | An `after` chain forms a loop. | Remove one of the offending `after` clauses or restructure. |
| `MigrationError::UnknownDependency` | An `after` references an id that no migration declares. | Fix the typo in the `after` clause or add the missing migration. |

## Recap

- A `query` is a name, a typed argument list, a record return type that
  describes the result row, and an indented SQL string body.
- `Q0010` warns on column types that aren't built-in, declared in the module,
  or a permitted generic.
- A `migration` block has `up`, `down`, and optionally `after <other>` to
  declare ordering.
- `ori db check --json` returns the `ori.db_check.v1` envelope: queries
  produce `diagnostics`; migrations produce a topologically sorted `ordered`
  list and an explicit `cycles` list.
- The planner refuses to guess on cycles — the `ordered` list goes empty and
  the exit code becomes non-zero.

## Next

Continue with [chapter 08: Patches and agents](./08-patches-and-agents.md). You
will write a Patch IR document by hand, dry-run it, observe how `P1010`
gracefully skips stale-target ops while structural failures abort, and then
exercise `ori agent map` at three budget levels.

For the long-form description of the SQL DSL and the migration runner contract
see [`docs/frameworks/API_AND_DATA.md`](../frameworks/API_AND_DATA.md).
