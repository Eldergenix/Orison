# Orison Governance

This document describes how decisions about the Orison language and
toolchain are made. It applies to everyone contributing to the project —
maintainers, regular contributors, and one-time PR authors.

The goal is to make the rules explicit so that contributors know what to
expect when they propose, review, or block a change.

---

## 1. Roles

### 1.1 Contributors

Anyone who opens an issue, posts on the discussion forum, or submits a
patch is a contributor. No formal status is required, and no application
is needed. Contributors are bound by [CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md).

### 1.2 Reviewers

Contributors with a track record of high-quality patches may be invited
by the maintainer team to become reviewers. A reviewer can:

- Approve patches in their area of expertise (the area is recorded in
  [MAINTAINERS.md](./MAINTAINERS.md) under `reviewers`).
- Request changes on any patch.
- Triage and label issues.

A reviewer's approval does not merge a patch — only a maintainer can
merge (see §1.3). Reviewers' approvals are required for any non-trivial
change in their area: a maintainer should not unilaterally bypass a
reviewer's `request_changes` without a written exception captured in
the PR.

A reviewer may step down at any time by opening a PR against
`MAINTAINERS.md`. A reviewer may be removed for prolonged inactivity (no
review activity in 6 months) by a maintainer-team vote (§2.2).

### 1.3 Maintainers

Maintainers are listed in [MAINTAINERS.md](./MAINTAINERS.md) under
`maintainers`. A maintainer can:

- Merge patches after the standard review process (§3).
- Cut releases (§4).
- Vote on governance and stability decisions (§2).
- Invite new reviewers and propose new maintainers (§1.4).

Maintainers act as stewards, not owners. The bar for "maintainer
decision" is consensus among the maintainer team, not unilateral
authority.

### 1.4 Becoming a maintainer

A reviewer becomes a maintainer by:

1. Sustained reviewer activity for at least 6 months.
2. A nomination by an existing maintainer, opened as an issue with the
   `governance:maintainer-nomination` label.
3. Approval by a supermajority (≥ 2/3) of the existing maintainers within
   14 days. Silence is approval after the 14-day window.

Maintainers may step down at any time by opening a PR against
`MAINTAINERS.md`. Maintainers may be removed for sustained inactivity
(no review or commit activity in 12 months) or for a code-of-conduct
violation, via the process in §2.3.

---

## 2. Decisions

### 2.1 Lazy consensus

Most decisions — including code changes, documentation updates, and
issue triage — are made by **lazy consensus**: a proposal is approved
unless an objection is raised within the review window. The default
window is the time the PR or issue is open for review (typically 48-72
hours for trivial changes, longer for substantial ones).

### 2.2 Maintainer vote

Governance and stability decisions are made by a recorded vote of the
active maintainer team. A decision is **adopted** by a simple majority of
maintainer votes cast within a 7-day window, except where this document
specifies a higher threshold.

A vote is held when:

- A change affects tier 1 or tier 2 stability (see
  [STABILITY.md](./STABILITY.md)).
- A change to this `GOVERNANCE.md`, `CODE_OF_CONDUCT.md`, or
  `MAINTAINERS.md` is proposed.
- A maintainer-team membership change is proposed (§1.4).
- A code-of-conduct enforcement action above a written warning is
  proposed (§2.3).

Maintainer votes are conducted publicly in the PR or issue. Maintainers
who do not vote within the window are counted as abstaining.

### 2.3 Code-of-conduct enforcement

Reports of code-of-conduct violations are handled by the maintainer team
following the process in [CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md).
Where an enforcement action involves removing a maintainer or reviewer,
the action requires a supermajority (≥ 2/3) of the remaining maintainer
team, with the affected person recused.

---

## 3. Patch lifecycle

A patch (PR) is reviewed by at least one reviewer in the relevant area
(see `MAINTAINERS.md`). After reviewer approval, a maintainer merges. If
no reviewer is assigned to the area, a maintainer self-reviews.

Patches that change tier-1 surfaces (see `STABILITY.md`) require an RFC
(see [docs/rfcs/PROCESS.md](./docs/rfcs/PROCESS.md)). The RFC must reach
**accepted** state before any implementation patch is merged.

Maintainers may merge their own patches after **one** independent
reviewer approval. Self-approval is not permitted for any change above
trivial (one-line typo, formatting, etc.).

A patch may be reverted by any maintainer in the 72-hour window after
merge if a regression is discovered. Reverts after that window require a
new PR through the normal review process.

---

## 4. Releases

Release cadence and version policy are documented in
[STABILITY.md §2](./STABILITY.md). The release process is:

1. A maintainer opens a PR that updates `Cargo.toml` versions and
   `CHANGELOG.md`.
2. The PR is reviewed under the normal process.
3. On merge, a maintainer tags the commit (`vX.Y.Z`) and pushes the tag.
4. CI builds the release artefacts and publishes them.

Hot-fix patch releases (`vX.Y.Z+1`) follow the same process with an
abbreviated review window (24 hours instead of 72) for security or
correctness fixes.

---

## 5. Resources

- [MAINTAINERS.md](./MAINTAINERS.md) — current maintainers and reviewers
- [CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md) — community standards
- [STABILITY.md](./STABILITY.md) — compatibility tiers
- [CONTRIBUTING.md](./CONTRIBUTING.md) — developer workflow
- [docs/rfcs/PROCESS.md](./docs/rfcs/PROCESS.md) — RFC process
- [SECURITY.md](./SECURITY.md) — security disclosure process

---

## 6. Changing this document

Changes to this `GOVERNANCE.md` require a maintainer vote (§2.2) and a
14-day public comment window before the vote opens.
