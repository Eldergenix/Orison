# Maintainers

This file lists the current maintainers and reviewers of the Orison
project. The roles and processes described here are defined in
[GOVERNANCE.md](./GOVERNANCE.md).

> **Status:** the project is in the bootstrap / alpha phase. The
> maintainer list below reflects the initial seed team. New maintainers
> and reviewers will be added via the process in `GOVERNANCE.md §1.4`.

---

## Contact

- **General contact:** open a GitHub issue or discussion.
- **Security contact:** see [SECURITY.md](./SECURITY.md) §2.
- **Code-of-conduct contact:** conduct@orison-lang.org (placeholder)

---

## Maintainers

A maintainer can merge patches, cut releases, and vote on governance
decisions. See `GOVERNANCE.md §1.3`.

| Handle | Areas | Joined |
|---|---|---|
| @Eldergenix | bootstrap compiler, governance, releases | 2026-05 |

(Add yourself here via PR when accepted as a maintainer per
`GOVERNANCE.md §1.4`.)

---

## Reviewers

A reviewer can approve patches in their declared area of expertise. See
`GOVERNANCE.md §1.2`.

| Handle | Area | Joined |
|---|---|---|
| _(none yet)_ | _(seed team is bootstrapping reviewer roles)_ | — |

The maintainer team is actively looking for reviewers. If you have a
sustained record of high-quality patches in any of the areas below,
please open an issue with the label `governance:reviewer-nomination`.

### Review areas

- **bootstrap compiler:** `crates/ori-compiler/*` (lexer, parser,
  resolver, type-check, effects, borrow, codegen).
- **runtime + interpreter:** `crates/ori-compiler/src/interp*`, async
  runtime, MIR execution.
- **package manager + registry:** `crates/ori-pkg/*`, version solver,
  lockfile, SBOM.
- **CLI + agent surface:** `crates/ori-cli/*`, `crates/ori-agent/*`,
  envelope schemas under `schemas/*.schema.json`.
- **LSP + editor integrations:** `crates/ori-lsp/*`,
  `extensions/vscode/*`, TreeSitter grammar.
- **standard library:** `stdlib/core`, `stdlib/std`, `stdlib/app`,
  `stdlib/platform`, `stdlib/labs`.
- **documentation + tutorials:** `README.md`, `docs/*`,
  tutorial fixtures.
- **governance + community:** `GOVERNANCE.md`, `CODE_OF_CONDUCT.md`,
  RFC process.

---

## Decision log

Significant governance decisions (maintainer additions, removals,
stability vote outcomes) are recorded here in reverse chronological
order. Each entry links to the originating issue or PR.

| Date | Decision | Link |
|---|---|---|
| 2026-05-17 | Seed maintainer team formed; GOVERNANCE.md adopted. | (initial commit) |

---

## Stepping down

Maintainers and reviewers may step down at any time by opening a PR
that removes their entry from this file. The PR is merged by another
maintainer with a brief acknowledgement; no vote is required.

---

## Inactivity

Reviewers with no review activity in 6 months and maintainers with no
review or commit activity in 12 months may be moved to an `emeritus`
section below by a maintainer-team vote (see `GOVERNANCE.md §1.2-1.3`).
Emeritus members may rejoin at the same level via a single maintainer
sponsorship.

### Emeritus

_(empty)_
