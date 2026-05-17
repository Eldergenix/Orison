# core — language primitives

Foundational types and zero-dep utilities. Every module in `core` parses
on a clean compiler and has zero ambient capabilities (`uses` clauses
empty). Code under `core` may not import `std`, `app`, `platform`, or
`labs` — it is the bottom of the dependency stack.

## Modules

| Module | Purpose |
|--------|---------|
| [`option.ori`](./option.ori) | `Option[T]` variant + `map` / `unwrap_or` / `is_some` / `is_none` |
| [`result.ori`](./result.ori) | `Result[T, E]` variant + `map_ok` / `map_err` / `unwrap_or` / `is_ok` |
| [`iter.ori`](./iter.ori) | `Iter[T]` + `next` / `map` / `filter` / `collect_list` / `count` |
| [`string.ori`](./string.ori) | `len` / `split` / `join` / `to_lower` / `to_upper` / `contains` |
| [`bytes.ori`](./bytes.ori) | `Bytes` + `from_string` / `to_string` / `len` |
| [`list.ori`](./list.ori) | `List[T]` + `push` / `pop` / `len` / `map` / `filter` |
| [`numeric.ori`](./numeric.ori) | `abs` / `min` / `max` / `clamp` / `pow` / `safe_div` / `safe_mul` |

## Stability

Every module in `core` is tier 2 (stable-with-editions) — see
`STABILITY.md`. Renames or removals require an edition transition.
Bodies will become real implementations as part of M27 (see
`GOAL.md`).

## Adding a module

A new `core` module requires:

1. The module file parses clean via `ori check --json`.
2. The module has zero `uses` declarations on its public functions
   (or the new effect appears in `core/effects.md` first).
3. A unit-test fixture under `tests/golden/stdlib/core/<module>.expected.json`.
4. An entry in this README's module table.
5. A `CHANGELOG.md` entry under the next-version section.
