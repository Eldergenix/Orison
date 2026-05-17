# Desktop Hello

A two-module desktop reference app:

- `src/domain.ori` declares `NoteId`, `NoteBody`, `Note`, `NoteList`, and
  the `NoteError` variant.
- `src/notes.ori` declares the `NoteStore` service with four functions:
  `list_notes`, `read_note`, `save_note`, `delete_note`.
- `src/main.ori` declares `boot()` with the `ui`, `fs.read`, `fs.write`
  effects.

Walk through the build in
[`docs/tutorial/11-desktop-app.md`](../../docs/tutorial/11-desktop-app.md).

## Quick check

```bash
for f in examples/desktop_hello/src/*.ori; do
  cargo run --release -p ori --quiet -- check --json "$f"
done
```

All three should exit `0` with no stdout.
