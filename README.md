# Orison Language Kit

**Orison** is a bootstrap repository for an agent-native programming language: a safe compiled full-stack language with Rust-class safety goals, Python-like readability, fast incremental builds, JSON-first diagnostics, structural patching, and compiler-native context artifacts for AI coding agents.

This repository is not a finished language implementation. It is a technical scaffold designed so an AI agent can continue implementation end-to-end with stable goals, schemas, task boundaries, tests, and compiler starter code.

## What's actually implemented

Honest scope snapshot. Use this matrix before quoting Orison capabilities anywhere external.
The authoritative breakdown is in `ORISON_AGENT_DEVELOPMENT_HANDOFF.md` (milestones M0–M19).

Updated 2026-05-16 after the multi-agent build-out (waves 1–4 — 30
sub-agents in total). **455 passing tests, 0 failing,
`python3 scripts/validate_all.py --full` green.** The full breakdown is
in `docs/INTEGRATION_REPORT.md`; the authoritative milestone plan
remains `ORISON_AGENT_DEVELOPMENT_HANDOFF.md`.

| Shipped (tested, gated, schema-versioned)                                                   | Skeleton (functional but explicitly bootstrap-grade)                                | Not yet (planned)                                |
|---------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------|--------------------------------------------------|
| Lexer + source manager with spans                                                           | Error-tolerant CST with content-derived stable node IDs                             | Production-grade native codegen with optimisation |
| Symbol-level + body-level parser (item AST + expression AST)                                | Body parser does not yet model binary operators or string interpolation             | Full mobile build pipeline (Xcode / Android Gradle) |
| Multi-module resolver with namespace separation, duplicates, cycles                         | Signature-level type checker + initial body type inference                          | Async / await runtime scheduling                  |
| CST-preserving formatter, idempotent                                                        | Effect propagation through call graph (`E0420` with `change_signature` fix)         | Macro / metaprogramming system                    |
| Patch IR validation (`ori.patch_check.v1`) + apply / dry-run / explain with stable IDs       | Borrow checker prototype (signature-level: B0010–B0050)                             | Cryptographic registry signing (lockfile checksum is FNV-1a stand-in) |
| Typed JSON diagnostics (`ori.diagnostic.v1`) with structured fixes                           | Tree-walking interpreter (`ori run` reports effects + executes simple bodies)       | Live model-in-the-loop benchmark harness          |
| Semantic capsules, agent maps, symbol cards, `agent diagnose`, `agent symbols`, `agent tests --affected`, `agent changed` | Wasm bytecode encoder (hand-rolled LEB128, hello-module = 39 bytes)                 | Conformance against the full intended SPECIFICATION |
| Capability manifest + policy diff (`E0410` undeclared, capability runtime denial test)       | Textual LLVM-IR-style codegen                                                       |                                                  |
| Exhaustive match check (`E0540`) + constant folding pass                                     | Documentation generator (`ori docs --format human / agent --budget N`)              |                                                  |
| Query engine + per-symbol fingerprints + one-hop invalidated dependents                      | Edition migration tool (`ori migrate --from / --to --dry-run`)                      |                                                  |
| Package manager (`ori-pkg`): manifest / lockfile / SBOM / audit / provenance                | SQL query type-shape checker (`Q0010` / `Q0020`) + migration toposort               |                                                  |
| LSP server (`ori-lsp`): hover, completion, rename, code actions from Patch IR fixes          | Per-symbol incremental cache (file hash + symbol fingerprint, single-process)       |                                                  |
| Security audit suite: capability bypass, lockfile tamper, SBOM shape, provenance failure, unsafe-surface report (asserts zero) | OpenAPI 3.1 extraction, UI manifest with a11y heuristics, wasm component manifest    |                                                  |
| CI/CD scaffolding: static / test / release / sbom workflows + Makefile targets               | Standard distribution: 20 modules across `core` / `std` / `app` / `platform` / `labs` |                                                  |
| `ori` CLI surface (27+ subcommands)                                                          | Four example apps: `demo_store`, `todo_app`, `blog`, `chat`                         |                                                  |
| 23 schema-versioned JSON contracts                                                           | Benchmark harness with real measurements (`BENCHMARKS.md` + `BENCHMARKS.results.json`) |                                                  |
| 249 passing tests; `python3 scripts/validate_all.py --full` green                            | Real conformance suite with 25 golden fixtures                                      |                                                  |

If a subsystem above is listed as "Skeleton" or "Not yet", do not describe it as working
*beyond* its stated shape in documentation, blog posts, or agent prompts. The bootstrap
ships with stable JSON contracts and CLI shapes so the next phases land without breaking
agent integrations.

## Quick start

The validated command list from `AGENTS.md`. Every command runs from a clean clone.

```bash
./scripts/install_hooks.sh
python3 scripts/validate_all.py --static-only
cargo fmt --all --check
cargo test --workspace
cargo run -p ori -- check --json examples/hello.ori
cargo run -p ori -- agent map --budget 2000 --json examples/fullstack/users.ori
```

After making a change, run:

```bash
cargo fmt --all --check
cargo test --workspace
cargo run -p ori -- check --json examples/hello.ori
cargo run -p ori -- check --json examples/fullstack/users.ori
cargo run -p ori -- capsule --json examples/fullstack/users.ori
cargo run -p ori -- patch check --json examples/agent_patch.json
python3 scripts/validate_all.py --full
```

See `docs/QUALITY_GATES.md` for the validation pyramid and
`docs/ARCHITECTURE_OVERVIEW.md` for the crate map.

## Product definition

Orison is intended to be:

- **Fast at runtime:** native AOT and WebAssembly-oriented compilation.
- **Fast to build:** incremental parser, query compiler, per-symbol invalidation, and a fast dev backend.
- **Safe by default:** no null, no exceptions, no unchecked shared mutation, no ambient capabilities.
- **Agent-compatible:** stable JSON diagnostics, semantic capsules, patch IR, symbol cards, and project maps.
- **Full-stack:** backend services, typed APIs, database access, UI views, design tokens, web, mobile, and Wasm targets.
- **Low-context:** compiler-generated summaries let agents request only the symbols, effects, tests, and docs required for a change.

## Repository map

```text
.
├── README.md                     # Overview and local usage
├── AGENTS.md                     # Mandatory operating instructions for coding agents
├── MEMORY.md                     # Durable architectural decisions
├── TASKS.md                      # End-to-end implementation backlog
├── GOAL.md                       # Product, technical, and agentic goals
├── CHANGELOG.md                  # Versioned project history
├── VALIDATION.md                 # Validation status for this archive
├── ORISON_AGENT_DEVELOPMENT_HANDOFF.md # Authoritative continuation plan for AI agents
├── CODE_REVIEW_REMEDIATION.md    # Review-blocking issues fixed in v0.1.1
├── Cargo.toml                    # Rust workspace for reference compiler scaffold
├── rust-toolchain.toml           # Toolchain pin for reproducible local work
├── crates/
│   ├── ori-compiler/             # Lexer/parser/diagnostics/capsule/patch contracts
│   ├── ori-agent/                # Agent context maps and symbol cards
│   └── ori-cli/                  # `ori` command-line tool
├── docs/                         # Language, compiler, framework, stdlib, security specs
├── schemas/                      # JSON Schemas for public agent/compiler contracts
├── examples/                     # Example Orison source and agent artifacts
├── scripts/                      # Local workflows, validation gates, and hook installers
├── .githooks/                    # Local pre-commit and pre-push quality gates
├── prompts/                      # Agent prompt templates and review checklists
├── tests/                        # Golden language and diagnostic fixtures
└── .github/workflows/ci.yml      # CI workflow
```

## Quick start

```bash
./scripts/install_hooks.sh
make quality-gate
cargo run -p ori -- doctor
cargo run -p ori -- check examples/hello.ori
cargo run -p ori -- check --json examples/bad_null.ori
cargo run -p ori -- agent map --budget 2000 --json examples/fullstack/users.ori
cargo run -p ori -- agent explain sym:store.users.fetch_user --json examples/fullstack/users.ori
cargo run -p ori -- capsule --json examples/fullstack/users.ori
cargo run -p ori -- patch check --json examples/agent_patch.json
```

## Current compiler scaffold

The starter compiler contains:

- source file model with spans and positions
- lexer for the MVP syntax surface
- symbol-oriented parser for modules, imports, declarations, signatures, and effects
- serde-backed JSON diagnostics
- semantic capsule generation
- agent map and symbol-card generation
- structured Patch IR validation
- CLI commands for `check`, `fmt`, `capsule`, `agent map`, `agent explain`, `patch check`, and `doctor`
- Rust tests for diagnostics, patch validation, capsules, agent maps, and formatting

The parser is intentionally minimal. It extracts module names, imports, top-level declarations, public symbols, effects, and simple style/safety diagnostics. The next implementation phase should replace this with the grammar-driven CST/AST pipeline described in `docs/language/GRAMMAR.ebnf` and `docs/compiler/ARCHITECTURE.md`.

## Dependency policy

The bootstrap implementation allows only foundational serialization dependencies:

- `serde`
- `serde_json`

Rationale: Orison's public compiler-agent contract is JSON. Hand-building JSON strings is not acceptable for a public schema contract. New dependencies require a rationale in `MEMORY.md`, tests, and a `CHANGELOG.md` entry.

## Required agent loop

```bash
cat GOAL.md MEMORY.md TASKS.md AGENTS.md
python3 scripts/validate_all.py --static-only
cargo fmt --all --check
cargo test --workspace
cargo run -p ori -- check --json examples/hello.ori
cargo run -p ori -- agent map --budget 2000 --json examples/fullstack/users.ori
# Make the smallest coherent change.
cargo fmt --all
cargo test --workspace
python3 scripts/validate_all.py --full
# Update TASKS.md, MEMORY.md, and CHANGELOG.md when architecture or milestones change.
```

## Design invariant

> Anything the compiler knows should be available to tools and agents through stable structured output.

That invariant drives the language, compiler, package manager, standard distribution, and framework strategy.


## Quality gates

Install local hooks:

```bash
./scripts/install_hooks.sh
```

Run static archive validation without Rust:

```bash
python3 scripts/validate_all.py --static-only
```

Run the full gate on a Rust-capable machine:

```bash
make quality-gate
```

The full gate checks repository layout, JSON/schema contracts, shell hooks, Rust source guardrails, formatting, clippy, workspace tests, and CLI contract smoke tests.

## Continuation handoff

Future coding agents should start with `ORISON_AGENT_DEVELOPMENT_HANDOFF.md`. It contains the remaining end-to-end implementation plan, quality gate policy, test matrix, schema policy, and first implementation tickets.
