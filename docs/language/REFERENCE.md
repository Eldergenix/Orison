# Orison Language Reference (Bootstrap Subset)

This reference documents the Orison subset the bootstrap compiler currently
recognizes. It is the authoritative shape for what `ori check`, `ori capsule`,
`ori openapi`, `ori ui`, `ori wasm`, `ori capability`, and `ori run` parse
today. See `docs/language/SPECIFICATION.md` for the *intended* full language;
this file documents what is actually wired.

> **Scope.** Bootstrap parsing is item-level: the lexer recognizes every
> token in `crates/ori-compiler/src/lexer.rs` and the parser captures
> top-level declarations with their signatures and `uses` clauses, but does
> not yet recover function bodies (wave 2 introduces `crates/ori-compiler/src/expr.rs`
> for body parsing). The reference below reflects what is observable through
> the public CLI today.

## File structure

```
module <dotted.name>
import <dotted.name>
... declarations ...
```

- Every `.ori` file must start with `module <name>`. Missing → `E0001`.
- Module names are dotted identifiers. Trailing dot or empty → `E0002`.
- Imports use dotted module paths; trailing dot → `E0003`.

## Declarations

The bootstrap recognizes these item-introducing keywords:

| Keyword       | Purpose                              | CST kind        |
|---------------|--------------------------------------|-----------------|
| `module`      | Module declaration (file header)     | `Module`        |
| `import`      | Module import                        | `Import`        |
| `fn`          | Function / route handler             | `Function`      |
| `type`        | Record / variant / newtype           | `Type`          |
| `service`     | Service declaration                  | `Service`       |
| `view`        | View (UI) declaration                | `View`          |
| `actor`       | Actor declaration                    | `Actor`         |
| `query`       | Query declaration                    | `Query`         |
| `migration`   | Migration block                      | `Migration`     |
| `capability`  | Capability declaration               | `Capability`    |

Each item introduces a [`Symbol`](../compiler/AGENT_CONTEXT_ABI.md) with a
stable id `sym:<module>.<name>` and a [`CstNode`](../compiler/ARCHITECTURE.md)
with a stable id `node:<module>.<kind>.<name>.<discriminant>`.

## Signatures

```
fn name(arg1: T1, arg2: T2) -> ReturnType uses effect1, effect2
```

Parsed left-to-right on the same line as the keyword. Parameters / return
type / effects all observable to `capsule`, `agent map`, `openapi`, etc.
Public functions without an explicit return type emit `W0301`.

## Types

The reference type forms the bootstrap recognises:

- **Primitives**: `Bool`, `Int`, `Int8…64`, `UInt`, `UInt8…64`, `Float32`,
  `Float64`, `Decimal`, `Char`, `Str`, `Bytes`, `Unit`, `Never`.
- **Newtype**: `type X wraps Y` (e.g. `type ProductId wraps Str`).
- **Record**: `type X = { f1: T1, f2: T2 }`.
- **Variant**: `type X = | A | B(field: T) | C`.
- **Generics**: `Option[T]`, `Result[T, E]`, `List[T]`, `Pair[A, B]`,
  `Fn(T) -> U`, `Iter[T]`, `Query[T]`, `Map[K, V]`, `Set[T]`.

Unknown type names (anything starting with an uppercase letter that is not a
builtin, declared, or in the permitted generics list) emit `W0501`.
A `Result` or `Option` without generic arguments emits `W0510`.

## Effects and capabilities

Effects are declared on functions and services via `uses <name1>, <name2>`.
The compiler knows these names (`crates/ori-compiler/src/effects.rs`):

`fs.read`, `fs.write`, `net.inbound`, `net.outbound`, `db.read`, `db.write`,
`env.read`, `process.spawn`, `crypto`, `time`, `random`, `ui`, `gpu`,
`unsafe`, `http`, `db`, `fs`, `net`, `auth`, `mail.send`

Any other identifier *starting with an uppercase letter* is treated as a
user-declared capability (declare with `capability Name` in the same module).
Other tokens (lowercase) emit `W0401`.

Package-level capability declarations live in `ori.toml`:

```toml
[capabilities]
declared = ["http", "db.read", "db.write"]
```

`ori capability --policy ...` compares declared vs used and reports the
diff in the `ori.capability.v1` JSON contract.

## Forbidden values

- `null` → `E0100`. Use `Option[T]` (`None` / `Some(value)`).
- `throw` → `E0101`. Use `Result[T, E]` (`Ok(value)` / `Err(error)`).

The `null` and `throw` detectors run on tokens *outside* strings and
comments, so the words may legally appear in comments and string literals.

## Diagnostic ID prefixes

| Prefix         | Subsystem                           |
|----------------|-------------------------------------|
| `E00**`        | Lexer / parser structural errors    |
| `E02**`        | Symbol resolution                   |
| `E04**`        | Effects / capabilities              |
| `W04**`        | Effect warnings                     |
| `W03**`        | Style warnings                      |
| `W05**`        | Type warnings                       |
| `P0***`/`P1***`| Patch IR                            |
| `B00**`        | Borrow / ownership (wave 2)         |
| `E11**`        | Body parser (wave 2)                |

Each diagnostic is documented under `docs/compiler/DIAGNOSTICS.md` and
the canonical JSON shape lives at `schemas/diagnostic.schema.json`.

## What's *not* yet in this reference

- Expression body parsing (wave 2 adds `expr.rs` / `body.rs`).
- Statement-level let / mut / assignment.
- Multi-line variant payload formatting.
- String interpolation / f-strings.
- Macros / metaprogramming.
- Async / await execution semantics (the keywords lex but the runtime is
  not yet implemented).
- Pattern matching beyond top-level arms.

When any of those appear in user-facing material, this reference must be
updated alongside the implementation.
