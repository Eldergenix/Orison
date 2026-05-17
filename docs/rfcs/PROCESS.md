# Orison RFC Process

This document specifies the Request-for-Comments (RFC) process used to evolve the
Orison language, compiler, package manager, agent ABI, JSON contracts, build outputs,
and governance. It is binding on maintainers and contributors. The index lives in
[`README.md`](./README.md); the blank starting point is
[`0000-template.md`](./0000-template.md).

---

## Table of contents

- [1. Why we have an RFC process](#1-why-we-have-an-rfc-process)
- [2. When an RFC is required](#2-when-an-rfc-is-required)
- [3. When an RFC is not required](#3-when-an-rfc-is-not-required)
- [4. How to write an RFC](#4-how-to-write-an-rfc)
- [5. Lifecycle](#5-lifecycle)
- [6. Roles](#6-roles)
- [7. Decision rules](#7-decision-rules)
- [8. Specific gates](#8-specific-gates)
  - [8.1 Cargo dependency additions](#81-cargo-dependency-additions)
  - [8.2 Schema-breaking changes](#82-schema-breaking-changes)
  - [8.3 Security-sensitive changes](#83-security-sensitive-changes)
  - [8.4 CLI subcommand additions](#84-cli-subcommand-additions)
  - [8.5 Effect or capability changes](#85-effect-or-capability-changes)
  - [8.6 Governance changes](#86-governance-changes)
- [9. Withdrawal, rejection, and supersession](#9-withdrawal-rejection-and-supersession)
- [10. Cross-references](#10-cross-references)

---

## 1. Why we have an RFC process

Orison's contracts compound. A JSON schema shipped today is an obligation for every
agent and tool that reads it tomorrow. A CLI subcommand becomes part of the public
surface the moment it appears in a release. A new dependency in the workspace is a
permanent supply-chain commitment.

The RFC process exists to make those commitments deliberate. It forces:

1. A written record of why a design is the way it is, separate from the code review.
2. A public period during which any contributor can weigh in.
3. A check that alternatives were considered and that prior art was consulted.
4. A check that the compatibility impact and migration path are stated up front.
5. A check that acceptance criteria are written *before* the implementation lands, so
   that "done" is testable rather than negotiable.

The process is not a hurdle for the sake of it. Section 3 enumerates everything that
explicitly does not need an RFC.

---

## 2. When an RFC is required

An RFC is required before any of the following land on the default branch:

1. **Any public JSON schema change** that is not strictly additive within an existing
   version. Examples requiring an RFC:
   - Adding a new schema file under [`schemas/`](../../schemas/).
   - Bumping any schema to a new version (e.g. publishing
     `ori.diagnostic.v2.schema.json` next to the existing v1 file).
   - Renaming or removing any field from any shipped schema.
2. **Any new top-level CLI subcommand** under `ori` (e.g. a new sibling of `check`,
   `doctor`, `agent`, `patch`, `capsule`, `audit`, `capability`, `coverage`,
   `migrate`, `bench`).
3. **Any new flag on an existing CLI subcommand** that changes the shape of the JSON
   envelope it emits.
4. **Any new dependency** added to the workspace `Cargo.toml` or to any crate
   `Cargo.toml`. Today only `serde` and `serde_json` are approved; see
   [section 8.1](#81-cargo-dependency-additions).
5. **Any new language feature** — new keyword, new statement kind, new expression
   kind, new type-system rule, new effect, new capability lattice rule.
6. **Any change to a non-negotiable invariant** listed in
   [`GOAL.md`](../../GOAL.md) section 3.
7. **Any breaking change** to any public Rust API exported by a crate under
   [`crates/`](../../crates/), or to any agent-visible contract.
8. **Any governance change** — adding or removing maintainers, creating a Special
   Interest Group (SIG), changing the conflict resolution rules in
   [`GOVERNANCE.md`](../GOVERNANCE.md), changing the Code of Conduct enforcement
   process in [`CODE_OF_CONDUCT.md`](../../CODE_OF_CONDUCT.md).
9. **Any new diagnostic id range** (e.g. introducing `E0500..=E0599`) or any change
   to an existing diagnostic's level (warning to error and vice versa).

If the change touches more than one of the above categories, one RFC covers them all;
do not file separate RFCs for the same change.

---

## 3. When an RFC is not required

The list in this section is exhaustive in spirit: anything not listed here defaults to
"file an issue and ask." But the common cases that explicitly bypass the RFC process
are:

- Bug fixes that restore behaviour documented in a schema, RFC, or test.
- Performance improvements that do not change any public contract.
- Internal refactors confined to a single crate that do not add a dependency and do
  not introduce a new public API.
- Documentation, example, and test additions.
- Diagnostic message wording changes (the *id* is the stable surface, not the
  message text).
- Adding examples to an existing `schemas/*.schema.json` file (the `examples` array
  is allowed to grow).
- Renaming a private function, constant, or module that is not re-exported.

If a "small" change starts to feel like it is reshaping the way users or agents
interact with the tool, stop and open an RFC. The cost of the RFC will be a small
fraction of the cost of unwinding a contract later.

---

## 4. How to write an RFC

1. Open a pre-RFC discussion issue using the
   [`rfc.md`](../../.github/ISSUE_TEMPLATE/rfc.md) issue template. State the problem
   in two or three paragraphs and link any related issues, prior threads, or
   external prior art. Do not propose a solution yet — the goal is to confirm the
   problem.
2. Copy [`0000-template.md`](./0000-template.md) into a new file named
   `NNNN-short-title.md`. Authors do not need to reserve `NNNN` in advance; the
   merging maintainer will renumber the file at merge time to keep the index
   contiguous. Use `XXXX-short-title.md` in the PR if you prefer.
3. Fill in every section of the template. "N/A" is an acceptable answer when
   genuinely not applicable; "TODO" is not. The required sections are:
   - Summary (one paragraph).
   - Motivation (why is this needed; what doesn't work today).
   - Detailed design.
   - Drawbacks.
   - Alternatives considered (at least one).
   - Prior art (other languages or comparable systems).
   - Unresolved questions.
   - Future possibilities.
   - Acceptance criteria (testable when done).
   - Compatibility impact (breaking? if yes, migration path).
4. Open a pull request against this directory. Link the pre-RFC issue from the PR
   description and the RFC body's metadata block.
5. Iterate based on review. Use commits (not force-pushes) so reviewers can see the
   evolution. Squash on merge.

Style rules:

- Wrap at approximately 100 columns. Tables and code fences may exceed this.
- Use ATX headings (`#`, `##`, `###`).
- No emoji. No marketing language. State the design.
- Every claim about how shipping code works must reference the relevant file or
  function path. RFCs are a record of reality, not aspiration.
- Cross-references use repo-relative paths so that they render correctly on GitHub
  and on any local copy.

---

## 5. Lifecycle

An RFC passes through the following phases. Transitions are recorded in the
metadata block at the top of the RFC document.

### 5.1 Pre-RFC

The author has opened a discussion issue but not yet a PR. The phase exists so that
problem framing happens before solution framing. There is no time limit. If the
discussion stalls, the author may close the issue and revisit later.

### 5.2 Open

The author has opened a PR. Maintainers and contributors review and request
changes. There is no minimum or maximum duration; the goal is consensus on the
design, not a clock.

### 5.3 Final Comment Period (FCP)

When a maintainer believes the RFC is ready, they comment "Entering FCP" on the PR
and apply the `fcp` label. FCP lasts a **minimum of seven days**. During FCP:

- Substantive new objections raised on the PR pause FCP; the maintainer who started
  FCP must address them and either restart FCP or move the RFC back to Open.
- Editorial comments do not pause FCP.
- A second maintainer must explicitly acknowledge ("acked, FCP can close") before
  the RFC can move to Merged.

### 5.4 Merged

The PR is merged. The RFC is now policy. A tracking issue is opened immediately
under the `rfc-impl` label, linking the RFC and listing the acceptance criteria as
a checklist.

### 5.5 Implemented

Every acceptance criterion checkbox on the tracking issue is checked. The RFC
metadata block is updated with the implementing commit range or release tag.

### 5.6 Stabilised

The implementation has shipped a release and remained unchanged for at least one
subsequent release cycle. Any unstable flags introduced for the feature have been
removed or promoted. The RFC metadata block is updated with the stabilising
release.

### 5.7 Superseded

A later RFC replaces this one. Both documents are updated to point at each other.
The superseded RFC remains in the index for historical reference.

---

## 6. Roles

- **Author** — the contributor who writes and shepherds the RFC. Authors do not
  need any specific privilege.
- **Reviewer** — anyone who comments on the PR. All comments are weighed; the
  weight increases with demonstrated familiarity with the affected subsystem.
- **Maintainer** — currently the BDFL (see [`GOVERNANCE.md`](../GOVERNANCE.md)).
  After the planned TC transition, the Technical Committee.
- **Implementer** — the contributor who lands the implementation. Frequently the
  same person as the author, but not required.

---

## 7. Decision rules

The bootstrap decision rule is BDFL: the repo owner has final say on every RFC
during the bootstrap era. See [`GOVERNANCE.md`](../GOVERNANCE.md) for the planned
transition to a Technical Committee model after 1.0.

In practice the bootstrap decision rule is exercised conservatively:

- An RFC with no maintainer objections after FCP is merged.
- An RFC with substantive objections is iterated until objections are resolved or
  the RFC is withdrawn.
- The BDFL only overrides during the bootstrap era to preserve a non-negotiable
  invariant from [`GOAL.md`](../../GOAL.md) section 3, or to break a deadlock that
  has persisted for more than 30 days.

After the TC transition, decisions move from BDFL fiat to lazy consensus among TC
members, with explicit voting reserved for cases where lazy consensus fails. The
exact rules will themselves require an RFC at that time.

---

## 8. Specific gates

The gates in this section are policy statements that override any conflicting
text elsewhere in this document. They exist because the categories below have
caused harm in other language ecosystems and Orison has chosen stricter rules to
avoid the same outcomes.

### 8.1 Cargo dependency additions

A new dependency in any `Cargo.toml` under [`crates/`](../../crates/) requires:

1. An RFC that includes:
   - The crate name, version range, license, and last-published date.
   - The exact subset of the crate's API the workspace will use.
   - A justification for why the functionality cannot be provided by `serde`,
     `serde_json`, or workspace-local code at acceptable cost.
   - A maintenance plan: who in the project is responsible for tracking upstream
     security advisories and version bumps.
2. A successful FCP and **two maintainer acknowledgements** explicitly approving the
   dependency. One maintainer is not enough.
3. A `CHANGELOG.md` entry in the bootstrap section explaining the rationale.

The current approved set is `serde` and `serde_json`. This is enforced by
[`scripts/validate_all.py`](../../scripts/validate_all.py) via the
`ALLOWED_WORKSPACE_DEPS` allow-list; any new dependency added without updating that
allow-list will fail the static gate.

### 8.2 Schema-breaking changes

A schema under [`schemas/`](../../schemas/) is a public API. The invariant from
[`GOAL.md`](../../GOAL.md) section 3.3 is binding: a shipped `schemas/*.schema.json`
file is a permanent contract.

- **Additive change within a version** (adding an optional field) does not need an
  RFC, but does need a CHANGELOG entry.
- **Any rename, removal, type change, or required-status change** within a shipped
  version is prohibited. The required path is to ship a new version file alongside
  the old one.
- **New version files** (e.g. `ori.diagnostic.v2.schema.json` next to
  `ori.diagnostic.v1.schema.json`) require an RFC. The RFC must include:
  - The full delta from v1 to v2.
  - The deprecation policy for v1 (minimum two release cycles of co-shipping).
  - The migration tool or guide for downstream consumers.
  - The doctor-report entry that lists both schemas.
- **The v1 file is never edited in place after the v2 ships.**

### 8.3 Security-sensitive changes

Any change that touches the threat model in [`SECURITY.md`](../../SECURITY.md), the
capability lattice, the audit rules under
[`crates/ori-pkg/src/audit.rs`](../../crates/ori-pkg/src/audit.rs), the effect
propagation pass under
[`crates/ori-compiler/src/effect_propagate.rs`](../../crates/ori-compiler/src/effect_propagate.rs),
or the provenance verification path requires an RFC.

For **vulnerabilities being reported**, the RFC process is the wrong tool. Follow
the disclosure path in [`SECURITY.md`](../../SECURITY.md). After the embargoed fix
lands, an RFC may be written retrospectively to document the change to the threat
model (this is encouraged but not required).

### 8.4 CLI subcommand additions

A new top-level `ori` subcommand requires an RFC that includes:

- The subcommand's exact argument grammar.
- The JSON envelope it emits (schema id, fields, examples).
- The Draft 2020-12 schema file that will be added under
  [`schemas/`](../../schemas/).
- The doctor-report entry that will be added.
- The CLI smoke test that will guard the contract.

### 8.5 Effect or capability changes

A new entry in the `KNOWN_EFFECTS` table at
[`crates/ori-compiler/src/effects.rs`](../../crates/ori-compiler/src/effects.rs),
or any change to the capability propagation rules at
[`crates/ori-compiler/src/effect_propagate.rs`](../../crates/ori-compiler/src/effect_propagate.rs),
or any change to the audit rules at
[`crates/ori-pkg/src/audit.rs`](../../crates/ori-pkg/src/audit.rs), requires an
RFC. The RFC must include the diagnostic id range it occupies and the migration
plan for callers that previously triggered `W0401` "unknown effect" on the new
name.

### 8.6 Governance changes

Adding or removing a maintainer, creating a SIG, dissolving a SIG, changing the
quorum or voting rules, or changing the conflict resolution process described in
[`GOVERNANCE.md`](../GOVERNANCE.md) requires an RFC with a minimum **14-day FCP**
(double the standard period) and explicit acknowledgement from the BDFL during the
bootstrap era, or from a TC majority after the TC transition.

---

## 9. Withdrawal, rejection, and supersession

- An author may **withdraw** their own RFC at any time before merge by closing the
  PR and stating the reason. Withdrawn RFCs are not archived in the index but the
  discussion remains searchable on GitHub.
- A maintainer may **reject** an RFC after FCP if the design conflicts with a
  non-negotiable invariant or with a higher-priority concurrent RFC. Rejected RFCs
  are merged (not closed) with a `Rejected: <date>` line in the metadata block and
  a paragraph explaining the rejection rationale. This preserves the rejected
  design as a public record and ensures the same design is not re-proposed without
  acknowledging the prior decision.
- An RFC is **superseded** when a later RFC replaces it. Both metadata blocks are
  updated; the superseded RFC remains in the index.

---

## 10. Cross-references

- [`docs/rfcs/README.md`](./README.md) — index of RFCs.
- [`docs/rfcs/0000-template.md`](./0000-template.md) — RFC template.
- [`docs/rfcs/MEETINGS.md`](./MEETINGS.md) — public meeting cadence.
- [`docs/GOVERNANCE.md`](../GOVERNANCE.md) — governance model and TC plan.
- [`CODE_OF_CONDUCT.md`](../../CODE_OF_CONDUCT.md) — community standards.
- [`CONTRIBUTING.md`](../../CONTRIBUTING.md) — local quality loop and PR rules.
- [`SECURITY.md`](../../SECURITY.md) — security policy and disclosure path.
- [`GOAL.md`](../../GOAL.md) — product spec, milestones, and invariants.
- [`.github/ISSUE_TEMPLATE/rfc.md`](../../.github/ISSUE_TEMPLATE/rfc.md) — pre-RFC
  issue template.
- [`schemas/`](../../schemas/) — JSON contract files referenced by section 8.2.
- [`scripts/validate_all.py`](../../scripts/validate_all.py) — enforces the
  dependency allow-list referenced by section 8.1.
