# Blog Example

A third small example app for the bootstrap toolchain. Companion to
`examples/demo_store/` (full-stack) and `examples/todo_app/` (CRUD).
The blog example exercises:

- Authored content typed as records with newtype IDs.
- Tagged variants for `PostStatus`.
- A typed service with both public and admin-only routes (capability:
  declared `auth`).
- Per-post views with design-token props.

## Acceptance commands

```bash
ori check --json examples/blog/src/domain.ori
ori check --json examples/blog/src/api.ori
ori openapi --json examples/blog/src/api.ori
ori ui --json examples/blog/src/ui.ori
ori capability --policy "http,db.read,db.write,auth" --json examples/blog/src/api.ori
```
