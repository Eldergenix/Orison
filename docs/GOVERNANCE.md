# Orison Governance

This document describes how the Orison project is governed: who makes decisions today,
who will make them after the planned transition to a Technical Committee, how Special
Interest Groups (SIGs) operate, how members are added and removed, and how conflicts
are resolved.

Governance changes to this document are themselves bound by the RFC process at
[`docs/rfcs/PROCESS.md`](./rfcs/PROCESS.md) section 8.6 (governance changes require an
RFC with a 14-day FCP).

---

## Table of contents

- [1. Current state: BDFL bootstrap](#1-current-state-bdfl-bootstrap)
- [2. Planned transition: Technical Committee after 1.0](#2-planned-transition-technical-committee-after-10)
- [3. Special Interest Groups (SIGs)](#3-special-interest-groups-sigs)
- [4. Membership lifecycle](#4-membership-lifecycle)
- [5. Conflict resolution](#5-conflict-resolution)
- [6. Code of Conduct enforcement](#6-code-of-conduct-enforcement)
- [7. Decision log](#7-decision-log)
- [8. Cross-references](#8-cross-references)

---

## 1. Current state: BDFL bootstrap

The project is currently governed by a Benevolent Dictator for Life (BDFL) model.
The BDFL is the repo owner:

- **BDFL**: [Eldergenix](https://github.com/Eldergenix), maintainer of
  [github.com/Eldergenix/Orison](https://github.com/Eldergenix/Orison).

In the bootstrap era the BDFL has final authority over:

- Merging RFCs (subject to the FCP and review requirements in
  [`docs/rfcs/PROCESS.md`](./rfcs/PROCESS.md)).
- Approving new dependencies (see [`docs/rfcs/PROCESS.md`](./rfcs/PROCESS.md)
  section 8.1 — note that the two-maintainer-ack requirement applies during the
  bootstrap by interpretation: the BDFL plus one additional reviewer with
  demonstrated familiarity with the affected subsystem).
- Adding and removing maintainers.
- Releasing new versions.
- Resolving conflicts that cannot be resolved by the parties involved.

The BDFL exercises this authority conservatively. In practice:

- An RFC with no maintainer objections after FCP is merged without BDFL
  intervention.
- An RFC with objections is iterated until the objections are resolved.
- The BDFL only overrides during the bootstrap era to preserve a non-negotiable
  invariant from [`GOAL.md`](./../GOAL.md) section 3, or to break a deadlock that
  has persisted for more than 30 days.

The BDFL is not a substitute for review. Every change still goes through the same
PR review, RFC process, and quality gate as every other contributor's work.

---

## 2. Planned transition: Technical Committee after 1.0

The BDFL model is appropriate for a project at the bootstrap scale because it is
fast and avoids process overhead while the design space is still settling. It is
not appropriate for the long term: a single point of decision-making does not
scale, does not survive maintainer turnover, and does not represent a community.

After the 1.0 release, governance will transition to a Technical Committee (TC):

- **Composition**: 5 to 9 members, drawn from active maintainers and SIG leads.
- **Term**: 12 months, staggered so that roughly one-third of seats turn over
  every four months.
- **Selection**: incoming members are nominated by existing TC members and
  confirmed by a simple majority vote of the current TC. Self-nomination is
  permitted.
- **Decision rule**: lazy consensus on routine matters; explicit vote (simple
  majority, with quorum equal to half of seated members plus one) on contested
  matters. A tied vote defers to a follow-up vote at the next meeting.
- **BDFL after transition**: the BDFL remains as a non-voting steward with a
  single-use veto on changes that conflict with the non-negotiable invariants
  in [`GOAL.md`](./../GOAL.md) section 3. The veto is exercised at most once per
  calendar year; if exercised more often the veto right is reviewed by the TC
  through the RFC process.

The exact transition mechanics — the first TC's composition, the first election,
the constitution document — will themselves require an RFC under
[`docs/rfcs/PROCESS.md`](./rfcs/PROCESS.md) section 8.6. The expected timing is
post-1.0, which on the current milestone schedule falls after M37 ships
([`GOAL.md`](./../GOAL.md) section 6).

---

## 3. Special Interest Groups (SIGs)

SIGs are the unit of subsystem ownership. A SIG is a named group of contributors
with deep familiarity with a specific subsystem; they have first-look authority on
RFCs and PRs touching that subsystem.

### 3.1 Initial SIGs (proposed at TC transition)

The five initial SIGs cover the natural seams in the codebase:

| SIG          | Scope                                                                                 |
| ------------ | ------------------------------------------------------------------------------------- |
| `compiler`   | `crates/ori-compiler/` — lexer, parser, type system, effect passes, codegen           |
| `runtime`    | The future runtime crate; the M48 capability-runtime gating; wasm and native runtime  |
| `stdlib`     | `stdlib/` — every shipped standard library module                                     |
| `frameworks` | The web, mobile, and desktop framework modules under `docs/frameworks/`               |
| `ecosystem`  | `crates/ori-pkg/`, the registry, the playground, tutorials, cookbook, migration guides |

A SIG is **not** a gatekeeping body. Any contributor may open a PR against any
subsystem. The SIG's role is:

- To be the first reviewer notified on PRs touching the subsystem.
- To carry institutional memory for the subsystem's design decisions.
- To shepherd RFCs that affect the subsystem.
- To maintain the subsystem's documentation under [`docs/`](./).

### 3.2 SIG composition

Each SIG has:

- A **lead** (one per SIG), responsible for organising the SIG's work and
  representing it to the TC.
- Two to six **members**, each with demonstrated familiarity with the subsystem.

SIG leads are appointed by the TC after the transition; during the bootstrap the
BDFL appoints them. Members are co-opted by the SIG itself, with notification to
the TC (or BDFL during bootstrap).

### 3.3 Creating, dissolving, or merging SIGs

Creating a new SIG, dissolving an existing one, or merging two SIGs requires an
RFC under [`docs/rfcs/PROCESS.md`](./rfcs/PROCESS.md) section 8.6. The RFC must
include:

- The scope of the new SIG (or the redistribution of scope for a dissolution or
  merge).
- The initial lead and members.
- The transition plan for in-flight RFCs and PRs.

---

## 4. Membership lifecycle

### 4.1 Becoming a contributor

Anyone who opens a PR is a contributor. There is no application process. The
expectation is that the PR follows [`CONTRIBUTING.md`](./../CONTRIBUTING.md) and
passes the quality gate.

### 4.2 Becoming a maintainer

A maintainer has commit access and may merge PRs from other contributors. The
criteria for maintainership are:

- At least 10 merged non-trivial PRs (bugfix, feature, or refactor that affected
  at least two files).
- At least one merged RFC as author or primary reviewer.
- A demonstrated record of constructive review on PRs other than their own.
- A standing in good faith with the Code of Conduct (no unresolved enforcement
  actions).

Nomination is by an existing maintainer; confirmation is by the BDFL during the
bootstrap and by simple majority of the TC after the transition. Nominations
include a brief case (one paragraph) and a link to the nominee's contribution
record.

### 4.3 Becoming a SIG member or lead

SIG members are co-opted by the SIG itself; SIG leads are appointed as described
in [section 3.2](#32-sig-composition).

### 4.4 Becoming a TC member

After the transition, TC members are selected as described in
[section 2](#2-planned-transition-technical-committee-after-10).

### 4.5 Stepping down

Any maintainer, SIG member, SIG lead, or TC member may step down at any time by
opening an issue or notifying the BDFL/TC privately. There is no penalty and no
required notice period (though courtesy advance notice is appreciated).

### 4.6 Removal

A maintainer, SIG member, SIG lead, or TC member may be removed for:

- Sustained inactivity (no contribution for 12 months). Inactivity-based removal
  is not a judgement; it is a housekeeping action. The removed person may
  re-apply at any time.
- A Code of Conduct violation as determined by the enforcement process in
  [section 6](#6-code-of-conduct-enforcement).
- A pattern of decisions that materially harm the project, as determined by an
  RFC opened specifically for the removal.

Removal during the bootstrap is decided by the BDFL after consultation with the
remaining maintainers. After the TC transition, removal of a TC member is
decided by a two-thirds vote of the remaining TC; removal of a maintainer or SIG
member is decided by simple majority.

---

## 5. Conflict resolution

The escalation ladder for technical disagreements is:

1. **Talk it out on the PR or issue.** Most disagreements are settled by one
   round of clarification.
2. **Open an RFC.** When the disagreement is over a design decision rather than
   an implementation detail, the RFC is the right forum. The structured
   sections force the parties to enumerate their positions explicitly.
3. **Request a SIG review.** If the disagreement is about a subsystem and the
   parties cannot find common ground, the SIG lead has the authority to call
   the question for the SIG.
4. **Escalate to the BDFL (bootstrap) or TC (post-transition).** Final
   technical authority rests there. The escalation is in the open: the person
   escalating opens an issue labelled `escalation` and links the prior
   discussion.

The escalation ladder for interpersonal conflicts is the Code of Conduct
enforcement process in [section 6](#6-code-of-conduct-enforcement); it is
separate from the technical escalation ladder and is not subordinate to it.

---

## 6. Code of Conduct enforcement

The Code of Conduct lives at [`CODE_OF_CONDUCT.md`](./../CODE_OF_CONDUCT.md) and is
Contributor Covenant 2.1 verbatim. Reports go to the contact address listed in
that document. The enforcement guidelines (Correction, Warning, Temporary Ban,
Permanent Ban) are described in the Code of Conduct itself.

During the bootstrap, enforcement decisions are made by the BDFL. After the TC
transition, enforcement decisions are made by a two-person rotation of TC members
who are not the subject of, or directly involved in, the report. The rotation
ensures no single person is permanently responsible and that the subject of any
report has a reviewer who can be impartial.

The contact address (currently `abuse@orison-language.org`, marked as TBD in
[`CODE_OF_CONDUCT.md`](./../CODE_OF_CONDUCT.md) until a real mailbox is
provisioned) is monitored at minimum every business day. Reports are
acknowledged within 72 hours.

---

## 7. Decision log

Significant governance decisions are recorded under
[`docs/rfcs/`](./rfcs/) as RFCs. There is no separate decision log; the RFC
archive is the log. This avoids the situation where the same decision appears
in two places and the two places drift.

The current entries relevant to governance are:

- [`docs/rfcs/0001-stable-node-ids.md`](./rfcs/0001-stable-node-ids.md) —
  technical RFC, retroactive.
- [`docs/rfcs/0002-capability-secured-effects.md`](./rfcs/0002-capability-secured-effects.md) —
  technical RFC, retroactive.
- [`docs/rfcs/0003-structural-patch-ir.md`](./rfcs/0003-structural-patch-ir.md) —
  technical RFC, retroactive.

Governance RFCs (TC transition, first SIG charter, etc.) will be added to this
list as they are merged.

---

## 8. Cross-references

- [`docs/rfcs/README.md`](./rfcs/README.md) — RFC index.
- [`docs/rfcs/PROCESS.md`](./rfcs/PROCESS.md) — RFC process (governance changes
  are bound by section 8.6).
- [`docs/rfcs/MEETINGS.md`](./rfcs/MEETINGS.md) — public meeting cadence and notes.
- [`CODE_OF_CONDUCT.md`](./../CODE_OF_CONDUCT.md) — community standards and
  enforcement guidelines.
- [`CONTRIBUTING.md`](./../CONTRIBUTING.md) — local quality loop.
- [`SECURITY.md`](./../SECURITY.md) — security policy (security incidents are
  governed by `SECURITY.md`, not this document).
- [`GOAL.md`](./../GOAL.md) — product spec and non-negotiable invariants
  (section 3).
