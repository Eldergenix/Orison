# Recipe 02: Add a column with a typed SQL migration

**Goal.** Walk a typed `Query` and the matching `migration` block from
their initial state through the addition of one column. You will see how
the migration graph topologically orders by `after` dependencies, how
`Q0010` flags unknown column types, and why the `Query` declaration is
the canonical contract between the database and your Orison code.

**Prerequisites.** A working `ori` binary and familiarity with
[tutorial 07](../tutorial/07-queries-and-migrations.md).

**Time:** ~10 minutes.

## 1. The starting schema

Save this as `src/queries.ori`:

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
```

Three things are happening here. The `query` declaration carries both a
typed shape (the record literal between `->` and the SQL string) and an
SQL body. The shape names the columns the SQL must return; the SQL
parameter placeholders (`$1`, `$2`, ...) are positional and align with
the parameter list left-to-right. The `migration` block declares an
`up` step and a `down` step, both as raw SQL strings.

Check the module:

```bash
ori check --json src/queries.ori; echo "exit=$?"
```

Empty stdout, exit 0. Now run the database envelope:

```bash
ori db check --json src/queries.ori | jq .
```

```json
{
  "schema": "ori.db_check.v1",
  "module": "store.queries",
  "queries":    { "diagnostics": [] },
  "migrations": {
    "schema":  "ori.migration_graph.v1",
    "ordered": ["create_products_table"],
    "cycles":  []
  }
}
```

The `migrations.ordered` array is the apply order: stable across runs,
deterministic across hosts. CI gates can hash this directly.

## 2. Add a new column

Now you want a `priority` column on `products`. There are three coupled
changes: the storage shape, the queries that consume it, and the
migration that mutates the schema. Edit `src/queries.ori` in lockstep:

```ori
module store.queries

type ProductId wraps Str

type Priority =
  | Low
  | Normal
  | High

query find_product_by_sku(sku: Str) -> {id: ProductId, sku: Str, name: Str, priority: Priority}
  "SELECT id, sku, name, priority FROM products WHERE sku = $1"

query list_active_products() -> {id: ProductId, sku: Str, name: Str, stock: Int, priority: Priority}
  "SELECT id, sku, name, stock, priority FROM products WHERE active = true"

migration create_products_table:
  up   "CREATE TABLE products (id text primary key, sku text unique, name text, stock int, active bool)"
  down "DROP TABLE IF EXISTS products"

migration add_priority_column after create_products_table:
  up   "ALTER TABLE products ADD COLUMN priority text DEFAULT 'Normal'"
  down "ALTER TABLE products DROP COLUMN priority"
```

The `after create_products_table` clause is the dependency edge that
forces topological ordering. If you flip the order in the source and
re-run `ori db check`, the apply order does not change — it follows
the `after` graph, not the file order.

Re-check:

```bash
ori check --json src/queries.ori; echo "exit=$?"
ori db check --json src/queries.ori \
  | jq '.migrations.ordered'
```

```json
["create_products_table", "add_priority_column"]
```

## 3. Trigger `Q0010` with a typo

Imagine you misremembered the variant name and wrote `Prioritty`:

```ori
module store.queries

type ProductId wraps Str

type Priority =
  | Low
  | Normal
  | High

query find_product_by_sku(sku: Str) -> {id: ProductId, sku: Str, name: Str, priority: Prioritty}
  "SELECT id, sku, name, priority FROM products WHERE sku = $1"

migration create_products_table:
  up   "CREATE TABLE products (id text primary key, sku text unique, name text)"
  down "DROP TABLE IF EXISTS products"
```

Run the database check:

```bash
ori db check --json src/queries.ori | jq '.queries.diagnostics'
```

The envelope reports:

```json
[
  {
    "schema":  "ori.diagnostic.v1",
    "id":      "Q0010",
    "level":   "error",
    "message": "query `find_product_by_sku` column `priority` references unknown type `Prioritty`"
  }
]
```

`Q0010` fires whenever a declared column type is neither a built-in,
nor a type declared in the same module, nor one of the permitted
generic constructors (`Option`, `Result`, `List`, `Pair`, `Fn`,
`Iter`, `Query`, `Map`, `Set`). Fix the typo and the diagnostic
disappears.

## 4. Why the shape contract matters

The `Query` declaration is load-bearing for the rest of the
toolchain. `ori capsule` surfaces each query as a symbol with
`[db.read]` or `[db.read, db.write]` effects depending on the SQL
verb. `ori capability` unions those effects with the module's set —
a `db.write` query in a module that does not declare `db.write`
trips `E0410`. `ori audit` treats `migration` blocks as audit
surface, so a destructive `DROP TABLE` in an `up` step shows up in
the `AUD000*` family.

## 5. Roll forward, then roll back

The migration graph is bidirectional. The `down` strings run in
reverse topological order: forward applies
`create_products_table -> add_priority_column`; rollback applies
`add_priority_column -> create_products_table`. The bootstrap does
not yet execute the SQL (M28 runtime), but the order is
deterministic, shape contracts are checked, and the rollback path
is declared — enough to wire CI gates that catch schema drift
before it touches a real database.

## 6. Checklist for adding any column

When you add a column in your project, walk this list:

- [ ] Edit the `Query` shape to add the new column with its type.
- [ ] Edit the SQL string in the query to project the new column.
- [ ] Add a new `migration <name> after <previous>:` block.
- [ ] Run `ori check` — verify exit 0.
- [ ] Run `ori db check` — verify the new migration appears at the
      end of `migrations.ordered`.
- [ ] Run `ori capability --policy ...` to confirm no effects have
      changed.
- [ ] Commit the change with the migration name in the message.
