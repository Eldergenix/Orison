# app — framework integration

Application-framework primitives: service declarations, view rendering
hooks, auth/session integration. Code in `app` may depend on `core`
and `std` but not on `platform` or `labs`.

## Modules

| Module | Purpose |
|--------|---------|
| [`services.ori`](./services.ori) | `Service` + `Route` declaration helpers |
| [`views.ori`](./views.ori) | `View` + `Props` declaration helpers |
| [`auth.ori`](./auth.ori) | `Session` + `Principal` + `require_principal` |

## Stability

Tier 2 (stable-with-editions). The current bodies are declarations;
real implementations land with M28 (backend dispatcher) and M29
(UI render pipeline). See `GOAL.md`.

## Dependency on capabilities

`app.auth.require_principal` requires the `auth` capability. A backend
service whose route binds an authenticated handler declares `uses
auth` on the handler function; the dispatcher (M28) propagates the
authenticated `Principal` through call frames.

## Adding a module

A new `app` module requires:

1. Parses clean via `ori check --json`.
2. Effect declarations match the underlying `std` modules it composes.
3. A reference example app under `examples/` that imports it.
4. A `CHANGELOG.md` entry.
