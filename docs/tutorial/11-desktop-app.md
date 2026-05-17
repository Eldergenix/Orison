# Chapter 11: Building a desktop app

**What you'll build.** A minimal desktop reference app — three modules,
one service, one entry point — driven through `ori check`, `ori capsule`,
`ori capability`, `ori package check`, and `ori build --target desktop`.
By the end you will understand which parts of the desktop builder pipeline
are wired today, which parts are scaffolded and waiting on the M30 desktop
builder agent, and where to look in the schema set for the contract you
will eventually receive back from a real bundling toolchain.

**Time:** ~10 minutes.

## 1. What ships today

`ori build --help` lists five targets:

```bash
cargo run --release -p ori -- build
```

```
usage: ori build [--target dev|release|wasm-component|llvm-text|mobile]
                 [--app-id <id>] [--platforms ios,android]
                 [--ui-kind ios-uikit|ios-swiftui|android-compose|android-view]
                 [--json] <file.ori>
```

There is no `desktop` listed in the usage line but the CLI accepts
`--target desktop` and returns a `ori.build_report.v1` envelope with an
empty `outputs` array. The full desktop builder lives behind the M30
agent (`docs/agents/` outlines the boundary): it consumes
`ori.desktop_manifest.v1` ([`schemas/desktop-manifest.schema.json`](../../schemas/desktop-manifest.schema.json))
and is responsible for producing macOS `.app` bundles, Linux `.AppImage`
or `.deb` artefacts, and Windows MSI/EXE installers. Until M30 lands,
the recipe in this chapter exercises everything that is wired today and
points out exactly which envelope fields the agent will populate.

The schema is already pinned, so any tooling you write against it will
forward-compatibly survive the agent shipping. It declares these
top-level fields on the embedded `ori.desktop_manifest.v1`:

- `platform` — `macos`, `linux`, or `windows`
- `bundle_id` — the reverse-DNS identifier (e.g. `com.example.hello`)
- `product_name`, `version`
- `binary_targets` — one or more Rust target triples
- `entitlements` — keyed permissions (camera, microphone, networking)
- `linux_categories`, `windows_subsystem`
- `capabilities_required` — the Orison capability list, propagated from
  the package manifest

## 2. Lay out the reference app

The complete reference app lives under
[`examples/desktop_hello/`](../../examples/desktop_hello). It has three
modules:

```bash
ls examples/desktop_hello/src/
# domain.ori   main.ori   notes.ori
```

Read [`examples/desktop_hello/src/domain.ori`](../../examples/desktop_hello/src/domain.ori):

```ori
module desktop_hello.domain

type NoteId wraps Str
type NoteBody wraps Str

type Note = {
  id: NoteId,
  body: NoteBody,
  created_at_unix_seconds: Int
}

type NoteList = {
  notes: List[Note]
}

variant NoteError =
  | NotFound(id: NoteId)
  | TooLong(max: Int, actual: Int)
  | StorageFailed(reason: Str)

fn empty_note_list() -> NoteList:
  return NoteList { notes: [] }
```

The service module declares the typed surface:

```ori
module desktop_hello.notes

import desktop_hello.domain

service NoteStore uses fs.read, fs.write, ui

fn list_notes() -> Result[NoteList, NoteError] uses fs.read
fn read_note(id: NoteId) -> Result[Note, NoteError] uses fs.read
fn save_note(body: NoteBody, now_unix_seconds: Int) -> Result[Note, NoteError] uses fs.write
fn delete_note(id: NoteId) -> Result[Unit, NoteError] uses fs.write
```

`main.ori` is the boot entry point:

```ori
module desktop_hello.main

import desktop_hello.domain
import desktop_hello.notes

fn boot() -> Unit uses ui, fs.read, fs.write:
  return Unit
```

Note the module is named `desktop_hello.notes`, not `desktop_hello.service`:
`service` is a reserved keyword and cannot appear as a dotted segment in
a module name. The bootstrap parser rejects such names with `E0002`.

## 3. Check every module

The first gate is `ori check`. Run it on each `.ori` file in turn:

```bash
for f in examples/desktop_hello/src/*.ori; do
  cargo run --release -p ori --quiet -- check --json "$f"
  echo "exit=$?"
done
```

Each invocation should emit nothing on stdout and exit `0`. An empty
output means zero diagnostics. If a module ever regresses, the offending
diagnostic is emitted as `ori.diagnostic.v1` lines on stdout (see
chapter 02).

## 4. Inspect the capsules

`ori capsule` returns the per-module semantic capsule. The `domain.ori`
module exports six symbols (five types plus one pure function):

```bash
cargo run --release -p ori --quiet -- capsule --json examples/desktop_hello/src/domain.ori \
  | jq '{module, exports: (.exports | length)}'
```

```json
{ "module": "desktop_hello.domain", "exports": 5 }
```

The `NoteStore` service shows up in the capsule for `notes.ori` with the
declared effects union:

```bash
cargo run --release -p ori --quiet -- capability --json examples/desktop_hello/src/notes.ori \
  | jq '.effects | map(.name)'
```

```json
["fs.read", "fs.write", "ui"]
```

## 5. The package manifest

[`examples/desktop_hello/ori.toml`](../../examples/desktop_hello/ori.toml)
mirrors the declared effects in `[capabilities].declared`:

```toml
[package]
name = "desktop_hello"
version = "0.1.0"
edition = "2027.1"
description = "Minimal desktop reference app: a single note-store service and a boot entry point."
license = "Apache-2.0"

[capabilities]
declared = ["ui", "fs.read", "fs.write"]
```

Validate it with `ori package check`:

```bash
cargo run --release -p ori --quiet -- package check --json examples/desktop_hello \
  | jq '{ok, lockfile_packages: (.lockfile.packages | length)}'
```

```json
{ "ok": true, "lockfile_packages": 4 }
```

The lockfile is regenerated on every check; it pins the four bootstrap
dependencies (`app`, `core`, `std`, plus `desktop_hello` itself).

## 6. Build for desktop (today)

`ori build --target desktop` is the call site the M30 agent will hook
into. Today it returns an empty `outputs` array — proof the request was
accepted, but no `.app` bundle is produced:

```bash
cargo run --release -p ori --quiet -- build \
  --target desktop \
  --app-id com.example.desktop_hello \
  --platforms macos \
  --json examples/desktop_hello/src/main.ori
```

```json
{
  "schema":         "ori.build_report.v1",
  "package":        "desktop_hello.main",
  "target":         "desktop",
  "units_compiled": 1,
  "cached_units":   0,
  "duration_ms":    0,
  "errors":         0,
  "warnings":       0,
  "emit_warnings":  [],
  "outputs":        []
}
```

When the M30 agent lands, the `outputs` array will gain one entry per
target triple with `kind: "desktop"` and a `manifest` field pinned to
`ori.desktop_manifest.v1`. The shape will look like the example in
[`schemas/desktop-manifest.schema.json`](../../schemas/desktop-manifest.schema.json),
including `bundle_id`, `binary_targets`, `entitlements`,
`linux_categories`, `windows_subsystem`, and the propagated
`capabilities_required`. For now, treat this command as the contract
test: any tooling that calls it should be tolerant of the empty
`outputs` array and ready for the populated one.

## 7. What gets bundled (planned)

The M30 desktop builder pipeline takes the same Wasm component you can
already produce today:

```bash
cargo run --release -p ori --quiet -- build --target wasm-component --json examples/desktop_hello/src/notes.ori \
  | jq '{outputs: .outputs | map({kind, byte_count, path})}'
```

```json
{
  "outputs": [
    {
      "kind":       "wasm-component",
      "byte_count": 37,
      "path":       "examples/desktop_hello/src/notes.ori.wasm"
    }
  ]
}
```

Delete the artefact afterwards: `rm -f examples/desktop_hello/src/notes.ori.wasm`.
That wasm component will become the application core; the desktop agent
wraps it in a platform shell:

| Platform | Bundle              | Entitlements derived from                      |
|----------|---------------------|------------------------------------------------|
| macOS    | `Foo.app/Contents/` | `capabilities_required` -> `Info.plist` keys   |
| Linux    | `.AppImage` or `.deb` | `linux_categories` -> `.desktop` `Categories=`  |
| Windows  | MSI or EXE          | `windows_subsystem` -> PE subsystem (`gui`/`console`) |

The bundle id (`com.example.desktop_hello`) is the same identifier you
pass to `--app-id`; it must match `^[a-z][a-z0-9-]*(\.[a-z][a-z0-9-]*)+$`.
The `binary_targets` list will default to the host triple plus any
cross-build triples your CI requests. Entitlements come straight from
the capability list: `fs.read` and `fs.write` map to a sandbox
read-write entitlement on macOS and to no extra keys on Linux; `ui` is a
no-op on all three platforms because the windowing system is the host
shell.

## 8. One-shell smoke

Until the agent ships, this is the smoke check a contributor should run
before opening a PR touching the desktop builder:

```bash
set -euo pipefail
for f in examples/desktop_hello/src/*.ori; do
  cargo run --release -p ori --quiet -- check --json "$f" > /dev/null
  cargo run --release -p ori --quiet -- capsule --json "$f" > /dev/null
  cargo run --release -p ori --quiet -- capability --json "$f" > /dev/null
done
cargo run --release -p ori --quiet -- package check --json examples/desktop_hello > /dev/null
cargo run --release -p ori --quiet -- build --target desktop \
  --app-id com.example.desktop_hello --platforms macos \
  --json examples/desktop_hello/src/main.ori > /dev/null
echo "desktop_hello: all gates pass"
```

Exit `0` on success. Once the M30 agent lands, add a JSON assertion on
`.outputs[0].manifest.bundle_id == "com.example.desktop_hello"`.

## Common errors

| Symptom | Cause | Fix |
|---------|-------|-----|
| `E0002` on `module desktop_hello.service` | `service` is reserved keyword. | Pick a different leaf segment (`notes`, `store`, ...). |
| `--target desktop` returns `"outputs": []` | The M30 desktop builder hasn't shipped. | Expected; assert on `errors: 0` for now. |
| `--app-id` rejected by your CI gate | Bundle id must match `^[a-z][a-z0-9-]*(\.[a-z][a-z0-9-]*)+$`. | Lowercase only, hyphens allowed, at least one dot. |
| `ori capability` lists an effect the package didn't declare | Service or function declares an effect missing from `[capabilities].declared`. | Add the effect to `ori.toml`. |

## Recap

- The bootstrap `ori build --target desktop` accepts the request and
  returns an empty-outputs `ori.build_report.v1`. The full bundling
  pipeline lives behind the M30 desktop builder agent.
- `ori.desktop_manifest.v1` is pinned; the schema documents the bundle
  id, binary targets, entitlements, and capability propagation that the
  agent will populate.
- Modules cannot use reserved keywords (`service`, `view`, `query`,
  `migration`, `capability`) as dotted segments in their names.
- The reference app under `examples/desktop_hello/` exercises the
  current contract: three modules, one service, one entry point, all
  effect-tagged.

## Next

[Chapter 12: Building a mobile app](./12-mobile-app.md) covers the
analogous `ori build --target mobile` flow, which is fully wired today.
