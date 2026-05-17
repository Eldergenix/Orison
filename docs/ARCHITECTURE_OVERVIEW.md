# Architecture Overview

This is a one-page map of the Orison codebase. It exists so an engineer or agent landing on
the repository can locate the right crate without reading every spec.

The long-form authoritative list of what is implemented, what is scaffolded, and what is
planned lives in `ORISON_AGENT_DEVELOPMENT_HANDOFF.md`. When this document and the handoff
disagree, the handoff wins.

## Crate map

```text
                          +-----------------------------------------------+
                          |                  ori-cli                      |
                          |  command dispatch: check / fmt / capsule /    |
                          |  agent map / agent explain / patch check /    |
                          |  doctor (planned: run, build, test, bench,    |
                          |  lsp, package, audit, sbom)                   |
                          +-----------------------+-----------------------+
                                                  |
            +-------------------------------------+-------------------------------------+
            |                                     |                                     |
            v                                     v                                     v
+-------------------------+      +---------------------------------+      +----------------------------+
|       ori-compiler      |      |           ori-agent             |      |           ori-pkg          |
|-------------------------|      |---------------------------------|      |----------------------------|
| source manager          |      | capsule producers               |      | ori.toml manifest parser   |
| lexer                   |      | agent map builder               |      | lockfile (planned)         |
| CST (planned)           |      | symbol-card builder             |      | dependency resolver (plan) |
| AST (planned)           |      | budget packer (planned)         |      | sbom emit (planned)        |
| resolver (planned)      |      | affected-test graph (planned)   |      | audit (planned)            |
| type checker (planned)  |      |                                 |      | capability diff (planned)  |
| effect checker (skeleton)|     +---------------------------------+      +----------------------------+
| borrow checker (planned)|                      ^
| HIR (planned)           |                      |
| MIR (planned)           |                      |
| interpreter (planned)   |                      |
| codegen (planned)       |                      |
| diagnostics (typed)     |                      |
| patch IR validator      |                      |
| formatter (whitespace)  |                      |
+-----------+-------------+                      |
            |                                    |
            +------------------+-----------------+
                               |
                               v
                  +----------------------------+
                  |          ori-lsp           |
                  |----------------------------|
                  | LSP server skeleton        |
                  | diagnostic parity (plan)   |
                  | hover/completion (planned) |
                  | code actions (planned)     |
                  +----------------------------+
```

## Pipeline (intended end state)

The intended ordering inside `ori-compiler` is:

```text
source text
  -> Lexer
  -> CST (error-tolerant, stable spans, preserved trivia)
  -> AST (lowered, stable node IDs)
  -> Resolver (modules, imports, visibility, symbol table)
  -> Type checker (records, variants, generics, Option/Result, protocols)
  -> Effect checker (effects, capabilities, package policy)
  -> Borrow checker (move/borrow, arenas, shared/weak)
  -> HIR (typed)
  -> MIR (basic blocks, instructions)
  -> Interpreter (dev) and Codegen (release: native + Wasm component)
```

At every stage the compiler must be able to emit the public contracts defined under
`schemas/`: diagnostics, capsules, agent maps, symbol cards, patch checks, and the planned
capability, manifest, lockfile, sbom, and benchmark schemas.

## What is bootstrap vs. what remains

This map describes the **intended** architecture. The current state is much smaller. See the
"What's actually implemented" matrix in `README.md` and the milestone breakdown M0–M19 in
`ORISON_AGENT_DEVELOPMENT_HANDOFF.md` for the authoritative status of each box above.

In short, today only the leftmost edge of the pipeline (source manager, lexer, symbol-level
parser) and the rightmost edges of the contracts (typed diagnostics, capsule emit, agent
map emit, patch validation) are wired end-to-end. The middle of the pipeline — real CST,
AST, resolver, type checker, effect/borrow checker, HIR, MIR, interpreter, codegen — is
scaffolded by documentation and tests only.

## Where to start reading

- Language surface: `docs/language/SPECIFICATION.md` and `docs/language/GRAMMAR.ebnf`.
- Compiler architecture detail: `docs/compiler/ARCHITECTURE.md`.
- Agent contracts: `docs/compiler/AGENT_CONTEXT_ABI.md` plus the schemas under `schemas/`.
- Patch IR: `docs/compiler/PATCH_IR.md`.
- Quality gates: `docs/QUALITY_GATES.md`.
- Demo target: `docs/examples/DEMO_APPLICATION.md` and `ORISON_DEMO_APPLICATION_GUIDE.md`.
