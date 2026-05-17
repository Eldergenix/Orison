# Orison Standard Distribution

This tree contains the source-of-truth `.ori` modules for the Orison standard
distribution. It is intentionally a *layered* distribution, not a single
monolithic standard library, so that applications, frameworks, and platform
adapters can each evolve at their own pace.

Each module here is parseable by the bootstrap `ori` compiler today. Bodies are
intentional stubs — they declare the public surface (types, variants, function
signatures, and effects) that downstream code can program against while the
self-hosted compiler matures. They are not yet runnable implementations.

## Layers

| Layer       | Stability    | Purpose                                                 |
|-------------|--------------|---------------------------------------------------------|
| `core`      | Highly stable | Always-available primitives: option, result, iter, list, string, bytes. |
| `std`       | Stable        | Common production libraries: json, http, validation, logging, config, time, sql. |
| `app`       | Evolving      | Application framework modules: services, views, auth.   |
| `platform`  | Reserved      | Platform adapters (web, wasm, ios, android, edge, gpu). Not yet authored. |
| `labs`      | Experimental  | Official experimental modules (autodiff, llm, agent, simd, robotics, embedded). Not yet authored. |

The layer for any subdirectory is its top-level folder under `stdlib/`. For
example, everything under `stdlib/core/` belongs to the `core` layer.

## Modules in this tree

### `core` layer — `stdlib/core/`

| Module         | File                  | Description                                                 |
|----------------|-----------------------|-------------------------------------------------------------|
| `core.option`  | `core/option.ori`     | Optional values that replace `null` with explicit cases.    |
| `core.result`  | `core/result.ori`     | Error values that replace exceptions with explicit cases.   |
| `core.iter`    | `core/iter.ori`       | Lazy iterator primitives used across collections and streams.|
| `core.string`  | `core/string.ori`     | Pure string utilities operating on `Str` values.            |
| `core.bytes`   | `core/bytes.ori`      | Immutable byte sequence type and conversions.               |
| `core.list`    | `core/list.ori`       | Persistent list type and structural transforms.             |

### `std` layer — `stdlib/std/`

| Module             | File                    | Description                                                  |
|--------------------|-------------------------|--------------------------------------------------------------|
| `std.json`         | `std/json.ori`          | JSON value model with total parse and stringify functions.   |
| `std.http`         | `std/http.ori`          | Outbound HTTP client primitives gated by `net.outbound`.     |
| `std.validation`   | `std/validation.ori`    | Accumulating validation results for form and DTO checks.     |
| `std.logging`      | `std/logging.ori`       | Structured logging via the platform `Log` capability.        |
| `std.config`       | `std/config.ori`        | Read-only access to environment configuration values.        |
| `std.time`         | `std/time.ori`          | Monotonic timestamps and elapsed duration helpers.           |
| `std.sql`          | `std/sql.ori`           | Typed SQL connection, query plan, and row primitives.        |

### `app` layer — `stdlib/app/`

| Module          | File                | Description                                                          |
|-----------------|---------------------|----------------------------------------------------------------------|
| `app.services`  | `app/service.ori`   | Declarative service and route surface for HTTP applications.         |
| `app.views`     | `app/view.ori`      | Declarative UI view primitives backed by the `ui` effect.            |
| `app.auth`      | `app/auth.ori`      | Session and principal extraction gated by the `auth` effect.         |

## Module naming notes

The bootstrap grammar reserves several lowercase identifiers as keywords
(`service`, `view`, `query`, `actor`, `migration`, `capability`, `match`,
`module`, `import`, ...). Module path segments and function names cannot
collide with those tokens or the source will fail to parse. This tree adopts
two minimal workarounds, both explicitly documented in the affected files:

- The application service module is declared as `module app.services`
  (plural), and the view module as `module app.views`. The files themselves
  remain at `app/service.ori` and `app/view.ori` to match the intended
  topology in `docs/stdlib/STANDARD_DISTRIBUTION.md`.
- The SQL execution entrypoint is exposed as `fn execute` rather than
  `fn query`, again to dodge the reserved keyword.

When the self-hosted compiler relaxes these constraints (or introduces a
distinct namespace for reserved words inside module paths and parameter
names), these accommodations will be revisited.

## Effect declarations

Functions list the side effects they perform after a `uses` clause, drawn from
the bootstrap-known effect set defined in
`crates/ori-compiler/src/effects.rs`. Logging uses an uppercase capability,
`Log`, because there is no built-in `log` effect in the bootstrap; the
compiler accepts any identifier that begins with an uppercase letter as a
user-declared capability.

## Validation

Every file in this tree should parse cleanly under the bootstrap checker:

```bash
for f in $(find stdlib -name '*.ori'); do
  echo "=== $f"
  cargo run -p ori -- check --json "$f" 2>&1 | head -20
done
```

Repository-wide static gates are run via:

```bash
python3 scripts/validate_all.py --static-only
```
