# Orison Migration Guides

This directory holds the per-release migration guides. There is one
guide per minor or major upgrade, named `<from>-to-<to>.md`. Each
guide is the authoritative description of every required source
change and every rollback path for that upgrade.

## Available guides

| From | To  | Guide | Status |
|------|-----|-------|--------|
| 0.1  | 0.2 | [0.1-to-0.2.md](./0.1-to-0.2.md) | Format locked, rules pending 0.2 cut. |

Future guides land here as releases ship. Every guide follows the
template in [`0.1-to-0.2.md`](./0.1-to-0.2.md): scope, edition
mechanics, per-rule table, migration tool invocation, rollback, and
explicit non-changes.

## The migration tool

Every guide is accompanied by an automated migration:

```bash
ori migrate --from <X> --to <Y> --dry-run [path]
```

The tool emits an `ori.migration_report.v1` envelope listing every
rule that fires, every Patch IR operation to apply, and any
diagnostics that block the migration. It is built on Patch IR
(see [cookbook recipe 03](../cookbook/03-agent-driven-refactor.md))
and inherits the same properties: auditable, idempotent,
structurally sound.

## Deprecation policy summary

The full deprecation policy is in
[`STABILITY.md` §6](../../STABILITY.md). The short version:

1. A surface that will be removed is marked `deprecated` in source
   (Rust attribute or schema field).
2. Using the deprecated surface emits a warning.
3. The deprecation persists for **at least two minor releases**
   (~6 months at the planned cadence).
4. Removal happens in a major release with a migration guide in
   this directory.

A deprecation never skips the warning phase, even if the surface is
"obviously misused." The policy is binding from 1.0.

## Migration-eligible surfaces

Migration guides cover **tier 2** (stable-with-editions) surfaces
from `STABILITY.md`: surface syntax not covered by tier 1, default
lints and their severity, standard distribution module names, and
default compiler-flag values.

Tier 1 surfaces (JSON schemas, CLI flag names, language keywords,
diagnostic IDs, public Rust types, the Patch IR op taxonomy, the
capability lifecycle) are not eligible for edition-gated migration.
Breaking changes there require a major version bump and a
`0.x-to-(x+1).0` guide.

A guide is published only when at least one rule would fire. A
patch release (0.1.4 to 0.1.5) never requires a guide; a minor
release that crosses an edition (0.1 to 0.2) does.
