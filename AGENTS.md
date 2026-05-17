# AGENTS.md

This file is mandatory operating guidance for AI coding agents working in this repository.

## Mission

Build Orison end-to-end as an agent-native programming language and toolchain. Preserve the product invariant:

> Anything the compiler knows should be available to tools and agents through stable structured output.

## Required first read

Before making code changes, read these files in order:

1. `README.md`
2. `GOAL.md`
3. `MEMORY.md`
4. `TASKS.md`
5. `ORISON_AGENT_DEVELOPMENT_HANDOFF.md`
6. `CODE_REVIEW_REMEDIATION.md`
7. `docs/compiler/ARCHITECTURE.md`
8. `docs/compiler/AGENT_CONTEXT_ABI.md`
9. `docs/language/SPECIFICATION.md`
10. `schemas/diagnostic.schema.json`
11. `schemas/patch.schema.json`
12. `schemas/capsule.schema.json`

## Required command loop

Run before changes:

```bash
./scripts/install_hooks.sh
python3 scripts/validate_all.py --static-only
cargo fmt --all --check
cargo test --workspace
cargo run -p ori -- check --json examples/hello.ori
cargo run -p ori -- agent map --budget 2000 --json examples/fullstack/users.ori
```

Run after changes:

```bash
cargo fmt --all --check
cargo test --workspace
cargo run -p ori -- check --json examples/hello.ori
cargo run -p ori -- check --json examples/fullstack/users.ori
cargo run -p ori -- capsule --json examples/fullstack/users.ori
cargo run -p ori -- patch check --json examples/agent_patch.json
python3 scripts/validate_all.py --full
```

If a change affects CLI behavior, also run:

```bash
cargo run -p ori -- doctor
cargo run -p ori -- help
```

## Coding rules

- Prefer small coherent changes.
- Do not rewrite unrelated files.
- Do not add third-party Rust dependencies unless the task explicitly requires them and updates `MEMORY.md`.
- Keep the bootstrap compiler deterministic.
- Use typed serialization for JSON contracts. Do not hand-build contract JSON with string concatenation.
- Preserve stable JSON field names unless the schema version changes.
- Add or update tests for new behavior.
- Keep CLI output stable for agents.
- Use explicit error IDs for diagnostics.
- Do not introduce hidden global state.
- Do not use `unwrap()`, `expect()`, `panic!`, `todo!`, `unimplemented!`, or `dbg!` in production Rust source.
- Do not implement unsafe Rust unless the task specifically requires it and documents why.

## Documentation update rules

Update these files when relevant:

- `TASKS.md` when task status changes.
- `MEMORY.md` when an architectural decision changes.
- `CHANGELOG.md` for externally visible changes.
- `docs/language/SPECIFICATION.md` when language semantics change.
- `docs/compiler/DIAGNOSTICS.md` when diagnostic shape changes.
- `schemas/*.json` when JSON contracts change.

## JSON contract rules

The following schemas are public contracts:

- `ori.diagnostic.v1`
- `ori.patch.v1`
- `ori.patch_check.v1`
- `ori.capsule.v1`
- `ori.agent_map.v1`
- `ori.symbol_card.v1`
- `ori.manifest.v1`
- `ori.change.v1`

If a breaking change is needed:

1. Add a new schema version.
2. Keep the old schema unless removal is explicitly approved.
3. Update examples.
4. Update docs.
5. Add migration notes to `CHANGELOG.md`.

## Agent-specific output discipline

When generating code:

- Prefer structured data and tests over prose-only explanations.
- Use symbol IDs in comments and docs where helpful.
- Keep examples minimal but executable by the scaffold where possible.
- Do not hallucinate completed features. Mark stubs clearly.

When debugging:

- Start from compiler diagnostics.
- Request the smallest relevant symbol context.
- Inspect `MEMORY.md` before proposing architecture changes.
- Validate with `cargo test`.

## Forbidden shortcuts

- Do not claim the language is implemented when only the scaffold exists.
- Do not replace JSON diagnostics with human-only text.
- Do not make the parser depend on formatting quirks that are not in the grammar.
- Do not add ambient filesystem, network, process, database, or environment access to the language model.
- Do not remove capability/effect declarations from the design.
- Do not validate JSON contracts with substring checks.

## Preferred implementation order

1. Stable lexer.
2. Error-tolerant CST.
3. AST lowering with stable node IDs.
4. Name resolution.
5. Type checker.
6. Effect checker.
7. Ownership/borrow checker prototype.
8. JSON diagnostics and fixes.
9. Semantic capsules.
10. Patch IR application.
11. Affected-test graph.
12. Dev backend.
13. Wasm backend.
14. Standard distribution modules.
15. Framework modules.

## Agent completion checklist

Before marking a task done:

- [ ] `python3 scripts/validate_all.py --full` passes on a Rust-capable machine.
- [ ] Tests pass.
- [ ] JSON output remains valid.
- [ ] Docs were updated if semantics changed.
- [ ] `TASKS.md` status was updated.
- [ ] `CHANGELOG.md` includes a concise note.
- [ ] No unrelated files were changed.
