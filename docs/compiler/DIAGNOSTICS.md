# Diagnostics

Orison diagnostics are JSON-first.

## Required fields

```json
{
  "schema": "ori.diagnostic.v1",
  "id": "E0312",
  "level": "error",
  "message": "match expression does not handle ApiErr.Db",
  "span": {
    "file": "app/users.ori",
    "start": { "line": 42, "column": 3 },
    "end": { "line": 47, "column": 5 }
  },
  "symbol": { "id": "sym:app.users.render_error" },
  "expected": ["ApiErr.NotFound", "ApiErr.BadEmail", "ApiErr.Db"],
  "found": ["ApiErr.NotFound", "ApiErr.BadEmail"],
  "fixes": [],
  "agent": {
    "summary": "Add a match arm for ApiErr.Db.",
    "minimal_context": ["sym:app.users.render_error", "type:app.users.ApiErr"],
    "docs": ["doc:error-handling.match-exhaustiveness"]
  }
}
```

## Error code families

| Family | Meaning |
|---|---|
| `E0000` | module and syntax errors |
| `E0100` | forbidden language constructs |
| `E0200` | names and symbols |
| `E0300` | types |
| `E0400` | effects and capabilities |
| `E0500` | ownership and borrowing |
| `E0600` | concurrency |
| `E0700` | services/routes |
| `E0800` | UI and accessibility |
| `E0900` | packages and supply chain |
| `W9000` | style and tooling warnings |
| `P0000` | patch validation |

## Repair candidates

Fixes must include:

- `kind`
- `description`
- `confidence`
- optional structured patch object

Human prose alone is insufficient for mature diagnostics.
