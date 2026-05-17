# Orison

**Orison** is an agent-native programming language and toolchain for building safe, fast, full-stack applications with a tight edit-check-repair loop.

Orison is designed for developers who want:

- Rust-class safety goals without Rust-class iteration friction.
- Python-like readability without giving up static checking.
- Go-like toolchain simplicity with richer application frameworks.
- Stable JSON diagnostics and compiler-generated context for AI coding agents.
- A standard distribution broad enough that most production apps do not start by assembling a dependency pile.

> Status: `0.1-bootstrap`. This repository contains the reference compiler scaffold, public specs, JSON contracts, validation gates, and example applications. It is not yet a production compiler. The current CLI can parse a limited syntax subset, emit diagnostics, generate agent maps/capsules, validate Patch IR, and support continuation work.

---

## Why Orison exists

Modern application development has converged on a few recurring tradeoffs:

| Problem | Orison direction |
|---|---|
| Safe languages can have slow or complex build loops. | Keep safety constant, split dev/release backends, and make incremental checking a primary compiler feature. |
| Fast languages can push safety onto the developer. | Make safe code the default: no null, no exceptions, explicit effects, explicit mutation, and capability-scoped external access. |
| AI agents waste context reading whole repositories. | Emit semantic capsules, symbol cards, budgeted agent maps, diagnostics, and structural patches as stable machine-readable contracts. |
| Full-stack apps repeat boilerplate across backend, database, API, UI, and mobile layers. | Ship typed framework primitives for services, routes, queries, schemas, UI views, design tokens, and capabilities. |
| Dependency sprawl slows builds and increases supply-chain risk. | Provide a standard distribution with `core`, `std`, `app`, `platform`, and `labs` layers. |
| Compiler errors often require humans to translate intent into fixes. | Make diagnostics patch-native with stable IDs, spans, minimal context, docs references, and repair candidates. |

The core invariant is:

> Anything the compiler knows should be available to tools and agents through stable structured output.

---

## Language at a glance

```ori
module store.users

import std.db
import std.json
import app.service
import app.ui

type UserId wraps UUID
type Email wraps Str

type User = {
  id: UserId,
  name: Str,
  email: Email
}

type ApiErr =
  | NotFound
  | BadEmail(Str)
  | Db(DbErr)

fn fetch_user(id: UserId) -> Result[User, ApiErr] uses db.read:
  let user = db.users.find(id)?
  return Ok(user)

service Users uses http, db.read, db.write:
  get "/users/{id:UserId}" -> Result[User, ApiErr]:
    return fetch_user(id)

view UserCard(user: User) -> Html uses ui:
  card:
    heading(level: 2, text: user.name)
    text(user.email.value)
```

Key language choices:

- **No `null`:** use `Option[T]`.
- **No exceptions:** use `Result[T, E]`.
- **Explicit effects:** functions declare `uses db.read`, `uses net.outbound`, `uses ui`, and similar capabilities.
- **Explicit mutation:** immutable bindings are the default.
- **Typed application constructs:** services, routes, queries, views, migrations, and capabilities are part of the checked language surface.
- **Agent-visible semantics:** public compiler output is structured JSON, not scraped terminal text.

---

## Current toolchain utilities

The bootstrap CLI is named `ori`.

| Command | Purpose | Current status |
|---|---|---|
| `ori check [--json] <file.ori>` | Parse a source file and emit diagnostics. | Implemented for scaffold syntax. |
| `ori fmt <file.ori>` | Format source. | Minimal whitespace formatter. |
| `ori capsule --json <file.ori>` | Emit a module semantic capsule. | Implemented for parsed symbols/imports/effects. |
| `ori agent map --budget N --json <file.ori>` | Emit budgeted agent context. | Implemented for a single source file. |
| `ori agent explain <symbol> --json <file.ori>` | Emit a symbol card. | Implemented for parsed symbols. |
| `ori agent capsule --json <file.ori>` | Agent alias for capsule output. | Implemented. |
| `ori patch check --json <patch.json>` | Validate structural Patch IR. | Implemented with semantic checks. |
| `ori doctor` | Emit toolchain health JSON. | Implemented as bootstrap report. |

Planned commands:

| Command | Purpose |
|---|---|
| `ori run` | Run a development build. |
| `ori test` | Run Orison tests. |
| `ori test --changed` | Run tests affected by changed symbols. |
| `ori build --dev` | Fast development build. |
| `ori build --release` | Optimized native build. |
| `ori build --target wasm-component` | WebAssembly component build. |
| `ori schema import openapi` | Import OpenAPI as typed Orison APIs. |
| `ori migrate` | Apply edition migrations. |
| `ori package audit` | Audit dependencies, capabilities, SBOM, and provenance. |

---

## Try the bootstrap CLI

Install local quality hooks, then run the available bootstrap commands:

```bash
./scripts/install_hooks.sh
python3 scripts/validate_all.py --static-only
cargo run -p ori -- doctor
cargo run -p ori -- check examples/hello.ori
cargo run -p ori -- check --json examples/bad_null.ori
cargo run -p ori -- agent map --budget 2000 --json examples/fullstack/users.ori
cargo run -p ori -- agent explain sym:store.users.fetch_user --json examples/fullstack/users.ori
cargo run -p ori -- capsule --json examples/fullstack/users.ori
cargo run -p ori -- patch check --json examples/agent_patch.json
```

Run the full quality gate on a Rust-capable machine:

```bash
make quality-gate
```

The full gate checks repository layout, JSON contracts, schema instances, shell hooks, source guardrails, Rust formatting, `cargo check`, clippy, workspace tests, and CLI smoke contracts.

---

## Demo application: Storefront

A guided demo application lives at:

```text
examples/demo_store/
```

The demo shows how Orison is intended to support a production-style app across domain models, typed database queries, backend services, UI views, capabilities, tests, and agent patch workflows.

Start here:

```bash
cat examples/demo_store/README.md
cat docs/examples/DEMO_APPLICATION.md
```

Bootstrap-compatible smoke commands:

```bash
cargo run -p ori -- check --json examples/demo_store/src/domain.ori
cargo run -p ori -- check --json examples/demo_store/src/api.ori
cargo run -p ori -- capsule --json examples/demo_store/src/api.ori
cargo run -p ori -- agent map --budget 3000 --json examples/demo_store/src/api.ori
cargo run -p ori -- patch check --json examples/demo_store/contracts/agent_patch_add_product_search.json
```

Future production-oriented commands, once the compiler/runtime are implemented:

```bash
ori check examples/demo_store
ori test examples/demo_store --changed
ori run examples/demo_store
ori build examples/demo_store --target wasm-component
ori build examples/demo_store --release
ori agent map examples/demo_store --budget 8000 --json
```

The demo is intentionally both a developer showcase and an implementation target. Any agent extending Orison should keep the demo app compiling against the newest implemented surface area.

---

## Agent-native development

Orison is designed so humans and AI agents share the same compiler truth.

Agent-facing artifacts:

| Artifact | Purpose |
|---|---|
| JSON diagnostics | Stable, schema-versioned compiler errors and warnings. |
| Semantic capsules | Compact module exports, imports, effects, invariants, and summaries. |
| Symbol cards | Single-symbol context for small targeted changes. |
| Agent maps | Budgeted repository/module context. |
| Patch IR | Structural source edits that can be validated before application. |
| Change manifests | Human/audit metadata for security, public API, capability, and test impact. |
| Affected-test graph | Planned mechanism for cheap, targeted validation. |

Example loop:

```bash
ori agent map --budget 4000 --json examples/demo_store/src/api.ori
ori check --json examples/demo_store/src/api.ori > diagnostics.jsonl
ori agent explain sym:demo.store.api.StoreApi --json examples/demo_store/src/api.ori
ori patch check --json examples/demo_store/contracts/agent_patch_add_product_search.json
ori test --changed
```

The goal is to reduce context consumption by replacing repository dumps with compiler-generated, schema-checked context.

---

## Standard distribution

Orison is specified as a standard distribution, not just a narrow standard library.

| Layer | Intended contents |
|---|---|
| `core` | Primitive types, `Option`, `Result`, iterators, memory helpers, testing base. |
| `std` | JSON, HTTP, filesystem, path, crypto, logging, config, validation, SQL, queues, schemas. |
| `app` | Services, routing, middleware, auth, forms, UI, design tokens, jobs, deployment, observability. |
| `platform` | Web, Wasm, iOS, Android, desktop, edge, GPU, tensor adapters. |
| `labs` | Experimental agent, LLM, autodiff, robotics, embedded, and SIMD modules. |

This repository currently specifies these modules and implements only the bootstrap compiler contracts required to continue building them.

---

## Safety model

Orison safe code is intended to prevent:

- invalid memory access;
- use-after-free;
- data races;
- unchecked shared mutable state;
- unhandled fallible results;
- accidental `null`-style absence;
- exception-style control flow;
- ambient filesystem, network, process, database, or environment access.

Effects and capabilities are part of function signatures:

```ori
fn charge(card: CardToken, amount: Money) -> Result[Charge, PayErr] uses StripeApi:
  return payments.charge(card, amount)
```

A package cannot silently acquire new external authority without its capability manifest changing.

---

## Repository map

```text
.
├── README.md                              # Developer-facing Orison overview
├── AGENTS.md                              # Mandatory instructions for AI coding agents
├── MEMORY.md                              # Durable architecture decisions
├── TASKS.md                               # End-to-end implementation backlog
├── GOAL.md                                # Product and technical goals
├── CHANGELOG.md                           # Versioned project history
├── VALIDATION.md                          # Validation status and commands
├── ORISON_AGENT_DEVELOPMENT_HANDOFF.md    # Authoritative continuation plan
├── crates/
│   ├── ori-compiler/                      # Reference compiler scaffold
│   ├── ori-agent/                         # Agent context outputs
│   └── ori-cli/                           # `ori` CLI
├── docs/
│   ├── language/                          # Language specification
│   ├── compiler/                          # Compiler and agent ABI specs
│   ├── examples/                          # Demo application guidance
│   ├── frameworks/                        # Backend/UI/API/mobile framework specs
│   ├── security/                          # Security model
│   ├── stdlib/                            # Standard distribution
│   └── roadmap/                           # MVP and milestone docs
├── schemas/                               # Public JSON schemas
├── examples/
│   ├── fullstack/                         # Small full-stack source example
│   └── demo_store/                        # Guided demo application
├── scripts/                               # Validation and hook scripts
├── tests/                                 # Golden fixtures
└── prompts/                               # Agent prompt templates
```

---

## Contributing to the language implementation

Future agents and engineers should start with:

```bash
cat GOAL.md MEMORY.md TASKS.md AGENTS.md ORISON_AGENT_DEVELOPMENT_HANDOFF.md VALIDATION.md
python3 scripts/validate_all.py --static-only
make quality-gate
```

Implementation priority:

1. Replace the scaffold parser with an error-tolerant CST.
2. Add AST lowering and stable node IDs.
3. Implement name resolution and module graphs.
4. Implement the type checker.
5. Implement effect and capability checking.
6. Implement ownership analysis.
7. Implement Patch IR application.
8. Add affected-symbol and affected-test graphs.
9. Implement `ori run`, `ori test`, and the dev backend.
10. Keep `examples/demo_store` as the end-to-end developer showcase.

Rules:

- Do not weaken validation gates to make a change pass.
- Do not hand-build public JSON output.
- Do not introduce `unwrap()`, `expect()`, `panic!`, `todo!`, `unimplemented!`, `dbg!`, or unsafe Rust in production compiler source.
- Do not add dependencies without rationale, tests, and changelog notes.
- Do not claim a feature is implemented because it is specified; distinguish current behavior from intended behavior.

---

## Quality gates

Install hooks:

```bash
./scripts/install_hooks.sh
```

Run static validation without Rust:

```bash
python3 scripts/validate_all.py --static-only
```

Run contract validation:

```bash
python3 scripts/validate_all.py --contracts-only
```

Run the full gate:

```bash
python3 scripts/validate_all.py --full
# or
make quality-gate
```

Pre-commit and pre-push hooks call the same validation entry point, so local checks and CI remain aligned.

---

## Current bootstrap limitation

The files in `docs/` and `examples/demo_store/` describe the intended language and application experience. The Rust implementation currently supports only enough syntax and contracts to serve as a safe continuation scaffold.

A feature is production-ready only when it has:

- compiler implementation;
- unit and golden tests;
- CLI contract tests;
- JSON schema coverage where applicable;
- documentation;
- demo-app coverage where applicable;
- passing full quality gate.
