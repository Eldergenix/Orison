# UI Framework

Orison includes a typed UI view system.

```ori
view UserCard(user: User) -> Html uses ui:
  card:
    heading(level: 2, text: user.name)
    text(user.email.value)
    button("Edit", action: route("/users/{user.id}/edit"))
```

## Design tokens

```ori
design AppTheme:
  color primary = "#445CFF"
  color danger = "#D92D20"
  space sm = 8.dp
  space md = 12.dp
  radius card = 12.dp
```

## Compiler checks

- Missing accessible labels.
- Invalid routes.
- Hardcoded design values when tokens are required.
- Unhandled async loading/error states.
- Unsafe HTML without capability.

## Targets

```text
Orison view tree
  -> web DOM adapter
  -> Wasm adapter
  -> mobile native adapter
  -> desktop adapter
```
