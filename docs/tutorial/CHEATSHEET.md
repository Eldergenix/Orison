# Orison Cheatsheet

One page. Every CLI subcommand. Every diagnostic ID prefix. Every keyword the
bootstrap parser recognises. Print it and keep it nearby.

For the tutorial index see [`README.md`](./README.md). For the authoritative
language reference see [`docs/language/REFERENCE.md`](../language/REFERENCE.md).

## CLI subcommands

### Compile and check

| Command                                          | Purpose                                                        |
|--------------------------------------------------|----------------------------------------------------------------|
| `ori check [--json] <file.ori>`                  | Parse + style check; emits `ori.diagnostic.v1` per finding.    |
| `ori fmt <file.ori>`                             | CST-preserving formatter; LF, trims trailing whitespace.       |
| `ori capsule [--json] <file.ori>`                | Per-module semantic capsule (`ori.capsule.v1`).                |
| `ori run [--entry <name>] [--json] <file.ori>`   | Tree-walking interpreter (`ori.run.v1`).                       |
| `ori build [--target ...] [--json] <file.ori>`   | Build report; targets: `dev`, `release`, `wasm-component`, `llvm-text`, `mobile`. |
| `ori bench [--samples N] [--json]`               | Self-benchmarks (`ori.benchmark.v1`); 32 suites.               |
| `ori docs [--format human|agent] [--budget N] [--json] [path]` | Doc generator (`ori.docs.v1`).                  |
| `ori doctor [--json]`                            | Health report + schema versions (`ori.doctor.v1`).             |

### Agent-facing

| Command                                          | Purpose                                                       |
|--------------------------------------------------|---------------------------------------------------------------|
| `ori agent map --budget N [--json] <file>`       | Budget-bounded symbol table (`ori.agent_map.v1`).             |
| `ori agent explain <sym> [--json] <file>`        | Single-symbol detail card (`ori.symbol_card.v1`).             |
| `ori agent symbols [--changed] [--json] <file>`  | Symbol enumeration (`ori.agent_symbol_list.v1`).              |
| `ori agent diagnose [--json] <file>`             | Status + top repair candidates (`ori.agent_diagnose.v1`).     |
| `ori agent tests --affected [--changed-name <n>...] [--json] <root>` | Per-file test selection (`ori.agent_tests.v1`). |
| `ori agent changed [--prev <prev.json>] [--json] <path>` | Per-symbol fingerprint diff (`ori.agent_changed.v1`). |

### Patch IR

| Command                                          | Purpose                                                       |
|--------------------------------------------------|---------------------------------------------------------------|
| `ori patch check [--json] <patch.json>`          | Validate Patch IR shape (`ori.patch_check.v1`).               |
| `ori patch apply [--dry-run] [--json] <patch> <src.ori>` | Apply Patch IR with stable ids (`ori.patch_apply.v1`). |
| `ori patch dry-run [--json] <patch> <src.ori>`   | Preview without writing disk (`ori.patch_apply.v1`).          |
| `ori patch explain [--json] <patch>`             | Intent + op count summary (`ori.patch_explain.v1`).           |

### Surfaces

| Command                                          | Purpose                                                       |
|--------------------------------------------------|---------------------------------------------------------------|
| `ori openapi [--json] <file>`                    | OpenAPI 3.1 extraction (`ori.openapi_report.v1`).             |
| `ori ui [--json] <file>`                         | UI manifest + a11y findings (`ori.ui_manifest.v1`).           |
| `ori wasm [--json] <file>`                       | Wasm component manifest (`ori.wasm_component.v1`).            |
| `ori capability [--policy a,b,c] [--json] <file>` | Effect → capability diff (`ori.capability.v1`).              |
| `ori design check [--tokens <file>] [--json] <module>` | Design-token enforcement (`ori.design_tokens_report.v1`).  |
| `ori db check [--json] <file>`                   | SQL query + migration validation (`ori.db_check.v1`).         |

### Tests and coverage

| Command                                          | Purpose                                                       |
|--------------------------------------------------|---------------------------------------------------------------|
| `ori test [--changed] [--json] <root>`           | Test discovery (`ori.agent_tests.v1`).                        |
| `ori coverage [--json] <path>`                   | Per-symbol test coverage (`ori.coverage_report.v1`).          |

### Package management

| Command                                          | Purpose                                                       |
|--------------------------------------------------|---------------------------------------------------------------|
| `ori package check [--json] [path]`              | Manifest + lockfile validation (`ori.package_check.v1`).      |
| `ori audit [--json] [path]`                      | Capability + dependency audit (`ori.audit_report.v1`).        |
| `ori sbom [--format <ori-native|spdx|cyclonedx>] [--json] [path]` | Software bill of materials (`ori.sbom.v1`). |
| `ori provenance verify [--json] <file.json>`     | Provenance check (`ori.provenance.v1`).                       |
| `ori publish --registry <path> --tarball <file> [--json] [path]` | Publish to local registry (`ori.publish_receipt.v1`). |
| `ori fetch --registry <path> <name>@<v> [--out <file>] [--json]` | Fetch from local registry.                  |
| `ori registry list --registry <path> [--json]`   | List registry contents (`ori.registry_list.v1`).              |
| `ori registry yank --registry <path> <name>@<v> --reason <r> [--json]` | Yank a published version.              |

### Imports and codegen

| Command                                          | Purpose                                                       |
|--------------------------------------------------|---------------------------------------------------------------|
| `ori schema import graphql <sdl> --module <name> [--json]` | SDL → Orison module (`ori.graphql_import.v1`).      |
| `ori schema import grpc <proto> --module <name> [--json]`  | proto3 → Orison module (`ori.rpc_import.v1`).        |
| `ori preprocess [--const k=v ...] [--allow-env X,Y] [--json] <file>` | Safe `${ENV}` / `@orison/X` substitution (`ori.preprocess.v1`). |
| `ori migrate --from <X> --to <Y> [--dry-run] [--json] [path]` | Edition migration plan (`ori.migration_report.v1`). |

### Editor

| Command                                          | Purpose                                                       |
|--------------------------------------------------|---------------------------------------------------------------|
| `ori lsp --stdio`                                | Language server: hover, completion, rename, code actions, workspace symbols, document symbols, definition, references. |

## Diagnostic ID prefixes

Every diagnostic conforms to `ori.diagnostic.v1`
([`schemas/diagnostic.schema.json`](../../schemas/diagnostic.schema.json)).
The leading letter encodes severity (`E` = error, `W` = warning) and the
first two digits encode the subsystem.

| Prefix     | Subsystem                                | Examples                          |
|------------|------------------------------------------|-----------------------------------|
| `E00**`    | Lexer / parser structural errors         | `E0001`, `E0002`, `E0003`         |
| `E01**`    | Forbidden language constructs            | `E0100` (null), `E0101` (throw)   |
| `E02**`    | Names and symbols                        | `E0200`, `E0201`, `E0211`, `E0220`, `E0230` |
| `E03**`    | Types                                    | (reserved; wave 2)                 |
| `E04**`    | Effects and capabilities                 | `E0410`, `E0420`                  |
| `E05**`    | Patterns / match (incl. exhaustiveness)  | `E0540`                           |
| `E06**`    | Concurrency                              | (reserved)                         |
| `E07**`    | Services / routes                        | (reserved)                         |
| `E08**`    | UI / accessibility                       | (reserved)                         |
| `E09**`    | Packages / supply chain                  | (reserved)                         |
| `E11**`    | Body parser (wave 2)                     | (reserved)                         |
| `W03**`    | Style warnings                           | `W0301` (no return type)          |
| `W04**`    | Effect warnings                          | `W0401` (unknown effect)          |
| `W05**`    | Type warnings                            | `W0501`, `W0510`, `W0530`, `W0531`, `W0540`, `W0541`, `W0542` |
| `W9***`    | Tooling style warnings                   | `W9001` (tabs)                    |
| `B00**`    | Borrow / ownership (wave 2)              | `B0010`–`B0050`                   |
| `Q00**`    | Query (SQL DSL) shape                    | `Q0010` (unknown column type), `Q0020` (duplicate shape) |
| `P0***`    | Patch IR structural                      | `P0000`–`P0004`, `P0100` (warning) |
| `P1***`    | Patch IR runtime                         | `P1000`–`P1003` (fatal), `P1010` (per-op skip) |
| `AUD****`  | Audit                                    | `AUD0001`, `AUD0002`              |
| `MOB****`  | Mobile manifest                          | `MOB0001`, `MOB0002`, `MOB0003`   |

## Effect names known to the compiler

The bootstrap recognises these effect identifiers without a `capability`
declaration. Anything starting uppercase is a user capability; anything
lowercase and not in this list emits `W0401`.

```
fs.read    fs.write    net.inbound   net.outbound
db.read    db.write    env.read      process.spawn
crypto     time        random        ui
gpu        unsafe      http          db
fs         net         auth          mail.send
```

## Keywords

Item-introducing:

```
module    import    fn    type    service
view      actor     query migration
capability
```

Bodies / expressions:

```
let    return    if    else    match    for    in
lambda try        Some  None    Ok       Err   Unit
```

Type forms:

```
wraps     Bool    Int     Int8/16/32/64
UInt      UInt8/16/32/64  Float32/Float64
Decimal   Char    Str     Bytes   Unit    Never
Option[T] Result[T,E]     List[T] Pair[A,B]
Fn(T)->U  Iter[T]         Query[T] Map[K,V] Set[T]
```

## Files and conventions

| Path                                        | Purpose                                          |
|---------------------------------------------|--------------------------------------------------|
| `ori.toml`                                  | Package manifest. `[capabilities].declared` is the policy. |
| `src/*.ori`                                 | Source modules. One module per file.             |
| `tests/*.ori`                               | Test modules. Functions named `test_*` are discovered. |
| `contracts/*.json`                          | Patch IR / change manifest contracts.            |
| `tokens.toml`                               | Optional design token file for `ori design check`. |

## File header conventions

- First non-blank line is always `module <dotted.name>` (`E0001` otherwise).
- Module names are dotted identifiers (`E0002` on trailing dot or empty segment).
- Imports use dotted module paths (`E0003` on trailing dot).
- Each top-level function declares an explicit return type (`-> Unit` if pure)
  (`W0301` otherwise on public functions).

## JSON envelope contract

Every CLI subcommand with a `--json` flag emits a single JSON object on
stdout. The `schema` field is always present and is the contract identifier
(e.g. `ori.diagnostic.v1`). Exit codes:

| Exit | Meaning                                                              |
|------|----------------------------------------------------------------------|
| 0    | No errors (warnings allowed).                                        |
| 1    | At least one error-level diagnostic.                                 |
| 2    | Usage error (missing argument, unknown flag).                        |

`ori check --json` emits one JSON object per diagnostic separated by `\n`.
All other commands emit a single JSON object.

## The quality gate

| Mode                                            | What it runs                                                  |
|-------------------------------------------------|---------------------------------------------------------------|
| `python3.13 scripts/validate_all.py --static-only` | Repository layout + JSON contract + shell hook checks.     |
| `python3.13 scripts/validate_all.py --contracts-only` | Schema contract checks only.                             |
| `python3.13 scripts/validate_all.py --pre-commit` | Adds `cargo fmt --check`, `cargo check --workspace`.       |
| `python3.13 scripts/validate_all.py --full`     | Adds `cargo clippy`, `cargo test --workspace`, six smoke CLI invocations. |

## Benchmark comparison

```bash
ori bench --samples 100 --json > /tmp/current.json
python3.13 scripts/compare_bench.py \
  --baseline BENCHMARKS.results.json \
  --current  /tmp/current.json \
  --threshold 20
```

Exit 1 if any metric's p50 is more than `--threshold` percent above the
baseline. Improvements (more than `--threshold` below) are reported as
informational and never block.

## See also

- [`docs/language/REFERENCE.md`](../language/REFERENCE.md) — authoritative
  syntax reference for the bootstrap subset.
- [`docs/language/SPECIFICATION.md`](../language/SPECIFICATION.md) — long-form
  intended language.
- [`docs/compiler/DIAGNOSTICS.md`](../compiler/DIAGNOSTICS.md) — diagnostic
  envelope shape.
- [`docs/compiler/PATCH_IR.md`](../compiler/PATCH_IR.md) — Patch IR contract.
- [`docs/language/EFFECTS_AND_CAPABILITIES.md`](../language/EFFECTS_AND_CAPABILITIES.md)
  — capability model in depth.
- [`BENCHMARKS.md`](../../BENCHMARKS.md) — measurement methodology.
- [`README.md`](../../README.md) — top-level project README.
