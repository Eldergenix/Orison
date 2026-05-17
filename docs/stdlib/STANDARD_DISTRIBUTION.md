# Standard Distribution

Orison should ship a standard distribution rather than a single monolithic standard library.

## Layers

### `core`

Always available and highly stable.

```text
core.types
core.option
core.result
core.mem
core.iter
core.cmp
core.hash
core.math
core.text
core.bytes
core.time
core.uuid
core.testing
```

### `std`

Common production libraries.

```text
std.collections
std.json
std.yaml
std.toml
std.xml
std.csv
std.regex
std.path
std.fs
std.env
std.process
std.net
std.http
std.websocket
std.crypto
std.logging
std.tracing
std.metrics
std.config
std.validation
std.schema
std.auth
std.mail
std.cache
std.queue
std.sql
std.migrations
std.openapi
std.graphql
std.grpc
std.html
std.css
std.markdown
std.images
std.archive
std.compression
```

### `app`

Application framework modules.

```text
app.service
app.router
app.middleware
app.auth
app.session
app.rate_limit
app.forms
app.ui
app.state
app.design
app.i18n
app.a11y
app.deploy
app.observability
app.jobs
app.events
```

### `platform`

Platform adapters.

```text
platform.web
platform.wasm
platform.ios
platform.android
platform.desktop
platform.edge
platform.gpu
platform.tensor
```

### `labs`

Experimental official modules.

```text
labs.autodiff
labs.llm
labs.agent
labs.simd
labs.robotics
labs.embedded
```

## Dependency goal

Most production applications should not need third-party packages for routing, HTTP, JSON, validation, auth basics, database access, migrations, logging, testing, UI primitives, forms, OpenAPI, or config.
