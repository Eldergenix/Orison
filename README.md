# Orison Language Kit

**Orison** is an agent-native programming language and toolchain: a
safe compiled full-stack language with Rust-class safety, Python-like
readability, fast incremental builds, JSON-first diagnostics,
structural patching, and compiler-native context artefacts for AI
coding agents.

Status: **bootstrap + early-alpha**. The compiler ships, the contracts
are stable, the example apps round-trip end-to-end. Production-grade
parity with Rust/Swift/Go (full HM-style inference, region-inference
borrow checker, optimising native codegen, M:N async runtime,
cryptographic registry signing) is documented in `docs/ROADMAP.md` and
remains future work.

## What's shipping

- **5 crates** (`ori-compiler`, `ori-agent`, `ori-cli`, `ori-lsp`,
  `ori-pkg`) — ~32,000 LOC, only `serde` + `serde_json` as
  dependencies.
- **456 passing tests, 0 failing.** `python3 scripts/validate_all.py
  --full` is green (rustfmt + clippy `-D warnings` + cargo test + six
  CLI contract smoke commands).
- **34 schema-versioned JSON contracts** under `schemas/` covering
  diagnostics, patches, capsules, agent maps, symbol cards, audits,
  SBOMs, benchmarks, manifests, lockfiles, mobile manifests, wasm
  components, and more.
- **27 standard distribution modules** under `stdlib/` across
  `core / std / app / platform / labs`.
- **30+ CLI subcommands** — see `ori --help`.
- **Real WebAssembly bytecode** (hand-rolled LEB128 encoder, 39-byte
  validating hello-module).
- **Real LSP server** (hover, completion, rename, code actions,
  workspace symbols, document symbols, go-to-definition, references —
  no third-party deps).
- **Real package manager** (manifest, lockfile, SBOM, audit, provenance,
  local registry stub with publish/fetch/list/yank).
- **Six example apps:** `demo_store` (canonical full-stack storefront),
  `todo_app`, `blog`, `chat`, `counter`, `feed_aggregator`.

For the comprehensive shipping/skeleton/not-yet matrix and per-suite
benchmark numbers see `BENCHMARKS.md` and `docs/ROADMAP.md`.

## Quick start

Requires Rust 1.92 (pinned in `rust-toolchain.toml`).

```bash
# Install pre-commit / pre-push hooks
./scripts/install_hooks.sh

# Build the release CLI
cargo build --release -p ori

# Smoke-test the canonical demo
target/release/ori check --json examples/demo_store/src/api.ori
target/release/ori capsule --json examples/demo_store/src/api.ori
target/release/ori agent map --budget 2000 --json examples/demo_store/src/api.ori
target/release/ori openapi --json examples/demo_store/src/api.ori
target/release/ori ui --json examples/demo_store/src/ui.ori
target/release/ori wasm --json examples/demo_store/src/api.ori
target/release/ori run examples/demo_store/src/main.ori
target/release/ori bench --samples 50 --json > bench.json
```

Every command emits a JSON envelope conforming to a schema under
`schemas/`. See `BENCHMARKS.md` for the perf numbers, `docs/ROADMAP.md`
for what's still to come, and `examples/demo_store/README.md` for the
full demo walkthrough.

## Run the full quality gate

```bash
python3 scripts/validate_all.py --full
```

Runs: required-file existence + JSON contract parse + Rust source
guardrails + `cargo fmt --check` + `cargo check --workspace` + `cargo
clippy --workspace --all-targets -- -D warnings` + `cargo test
--workspace` + six CLI contract smoke commands.

## Design invariant

> Anything the compiler knows is available to tools and agents through
> stable structured output.

That invariant drives the language, compiler, package manager, standard
distribution, and framework strategy.

## Product definition

- **Fast at runtime** — native AOT + WebAssembly compilation targets.
- **Fast to build** — incremental query compiler, per-symbol
  invalidation, fast dev backend.
- **Safe by default** — no null, no exceptions, no unchecked shared
  mutation, no ambient capabilities.
- **Agent-compatible** — stable JSON diagnostics, semantic capsules,
  Patch IR, symbol cards, agent maps.
- **Full-stack** — backend services, typed routes, typed database
  queries, UI views, design tokens, web / mobile / Wasm targets.
- **Low-context** — compiler-generated summaries let agents request
  only the symbols, effects, tests, and docs needed for a change.

## Repository layout

```
.
├── README.md
├── LICENSE
├── CHANGELOG.md
├── CONTRIBUTING.md
├── SECURITY.md
├── BENCHMARKS.md
├── Cargo.toml             # Rust workspace
├── ori.toml               # package manifest
├── rust-toolchain.toml
├── Makefile
├── crates/
│   ├── ori-compiler/      # Lexer / parser / CST / type checker /
│   │                      # body parser / type inference / effect
│   │                      # propagation / borrow / HIR / MIR /
│   │                      # interpreter / wasm encoder / codegen
│   ├── ori-agent/         # Agent context maps + symbol cards
│   ├── ori-cli/           # `ori` command-line tool
│   ├── ori-lsp/           # Language Server Protocol implementation
│   └── ori-pkg/           # Package manifest / lockfile / SBOM /
│                          # audit / provenance / registry
├── docs/
│   ├── ROADMAP.md         # Delta to production-grade
│   ├── language/          # Language specification + reference
│   ├── compiler/          # Architecture, build system, diagnostics
│   ├── frameworks/        # Backend, UI, mobile, API & data
│   └── stdlib/            # Standard distribution overview
├── examples/              # Example .ori apps
├── schemas/               # Draft 2020-12 JSON Schema contracts
├── stdlib/                # Orison standard distribution sources
├── tests/golden/          # Cross-crate golden fixtures
├── scripts/               # Validation gate + hook installer
└── .githooks/             # Local pre-commit / pre-push gates
```

## Dependency policy

The bootstrap implementation allows only foundational serialisation
dependencies: `serde` and `serde_json`. Public compiler-agent contracts
are JSON; hand-building JSON strings is not acceptable for a public
schema contract. New dependencies require a rationale in
`CHANGELOG.md`.

## Contributing

See `CONTRIBUTING.md` for the developer workflow and `BENCHMARKS.md`
for the performance regression policy.

## License

Apache-2.0. See `LICENSE`.
