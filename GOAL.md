# Orison Goal

## Primary goal

Create a new programming language and toolchain that enables secure, fast, production-grade application development by humans and AI agents with materially lower context-token consumption and lower iteration cost.

## One-sentence definition

Orison is a memory-safe, statically typed, compiled full-stack application language with a small readable syntax, Rust-inspired safety, Python-like ergonomics, Go-like tool simplicity, WebAssembly/native/mobile targets, a comprehensive official standard distribution, capability-secured effects, JSON-first diagnostics, structural Patch IR, and a compiler-native Agent Context ABI.

## Non-negotiable product outcomes

1. **Fast agent iterations**
   - Agents can inspect compact symbol maps instead of whole files.
   - Diagnostics provide typed repair candidates.
   - Patches can be structural, not line-oriented.
   - Affected tests can be selected from the dependency graph.

2. **Type-safe fixes**
   - Public APIs have explicit signatures.
   - No null.
   - No exceptions.
   - Exhaustive matching.
   - Result and Option are first-class.

3. **JSON diagnostics**
   - Every diagnostic must have a stable machine-readable form.
   - Diagnostics include affected symbol IDs, expected/found data, repair options, and minimal context hints.

4. **AI-agent compatibility**
   - The compiler emits semantic capsules, symbol cards, change manifests, and capability summaries.
   - Agents should be able to query the compiler instead of inferring project state from raw text.

5. **Reduced token consumption**
   - Project maps and capsules should be budgeted.
   - Symbol explanations should be compact and stable.
   - Patch IR should avoid whole-file rewrites.

6. **Production-grade full-stack applications**
   - Backend services, typed routes, API schemas, database queries, UI views, auth, validation, logging, tracing, testing, package management, and deployment metadata are first-class.

## Technical success criteria

- Safe code cannot trigger use-after-free, double-free, invalid aliasing, uninitialized reads, or data races.
- Incremental checks for common edits should be sub-second in medium projects.
- `ori check --json` is stable enough to be consumed by agents and CI.
- `ori agent map --budget N --json` returns useful context within a fixed token budget.
- `ori patch check` rejects invalid structural patches before they are applied.
- Most full-stack apps can start with official distribution modules only.

## First production wedge

The first product should focus on:

> Safe full-stack web/backend applications with agent-optimized iteration.

Do not start with game engines, embedded systems, or neural-network training as the primary wedge. Those should become later platform layers.
