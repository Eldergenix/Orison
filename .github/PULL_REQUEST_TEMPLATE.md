<!--
Thanks for contributing to Orison. Fill in every section. Reviewers will
bounce PRs that leave required sections blank.

Required reading before opening this PR:
- AGENTS.md (mandatory for AI coding agents)
- ORISON_AGENT_DEVELOPMENT_HANDOFF.md (authoritative gates and roadmap)
- docs/CONTRIBUTING.md (developer workflow)
- docs/QUALITY_GATES.md (validation pyramid)
- docs/CI.md (workflow matrix)
-->

## Change summary

<!-- One paragraph describing what changed and why. Link to the relevant
TASKS.md entry or handoff milestone (e.g. "M0 — Repository control plane"). -->

-

## Scope

<!-- Tick exactly the boxes that apply. -->

- [ ] Compiler (`crates/ori-compiler/`)
- [ ] Agent ABI (`crates/ori-agent/`)
- [ ] CLI (`crates/ori-cli/`)
- [ ] Standard library (`stdlib/`)
- [ ] Documentation (`docs/`, root `.md` files)
- [ ] Tooling / CI / hooks (`scripts/`, `.github/`, `Makefile`, `.githooks/`)
- [ ] Schemas (`schemas/`)
- [ ] Examples (`examples/`)
- [ ] Other (describe):

## Schema and contract impact

<!-- Required if the PR touches schemas/, public JSON output, CLI flags, or
the agent ABI. Otherwise write "none". -->

- Public schemas changed:
- New schema versions introduced:
- Backwards compatibility plan (additive change, dual-version coexistence, or
  approved breaking change with migration note in CHANGELOG.md):
- Example fixtures updated:

## Test changes

<!-- Describe new or updated tests. New behaviour without a test is a bug
waiting to happen and will block merge. -->

- New tests added (paths):
- Golden fixtures updated (paths):
- Manual reproductions performed:

## Documentation updates

- [ ] `CHANGELOG.md` updated for externally visible changes.
- [ ] `TASKS.md` status updated where relevant.
- [ ] `MEMORY.md` updated if an architectural decision changed or a new
      dependency was added.
- [ ] `docs/` updated if language, compiler, or contract semantics changed.
- [ ] Inline doc comments updated for changed public Rust APIs.

## Quality gate status

Run locally before requesting review. Tick after the command exits 0.

- [ ] `make gate-fast` (static gate, no Rust toolchain required)
- [ ] `make gate-pre-commit` (static + `cargo fmt --check` + `cargo check`)
- [ ] `make gate-full` (full local equivalent of CI)
- [ ] `make test`
- [ ] `make fmt-check`
- [ ] `make clippy`
- [ ] `make check` and any other CLI smoke tests touched by the change

If any gate was skipped, explain why (e.g. "macOS-only path; ran on Linux
runner via CI"):

## Risk and rollback

<!-- One paragraph. What is the blast radius if this regresses production?
How do we roll it back? Reference the relevant commit or schema version. -->

-

## Follow-ups

<!-- Anything intentionally deferred. Link to a TASKS.md entry or an issue. -->

-
