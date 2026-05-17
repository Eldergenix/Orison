---
name: Bug report
about: A reproducible defect in the Orison compiler, CLI, agent ABI, or tooling
title: "bug: "
labels: [bug, triage]
assignees: []
---

<!--
Before filing: please confirm you have read AGENTS.md, docs/CONTRIBUTING.md,
and docs/QUALITY_GATES.md. Bugs caused by skipping the documented workflow
will be closed without action.
-->

## Summary

<!-- One sentence: what is broken? -->

## Environment

- Orison commit (or release): <!-- e.g. main @ abc1234 -->
- Output of `ori doctor`:

```text
<paste here>
```

- OS and architecture (e.g. macOS 14 / aarch64-apple-darwin):
- Rust toolchain (`rustc --version`):
- Python version (`python3 --version`):

## Minimal reproducer

<!-- Smallest possible input that triggers the bug. Prefer a single .ori file
plus a single command. If the bug is in a JSON contract, include the input
JSON and the schema it should match. -->

Steps:

1.
2.
3.

Command:

```bash
```

## Expected behaviour

<!-- What should have happened? Reference the schema, doc, or test that
defines correctness. -->

## Actual behaviour

<!-- What actually happened? Include the full error output and the JSON of
any structured diagnostics (`ori check --json`). -->

```text
```

## Quality gate status on the reproducer

- [ ] `python3 scripts/validate_all.py --static-only` passes on the
      reproducer repo state.
- [ ] `cargo test --workspace` passes (or fails with a different error than
      this bug).
- [ ] The bug is reproducible from a clean clone (no uncommitted state).

## Severity

- [ ] Blocks the compiler from producing output
- [ ] Produces wrong output silently (correctness)
- [ ] Breaks a documented public JSON contract
- [ ] Regresses a benchmark in `BENCHMARKS.md`
- [ ] Cosmetic / documentation only

## Additional context

<!-- Logs, screenshots, links to related issues or to commits that introduced
the bug. -->
