# Error Model

Orison has no exceptions.

## Result

```ori
type Result[T, E] = Ok(T) | Err(E)
```

## Option

```ori
type Option[T] = Some(T) | None
```

## Propagation

`?` propagates `Err` or `None` from compatible return contexts.

```ori
fn get_name(raw: Json) -> Result[Str, JsonErr]:
  return raw.str("name")?
```

## Exhaustiveness

All variant matches are exhaustive by default.

```ori
match result:
  Ok(value): render(value)
  Err(err): render_error(err)
```

## Diagnostics

Error diagnostics should prefer type-directed repair suggestions:

- add missing match arm;
- wrap value in `Ok`;
- map error type;
- replace `throw` with `Err`;
- replace `null` with `None`.
