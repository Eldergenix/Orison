# Orison Stability Policy

This document is the **stability commitment** for Orison. Every shipping
artefact lists which compatibility tier it belongs to and what the
project promises about future change.

> **Status:** alpha. The tiering below is in effect *as the project
> progresses toward 1.0*. Until 1.0, breaking changes are permitted in
> every tier with a `CHANGELOG.md` entry and a migration note; the doc
> below is the contract that becomes binding at 1.0.

---

## 1. Compatibility tiers

### Tier 1 — Stable (binding from 1.0)

Breaking changes require a major-version bump (2.0, 3.0, ...). Within
a major series, every shipping artefact in this tier must remain
backward compatible.

Artefacts in tier 1:
- **JSON schema contracts** under `schemas/*.schema.json`. A schema
  named `ori.diagnostic.v1` is immutable forever; the only path to
  changing it is adding `ori.diagnostic.v2` alongside.
- **CLI subcommand names + flag names + exit codes** documented in
  the help text. Renaming a flag is breaking; deprecating one is
  permitted with a warning emitted for ≥ 2 minor versions before
  removal.
- **Language keywords** documented in `docs/language/REFERENCE.md`.
  Adding a keyword in a way that breaks existing source is a major
  version bump, gated on an edition transition (see Tier 2).
- **Diagnostic IDs** that are documented. An emitted `E0100` stays
  `E0100`; an ID is never re-purposed. Adding a new diagnostic is
  minor (it can't break existing valid source). Demoting a previously
  emitted error to a warning is minor; promoting a warning to an
  error is breaking (gated on edition).
- **Public Rust types in `ori-compiler::ast::*`, `::diagnostic::*`,
  `::source::*`, `::patch::*`** (the public surface consumed by
  agent libraries).
- **The Patch IR operation taxonomy** in `schemas/patch.schema.json`
  (`replace_node`, `insert_node`, etc.). New ops are minor;
  removing or changing semantics of an existing op is breaking.
- **The capability lifecycle** documented in `SECURITY.md`: the rules
  for `[capabilities].declared`, `uses` clauses, audit, propagation,
  and runtime enforcement.

### Tier 2 — Stable-with-editions

Breaking changes are permitted at edition boundaries (each edition is
declared in `ori.toml` as `edition = "YYYY.N"`). Tools provide
automated migration via `ori migrate --from X --to Y --dry-run`.

Artefacts in tier 2:
- **Surface syntax not covered by tier 1** — every parseable
  construct that isn't a keyword.
- **Default lints** and their severity (warning → error promotion is
  edition-gated).
- **Standard distribution module names + namespacing.** Renaming
  `stdlib/std/sql.ori` is an edition transition with a migration tool
  rule.
- **Default values for compiler flags.** Changing the default of a
  flag mid-edition that materially changes diagnostics output is
  breaking.

### Tier 3 — Experimental

May break in any minor version with a `CHANGELOG.md` entry. Modules
under `stdlib/labs/` are inherently tier 3. Anything marked
`#[experimental]` in source is tier 3.

### Tier 4 — Internal

No stability guarantees. Module-internal types, helper functions,
private fields in public structs.

---

## 2. The version policy

Orison follows **semantic versioning** (`major.minor.patch`):

- **Patch** (`x.y.Z` → `x.y.Z+1`): bug fixes only. No new public API.
  No diagnostic-ID renames. Compatible with all consumers of `x.y`.
- **Minor** (`x.Y.z` → `x.Y+1.0`): new public API, new diagnostics,
  new CLI subcommands or flags. Existing tier-1 artefacts unchanged.
- **Major** (`X.y.z` → `X+1.0.0`): breaking changes to tier-1
  artefacts. Requires migration guide in `docs/migration/<X>-to-<X+1>.md`.

Edition transitions are **independent** of language version. An edition
is a tier-2 contract that ships as part of a minor or major release.

---

## 3. Schema lifecycle

Every public JSON envelope follows this lifecycle:

```
[draft]      → schemas/<name>.schema.json adds, marked `draft: true` in $id query string
[stable v1]  → drop `draft: true`; downstream consumers may depend on the shape
[stable v2]  → add schemas/<name>-v2.schema.json alongside; v1 remains; ori command emits whichever the user requests via --schema-version
[deprecated] → v1 marked deprecated in $description; emits a warning if requested for ≥ 2 minor releases
[removed]    → v1 removed in next major; v2 is the only shape available
```

Today every shipping schema is **stable v1**. There are no drafts.

Adding a schema-versioned field (additive change) to a stable schema
is a tier-1 minor change. Removing a field, renaming a field, or
changing a field's type is a major change.

---

## 4. Diagnostic ID policy

A diagnostic ID is a permanent identifier. The ID space is partitioned
by prefix (`E` = error, `W` = warning, `S` = lexer string, `N` =
numeric literal, `P` = patch, `Q` = SQL, `B` = borrow, `D` = design
tokens, `MOB` = mobile, `PRE` = preprocessor, `A` = async runtime,
`R` = runtime, `AUD` = audit). The four-digit suffix is allocated per
prefix and never reused.

Demoting an error to a warning is a minor change (existing valid source
remains valid). Promoting a warning to an error is breaking (gated on
edition).

Adding a new diagnostic ID is minor as long as it can only be
triggered by code that previously was already flagged with a different
diagnostic, or by code that compiled but exhibited a bug.

---

## 5. Pre-release semantics

The repo will tag pre-release versions as `0.X.Y-alpha.N`,
`0.X.Y-beta.N`, `0.X.Y-rc.N`. Pre-release versions inherit no
stability guarantees from this document — they are tier 3 in their
entirety.

The first 1.0.0 release is the binding event. Until then, this
document describes the *intended* policy and the project tracks
violations in `CHANGELOG.md` as practice.

---

## 6. Deprecation policy (binding at 1.0)

A public surface is deprecated by:

1. Adding a `#[deprecated(since = "X.Y", note = "...")]` attribute
   in source (Rust) or a `"deprecated_since"` field in schema (JSON).
2. Emitting a warning when the surface is invoked.
3. Documenting the alternative in `CHANGELOG.md`.
4. Maintaining the deprecation through at least the next two minor
   releases (≥ 6 months at the planned cadence).
5. Removing in a major release with a migration guide.

A deprecation cannot skip the warning phase, even if the surface is
"obviously misused."

---

## 7. Security exceptions

A vulnerability fix may break tier-1 stability if and only if:
- The fix is documented in `SECURITY.md` with the threat model.
- The break is the minimum required to close the vulnerability.
- A migration path is documented.
- The break is announced with ≥ 7 days notice unless an active
  exploit makes that infeasible.

The intent is: a CVE fix can break compat in a patch release if the
alternative is leaving users vulnerable. We will not weaponise this
exception for non-security work.

---

## 8. Reference implementation versus specification

The Rust bootstrap (`crates/ori-compiler`) is the reference
implementation. The specification is `docs/language/SPECIFICATION.md`.
Where they disagree:

- **In pre-1.0**: the implementation wins; the spec gets corrected.
- **In post-1.0**: the spec wins; the implementation is patched.

Self-hosted stages (Stage 1, Stage 2) ship under the same stability
policy as the Rust bootstrap. The stage that is "canonical" in a
given release is named in the `ori doctor` output.

---

## 9. Stability of unstable areas

Areas marked "experimental" / "labs" / "draft" / "unstable" in source
or documentation are explicitly outside this policy. Examples today:

- `stdlib/labs/experimental.ori`.
- The textual codegen scaffold (`crates/ori-compiler/src/codegen_text.rs`).
- The local registry stub (`crates/ori-pkg/src/registry.rs`).
- Model-in-loop benchmark output formats (M33, not yet shipping).
- Self-hosting stage1/stage2 byte-equality (M36, planned).

These may change shape, break, or be removed without warning until
they're promoted to a tiered status.

---

## 10. How to file a stability question

Open an issue using `.github/ISSUE_TEMPLATE/feature_request.md` with
the label `stability`. The maintainers will respond with the relevant
tier and the timeline for resolution.

For breaking changes to tier 1 or tier 2 before 1.0, the process is
the same as feature work: an RFC (see `docs/rfcs/PROCESS.md`).

---

## 11. Cross-references

- [`README.md`](./README.md) — shipping inventory.
- [`SECURITY.md`](./SECURITY.md) — security policy + threat model.
- [`CHANGELOG.md`](./CHANGELOG.md) — record of every change.
- [`CONTRIBUTING.md`](./CONTRIBUTING.md) — developer workflow.
- [`GOAL.md`](./GOAL.md) — milestones and definition of production-ready.
- [`docs/ROADMAP.md`](./docs/ROADMAP.md) — delta to production grade.
- [`docs/rfcs/PROCESS.md`](./docs/rfcs/PROCESS.md) — RFC process for tier-1/tier-2 changes.
