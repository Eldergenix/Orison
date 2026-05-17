# RFC NNNN: Short title

| Field           | Value                                                    |
| --------------- | -------------------------------------------------------- |
| RFC number      | NNNN (assigned at merge time)                            |
| Title           | Short title                                              |
| Authors         | Your name <your-email@example.com>                       |
| Status          | Draft                                                    |
| Pre-RFC issue   | https://github.com/Eldergenix/Orison/issues/XXXX         |
| PR              | https://github.com/Eldergenix/Orison/pull/XXXX           |
| Created         | YYYY-MM-DD                                               |
| FCP entered     |                                                          |
| Merged          |                                                          |
| Implemented     |                                                          |
| Stabilised      |                                                          |
| Supersedes      |                                                          |
| Superseded by   |                                                          |

> Delete every instructional comment block before opening the PR. The headings and
> the metadata block above are required; the prose is yours to write.

---

## Table of contents

- [Summary](#summary)
- [Motivation](#motivation)
- [Detailed design](#detailed-design)
- [Drawbacks](#drawbacks)
- [Alternatives considered](#alternatives-considered)
- [Prior art](#prior-art)
- [Unresolved questions](#unresolved-questions)
- [Future possibilities](#future-possibilities)
- [Acceptance criteria](#acceptance-criteria)
- [Compatibility impact](#compatibility-impact)

---

## Summary

> One paragraph. State the proposal so that a maintainer reading only this paragraph
> can decide whether the RFC is in scope and worth reviewing in detail. Do not
> describe the motivation here; the next section is for that.

## Motivation

> Why is this needed? What does not work today? Describe the user (human or agent),
> the task, and the specific friction or failure they encounter. Reference concrete
> file paths, diagnostic ids, schemas, or benchmark numbers where possible.
>
> If the answer to "what does not work today?" is "nothing — this is an
> improvement," the RFC is probably premature; reframe it around the underlying
> problem.

## Detailed design

> The bulk of the RFC. Describe the design at enough depth that a reviewer can
> identify the affected files, contracts, and tests without having to ask. Required
> sub-content:
>
> - The exact public surface change (new CLI flag grammar, new schema id and
>   fields, new keyword, new effect name, etc.).
> - The implementation strategy (which crates, which passes, which existing data
>   structures).
> - The diagnostic ids occupied, if any.
> - The CHANGELOG entry that will accompany the implementation.
> - How the design preserves the non-negotiable invariants in
>   [`GOAL.md`](../../GOAL.md) section 3.

## Drawbacks

> Why might this be a bad idea? Every design has costs. Examples to consider:
>
> - Increased surface area to maintain.
> - Risk of breaking downstream consumers despite the migration plan.
> - Risk of performance regression (cite the relevant benchmark id in
>   [`BENCHMARKS.md`](../../BENCHMARKS.md) if applicable).
> - Risk of weakening the security posture in [`SECURITY.md`](../../SECURITY.md).
> - Cognitive cost for new contributors.
>
> "There are no drawbacks" is almost never true; if you cannot find any, the RFC
> may not be fully thought through.

## Alternatives considered

> List at least one alternative. "Do nothing" is a valid alternative if the cost of
> doing so is honestly stated. For each alternative, describe what it would look
> like and why it was rejected. Reviewers often catch design issues by inverting
> the rejection rationale here.

## Prior art

> How do other languages or comparable systems solve this problem? Cite specific
> language docs, RFCs, or specifications. Examples:
>
> - Rust RFCs (https://github.com/rust-lang/rfcs).
> - Swift evolution proposals.
> - TC39 proposals for JavaScript.
> - Koka or Effekt effect-system papers.
> - CRDT or operational-transform literature for collaborative editing.
>
> "I did not find prior art" should itself be supported with the search you ran.

## Unresolved questions

> List the open questions that should be answered before this RFC is merged, and
> separately the questions that can be deferred to implementation. Empty list is
> acceptable for tightly scoped RFCs; a long list is acceptable for exploratory
> ones.

## Future possibilities

> Extensions that this design enables but does not commit to. This section is
> non-binding; it gives reviewers a sense of the trajectory without requiring the
> RFC to specify it.

## Acceptance criteria

> Testable criteria for "done." A reviewer should be able to read this list and
> write the tests that prove the feature works. Each item is a checkbox so the
> tracking issue can mirror them.
>
> - [ ] Criterion 1 (cite the test file path that will assert it).
> - [ ] Criterion 2.
> - [ ] Criterion 3.

## Compatibility impact

> Is the change breaking? If yes, the migration path is required. Address:
>
> - Source compatibility (existing `.ori` programs).
> - JSON contract compatibility (which schemas grow, which version files are added).
> - Rust API compatibility (which crate exports change).
> - CLI compatibility (which subcommands or flags change).
> - Agent ABI compatibility (capsules, maps, symbol cards, patches).
>
> If the answer is "fully additive within an existing version, no migration
> needed," state that explicitly.
