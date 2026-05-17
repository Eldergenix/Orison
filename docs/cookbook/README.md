# Orison Cookbook

The cookbook is a set of task-oriented recipes. Each one is
self-contained, runs in 10-30 minutes, and ends with a working
artefact. Unlike the [tutorial](../tutorial/README.md), which walks
the language linearly, the cookbook assumes you know the basics and
want to *get a specific thing done*.

## Recipes

| Recipe | Topic |
|--------|-------|
| [01 — REST API from scratch](./01-rest-api-from-scratch.md) | Typed TODO API with `service`, routes, capability declarations, and OpenAPI 3.1. |
| [02 — Typed SQL migration](./02-typed-sql-migration.md) | Add a column with a `migration` block; trigger `Q0010`. |
| [03 — Agent-driven refactor](./03-agent-driven-refactor.md) | `ori agent map` + `ori patch apply` to rename across a workspace. |
| [04 — Capability token flow](./04-capability-token-flow.md) | Issue, delegate, attenuate, revoke a capability token end-to-end. |
| [05 — Going to wasm](./05-going-to-wasm.md) | Build to a wasm component, generate WIT, run under wasmtime. |
| [06 — Formatter in CI](./06-using-the-formatter-in-ci.md) | Wire `ori fmt --check` into a precommit hook and a CI gate. |
| [07 — Publishing a package](./07-publishing-a-package.md) | `ori package check` to `ori publish --dry-run` to tag to release. |
| [08 — Debugging a borrow error](./08-debugging-a-borrow-error.md) | Read a `B0010` / `B0060` diagnostic, find the move site, fix it. |

## Prerequisites

Every recipe assumes:

- A working `ori` binary on your `PATH`. See
  [tutorial 01](../tutorial/01-install.md) for installation.
- A POSIX shell (`bash`-compatible).
- `jq` for inspecting JSON envelopes.
- `git` for version-controlled steps.

A few recipes require additional tools: recipe 05 needs wasmtime
>= 18; recipe 07 needs a writable local directory for the registry.
Every recipe is reproducible against the example tree in
[`examples/`](../../examples).

## How to follow a recipe

Each recipe follows the same shape: goal, prerequisites and time,
numbered walkthrough with expected output for every command, edge
cases, and a closing checklist. Work through one by reading the
goal, creating the working directory the recipe names, and running
the commands top-to-bottom. When a command shows expected output,
compare your local output — a divergence usually means an outdated
`ori` binary or a typo.

Every `ori` code block in this cookbook is extracted by
`scripts/validate_all.py` and fed to `ori check --json`. The build
breaks if any block emits an error. The same gate covers the
[tutorial](../tutorial/README.md) and the
[migration guides](../migration/README.md).

## Conventions

Commands are shown as `ori`, not `cargo run --release -p ori ...`.
JSON envelopes are pretty-printed with `| jq .`; the shape is the
contract. "In production" means the M37 release; "in the bootstrap"
means today.
