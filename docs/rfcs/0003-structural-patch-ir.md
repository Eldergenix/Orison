# RFC 0003: Structural Patch IR

| Field           | Value                                                              |
| --------------- | ------------------------------------------------------------------ |
| RFC number      | 0003                                                               |
| Title           | Structural Patch IR                                                |
| Authors         | Orison core (BDFL: Eldergenix)                                     |
| Status          | Shipped                                                            |
| Pre-RFC issue   | n/a (retroactive)                                                  |
| PR              | n/a (retroactive)                                                  |
| Created         | 2026-05-17                                                         |
| FCP entered     | 2026-05-17                                                         |
| Merged          | 2026-05-17                                                         |
| Implemented     | bootstrap                                                          |
| Stabilised      | bootstrap                                                          |
| Supersedes      |                                                                    |
| Superseded by   |                                                                    |

> This RFC documents an already-shipping design retroactively. Every claim
> references the shipping implementation under
> [`crates/ori-compiler/src/patch.rs`](../../crates/ori-compiler/src/patch.rs) and
> [`crates/ori-compiler/src/patch_apply.rs`](../../crates/ori-compiler/src/patch_apply.rs).

---

## Table of contents

- [Summary](#summary)
- [Motivation](#motivation)
- [Detailed design](#detailed-design)
  - [Document shape](#document-shape)
  - [Operation taxonomy](#operation-taxonomy)
  - [Required fields per operation](#required-fields-per-operation)
  - [Validation pass: `ori patch check`](#validation-pass-ori-patch-check)
  - [Apply pass: `ori patch apply` and `ori patch dry-run`](#apply-pass-ori-patch-apply-and-ori-patch-dry-run)
  - [Partial-apply semantics](#partial-apply-semantics)
  - [Stale-target handling](#stale-target-handling)
  - [Fatal-error classification](#fatal-error-classification)
  - [Schemas and envelopes](#schemas-and-envelopes)
- [Drawbacks](#drawbacks)
- [Alternatives considered](#alternatives-considered)
- [Prior art](#prior-art)
  - [Operational Transformation](#operational-transformation)
  - [CRDT-based collaborative editing](#crdt-based-collaborative-editing)
  - [Tree-sitter edit ranges](#tree-sitter-edit-ranges)
  - [Language Server Protocol code actions](#language-server-protocol-code-actions)
  - [JSON Patch (RFC 6902)](#json-patch-rfc-6902)
- [Unresolved questions](#unresolved-questions)
- [Future possibilities](#future-possibilities)
- [Acceptance criteria](#acceptance-criteria)
- [Compatibility impact](#compatibility-impact)

---

## Summary

Patch IR is the canonical structural change format for Orison. A Patch IR document
is a small JSON object with a stable `schema` field, a human-readable `intent`, a
non-empty `operations` array, and an optional `tests` field. Each operation names
a target node by stable id (RFC 0001) and a kind from a closed taxonomy:
`replace_node`, `insert_node`, `delete_node`, `rename_symbol`, `add_import`,
`remove_import`, `change_signature`, `insert_match_arm`, `add_field`,
`remove_field`, `add_protocol_impl`, `update_route`, `update_view`, `add_test`.
Patches are validated (`ori patch check`), applied (`ori patch apply`), dry-run
(`ori patch dry-run`), and explained (`ori patch explain`) through a uniform JSON
contract. Stale targets fail the affected operation with `P1010` but other
operations in the same patch still apply (partial-apply); structural errors fail
the whole patch.

## Motivation

Agents and refactoring tools need a change format that is:

1. **Structurally addressed.** Targets are stable node ids (RFC 0001), not line
   numbers. The id of a function survives whitespace and unrelated edits, so a
   patch authored against revision R applies cleanly to revision R' even if
   intervening edits have shifted lines.
2. **Closed-vocabulary.** The set of operations is small, named, and reviewable.
   An agent cannot smuggle arbitrary text rewrites under the cover of a
   structured envelope.
3. **Partially-applicable.** A patch with five operations should succeed for the
   four whose targets are live and skip-with-diagnostic the one whose target has
   moved or been deleted. The all-or-nothing semantics of text diffs cause
   unnecessary retries.
4. **Round-trippable through the same JSON contract** the rest of the toolchain
   uses (capsules, maps, symbol cards, diagnostics).

The compound effect, measured in the loop: an LLM emits a Patch IR object,
`ori patch check` validates it in roughly 1.5 µs at p50 (see
[`BENCHMARKS.md`](../../BENCHMARKS.md)), `ori patch apply` resolves and applies
it without touching disk in dry-run mode. The edit-check-repair loop completes
in under 100 µs at p50 ([`GOAL.md`](../../GOAL.md) section 3.8).

## Detailed design

### Document shape

A Patch IR document is a JSON object with the following top-level fields:

| Field        | Required | Description                                                                 |
| ------------ | -------- | --------------------------------------------------------------------------- |
| `schema`     | yes      | The literal string `"ori.patch.v1"` (constant `PATCH_SCHEMA`)               |
| `intent`     | yes      | Non-empty human-readable string describing the goal of the patch            |
| `operations` | yes      | Non-empty array of operation objects                                        |
| `tests`      | no       | Optional object naming the tests that should run to validate the patch     |

The schema id is enforced by `check_patch_json` at
[`crates/ori-compiler/src/patch.rs`](../../crates/ori-compiler/src/patch.rs). Both
`schema` and `intent` are checked at the document level; both produce error-level
diagnostics if missing.

The `tests` field is optional but its absence produces a warning (`P0100`,
"patch file does not declare validation tests"). The intent is to nudge authors
toward declaring what proves the patch is correct without making it mandatory
for trivial edits.

### Operation taxonomy

The closed vocabulary is defined as the `KNOWN_OPERATIONS` constant in
`patch.rs`:

```
replace_node, insert_node, delete_node,
rename_symbol, add_import, remove_import,
change_signature, insert_match_arm,
add_field, remove_field,
add_protocol_impl,
update_route, update_view,
add_test
```

The vocabulary is closed by design. An `op` value not in this list fails with
`P1002` ("operation uses unknown op"). Adding a new operation kind requires an
RFC; see [`docs/rfcs/PROCESS.md`](./PROCESS.md) section 2.

The taxonomy partitions by structural intent:

- **Node-level structural edits**: `replace_node`, `insert_node`, `delete_node`.
  Address a `node:` id and rewrite, insert near, or delete the line(s) backing it.
- **Symbol-level edits**: `rename_symbol`, `change_signature`. Address a `sym:` id
  (or a `node:` id at the symbol declaration) and rewrite the symbol's header.
- **Import edits**: `add_import`, `remove_import`. `add_import` does not require
  a target (it is inserted after existing imports or after the `module`
  declaration via `insert_after_imports_or_module` in `patch_apply.rs`);
  `remove_import` targets the specific import node.
- **Type-shape edits**: `add_field`, `remove_field`, `add_protocol_impl`. Address
  the record or protocol declaration.
- **Match-expression edits**: `insert_match_arm`. Addresses a match expression
  and inserts a new `| pattern => body` arm.
- **Framework edits**: `update_route`, `update_view`. Address a route or view
  declaration and rewrite it.
- **Test edits**: `add_test`. Appends a test (no target required).

### Required fields per operation

The `required_fields` table in `patch.rs` is the source of truth:

| Op                   | Required fields                |
| -------------------- | ------------------------------ |
| `replace_node`       | `target`                       |
| `insert_node`        | `target`, `position`           |
| `delete_node`        | `target`                       |
| `rename_symbol`      | `from`, `to`                   |
| `add_import`         | `text`                         |
| `remove_import`      | `target`                       |
| `change_signature`   | `target`, `text`               |
| `insert_match_arm`   | `target`, `pattern`, `body`    |
| `add_field`          | `target`, `text`               |
| `remove_field`       | `target`                       |
| `add_protocol_impl`  | `target`, `text`               |
| `update_route`       | `target`, `text`               |
| `update_view`        | `target`, `text`               |
| `add_test`           | `text`                         |

A missing required field fails with `P1003` ("operation is missing required
field"). The diagnostic carries the field name in `expected` so a downstream
tool can render the fix automatically.

### Validation pass: `ori patch check`

`check_patch_json` performs purely structural validation. It does not parse the
target source, does not resolve ids, and does not write anywhere. Its outputs are:

- A `PatchCheckResult` carrying `valid: bool` and `diagnostics: Vec<Diagnostic>`.
- A canonical `ori.patch_check.v1` JSON envelope produced by `to_json`.

The diagnostic id space is:

| Id      | Level   | Meaning                                                |
| ------- | ------- | ------------------------------------------------------ |
| `P0000` | error   | Patch file is not valid JSON                           |
| `P0001` | error   | Patch file must declare `schema: ori.patch.v1`         |
| `P0002` | error   | Patch file must include at least one operation         |
| `P0003` | error   | Patch root must be a JSON object                       |
| `P0004` | error   | Patch file must include a non-empty `intent`           |
| `P0005` | error   | `operations` must be an array                          |
| `P0100` | warning | Patch file does not declare validation tests            |
| `P1000` | error   | Operation N must be an object                          |
| `P1001` | error   | Operation N is missing `op`                            |
| `P1002` | error   | Operation N uses unknown op                            |
| `P1003` | error   | Operation N is missing a required field                |

### Apply pass: `ori patch apply` and `ori patch dry-run`

`apply_patch` at
[`crates/ori-compiler/src/patch_apply.rs`](../../crates/ori-compiler/src/patch_apply.rs)
resolves each operation's `target` against the parsed CST and AST of the source
file. The `resolve_line` closure recognises three id prefixes:

- `node:` — looked up in the CST via `Cst::find`.
- `sym:` — looked up in the AST via `Module::symbols.iter().find`.
- `mod:` — looked up as the module's own symbol id, with a `sym:` to `mod:`
  fallback.

The pass is **a pure function over its inputs**: even in non-`dry_run` mode,
`apply_patch` does not write to disk. The caller (the CLI) is responsible for
persisting the `after` field of the report.

Position handling for `insert_node` accepts:

- The bare keywords `"before"` and `"after"` (default if absent: `"after"`).
- The directives `"after:<id>"` or `"before:<id>"`, which override the anchor.

This is what makes "insert this arm immediately after the existing `Some(_)`
arm" expressible as a single operation.

### Partial-apply semantics

When a patch has multiple operations and some targets are stale, the bootstrap
**applies the operations whose targets are live and skips the ones whose
targets are stale**, producing a single `PatchApplyReport` that records:

- `operations_attempted` — total ops in the patch.
- `operations_applied` — ops that applied successfully.
- `diagnostics` — one `P1010` per stale-target skip, plus any fatal errors.
- `applied: bool` — `true` iff at least one op applied and no fatal error
  occurred.
- `after: Option<String>` — present only when `applied` is `true`.

The contract is enforced by the fatal-error classification in `apply_patch`:

```rust
let fatal_codes = ["P1000", "P1001", "P1002", "P1003"];
let has_fatal = diagnostics
    .iter()
    .any(|d| d.is_error() && fatal_codes.iter().any(|c| d.id == *c));
```

`P1010` (stale target) is deliberately excluded from the fatal list. This is the
key semantic property that distinguishes Patch IR from a text-diff: the
operations are independent and may individually succeed or fail.

### Stale-target handling

A stale target — an id that does not resolve against the current CST/AST —
produces `P1010` ("operation references unknown node id"):

```
Diagnostic::error(
    "P1010",
    format!("operation {index} references unknown node id `{target}`"),
    Span::dummy(path.to_string()),
)
.with_expected(vec!["a node id present in the current CST".to_string()])
.with_found(vec![target.to_string()])
.with_agent_summary("Re-resolve target ids against the latest CST before applying.")
.with_docs(vec!["doc:patch.targets".to_string()])
```

The diagnostic instructs the agent to re-resolve, which it can do cheaply via
`ori agent map`. This is the loop's repair step.

### Fatal-error classification

A patch is fatal when any operation is structurally malformed (`P1000`,
`P1001`, `P1003`) or uses an unsupported op (`P1002`). Fatal errors abort the
whole patch (`applied: false`, `after: None`) because partial application of a
structurally invalid document would leave the source in an undefined state.

By contrast, stale targets are **per-op**: other operations in the same patch
still apply, and the report enumerates which.

### Schemas and envelopes

Two shipped envelopes carry the Patch IR contract:

- `ori.patch.v1` — the input document schema, at
  [`schemas/patch.schema.json`](../../schemas/patch.schema.json).
- `ori.patch_check.v1` — the validation report, at
  [`schemas/patch-check.schema.json`](../../schemas/patch-check.schema.json).

`PatchApplyReport` carries `schema: "ori.patch_apply.v1"` in its serialised
form; the file naming for that schema follows the same convention.

The doctor report enumerates these schemas alongside every other contract; any
agent that needs to discover the active schema set can call `ori doctor` rather
than scanning the source tree.

## Drawbacks

1. **Line-based editing under the hood.** The bootstrap apply engine resolves
   target ids to source line numbers and operates with `insert_line`,
   `replace_line`, `delete_line` helpers. This works because the structural
   addressing is what is stable; the line is just where the resolved node
   currently lives. The trade is that operations that span multiple lines per
   node may produce slightly less-clean output than a full CST round-trip
   would. Future work can move to a CST round-trip without changing the public
   contract.
2. **Closed vocabulary requires RFCs to extend.** Adding a new operation kind
   is a process-bound change. This is by design (every public surface change is
   reviewed) but does mean that one-off custom operations are not supported;
   authors must compose existing ops.
3. **No batching across files in a single document.** The bootstrap applies
   patches one source file at a time. A multi-file refactor is a sequence of
   single-file patches. Multi-file Patch IR is a future-possibility item.
4. **The `tests` field is opt-in.** A patch with no `tests` field produces only
   a warning. This is a deliberate trade between author burden and audit
   strength; the warning is the cue.

## Alternatives considered

- **Text-diff (unified diff or git patches).** Rejected because text-diff
  drifts on every unrelated edit and supports no partial apply that does not
  involve fuzzy matching. Fuzzy matching is exactly what the agent loop must
  not depend on.
- **Tree-sitter edits.** Tree-sitter offers `(start_byte, old_end_byte,
  new_end_byte, start_position, old_end_position, new_end_position)` edit
  records. These describe what changed but not the intent; they cannot be
  validated structurally without applying them; and they tie the consumer to
  tree-sitter's parser state.
- **CRDT or operational transform.** OT and CRDTs solve a different problem
  (concurrent collaborative editing). They do not naturally express "structural
  refactor" intent and they do not validate cleanly out-of-band. See
  [Prior art](#prior-art) below.
- **LSP code actions only.** Code actions ship as `WorkspaceEdit` documents,
  which are text-range based. Orison does emit code actions, but the `data`
  field round-trips a Patch IR document so the same contract serves both the
  LSP and CLI paths.
- **A free-form "describe the change" field consumed by an LLM at apply
  time.** Rejected because it would couple the toolchain to a specific model
  and would not be auditable as a deterministic transformation.

## Prior art

### Operational Transformation

OT (originated for collaborative editing in the early '90s, see Ellis &
Gibbs 1989) represents edits as text operations and transforms concurrent
operations against each other to converge. The transformation rules are
notoriously hard to get right, and the abstraction has nothing to say about
structural intent. Patch IR is solving the structural-intent problem; OT is
solving the convergence problem. They are complementary, not alternatives.

### CRDT-based collaborative editing

Yjs, Automerge, and similar CRDTs solve the same convergence problem as OT
with a different theoretical foundation. They still operate at the text or
sequence level and still do not express structural intent. Patch IR sits one
level up.

### Tree-sitter edit ranges

Tree-sitter is a parser library used in many editors. Its `TSInputEdit`
struct describes byte/position ranges that changed. Patch IR is closer to a
*refactoring engine* than to a parser-edit format: the focus is on what the
author meant, not on what bytes changed.

### Language Server Protocol code actions

LSP code actions are the closest production design to Patch IR. They ship a
`WorkspaceEdit` containing per-document text edits. Orison's LSP integration
emits code actions whose `data` field carries a full Patch IR document so that
the round-trip through `applyEdit` does not lose structural addressing. The
Patch IR contract is what the LSP wraps, not a replacement.

### JSON Patch (RFC 6902)

RFC 6902 defines `add`, `remove`, `replace`, `move`, `copy`, `test` as
operations over JSON documents. The shape is similar in spirit; the scope is
not. JSON Patch targets JSON values via JSON Pointer; Orison's Patch IR
targets *source code nodes* via stable structural ids. The vocabulary,
validation, and apply semantics are all source-code-specific.

## Unresolved questions

- Should multi-file patches be expressible as a single Patch IR document, or
  remain a sequence of per-file documents?
- Should `add_test` accept a target id to control insertion location rather
  than always appending?
- Should the apply pass move from line-based to CST-round-trip insertion for
  multi-line operations?
- Should there be an `op: "noop"` to allow authors to label intent without
  producing an edit (useful for staged plans)?

## Future possibilities

- **CST-round-trip apply.** Replace `insert_line`/`replace_line`/`delete_line`
  with operations that splice the CST and re-emit the source.
- **Multi-file Patch IR** as a single document with per-file operation groups.
- **Patch composition.** A `compose` operator that builds a Patch IR document
  from a sequence of named sub-patches.
- **Patch IR diffing.** Given two source revisions, derive the Patch IR
  document that transforms one into the other (the inverse of apply). Useful
  for review tooling that wants to display a structural rather than textual
  diff.

## Acceptance criteria

- [x] `PATCH_SCHEMA = "ori.patch.v1"` and `PATCH_CHECK_SCHEMA = "ori.patch_check.v1"`
      are stable constants in
      [`crates/ori-compiler/src/patch.rs`](../../crates/ori-compiler/src/patch.rs).
- [x] `KNOWN_OPERATIONS` enumerates exactly the 14 operation kinds in
      [Operation taxonomy](#operation-taxonomy).
- [x] `check_patch_json` enforces document shape with diagnostics `P0000`
      through `P0005` and operation shape with `P1000` through `P1003`
      ([`crates/ori-compiler/src/patch.rs`](../../crates/ori-compiler/src/patch.rs)).
- [x] `apply_patch` resolves `node:`, `sym:`, and `mod:` ids through the
      shared `resolve_line` closure
      ([`crates/ori-compiler/src/patch_apply.rs`](../../crates/ori-compiler/src/patch_apply.rs)).
- [x] Stale targets produce `P1010` per-op and do not abort the patch
      ([`crates/ori-compiler/src/patch_apply.rs`](../../crates/ori-compiler/src/patch_apply.rs);
      see the `fatal_codes` list).
- [x] Fatal errors (`P1000`, `P1001`, `P1002`, `P1003`) abort the patch and
      produce `applied: false, after: None`.
- [x] The full workspace test suite exercises both `patch.rs` and
      `patch_apply.rs` (the suite is the gate; see
      [`CONTRIBUTING.md`](../../CONTRIBUTING.md)).
- [x] `schemas/patch.schema.json` and `schemas/patch-check.schema.json` are
      shipped and validated by the static gate.

## Compatibility impact

This RFC documents the bootstrap state and is non-breaking by definition.

Future evolution is governed by the rules in
[`docs/rfcs/PROCESS.md`](./PROCESS.md):

- Adding a new operation kind is additive but requires an RFC under
  [section 2](./PROCESS.md#2-when-an-rfc-is-required) (new public surface).
- Changing the required-fields table for an existing op is breaking and
  requires the schema-version path under
  [section 8.2](./PROCESS.md#82-schema-breaking-changes).
- Changing the partial-apply or fatal-error classification is a behaviour
  change to a public contract and requires an RFC.
- The `ori.patch.v1`, `ori.patch_check.v1`, and `ori.patch_apply.v1` schemas
  are subject to the schema-breaking-change rules.
