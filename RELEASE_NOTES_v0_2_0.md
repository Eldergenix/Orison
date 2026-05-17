# Orison v0.2.0 — Alpha-shaped bootstrap

## Summary

After four parallel agent build-out waves (30 sub-agents total) on
2026-05-16, the Orison bootstrap reaches alpha shape: a real CST + AST
pipeline, body-level analysis (parser, type inference, exhaustive
match, constant folding, executing interpreter), a borrow checker
prototype, effect propagation through the call graph, real wasm
bytecode emission, a comprehensive LSP, a full package manager with
SBOM/audit/provenance + local registry stub, schema and rpc importers,
documentation + migration tooling, mobile manifest generation, a
cooperative async scheduler, and 26 schema-versioned JSON contracts.

## Highlights

- **348+ passing tests**, `python3 scripts/validate_all.py --full`
  green.
- **30 CLI subcommands** across check, fmt, agent, patch, lsp,
  package, audit, sbom, provenance, run, build, bench, openapi, ui,
  wasm, capability, test, docs, migrate, db, coverage, schema import,
  publish, fetch, registry, design, preprocess, doctor.
- **Real wasm bytecode**: hand-rolled LEB128 encoder produces a
  validating 39-byte hello-module exporting `main -> i32 42`.
- **Executing interpreter**: `ori run examples/hello.ori` now reports
  `value:` from the body parser + tree-walking executor.
- **6 example apps**: demo_store (full-stack), todo_app (CRUD), blog
  (auth-gated), chat (websocket + queue), counter (minimal UI),
  feed_aggregator (HTTP + queue).
- **24+ stdlib modules** across core / std / app / platform / labs.
- **LSP**: hover, completion, rename, code actions, workspace symbols,
  document symbols, go-to-definition, references.
- **Security audit suite**: capability bypass, lockfile tamper, SBOM
  shape, provenance failure, unsafe-surface report (asserts zero
  workspace-wide), capability runtime denial.
- **Bench**: real numbers committed in BENCHMARKS.md and
  BENCHMARKS.results.json (warm check ~20µs, cold check ~2.5µs, patch
  validate ~0.7µs).
- **CI/CD**: GitHub Actions workflows (static / test / release /
  sbom), Makefile help targets, PR + issue templates,
  CONTRIBUTING.md, CI.md.

## Schemas

26 contract schemas: `agent-changed`, `agent-map`, `agent-symbol-list`,
`agent-tests`, `audit-report`, `benchmark`, `build-report`,
`capability`, `capsule`, `change`, `coverage-report`, `design-tokens-report`,
`diagnostic`, `doctor`, `graphql-import`, `lockfile`, `lsp-code-action`,
`manifest`, `migration-graph`, `migration-report`, `mobile-manifest`,
`openapi-report`, `patch`, `patch-check`, `preprocess`, `provenance`,
`publish-receipt`, `registry-list`, `rpc-import`, `sbom`,
`symbol-card`, `ui-manifest`, `wasm-component`.

## What's *not* in v0.2.0

See `docs/ROADMAP.md` for the gap between this release and production
grade. Notable absences: real native AOT codegen, M:N async runtime,
cryptographic registry signing, region-inference borrow checker,
full bidirectional type inference, and Rust-equivalent optimisation
passes. The bootstrap ships *contracts* designed to remain stable as
those land.

## Upgrade notes

- `import app.service` → `import app.services` (renamed because
  `service` is a reserved keyword).
- `import std.db` → `import std.sql` (renamed to match the shipped
  module).
- See `ori migrate --from 2027.1 --to 2028.1 --dry-run --json` for
  automated detection.

## Honest scope reminder

Orison v0.2.0 is alpha-shaped, not production. Subsystems listed in
`docs/ROADMAP.md` under "What's *not* yet shipping" must not be
described as working beyond their stated shape.
