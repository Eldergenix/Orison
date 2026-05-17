# Recipe 01: Build a TODO REST API from scratch in 30 minutes

**Goal.** Ship a working typed HTTP API with three routes, a typed domain
model, a capability budget, and a generated OpenAPI 3.1 description — all
before your coffee gets cold. Every artefact in this recipe is reproducible
from the example tree at `examples/todo_app/`.

**Prerequisites.** A working `ori` binary on your `PATH` (see
[tutorial 01](../tutorial/01-install.md)) and a shell.

**Time:** ~30 minutes (15 typing, 15 inspecting envelopes).

## 1. Lay out the package

A minimal Orison package is a directory with `ori.toml` and a `src/`
directory containing one module per file. The TODO API needs three modules:

- `todo_app.domain` — the typed data model.
- `todo_app.storage` — the CRUD surface, with effects but no HTTP.
- `todo_app.api` — the HTTP service that wires storage to the network.

```bash
mkdir -p todo_app/src && cd todo_app
cat > ori.toml <<'TOML'
[package]
name        = "todo_app"
version     = "0.1.0"
edition     = "2027.1"
description = "Typed TODO REST API."
license     = "Apache-2.0"

[capabilities]
declared = ["http", "db.read", "db.write"]
TOML
```

The `[capabilities].declared` list is the package's effect budget. Any
function that uses an effect outside this list raises `E0410` at
`ori check`, and any caller that forgets to declare a transitively-used
effect raises `E0420`. The bootstrap enforces this statically — there is
no runtime escape hatch.

## 2. Declare the domain

Save this as `src/domain.ori`:

```ori
module todo_app.domain

type TodoId wraps Str

type Todo = {
  id: TodoId,
  title: Str,
  done: Bool,
  priority: Priority
}

type Priority =
  | Low
  | Normal
  | High

fn new_todo(id: TodoId, title: Str) -> Todo:
  return Todo { id: id, title: title, done: false, priority: Normal }

fn mark_done(todo: Todo) -> Todo:
  return Todo { id: todo.id, title: todo.title, done: true, priority: todo.priority }
```

Three things to notice. First, `TodoId wraps Str` is a *newtype*: it has
the same memory layout as `Str` but the type checker refuses to let you
pass an arbitrary `Str` where a `TodoId` is expected. Second, the
`Priority` variant has no payload constructors — each arm is a singleton.
Third, the two pure helpers carry no `uses` clause, which means they are
side-effect-free and can be called from any context.

Check the file:

```bash
ori check --json src/domain.ori; echo "exit=$?"
```

Expected: empty stdout, `exit=0`.

## 3. Declare the storage layer

Save this as `src/storage.ori`:

```ori
module todo_app.storage

import todo_app.domain

variant StorageError =
  | NotFound
  | Conflict(reason: Str)

fn insert(todo: Todo) -> Result[Todo, StorageError] uses db.write:
  return Ok(todo)

fn fetch(id: TodoId) -> Result[Todo, StorageError] uses db.read:
  return Err(NotFound)

fn list_all() -> List[Todo] uses db.read:
  return []

fn remove(id: TodoId) -> Result[Unit, StorageError] uses db.write:
  return Ok(Unit)
```

`StorageError` is declared with `variant` (interchangeable with `type X = | A | B`)
to underline that this is an error sum type. Notice that `insert` and
`remove` declare `db.write`, while `fetch` and `list_all` declare only
`db.read`. The split lets you give a read-only replica connection the
read functions and the leader the write functions — the type system
enforces the partition.

Check it. Watch how `ori capsule` reports the effects per symbol:

```bash
ori capsule --json src/storage.ori | jq '.exports[] | {id, kind, effects}'
```

You will see four function entries, each carrying the effect set you
declared. The capsule envelope (`ori.capsule.v1`) is the structural
summary every other tool builds on.

## 4. Declare the HTTP service

Save this as `src/api.ori`:

```ori
module todo_app.api

import todo_app.domain
import todo_app.storage

service Todos uses http, db.read, db.write

fn get_todos() -> List[Todo] uses http, db.read:
  return []

fn get_todo(id: TodoId) -> Result[Todo, StorageError] uses http, db.read:
  return Err(NotFound)

fn post_todo(todo: Todo) -> Result[Todo, StorageError] uses http, db.write:
  return Ok(todo)

fn delete_todo(id: TodoId) -> Result[Unit, StorageError] uses http, db.write:
  return Ok(Unit)
```

The `service Todos uses http, db.read, db.write` line declares the
service's effect budget. Every route function inside the same module
must use a subset of that set, or `ori check` raises `E0410`. The
bootstrap derives HTTP routes from the function name:

| Function prefix | HTTP method |
|-----------------|-------------|
| `get_<rest>`    | `GET /<rest>`    |
| `post_<rest>`   | `POST /<rest>`   |
| `put_<rest>`    | `PUT /<rest>`    |
| `delete_<rest>` | `DELETE /<rest>` |
| `patch_<rest>`  | `PATCH /<rest>`  |

Anything that does not match a prefix is still exported, but does not
appear in the OpenAPI description. The function's positional parameters
become path parameters in declaration order; the return type becomes
the response body shape.

## 5. Generate OpenAPI 3.1

```bash
ori openapi --json src/api.ori | jq '{services, route_count: (.routes | length)}'
```

```json
{ "services": ["Todos"], "route_count": 4 }
```

Inspect a single route to see how parameters and effects propagate:

```bash
ori openapi --json src/api.ori \
  | jq '.routes[] | select(.method=="GET" and .path=="/todo")'
```

The handler symbol, params (each typed with its source-level type, e.g.
`TodoId`), response type, and effect set are all surfaced from the
parsed signature. No runtime introspection, no decorators — the
description is a pure projection of the source.

## 6. Verify the capability budget

```bash
ori capability --policy "http,db.read,db.write" --json src/api.ori \
  | jq '.policy'
```

```json
{
  "declared":   ["db.read", "db.write", "http"],
  "undeclared": [],
  "unused":     []
}
```

Empty `undeclared` and `unused` means the manifest budget and the
observed effects agree. Drop `db.write` from `--policy` and the
envelope flags every write route as `undeclared`. CI gates should run
this with the manifest's `[capabilities].declared` and fail on any
diff.

## 7. What you have now

Four shell commands later you have: typed domain, partitioned storage
effects, a service with a declared budget, a machine-readable OpenAPI
3.1 description, and a capability audit. Next steps:

- Add tests under `tests/` — see [recipe 06](./06-using-the-formatter-in-ci.md)
  for the CI gate that runs them.
- Wire the routes to real HTTP — the dispatcher table is exposed via
  `ori capability check --dry-run`, covered in
  [recipe 04](./04-capability-token-flow.md).
- Compile to wasm — covered in [recipe 05](./05-going-to-wasm.md).
