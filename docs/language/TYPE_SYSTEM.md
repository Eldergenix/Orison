# Type System

Orison is statically typed with local inference.

## Core properties

- Nominal records and variants.
- Zero-cost newtypes.
- Local type inference.
- Explicit public function signatures.
- Generic functions and types.
- Protocol-based polymorphism.
- Exhaustive pattern matching.
- No null.
- No exceptions.

## Inference boundary

Inference should not cross public API boundaries. This keeps capsules and symbol cards compact and stable for agents.

Allowed:

```ori
let count = users.len()
```

Required:

```ori
fn count_users(users: List[User]) -> Int:
  return users.len()
```

## Option and Result

```ori
type Option[T] = Some(T) | None
type Result[T, E] = Ok(T) | Err(E)
```

The compiler must reject ignored `Result` values unless they are explicitly discarded.

## Protocols

Protocols are resolved statically where possible. Dynamic dispatch requires explicit syntax and should be rare.

## Future checker tasks

- Implement Hindley-Milner-like local inference with nominal constraints.
- Add protocol obligation solving.
- Add exhaustiveness checking.
- Add type-directed diagnostics with patch suggestions.
