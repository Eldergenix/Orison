# Orison Language Specification

Version: `0.1-bootstrap`

This document defines the intended language semantics. The current compiler scaffold does not yet implement this full specification.

## 1. Design principles

Orison source must be:

1. readable by humans;
2. statically checkable by the compiler;
3. navigable by AI agents;
4. safe by default;
5. fast enough for production systems and applications.

The compiler must expose structural information to agents through stable machine-readable schemas.

## 2. File and module model

- Source files use `.ori`.
- Every file begins with a module declaration.
- Module names are dotted lowercase identifiers.
- Imports are explicit.
- Import side effects are forbidden.

```ori
module app.users

import std.json
import std.http
```

## 3. Declarations

Top-level declarations:

```text
module
import
type
fn
protocol
impl
service
view
actor
query
migration
capability
extern
```

## 4. Bindings

```ori
let name = "Ada"      // immutable binding
var count = 0         // mutable binding
```

Mutation is explicit and localized.

## 5. Primitive types

```text
Bool
Int Int8 Int16 Int32 Int64
UInt UInt8 UInt16 UInt32 UInt64
Float32 Float64
Decimal
Char
Str
Bytes
Unit
Never
```

## 6. Records

```ori
type User = {
  id: UserId,
  name: Str,
  email: Email
}
```

Records are nominal types. Field order is not semantically significant except for ABI layout after lowering.

## 7. Variants

```ori
type ApiErr =
  | NotFound
  | BadEmail(Str)
  | Db(DbErr)
```

Pattern matching over variants must be exhaustive unless explicitly marked partial in a controlled context.

## 8. Newtypes

```ori
type UserId wraps UUID
type Email wraps Str
```

Newtypes are zero-cost in optimized builds but remain distinct in type checking.

## 9. Functions

```ori
fn fetch_user(id: UserId) -> Result[User, ApiErr] uses db.read:
  ...
```

Public functions require explicit parameter and return types.

## 10. Generics

```ori
fn identity[T](value: T) -> T:
  return value
```

Generic arguments use square brackets.

## 11. Protocols

Orison uses protocols instead of inheritance.

```ori
protocol Encodable[T]:
  fn encode(value: T) -> Bytes

impl Encodable[User]:
  fn encode(value: User) -> Bytes:
    return json.encode(value).bytes()
```

## 12. Errors

Orison has no exceptions. Errors are values.

```ori
fn load_config(path: Path) -> Result[Config, FsErr] uses fs.read:
  let text = fs.read_text(path)?
  return toml.decode[Config](text)
```

The `?` operator is allowed only in functions returning `Result` or `Option`.

## 13. Null

There is no `null`.

Use:

```ori
Option[T] = Some(T) | None
```

## 14. Effects

Effects declare external capabilities and observable side effects.

```ori
fn send_email(to: Email, body: Str) -> Result[Unit, MailErr] uses mail.send:
  ...
```

Effects are part of the function signature.

## 15. Capabilities

Capabilities are named bundles of effects and constraints.

```ori
capability StripeApi:
  net.outbound = ["api.stripe.com"]
  secrets = ["STRIPE_API_KEY"]
```

## 16. Memory model

- Values are owned by default.
- Moving a non-copy value invalidates the old binding.
- Borrowing is explicit in public APIs.
- Mutable aliases are forbidden in safe code.
- Shared mutable state requires concurrency-safe wrappers.

## 17. Concurrency

Orison supports structured concurrency and actors.

```ori
task_scope scope:
  let users = scope.spawn(fetch_users())
  let plans = scope.spawn(fetch_plans())
  return combine(await users?, await plans?)
```

Actors isolate mutable state.

## 18. Services

Services define typed HTTP APIs.

```ori
service Users uses http, db.read:
  get "/users/{id:UserId}" -> Result[User, ApiErr]:
    return db.users.find(id)
```

## 19. Views

Views define typed UI trees.

```ori
view UserCard(user: User) -> Html uses ui:
  card:
    heading(level: 2, text: user.name)
```

The compiler checks route validity, design token use, and accessibility rules where possible.

## 20. Queries and migrations

```ori
query FindUser(id: UserId) -> Option[User]:
  select id, name, email
  from users
  where id = $id
```

SQL-like query declarations are statically validated against schema metadata when available.

## 21. Unsafe

Unsafe operations are explicit and effect-tracked.

```ori
unsafe fn from_raw(ptr: Ptr[UInt8], len: Int) -> Bytes:
  ...
```

Unsafe effects propagate into capsules and package metadata.

## 22. Editions

Language evolution is edition-based.

```ori
package "app":
  edition "2027.1"
```

Breaking language changes require an edition migration path.
