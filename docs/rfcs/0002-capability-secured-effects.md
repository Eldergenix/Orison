# RFC 0002: Capability-secured effects

| Field           | Value                                                              |
| --------------- | ------------------------------------------------------------------ |
| RFC number      | 0002                                                               |
| Title           | Capability-secured effects                                         |
| Authors         | Orison core (BDFL: Eldergenix)                                     |
| Status          | Shipped (static enforcement). Runtime gating is M48.               |
| Pre-RFC issue   | n/a (retroactive)                                                  |
| PR              | n/a (retroactive)                                                  |
| Created         | 2026-05-17                                                         |
| FCP entered     | 2026-05-17                                                         |
| Merged          | 2026-05-17                                                         |
| Implemented     | bootstrap (static-analysis layer only)                             |
| Stabilised      | bootstrap (static layer); runtime layer deferred to M48            |
| Supersedes      |                                                                    |
| Superseded by   |                                                                    |

> This RFC documents an already-shipping design retroactively. Every claim
> references the shipping implementation under
> [`crates/ori-compiler/src/`](../../crates/ori-compiler/src/) and
> [`crates/ori-pkg/src/`](../../crates/ori-pkg/src/).

---

## Table of contents

- [Summary](#summary)
- [Motivation](#motivation)
- [Detailed design](#detailed-design)
  - [Layer 1: per-function effects](#layer-1-per-function-effects)
  - [Layer 2: package capability policy](#layer-2-package-capability-policy)
  - [Layer 3: call-graph propagation](#layer-3-call-graph-propagation)
  - [Layer 4: audit at the package boundary](#layer-4-audit-at-the-package-boundary)
  - [Layer 5: target-level capability projection](#layer-5-target-level-capability-projection)
  - [Diagnostic id space](#diagnostic-id-space)
  - [Non-negotiable invariants this design preserves](#non-negotiable-invariants-this-design-preserves)
- [Drawbacks](#drawbacks)
- [Alternatives considered](#alternatives-considered)
- [Prior art](#prior-art)
  - [Koka](#koka)
  - [Effekt](#effekt)
  - [Rust](#rust)
  - [Pony, Newspeak, and the object-capability lineage](#pony-newspeak-and-the-object-capability-lineage)
  - [WASI](#wasi)
- [Unresolved questions](#unresolved-questions)
- [Future possibilities](#future-possibilities)
- [Acceptance criteria](#acceptance-criteria)
- [Compatibility impact](#compatibility-impact)

---

## Summary

Every Orison function declares the effects it performs with a `uses ...` clause.
Every package declares the capabilities it grants under `[capabilities].declared`
in `ori.toml`. The compiler enforces both layers statically: a function transitively
performing an effect it has not declared fails with `E0420`; a function declaring an
effect that the package policy does not grant fails with `E0410`; a dependency
requiring a capability the root has not granted fails the audit with `AUD0001`.
Runtime gating is staged for M48; the bootstrap is static-analysis only.

## Motivation

Existing production languages take one of two positions on effects:

1. **Invisible.** Python, Go, TypeScript, and most other mainstream languages
   leave effects entirely implicit. Any function may open a socket, write a file,
   or read an environment variable; the type system does not record it. The result
   is that supply-chain attacks have a free hand and security audits are manual.
2. **Fully type-system-encoded.** Haskell, Koka, and Effekt encode effects in the
   type system. This is rigorous, but the cognitive load is high and the package
   boundary still leaks: a package whose internal types reference `IO` does not
   tell you what capabilities the deployable will demand.

Orison takes a third position: effects are first-class on functions *and the
package boundary is the capability contract*. The compiler enforces a two-layer
discipline:

- At the function layer, every effect a function performs (directly or
  transitively) must be in its `uses` clause.
- At the package layer, every effect any function in the package performs must be
  declared in `ori.toml` under `[capabilities].declared`.

The result is that a maintainer reading a single `ori.toml` knows exactly which
capabilities the deployable demands, and a CI gate (`ori audit`) prevents
dependency creep — a transitive dependency cannot quietly acquire `fs.write`
because the root must explicitly grant it.

This is consistent with the non-negotiable invariant in
[`GOAL.md`](../../GOAL.md) section 3.2: **no ambient capabilities**. There is no
`open()` that just works; the call site must declare `fs.read` and the package
must grant it.

## Detailed design

### Layer 1: per-function effects

A function declares its effects with a `uses ...` clause:

```ori
fn post_checkout(cart: Cart) -> Result[Order, CheckoutError] uses http, db.write
```

The set of recognised effect names lives in
[`crates/ori-compiler/src/effects.rs`](../../crates/ori-compiler/src/effects.rs)
as the constant `KNOWN_EFFECTS`. Named capabilities (identifiers starting with an
ASCII uppercase letter) are recognised in addition; the helper
`is_known_effect_or_capability` implements the dual recognition rule.

An effect name that is neither in `KNOWN_EFFECTS` nor a capability identifier
emits the warning `W0401` ("unknown effect or capability") via
`unknown_effect_diagnostic` in
[`crates/ori-compiler/src/effect_check.rs`](../../crates/ori-compiler/src/effect_check.rs).
The diagnostic carries the full known list in `expected`, the observed name in
`found`, and an agent summary instructing the reader to declare the capability or
use a known effect name.

### Layer 2: package capability policy

A package's `ori.toml` carries:

```toml
[capabilities]
declared = ["http", "db.read", "db.write"]
```

The compiler's effect-check pass (`effect_diagnostics` in
`effect_check.rs`) takes a `declared_policy: &[String]` argument and:

- Skips the policy check entirely when `declared_policy` is empty (so that
  unconfigured packages and tests do not get spurious errors).
- For every symbol with at least one effect not in `declared_policy`, emits
  `E0410` ("effect ... is used by ... but is not in the package capability
  policy") via `undeclared_effect_diagnostic`. The diagnostic carries the symbol
  id, the expected fix ("declare ... in [capabilities].declared"), and a
  pointer to `doc:effects.policy`.

The package's `[capabilities].declared` list is the authoritative answer to the
question "what does this deployable demand?" The list is reported in the capability
manifest (`ori.capability.v1`) produced by `build_capability_manifest` in
`effect_check.rs`. The manifest also reports two diff fields:

- `policy.undeclared` — effects observed but not declared.
- `policy.unused` — effects declared but not observed.

### Layer 3: call-graph propagation

Per-function declarations are not enough on their own: a caller can transitively
perform an effect because one of its callees does. The propagation pass at
[`crates/ori-compiler/src/effect_propagate.rs`](../../crates/ori-compiler/src/effect_propagate.rs)
closes the gap:

1. `build_effect_graph` walks every function body, collects every
   `Call(Var(name), _)` whose `name` matches another function symbol in the same
   module, and seeds the per-function effect set with the declared `uses` clause.
2. `propagate_effects` iterates to a fixpoint: a function's effective effect set
   becomes the union of its declared effects and the effective effects of every
   transitively reachable callee. Cycles are handled naturally because the
   iteration stops when no set grows.
3. `propagation_diagnostics` turns each "inferred ⊋ declared" gap into an
   `E0420` diagnostic carrying:
   - A human message naming the offending callee and effect.
   - The symbol id.
   - The expected effect set in `expected`.
   - The currently-declared set in `found`.
   - A `change_signature` Patch IR fix (RFC 0003) that appends the missing
     effect to the function's `uses` clause. The fix is embedded as a complete
     Patch IR document with `schema: "ori.patch.v1"`, an intent describing the
     change, a single `change_signature` operation targeting the symbol id, and
     a `tests.run` field naming the relevant cargo test.

The propagation pass is **module-local in the bootstrap**: cross-module calls are
not yet traversed. This is acknowledged in the module-level documentation; it is
sufficient for the bootstrap because the package boundary check at layer 4 still
catches any escape.

### Layer 4: audit at the package boundary

The package manager runs `run_audit` at
[`crates/ori-pkg/src/audit.rs`](../../crates/ori-pkg/src/audit.rs) to compare:

- The root manifest's `[capabilities].declared` list.
- Every dependency manifest's `[capabilities].declared` list.

Three rules fire:

- `AUD0001` (error) — a dependency requires a capability the root has not
  granted. Identified by `package:<name>@<version>` in the finding `target`.
- `AUD0002` (info) — the root declares a capability that no dependency requires.
- `AUD0003` (warn) — duplicate package versions in the resolved graph
  (capability-relevant because two versions may differ in their required
  capabilities).

The audit emits `ori.audit_report.v1`. The exhaustive enumeration is in the
constant `AUDIT_RULES`.

Findings sort by `(severity, id, target)` so the JSON envelope is byte-stable
across runs — a property the agent ABI depends on.

### Layer 5: target-level capability projection

For mobile builds, the same per-function effect declarations project into the
mobile manifest's `permissions` array via
[`crates/ori-compiler/src/mobile.rs`](../../crates/ori-compiler/src/mobile.rs).
For example, `net.outbound | net.inbound | http | net` all project to the
`network` permission. The projection is deterministic and tested via
`net_outbound_produces_network_permission` and friends in the same file.

This is what makes "the compiler-emitted manifest at each target gates which
capabilities the deployable receives" (see [`GOAL.md`](../../GOAL.md) section 2.5)
true in practice: the source declares effects, the package grants capabilities,
the target manifest expresses the union in the form the target platform
understands.

### Diagnostic id space

| Id          | Level   | Pass                | Meaning                                                       |
| ----------- | ------- | ------------------- | ------------------------------------------------------------- |
| `W0401`     | warning | `effect_check`      | Unknown effect or capability name                             |
| `E0410`     | error   | `effect_check`      | Effect used by symbol but not in package policy               |
| `E0420`     | error   | `effect_propagate`  | Caller transitively requires effect not declared on signature |
| `AUD0001`   | error   | `audit`             | Dependency requires capability root has not granted           |
| `AUD0002`   | info    | `audit`             | Root declares capability no dependency requires               |
| `AUD0003`   | warn    | `audit`             | Duplicate package versions in graph                           |

### Non-negotiable invariants this design preserves

- **No ambient capabilities** ([`GOAL.md`](../../GOAL.md) section 3.2). Every
  effect performed is named at three layers: signature, package, manifest.
- **Schemas are public APIs** ([`GOAL.md`](../../GOAL.md) section 3.3). The two
  envelopes this design produces — `ori.capability.v1` and `ori.audit_report.v1`
  — both ship as Draft 2020-12 schemas under
  [`schemas/`](../../schemas/) and are validated by the static gate.
- **Anything the compiler knows is available to tools** ([`GOAL.md`](../../GOAL.md)
  section 3.4). Both envelopes are JSON-serialised typed structs; `ori audit`
  and `ori capability` emit them via `to_json`.

## Drawbacks

1. **Module-local propagation.** The bootstrap does not traverse cross-module
   call edges. A caller in module A that calls into module B's function with an
   effect will not pick up the effect at layer 3, only at layer 4 (the audit).
   This is acceptable because the audit catches escape, but it does mean a
   developer iterating in a single module may not see an `E0420` until they
   commit to a build.
2. **Effect-name typos cost a warning, not an error, by default.** `W0401` is a
   warning when no policy is declared. This is intentional (so that tests and
   small examples can use any effect name freely) but it means the type check
   alone does not catch typos in effect names; the policy check does.
3. **Fix patches are signature-string edits.** `append_effect_to_uses` is a
   defensive string editor over the original signature text. It does not
   re-pretty-print the function header. For unusual signature formatting the
   result may be syntactically odd; it will still parse.
4. **Runtime is unguarded in the bootstrap.** Static analysis prevents the call
   from being written; it does not prevent a sufficiently determined runtime
   from performing the effect via a path the analysis does not see (e.g. an FFI
   boundary, or — once shipped — a reflective host call). M48 is the milestone
   that closes this gap.

## Alternatives considered

- **Type-system encoding only (Haskell/Koka style).** Rejected because the
  package boundary still leaks; a maintainer reading `package.yaml` cannot tell
  what capabilities the deployable will demand at runtime.
- **Capability tokens passed at construction (object-capability style).**
  Considered and partially adopted at runtime (M48); the bootstrap chose the
  declarative `uses` clause because it is statically auditable without running
  the program.
- **Annotation-based, no enforcement (the "documentation" alternative).**
  Rejected because the security posture depends on enforcement; an annotation
  the compiler does not check is no better than a comment.
- **Single layer at the package boundary only.** Rejected because per-function
  declarations are what enables the `E0420` propagation diagnostic; without
  them, the loop cannot tell a developer which line to fix.

## Prior art

### Koka

Koka treats effects as part of the function type. `fn read-file(path : string) :
io string` is `io`-effectful. The effect row is checked by unification; effect
polymorphism is supported via row variables. Koka's effect system is more
expressive than Orison's; it is also harder to surface to non-PL-specialist
contributors. Orison borrows the principle "effects are first-class on functions"
without adopting effect rows.

### Effekt

Effekt extends the Koka approach with effect handlers as first-class language
constructs (`handle { ... } with { ... }`). The handler model is powerful but
expensive: the compiler must lower handlers into capability-passing code.
Orison's bootstrap does not adopt handlers; the planned runtime layer (M48) may
introduce a capability-passing path inspired by Effekt but does not commit to
handler syntax.

### Rust

Rust models filesystem and network access via standard-library APIs without any
effect tracking. The `unsafe` keyword is the closest analogue to an effect, and
it is single-bit. Orison rejects this: a single `unsafe` flag does not give the
maintainer enough information to audit a package.

### Pony, Newspeak, and the object-capability lineage

Pony's reference capabilities (`iso`, `val`, `ref`, etc.) and Newspeak's
object-capability discipline are precedent for "no ambient authority." Orison's
package boundary takes the principle and applies it at the deployable rather
than the object level — appropriate because a deployable is the unit at which a
maintainer reasons about authority.

### WASI

WASI's preopens are the most directly comparable design in production today: a
WASM module is granted explicit filesystem and network capabilities at
instantiation, with no implicit access. Orison's mobile and wasm-component
manifests (layer 5) are designed to interop with this model.

## Unresolved questions

- Should `KNOWN_EFFECTS` grow toward a smaller, more carefully partitioned set
  (e.g. separate `net.outbound.tls` from `net.outbound.plaintext`)? The current
  list is intentionally coarse; refining it is a future RFC.
- Should `E0420`'s `change_signature` fix offer a "remove the call" alternative
  when the caller's intent is clearly not to acquire the effect? Today the only
  suggestion is to declare the effect.
- How should named capabilities (uppercase identifiers) be resolved to their
  effect rows? Today they are opaque tokens; a future RFC may add an
  introspection schema.

## Future possibilities

- **Runtime gating (M48).** Bring the static contract into the runtime via
  capability tokens carried in the deployable's manifest.
- **Cross-module call-graph propagation.** Extend `effect_propagate` beyond
  module-local edges once the body parser exposes resolved cross-module calls.
- **Effect handlers** as a first-class language feature, conditional on the
  runtime layer.
- **Capability composition syntax** for finer-grained policies (e.g.
  `fs.read("./assets/**")` as a path-scoped capability).

## Acceptance criteria

- [x] `KNOWN_EFFECTS` is a closed, statically-checked list in
      [`crates/ori-compiler/src/effects.rs`](../../crates/ori-compiler/src/effects.rs).
- [x] `effect_diagnostics` emits `E0410` for any effect used outside the
      declared package policy
      ([`crates/ori-compiler/src/effect_check.rs`](../../crates/ori-compiler/src/effect_check.rs);
      test `diagnostics_flag_undeclared_effect`).
- [x] `effect_diagnostics` emits `W0401` for unknown effect names
      ([`crates/ori-compiler/src/effect_check.rs`](../../crates/ori-compiler/src/effect_check.rs)).
- [x] `propagation_diagnostics` emits `E0420` with a `change_signature` Patch
      IR fix when a caller transitively requires an undeclared effect
      ([`crates/ori-compiler/src/effect_propagate.rs`](../../crates/ori-compiler/src/effect_propagate.rs)).
- [x] `run_audit` enforces `AUD0001`, `AUD0002`, `AUD0003` at the package
      boundary
      ([`crates/ori-pkg/src/audit.rs`](../../crates/ori-pkg/src/audit.rs);
      tests under `crates/ori-pkg/tests/audit_capability_diff.rs` and
      `crates/ori-pkg/tests/capability_bypass.rs`).
- [x] `build_capability_manifest` emits the `ori.capability.v1` envelope with
      `policy.undeclared` and `policy.unused` diff fields.
- [x] Mobile target manifest projects effects into platform permissions
      deterministically
      ([`crates/ori-compiler/src/mobile.rs`](../../crates/ori-compiler/src/mobile.rs)).
- [ ] Runtime gating — deferred to M48; not part of this RFC's acceptance.

## Compatibility impact

This RFC documents the bootstrap state and is non-breaking by definition.

Any future change to the effect or capability surface will require its own RFC
under [section 8.5 of the process](./PROCESS.md#85-effect-or-capability-changes).
The relevant constraints are:

- Adding an effect to `KNOWN_EFFECTS` is additive but may convert previously
  `W0401`-warning callers to passing without warning; downstream tools that
  asserted on the warning text would need to update.
- Renaming or removing an effect is breaking and requires a deprecation period
  with both names recognised.
- The `ori.capability.v1` and `ori.audit_report.v1` schemas are subject to the
  schema-breaking-change rules in
  [section 8.2 of the process](./PROCESS.md#82-schema-breaking-changes).
