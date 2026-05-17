# labs — incubating APIs

Experimental modules with **no stability guarantees**. The labs layer
is where APIs go to mature before being promoted to `core`, `std`,
`app`, or `platform`.

## Modules

| Module | Purpose |
|--------|---------|
| [`experimental.ori`](./experimental.ori) | Feature-flag plumbing |

## Promotion policy

A `labs` module is promoted when:

1. It has stable signatures (no breaking changes for ≥ 2 minor
   releases).
2. It has integration tests in the target layer's test directory.
3. It has at least one reference example under `examples/`.
4. The promotion is documented as an RFC (see `docs/rfcs/PROCESS.md`).

## Adding a module

A new `labs` module requires only that it parses cleanly via
`ori check --json` and has a one-line description in this table.
There is no other commitment — `labs` is the lowest-friction tier
for experimentation.
