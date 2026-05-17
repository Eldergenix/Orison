# Orison Bootstrap Integration Report

> **Date:** 2026-05-16.
> **Scope:** End-to-end summary of what the bootstrap toolchain delivers
> after **waves 1, 2, 3, and 4** of the multi-agent build-out (30 sub-agents
> dispatched in parallel across the 4 waves).
> **Status:** Bootstrap + alpha-shaped. See `MEMORY.md` D014 for the honest
> scope statement. The "production-ready" bar (full type inference inside
> bodies executing arbitrary expressions, complete borrow checker with
> region inference, native codegen with optimisation passes, cryptographic
> registry + signed artefacts, conformance against a real language spec)
> is **not yet met** and is years of work past the current shape. See
> `docs/ROADMAP.md` for the explicit delta.

## Workspace

| Crate          | LOC (approx) | Notes                                                          |
|----------------|--------------|----------------------------------------------------------------|
| `ori-compiler` | ~15,000      | Lexer, parser, CST, resolver, type checker, body parser, type inference, exhaustive check, const fold, effects, effect propagation, borrow checker, HIR, MIR, interpreter, executing interpreter, async scheduler, patch apply, bench, openapi, ui, design tokens, mobile, wasm component, wasm bytecode encoder, textual codegen, docs, migrate, query, formatter, incremental, coverage, graphql import, rpc import, preprocessor, SQL DSL, migration graph |
| `ori-agent`    | ~400         | Agent map, symbol cards, diagnose, symbols, tests, changed, doctor |
| `ori-cli`      | ~2,500       | 30+ subcommands wired across all crates                        |
| `ori-lsp`      | ~2,400       | LSP with hover, completion, rename, code actions, workspace symbols, document symbols, go-to-def, references |
| `ori-pkg`      | ~2,200       | Manifest, lockfile, SBOM, audit, provenance, local registry stub |

Total: **455 passing tests / 0 failing** (last verified
`cargo test --workspace` on `aarch64-apple-darwin`, rustc 1.92,
`python3 scripts/validate_all.py --full` green).

## Schemas (33 published)

`agent-changed`, `agent-map`, `agent-symbol-list`, `agent-tests`,
`audit-report`, `benchmark`, `build-report`, `capability`, `capsule`,
`change`, `coverage-report`, `design-tokens-report`, `diagnostic`,
`doctor`, `graphql-import`, `lockfile`, `lsp-code-action`, `manifest`,
`migration-graph`, `migration-report`, `mobile-manifest`,
`openapi-report`, `patch`, `patch-check`, `preprocess`, `provenance`,
`publish-receipt`, `registry-list`, `rpc-import`, `sbom`, `symbol-card`,
`ui-manifest`, `wasm-component`.

All conform to Draft 2020-12.

## CLI surface (30+ commands)

```
ori check / fmt / capsule
ori agent map / explain / symbols / diagnose / tests / changed
ori patch check / apply / dry-run / explain
ori lsp --stdio
ori package check
ori audit / sbom / provenance verify
ori publish / fetch / registry list / registry yank
ori run [--entry <name>]
ori build [--target dev|release|wasm-component|llvm-text|mobile]
ori bench [--samples N]
ori openapi / ui / wasm / capability / design check
ori test [--changed]
ori coverage / db check
ori docs [--format human|agent] [--budget N]
ori migrate --from X --to Y [--dry-run]
ori schema import graphql / grpc
ori preprocess
ori doctor
```

## Standard distribution (28 modules)

```
stdlib/core/{option, result, iter, string, bytes, list, numeric}.ori
stdlib/std/{json, http, validation, logging, config, time, sql,
           queue, mail, websocket, process, tasks, cache, url}.ori
stdlib/app/{services, views, auth}.ori
stdlib/platform/{web, mobile}.ori
stdlib/labs/experimental.ori
```

Every module starts with `module <name>`, declares its effects via
known names or capabilities, and produces clean `ori check --json`.

## Example apps (7)

```
examples/demo_store/      — canonical full-stack storefront (Stage 1 ✓)
examples/todo_app/        — minimal CRUD example
examples/blog/            — auth-gated routes + status variants
examples/chat/            — websocket + queue + variant payloads
examples/counter/         — single-view minimal UI demo
examples/feed_aggregator/ — periodic worker over HTTP + queue
examples/fullstack/       — legacy users-service example
```

All four parse clean, emit valid OpenAPI / UI / WASM / capability JSON,
and (where applicable) drive `ori run` to a non-error exit.

## Diagnostic ID space

| Prefix     | Subsystem                                                |
|------------|----------------------------------------------------------|
| `E00**`    | Lexer / parser structural errors                         |
| `E0100`    | `null` usage                                             |
| `E0101`    | `throw` usage                                            |
| `E02**`    | Symbol resolution (duplicates, imports, cycles)          |
| `E04**`    | Effects (`E0410` undeclared, `E0420` propagation)        |
| `E05**`    | Type system (`E0540` exhaustive, `E0541` redundant)      |
| `W03**`    | Style (missing return type, ...)                         |
| `W04**`    | Effect warnings (`W0401` unknown effect)                 |
| `W05**`    | Type warnings (unknown type, missing generic arity)      |
| `B00**`    | Borrow / ownership                                       |
| `P0***`    | Patch IR validation                                      |
| `P1***`    | Patch IR application                                     |
| `Q00**`    | Query / SQL                                              |
| `E11**`    | Body parser                                              |
| `R00**`    | Runtime / interpreter                                    |

## Benchmarks (Apple Silicon, n=100)

See `BENCHMARKS.md` for the full table; raw data lives in
`BENCHMARKS.results.json`. Headlines:

- Cold check: ~2.5 µs.
- Warm check: ~20 µs.
- CST parse: ~28 µs.
- Patch validate: ~0.7 µs.
- Patch apply (dry-run): ~9.5 µs.
- Formatter: ~1.2 µs.
- Capsule generate: ~23 µs.

These are deterministic in-process measurements after 2 warm-up
iterations; not portable across architectures.

## What is *not* in this report

- Native binary codegen (LLVM/Cranelift) — only textual IR + minimal
  wasm bytecode shipping.
- Complete pattern matching with nested / guard arms — bootstrap covers
  literal + variable patterns.
- Distributed registry with cryptographic signing — lockfile checksum
  is a deterministic non-crypto stand-in.
- Full mobile build pipeline (Xcode / Android Gradle integration).
- Async / await execution semantics — the keywords lex but the runtime
  does not yet schedule.
- Live model-in-the-loop benchmark.

Each of those is named in `ORISON_AGENT_DEVELOPMENT_HANDOFF.md` as a
follow-up milestone; the current bootstrap *prepares* for them with
versioned schemas, deterministic JSON contracts, and stable CLI shapes
so that landing them does not break agent integrations.

## Quality gate

```
python3 scripts/validate_all.py --full
```

Last full run on 2026-05-16: **validation passed** — comprising rustfmt
`--check`, `cargo check --workspace --all-targets`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, and the six CLI contract smoke commands.
