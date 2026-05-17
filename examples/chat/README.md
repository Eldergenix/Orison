## Chat example

A fourth example app exercising the bootstrap's websocket + queue stubs.
Companion to `examples/demo_store/` (full-stack), `examples/todo_app/`
(CRUD), and `examples/blog/` (auth-gated routes).

The chat example shows:

- Typed messages with payload variants.
- A typed websocket surface.
- An admin-only `prune` route (capability `auth`).

## Acceptance commands

```bash
ori check --json examples/chat/src/domain.ori
ori check --json examples/chat/src/api.ori
ori openapi --json examples/chat/src/api.ori
ori capability --policy "http,db.read,db.write,net.inbound,net.outbound,auth" \
  --json examples/chat/src/api.ori
ori capsule --json examples/chat/src/api.ori
```
