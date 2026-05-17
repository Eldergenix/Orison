# RFC 0001: Stable node IDs

| Field           | Value                                                              |
| --------------- | ------------------------------------------------------------------ |
| RFC number      | 0001                                                               |
| Title           | Stable node IDs                                                    |
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

> This RFC documents an already-shipping design retroactively, so that subsequent
> changes to node identity have a written baseline to evolve from. Every claim in
> this document references the shipping implementation in
> [`crates/ori-compiler/src/node_id.rs`](../../crates/ori-compiler/src/node_id.rs).

---

## Table of contents

- [Summary](#summary)
- [Motivation](#motivation)
- [Detailed design](#detailed-design)
  - [Encoding](#encoding)
  - [Construction](#construction)
  - [Hashing](#hashing)
  - [Stability properties](#stability-properties)
  - [Module-, symbol-, and CST-level scopes](#module--symbol--and-cst-level-scopes)
  - [Where IDs are produced and consumed](#where-ids-are-produced-and-consumed)
- [Drawbacks](#drawbacks)
- [Alternatives considered](#alternatives-considered)
  - [Line and column positions](#line-and-column-positions)
  - [Sequential per-file counters](#sequential-per-file-counters)
  - [Cryptographic hash of node text](#cryptographic-hash-of-node-text)
  - [UUIDs persisted in the source file](#uuids-persisted-in-the-source-file)
- [Prior art](#prior-art)
- [Unresolved questions](#unresolved-questions)
- [Future possibilities](#future-possibilities)
- [Acceptance criteria](#acceptance-criteria)
- [Compatibility impact](#compatibility-impact)

---

## Summary

Every node in an Orison concrete syntax tree (CST) carries a stable, human-readable
identifier of the form `node:<module>.<kind>.<name>.<disc>`, where `<disc>` is a
16-character hex digest derived from a salted FNV-1a hash of
`(parent_id, kind, name, sibling_index, signature)`. The same identity scheme is
extended to symbol-level IDs (`sym:`) and module-level IDs (`mod:`). The IDs are the
canonical address for everything that points at a piece of source code: Patch IR
operations, agent capsules, symbol cards, code-action `data` fields, and doctor
reports. They survive whitespace edits, comment edits, and edits to unrelated
siblings in the same file.

## Motivation

Every other production language treats text as the canonical change unit. Diffs,
patches, line numbers, and snippets are all text-coordinate. This is fine for
humans; it is corrosive for agents:

1. **Line numbers drift.** Every comment, blank line, or unrelated edit invalidates
   downstream references. An agent that emits "modify line 42" against a file that
   has since gained two imports is wrong without recourse.
2. **Snippets are ambiguous.** "Edit the `handle` function" matches every overload,
   every shadowing definition, and every helper that happens to be named `handle`.
3. **Round-tripping is lossy.** When an agent emits a JSON instruction, the
   compiler has no efficient way to confirm "you mean *this* exact node" without
   re-parsing the surrounding text.

The compound effect is that agent-driven workflows incur a tax on every step: more
context to read, more clarification round-trips, and more retries when a stale
coordinate slips through.

Orison's wedge is the edit-check-repair loop (see [`GOAL.md`](../../GOAL.md)
section 1). A stable identity for every node in the tree is the foundation on which
the rest of the loop is built: Patch IR (RFC 0003) addresses nodes by id; agent
maps emit ids; capsules summarise them; symbol cards link to them; LSP code actions
carry them in the `data` field for round-trip; doctor reports name them when
diagnosing inconsistencies.

## Detailed design

### Encoding

A node id is a string with the prefix `node:` followed by four dot-separated
components:

```
node:<module>.<kind>.<name>.<disc>
```

- `<module>` is the source module name (the value after `module` in the source).
- `<kind>` is a short, stable categorical label such as `fn`, `record`, `route`,
  `view`, `arm`, `import`.
- `<name>` is the user-visible identifier for the node, or a placeholder such as
  `_anon` for nodes that have no name.
- `<disc>` is a 16-character lowercase hex string, the FNV-1a 64-bit digest of the
  structural fingerprint described under [Construction](#construction).

The format is intentionally readable. A maintainer skimming a Patch IR document
can tell at a glance which node a `target` refers to without consulting any other
artefact. Agents do not need a translation table.

Symbol-level identifiers use the prefix `sym:<module>.<name>` and module-level
identifiers use `mod:<module>`. The Patch IR apply pass at
[`crates/ori-compiler/src/patch_apply.rs`](../../crates/ori-compiler/src/patch_apply.rs)
resolves all three prefixes — `node:`, `sym:`, and `mod:` — through a shared
`resolve_line` closure so any tool may use whichever scope is most convenient.

### Construction

`make_node_id` in
[`crates/ori-compiler/src/node_id.rs`](../../crates/ori-compiler/src/node_id.rs)
takes five inputs:

```rust
pub fn make_node_id(
    module: &str,
    parent: Option<&NodeId>,
    kind: &str,
    name: &str,
    sibling_index: usize,
    signature: &str,
) -> NodeId
```

The discriminant is `fnv1a_64_str(&[parent_str, kind, name, sibling_index_str,
signature])`. Including the parent id makes the fingerprint hierarchical:
identical-looking nodes in different scopes get different ids. Including the
sibling index makes duplicate-named siblings (which the parser does not prohibit
in all positions) distinguishable. Including the signature makes overload-like
distinctions (e.g. two `fn handle` differing only by argument types) collision-free.

### Hashing

The hash function is FNV-1a 64-bit, also implemented in `node_id.rs`:

```rust
const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_PRIME: u64 = 0x100_0000_01b3;

pub fn fnv1a_64(bytes: &[u8]) -> u64 { /* ... */ }
pub fn fnv1a_64_str(parts: &[&str]) -> u64 { /* parts joined by '|' */ }
```

FNV-1a was chosen because:

1. It is deterministic across platforms and Rust versions.
2. It has no third-party dependency (the bootstrap dep policy is
   `serde + serde_json` only).
3. It is fast enough that hashing the entire CST is well inside the sub-100 µs
   budget the loop allots to a single check (see
   [`BENCHMARKS.md`](../../BENCHMARKS.md)).
4. The collision risk at the scale of a single source file is negligible; the
   tuple `(parent_id, kind, name, sibling_index, signature)` already disambiguates
   the structural axes, so the hash is collision-resistant *within a file* by
   construction.

FNV-1a is **not cryptographic**. This is acknowledged in
[`SECURITY.md`](../../SECURITY.md): the bootstrap does not defend against
sophisticated supply-chain attacks via hash collisions. Node ids are an identity
mechanism for tooling, not an integrity mechanism.

### Stability properties

The following properties hold and are exercised by unit tests in `node_id.rs` and
by integration tests under `crates/ori-compiler/tests/` and
`crates/ori-cli/tests/`:

1. **Whitespace-stable.** Adding or removing blank lines, comments, or indentation
   inside or around a node does not change its id, because none of those inputs
   feed into the fingerprint.
2. **Sibling-stable.** Adding an unrelated sibling to a parent shifts the
   `sibling_index` of nodes after the insertion point, so those nodes get new ids.
   Nodes before the insertion point and in unrelated parents are unaffected.
   (This is a deliberate trade: full re-stability under sibling insertion would
   require persisting a per-node UUID in the source, which we have explicitly
   rejected — see [Alternatives considered](#alternatives-considered).)
3. **Signature-sensitive.** Two functions with the same module, parent, kind,
   name, and sibling index but different signatures get different ids. Confirmed
   by `ids_differ_for_signature` in `node_id.rs`.
4. **Deterministic across runs.** Same input, same id, on every platform the
   workspace targets. Confirmed by `ids_are_stable_for_same_inputs` and
   `fnv1a_is_deterministic`.

### Module-, symbol-, and CST-level scopes

The same fingerprint approach scales to three layers:

- **CST nodes** (`node:`) — produced by `parse_cst` in
  [`crates/ori-compiler/src/cst.rs`](../../crates/ori-compiler/src/cst.rs).
- **Symbol nodes** (`sym:`) — every public symbol in a module's AST has an id
  derived from the same fingerprint at the symbol level.
- **Modules** (`mod:`) — top-level identifier for the module declaration.

The Patch IR apply engine resolves all three prefixes via the shared `resolve_line`
closure at
[`crates/ori-compiler/src/patch_apply.rs:103`](../../crates/ori-compiler/src/patch_apply.rs).
This is what makes "address by id" a uniform contract regardless of how coarse or
fine-grained the calling tool needs to be.

### Where IDs are produced and consumed

Production sites:

- `parse_cst` constructs CST node ids.
- `parse_source` constructs symbol and module ids during AST construction.
- Agent capsules and maps emit ids in their JSON envelopes.
- Diagnostics carry `symbol` ids in the `symbol` field where applicable.

Consumption sites:

- Patch IR `target` fields (RFC 0003).
- LSP code-action `data` fields, so that `applyEdit` round-trips back to the
  compiler with the original id.
- Doctor reports, when listing inconsistencies between expected and observed
  contracts.
- Agent maps emitted by `ori agent map`.
- Symbol cards emitted by `ori agent symbol` and friends.

## Drawbacks

1. **Sibling-index drift.** Inserting a sibling shifts ids for everything after it
   in the same parent. Tools that store ids long-term must be prepared to
   re-resolve. The trade is acceptable because the alternative (persisting UUIDs
   in source) violates the "source files are authored by humans" property.
2. **Signature dependence.** Changing a function's signature changes its id, so
   tools that stored the old id will receive a stale-target diagnostic
   (`P1010`, see RFC 0003). This is by design: a signature change is a semantic
   change, and consumers should re-resolve.
3. **No cryptographic guarantees.** FNV-1a is not collision-resistant against an
   adversary. Within a single file the structural tuple removes the need for
   cryptographic strength; across files an adversary could in principle craft a
   collision, but a collision is harmless because the `<module>` and `<name>`
   prefixes are part of the visible id, so the collision would only affect the
   `<disc>` tail.
4. **Readable but verbose.** A typical id is ~40 to 60 characters. This is fine
   for JSON payloads but inflates the size of agent maps over a large workspace.
   Mitigated by the budget-limited `ori agent map --budget N` envelope.

## Alternatives considered

### Line and column positions

Tools could address nodes by `path:line:col`. This is the de facto standard in
LSP, gcc, rustc, and almost every other compiler.

Rejected because: line/column coordinates drift on every edit. The whole motivating
problem is precisely that drift; a system that solves it by pretending it does not
exist cannot help the agent loop.

### Sequential per-file counters

Each node gets an integer that increments in source order. New nodes get the next
free integer.

Rejected because: the counter must either be persisted in the source (rejected,
see UUIDs below) or recomputed on every parse (in which case it is just a more
opaque version of line numbers — equally drift-prone).

### Cryptographic hash of node text

The id is the SHA-256 of the canonicalised node text.

Rejected because:

1. SHA-256 is a third-party dependency in Rust (`sha2`) and the bootstrap dep
   policy forbids new deps without an RFC and two-maintainer ack
   (see [`docs/rfcs/PROCESS.md`](./PROCESS.md) section 8.1).
2. Cryptographic strength is unnecessary for a tooling identity scheme.
3. SHA-256 of text bakes in formatting; the canonicalisation step would
   re-introduce FNV-1a-like considerations to be deterministic.

### UUIDs persisted in the source file

Each node gets a UUID stored in source, e.g. as a comment annotation.

Rejected because: it makes source files non-human-authored. The whole point of
Orison is that source remains the human-readable artefact; tools derive everything
they need from it.

## Prior art

- **Tree-sitter** uses `(kind, byte_range)` as its node identity. Drift on edits
  is handled by the incremental re-parse, but consumers outside the parse session
  cannot address nodes stably.
- **Rust** uses `DefId` and `HirId` internally. Both are session-scoped integers,
  not designed for cross-tool stability.
- **Swift** uses `USR` (Unified Symbol Resolution) strings for cross-module
  identity. Those are symbol-level, not node-level, but the human-readable string
  approach mirrors what Orison does at `sym:` scope.
- **Clojure tools.deps** uses content hashes for dependency identity. Same
  hash-based principle, different scope.
- **Operational transform** and **CRDT** literature treat node identity as the
  central problem of collaborative editing. Orison's structural-fingerprint
  approach is closer to the "stable IDs with structural lineage" branch (e.g.
  Yjs's `Item.id` is `(client, clock)`) than to the position-transformation
  branch.

## Unresolved questions

- Should there be a compatibility shim that accepts the old line/column address
  form during a transition period? (Currently no; tools either speak ids or fall
  back to source coordinates internally.)
- Should the discriminant include a salt that differs per workspace, so that ids
  from two unrelated projects cannot accidentally collide? (Currently no; the
  collision risk is acceptable.)

## Future possibilities

- Persist a content-addressable index of `(id, source-range)` to disk for
  cross-process consumers (LSP, editor extensions).
- Promote `mod:` and `sym:` to first-class targets for additional Patch IR
  operations beyond the current set (today they can be addressed for line-level
  ops; more granular operations would benefit from sub-symbol structural ids
  that the current scheme already provides via `node:`).
- Surface ids in formatted CLI output as clickable URIs for editor integrations.

## Acceptance criteria

- [x] `make_node_id` produces deterministic, format-insensitive ids
      (`ids_are_stable_for_same_inputs` in
      [`crates/ori-compiler/src/node_id.rs`](../../crates/ori-compiler/src/node_id.rs)).
- [x] Sibling-index changes produce distinct ids
      (`ids_differ_for_sibling_index`).
- [x] Signature changes produce distinct ids
      (`ids_differ_for_signature`).
- [x] FNV-1a hash is deterministic across runs (`fnv1a_is_deterministic`).
- [x] Patch IR resolves `node:`, `sym:`, and `mod:` targets uniformly through
      the `resolve_line` closure in
      [`crates/ori-compiler/src/patch_apply.rs`](../../crates/ori-compiler/src/patch_apply.rs).
- [x] The full workspace test suite passes
      (`cargo test --workspace` is the gate; see
      [`CONTRIBUTING.md`](../../CONTRIBUTING.md)).

## Compatibility impact

This RFC documents the bootstrap state. There is no compatibility impact from the
RFC itself; the implementation predates the document.

Any future change to the id scheme — adding a component to the fingerprint,
changing the hash, changing the encoding — is a breaking change to the agent ABI
and will require a follow-up RFC with:

- A new schema version for every envelope that carries ids (capsule, agent map,
  patch, symbol card).
- A deprecation period during which both encodings are emitted.
- A migration tool that rewrites stored ids.
