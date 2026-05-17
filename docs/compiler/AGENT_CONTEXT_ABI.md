# Agent Context ABI

The Agent Context ABI is a stable machine-readable interface between Orison projects and AI coding agents.

## Bootstrap commands

These are implemented in the current scaffold:

```bash
ori agent map --budget 4000 --json examples/fullstack/users.ori
ori agent explain sym:store.users.fetch_user --json examples/fullstack/users.ori
ori capsule --json examples/fullstack/users.ori
ori agent capsule --json examples/fullstack/users.ori
```

## Planned commands

```bash
ori agent symbols --changed --json
ori agent diagnose --json
ori agent patch --from diagnostics.jsonl
ori agent tests --affected sym:app.users.fetch_user --json
```

## Agent map

An agent map is a budgeted project/module summary. It must include:

- schema version;
- module name;
- budget and estimated usage;
- truncation status;
- imports;
- symbol IDs, names, kinds, signatures, and effects;
- diagnostic counts.

## Semantic capsule

Each module emits a capsule containing:

- module name;
- source path;
- content hash;
- exported symbols;
- imports;
- signatures;
- effects;
- tests;
- invariants;
- compact agent summary.

## Symbol card

A symbol card is the smallest useful context unit for an agent.

```json
{
  "schema": "ori.symbol_card.v1",
  "found": true,
  "id": "sym:app.users.fetch_user",
  "name": "fetch_user",
  "kind": "function",
  "signature": "fn fetch_user(id: UserId) -> Result[User, ApiErr] uses db.read",
  "effects": ["db.read"],
  "source_span": {
    "file": "app/users.ori",
    "start_line": 23,
    "end_line": 23
  },
  "summary": "function `fetch_user` in module `app.users`.",
  "module": "app.users"
}
```

## Context budget behavior

`ori agent map --budget N` must eventually:

- prioritize changed symbols;
- include diagnostics first;
- include public API signatures;
- include effects and tests;
- omit full function bodies unless requested;
- report estimated budget usage.

The current scaffold uses a simple source-order budget. Replacing that with dependency-aware packing is tracked in `TASKS.md`.

## Agent cost metrics

The toolchain should eventually track:

- tokens per accepted patch;
- average context size;
- diagnostics-to-fix success;
- regression rate;
- patch minimality;
- affected-test precision.
