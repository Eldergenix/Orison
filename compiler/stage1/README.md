# Orison Stage 1 Self-Hosting Prototype

This directory holds the first piece of Orison-in-Orison source the
project has ever shipped. It is intentionally tiny. The goal is to
prove that the surface syntax is rich enough to *describe* its own
front-end, not yet to execute it.

The full design lives in [`docs/compiler/SELF_HOSTING.md`](../../docs/compiler/SELF_HOSTING.md);
this README is the operational summary the next contributor needs in
order to extend it.

## The stage model

The effort is split into three stages so bugs caught early do not
poison later artefacts (see SELF_HOSTING.md §2).

- **Stage 0 — Rust bootstrap.** Authoritative compiler in
  `crates/ori-compiler/`. Defines every structured-output envelope
  (`ori.diagnostic.v1`, `ori.capsule.v1`, ...). Conformance oracle.
- **Stage 1 — Orison surface declarations (this directory).** A
  hand-written Orison module declaring the data types and function
  signatures of a real Orison front-end. The Rust bootstrap parses this
  source cleanly; the interpreter cannot yet execute the bodies, so
  every `fn` here returns a placeholder. Syntactic feasibility gate.
- **Stage 2 — Executable Orison compiler.** Once the runtime primitives
  below land, Stage 1's stubs are replaced with real implementations,
  and the resulting Orison binary must byte-match the Rust bootstrap on
  every fixture in `examples/`. Semantic conformance gate.

## What is shipped here

- [`parser.ori`](./parser.ori) — declares `ModuleDecl`, `ItemDecl`
  (with `Function`, `Type`, `Service`, `View` constructors), `ParseError`,
  and the top-level `parse_module(source: String) -> Result[ModuleDecl,
  ParseError]` entrypoint plus a handful of helpers
  (`parse_dotted_name`, `parse_item_header`, `is_ident_start`,
  `is_ident_continue`, `empty_module`).
- [`lexer.ori`](./lexer.ori) — declares the `Token` variant (six
  constructors mirroring `crates/ori-compiler/src/lexer.rs::TokenKind`)
  and the entrypoint `lex(source: String) -> List[Token]` plus
  predicates (`is_keyword`, `is_ident_start`, `is_ident_continue`,
  `is_whitespace`) and the `eof_at` constructor helper.

Every function body is a placeholder. The shape is what matters: the
parity test in `crates/ori-compiler/tests/stage1_parity.rs` asserts that
the Rust bootstrap parses both files with zero errors and that each
module declares the expected exported symbols.

## What blocks Stage 2

The same blockers that gate M27 in the bootstrap also gate the move
from declarations to executable code here:

1. **Lambda body execution in the interpreter.** The Stage 1 stubs
   compile to `Lambda` values the interpreter cannot yet apply outside
   trivial fixtures (`crates/ori-compiler/src/interp_exec.rs`).
2. **`List` and `String` runtime primitives** — `list.push`, `list.len`,
   `str.len`, `str.char_at`, `str.slice` are all M27-deferred in
   `stdlib/core/list.ori` and `stdlib/core/string.ori`.
3. **Generic type instantiation.** Stage 2 needs `Result[T, E]` and
   `List[T]` to monomorphise at use sites; the current type checker
   carries them as opaque shapes only.

Until all three land, Stage 1 stays as a parse-only artefact — exactly
the design SELF_HOSTING.md §7 prescribed.

## The path to Stage 2

When the blockers clear: replace stub bodies with real scanning loops
(only `core` and `std` capsules), add fixture-driven golden tests that
diff Orison-side output against the Rust bootstrap on every file under
`examples/`, and promote the parity test from "shape only" to "byte
identical". At that point the prototype becomes a real Stage 2 candidate
and the tracker in SELF_HOSTING.md §9 advances one row.
