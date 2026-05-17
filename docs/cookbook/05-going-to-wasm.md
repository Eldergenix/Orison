# Recipe 05: Compile an Orison service to a wasm component

**Goal.** Take a typed Orison service, emit the wasm component manifest
that describes its world, generate a WIT (`wasm interface type`) skeleton
from it, build the component, and run it under wasmtime. By the end you
will know which steps the bootstrap ships today, which steps depend on
the external wasmtime toolchain, and how to wire the whole pipeline
together for CI.

**Prerequisites.** A working `ori` binary,
[wasmtime](https://wasmtime.dev/) >= 18 installed (`brew install wasmtime`
on macOS, distro package on Linux), and `jq` for inspecting envelopes.

**Time:** ~25 minutes.

## 1. The wasm story in one paragraph

Orison's wasm target produces a *component* (not a core module). A
component declares an interface in WIT, imports what it needs from the
host, and exports what it provides. The bootstrap's `ori wasm`
subcommand emits a manifest (`ori.wasm_component.v1`) that names the
world, the exports, the imports, and the capability set. Today this
manifest is consumed by the build pipeline to produce a `.wasm` file;
the same manifest also drives the WIT skeleton you hand to wasmtime.

The hard line: anything inside the Orison toolchain is in-tree and
binding under tier 1; the wasmtime command at the end of this recipe
is external and not covered by Orison's stability policy. Pin a
wasmtime version in CI.

## 2. Set up a small service

Save this as `src/greeter.ori`:

```ori
module greeter.svc

service Greeter uses http

fn get_hello() -> Str uses http:
  return "hello, world"

fn get_named(name: Str) -> Str uses http:
  return "hello"
```

The two routes are both `GET` (the bootstrap derives the method from
the function-name prefix; see [recipe 01](./01-rest-api-from-scratch.md)).
The whole service declares only `http`, which keeps the wasm import
list tiny.

Check:

```bash
ori check --json src/greeter.ori; echo "exit=$?"
```

Empty stdout, exit 0.

## 3. Inspect the wasm component manifest

```bash
ori wasm --json src/greeter.ori | jq .
```

```json
{
  "schema":        "ori.wasm_component.v1",
  "module":        "greeter.svc",
  "world":         "greeter-svc-world",
  "exports": [
    { "name": "Greeter",   "kind": "service",
      "signature": "service Greeter uses http" },
    { "name": "get_hello", "kind": "function",
      "signature": "fn get_hello() -> Str uses http" },
    { "name": "get_named", "kind": "function",
      "signature": "fn get_named(name: Str) -> Str uses http" }
  ],
  "imports":      [],
  "capabilities": ["http"],
  "build_target": "wasm32-component"
}
```

Five things to notice. First, `world` is derived from the module name
by lowercasing and replacing dots with hyphens — that becomes the WIT
world name. Second, every export carries its original `signature` so
the downstream WIT generator can reproduce types accurately. Third,
the `imports` list is empty here because the module imports nothing
across modules; in a real app this would name every cross-module
dependency. Fourth, `capabilities` is the union of effects across the
service — the host will be asked to provide tokens for exactly this
set. Fifth, `build_target` is always `wasm32-component` today; the
bootstrap does not yet emit core modules.

## 4. Build the component

```bash
ori build --target wasm-component --json src/greeter.ori \
  | jq '{ok, artifact, target, manifest_schema: .manifest.schema}'
```

```json
{
  "ok":              true,
  "artifact":        "target/wasm32-component/greeter_svc.wasm",
  "target":          "wasm-component",
  "manifest_schema": "ori.wasm_component.v1"
}
```

The build envelope (`ori.build_report.v1`) includes the path to the
emitted `.wasm` file and the embedded manifest. Today the bootstrap's
wasm encoder produces a structurally correct component (the 39-byte
hello-module passes wasmtime's validator) with placeholder bodies; the
production codegen lands in milestone M25.

## 5. Generate the WIT skeleton

The WIT generator reads the component manifest and emits a single
`.wit` file. Two paths today:

- The bootstrap exposes the manifest via `ori wasm --json`. You write
  a small script that converts the manifest's exports into WIT
  function declarations.
- The forthcoming `ori wasm --emit-wit` flag (milestone M25b) does
  this conversion in-tree. Until then, the script-based path is the
  documented approach.

A minimal hand-rolled WIT for the `greeter` service:

```wit
package greeter:svc@0.1.0;

world greeter-svc-world {
  export greeter: interface {
    get-hello: func() -> string;
    get-named: func(name: string) -> string;
  }
}
```

Field name translation: WIT uses kebab-case, while Orison uses
snake_case. The translation is mechanical — every underscore becomes
a hyphen. The bootstrap manifest preserves the original names so the
generator can reproduce the mapping deterministically.

## 6. Run under wasmtime

This step is **external** to the Orison toolchain. The exact commands
depend on your wasmtime version; the canonical invocation is:

```bash
# External: pin wasmtime version in your CI
wasmtime serve --wasi http \
  target/wasm32-component/greeter_svc.wasm \
  --addr 127.0.0.1:8080
```

`wasmtime serve` starts an HTTP listener that dispatches requests to
the component's exported `wasi:http/handler` interface. In another
shell:

```bash
curl -sS http://127.0.0.1:8080/hello
# hello, world
```

If `wasmtime serve` errors out with "missing import," check that the
component manifest's `capabilities` set is a subset of what your
wasmtime build supports. The current bootstrap component declares only
`http`, which maps to `wasi:http`.

## 7. Wiring into CI

A minimal CI workflow runs five steps:

```yaml
- name: orison check
  run: ori check src/greeter.ori

- name: orison wasm manifest
  run: ori wasm --json src/greeter.ori | jq -e '.capabilities | length > 0'

- name: orison build wasm
  run: ori build --target wasm-component --json src/greeter.ori

- name: wasmtime validate
  run: wasmtime compile target/wasm32-component/greeter_svc.wasm

- name: smoke test
  run: |
    wasmtime serve --wasi http target/wasm32-component/greeter_svc.wasm \
      --addr 127.0.0.1:8080 &
    PID=$!
    sleep 1
    curl -fsS http://127.0.0.1:8080/hello
    kill $PID
```

Two things to pin:

- The wasmtime version. The bootstrap is tested against wasmtime 18.
  Newer wasmtime releases occasionally tighten component validation;
  pin the major version in your CI image.
- The Orison toolchain version. The wasm manifest schema
  (`ori.wasm_component.v1`) is tier-1 stable and additive-only, but
  the build artefact layout under `target/` is tier-2 (subject to
  edition transition). Use the manifest's `artifact` field, not a
  hard-coded path.

## 8. What's in scope for stability

Tier 1 (binding at 1.0): the `ori.wasm_component.v1` schema and
every field name; the `--target wasm-component` flag; the `world`
naming rule (module-name to kebab-case). Tier 2 (edition-gated):
the default `build_target` triple. External (outside Orison's
stability policy): the wasmtime binary, its flags, and the bundled
WASI implementation — pin a version in CI.
