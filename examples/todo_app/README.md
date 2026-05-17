# Todo App Example

A second, smaller demo that exercises the bootstrap toolchain end-to-end.
Companion to the canonical demo at `examples/demo_store/` — the storefront
focuses on the full-stack story; this example zooms in on the
edit-check-repair loop with a minimal domain.

## Files

- `ori.toml` — package manifest with declared capabilities.
- `src/domain.ori` — `Todo`, `TodoId`, `Priority` types.
- `src/storage.ori` — typed CRUD queries.
- `src/api.ori` — service exposing GET/POST/DELETE routes.
- `src/main.ori` — boot function that wires the API together.
- `tests/todo_smoke.ori` — smoke tests.

## Acceptance commands

```bash
ori check --json examples/todo_app/src/domain.ori
ori check --json examples/todo_app/src/api.ori
ori capsule --json examples/todo_app/src/api.ori
ori agent map --budget 2500 --json examples/todo_app/src/api.ori
ori openapi --json examples/todo_app/src/api.ori
ori capability --policy "http,db.read,db.write" --json examples/todo_app/src/api.ori
ori wasm --json examples/todo_app/src/api.ori
ori run examples/todo_app/src/main.ori
```

All commands should exit 0; the `openapi` output should expose three routes;
`capability --policy` should report zero `undeclared` and zero `unused`.
