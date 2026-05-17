# std — typical production application needs

Modules for HTTP, JSON, SQL, validation, logging, time, crypto, and
similar everyday production code. Each module declares its required
capabilities via `uses` on its public functions; the package
`[capabilities].declared` policy in `ori.toml` gates whether an app
may import the module at all.

## Modules

| Module | Capabilities required | Purpose |
|--------|----------------------|---------|
| [`json.ori`](./json.ori) | none | Parse + serialize JSON values |
| [`http.ori`](./http.ori) | `net.outbound` | HTTP/1.1 + HTTP/2 client (planned: TLS) |
| [`validation.ori`](./validation.ori) | none | Composable validators with structured errors |
| [`logging.ori`](./logging.ori) | `Log` (user-declared) | Structured logging |
| [`config.ori`](./config.ori) | `env.read` | Configuration from env + files |
| [`time.ori`](./time.ori) | `time` | Clocks + durations + timezone handling |
| [`sql.ori`](./sql.ori) | `db.read` + `db.write` | Connection pool + parameterised queries |
| [`queue.ori`](./queue.ori) | varies | In-memory + backed queues |
| [`mail.ori`](./mail.ori) | `mail.send` | SMTP client |
| [`websocket.ori`](./websocket.ori) | `net.inbound` + `net.outbound` | RFC 6455 WebSocket |
| [`process.ori`](./process.ori) | `process.spawn` | Subprocess spawn + pipes |
| [`tasks.ori`](./tasks.ori) | none | `Future[T]` + `join` / `select_first` / `with_timeout` |
| [`cache.ori`](./cache.ori) | none | LRU + TTL + size-bounded cache |
| [`url.ori`](./url.ori) | none | RFC 3986 URL parsing |

## Stability

Every module in `std` is tier 2 (stable-with-editions). Renames or
removals require an edition transition. Bodies will become real
implementations as part of M27 (see `GOAL.md`).

## Capability discipline

`std.http.get(url)` requires `uses net.outbound` on the calling
function. The capability is *not* implicitly granted — the call site
must declare the effect. If the package `[capabilities].declared`
policy in `ori.toml` does not include `net.outbound`, the audit
emits an `AUD0001` error and the build fails.

This is the same rule for every effect — no exceptions, no implicit
ambient acquisitions. See `SECURITY.md` §3.2 for the rationale.

## Adding a module

A new `std` module requires:

1. The module file parses clean via `ori check --json`.
2. Every public function declares its effects honestly via `uses`.
3. The module's required capabilities are documented in the table
   above.
4. An integration test under `crates/ori-pkg/tests/std_<name>.rs`.
5. A bench suite entry in `crates/ori-compiler/src/bench.rs`
   (lazy-loaded; only runs if the implementation exists).
6. A `CHANGELOG.md` entry.
