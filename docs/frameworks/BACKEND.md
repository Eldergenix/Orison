# Backend Framework

Orison includes a typed service framework.

```ori
service Users uses http, db.read:
  get "/users/{id:UserId}" -> Result[User, ApiErr]:
    return db.users.find(id)

  post "/users" body CreateUser -> Result[User, ApiErr] uses db.write:
    return db.users.insert(User.create(body.name, body.email)?)
```

## Generated artifacts

- OpenAPI schema.
- Typed client SDK.
- Route tests.
- Input validators.
- Auth policy checks.
- Observability spans.
- Capability manifest.
- Semantic capsule.

## Agent advantages

Route declarations are compact and typed. Agents can add endpoints without copying router boilerplate, schema boilerplate, and validation boilerplate into context.
