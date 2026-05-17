# Orison RFCs

This directory is the authoritative archive of Request-for-Comments (RFC) documents that
shape the Orison language, compiler, package manager, agent ABI, JSON contracts, and
public governance. RFCs are how non-trivial change happens. If a change is large enough
that a reader six months from now would need to ask "why does it work this way?" — it
should have an RFC.

The full process, lifecycle, and authoring rules live in
[`PROCESS.md`](./PROCESS.md). The blank starting point for new proposals is
[`0000-template.md`](./0000-template.md). Public meeting cadence and notes archive
structure are described in [`MEETINGS.md`](./MEETINGS.md).

---

## Table of contents

- [What an RFC is](#what-an-rfc-is)
- [What does not need an RFC](#what-does-not-need-an-rfc)
- [Index of RFCs](#index-of-rfcs)
- [Lifecycle at a glance](#lifecycle-at-a-glance)
- [How to participate](#how-to-participate)
- [Numbering](#numbering)
- [Cross-references](#cross-references)

---

## What an RFC is

An RFC is a written, version-controlled proposal that:

1. States a problem grounded in something that does not work today.
2. Proposes a single, named design.
3. Documents drawbacks, alternatives considered, and prior art from other languages.
4. Lists acceptance criteria that are testable when the design has shipped.
5. Calls out compatibility impact and, when breaking, the migration path.

RFCs are required before:

- Any change to a public JSON schema under [`schemas/`](../../schemas/) that is not
  strictly additive within an existing version. New version files (`v2` next to `v1`)
  always require an RFC.
- Any new top-level CLI subcommand or any change to an existing subcommand's output
  envelope.
- Any new dependency added to the workspace `Cargo.toml`.
- Any new language construct, keyword, effect name, or capability lattice change.
- Any change to a non-negotiable invariant in [`GOAL.md`](../../GOAL.md) section 3.
- Any governance change — adding maintainers, creating SIGs, changing the conflict
  resolution rules in [`GOVERNANCE.md`](../GOVERNANCE.md).

See [`PROCESS.md`](./PROCESS.md) for the full enumeration.

---

## What does not need an RFC

The bar is meaningful change, not paperwork. The following are explicitly out of scope
for the RFC process and should land as regular pull requests:

- Bug fixes that restore documented behaviour.
- Performance improvements that preserve every public contract and are covered by an
  entry in [`BENCHMARKS.md`](../../BENCHMARKS.md).
- Internal refactors that do not change any `schemas/*.schema.json`, do not add a
  dependency, and do not introduce a new public API surface.
- Documentation, examples, and test additions that do not rename or remove public
  symbols.
- Diagnostic message wording (the diagnostic *id*, e.g. `E0420`, is the stable
  surface; the message is not).

When in doubt, open a discussion issue first. The cost of a 200-word issue is far
lower than the cost of a misdirected PR.

---

## Index of RFCs

The numbering scheme is described under [Numbering](#numbering). The status field
follows the lifecycle in [`PROCESS.md`](./PROCESS.md).

### Accepted

| Number | Title                                                              | Status   |
| ------ | ------------------------------------------------------------------ | -------- |
| 0001   | [Stable node IDs](./0001-stable-node-ids.md)                       | Shipped  |
| 0002   | [Capability-secured effects](./0002-capability-secured-effects.md) | Shipped  |
| 0003   | [Structural Patch IR](./0003-structural-patch-ir.md)               | Shipped  |

### Proposed

(none yet — open a pull request adding `NNNN-short-title.md` against this directory)

### Rejected / withdrawn

(none yet — rejected RFCs are kept here permanently for posterity, with the rejection
rationale appended to the document)

---

## Lifecycle at a glance

```
        Pre-RFC discussion (issue or chat)
                     |
                     v
        PR opened with NNNN-slug.md
                     |
                     v
        Review and revision (open period)
                     |
                     v
        Final Comment Period (FCP) — minimum 7 days
                     |
                     v
       Merged ----> Implementation ----> Stabilised
                     |                       |
                     v                       v
                  (tests)              (released, schemas
                                        frozen, no longer
                                        unstable-flagged)
```

A more precise description, including who can move an RFC between phases, is in
[`PROCESS.md`](./PROCESS.md).

---

## How to participate

1. Read [`PROCESS.md`](./PROCESS.md) end to end.
2. Open a pre-RFC discussion issue using
   [`.github/ISSUE_TEMPLATE/rfc.md`](../../.github/ISSUE_TEMPLATE/rfc.md).
3. Copy [`0000-template.md`](./0000-template.md) to `NNNN-short-title.md` (where
   `NNNN` is the next unused number; PR authors do not need to reserve it in advance
   — the merging maintainer assigns the final number).
4. Open a pull request and link it from the pre-RFC issue.
5. Iterate based on review.
6. After FCP closes, a maintainer merges the RFC and opens a tracking issue for the
   implementation.

---

## Numbering

RFC numbers are zero-padded four-digit identifiers, assigned monotonically at merge
time. They are never reused. A rejected RFC keeps its number. A superseding RFC
references the original (e.g. "supersedes RFC 0001") and the original document is
updated to point forward (e.g. "superseded by RFC 0042").

The number `0000` is reserved permanently for the template and must never be used as
a real RFC.

---

## Cross-references

- [`docs/rfcs/PROCESS.md`](./PROCESS.md) — full process specification.
- [`docs/rfcs/MEETINGS.md`](./MEETINGS.md) — public meeting cadence and notes.
- [`docs/GOVERNANCE.md`](../GOVERNANCE.md) — current governance model and TC plan.
- [`CODE_OF_CONDUCT.md`](../../CODE_OF_CONDUCT.md) — community standards.
- [`CONTRIBUTING.md`](../../CONTRIBUTING.md) — local quality loop and PR rules.
- [`SECURITY.md`](../../SECURITY.md) — security policy and the threat model.
- [`GOAL.md`](../../GOAL.md) — product spec, milestones, and invariants.
