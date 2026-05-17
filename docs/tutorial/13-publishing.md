# Chapter 13: Publishing

**What you'll build.** The full package lifecycle for a tiny one-module
library: validate the manifest with `ori package check`, generate the
SBOM with `ori sbom`, audit the dependency tree with `ori audit`,
publish to a local registry directory with `ori publish`, list its
contents, and yank the published version. Along the way you will see
the lockfile that the version solver writes, the `ori.publish_receipt.v1`
envelope returned by the registry, and the `ori.sandbox_result.v1`
contract used by the M37 publish workflow agent to fence build steps.

**Time:** ~10 minutes.

## 1. The reference package

[`examples/publish_hello/`](../../examples/publish_hello) is a
deliberately minimal library:

```bash
ls examples/publish_hello/
# README.md   ori.toml   src/
```

```ori
module publish_hello.greeter

type Greeting wraps Str

fn greet(name: Str) -> Greeting:
  return Greeting { value: "Hello, " }

fn greet_loud(name: Str) -> Greeting:
  return Greeting { value: "HELLO" }
```

The manifest declares no capabilities (the module is pure) and pulls in
two stub dependencies, `core` and `std`:

```toml
[package]
name = "publish_hello"
version = "0.1.0"
edition = "2027.1"
description = "Tiny one-module library used by the publishing tutorial."
license = "Apache-2.0"

[capabilities]
declared = []

[dependencies]
core = "*"
std = "*"
```

## 2. `ori package check` — manifest + lockfile

`ori package check` reads `ori.toml`, runs the version solver against
the dependency tree, and writes a fresh lockfile inside the returned
envelope. The shape conforms to `ori.package_check.v1`:

```bash
cargo run --release -p ori --quiet -- package check --json examples/publish_hello \
  | jq '{ok, lockfile_packages: (.lockfile.packages | length), schema}'
```

```json
{
  "ok":                true,
  "lockfile_packages": 3,
  "schema":            "ori.package_check.v1"
}
```

The lockfile (`ori.lockfile.v1`) pins three entries: `core`, `std`, and
`publish_hello` itself.

```bash
cargo run --release -p ori --quiet -- package check --json examples/publish_hello \
  | jq '.lockfile.packages[] | {name, version, source, checksum}'
```

```json
{ "name": "core",          "version": "",      "source": "unresolved-bootstrap", "checksum": "34dd875085d2e9df2778499b3b93fdef" }
{ "name": "publish_hello", "version": "0.1.0", "source": "path+examples/publish_hello", "checksum": "8be450a608778567e68329bc2f7997f7" }
{ "name": "std",           "version": "",      "source": "unresolved-bootstrap", "checksum": "cb64acc03c0fdebb3a77e18b99eaaf1b" }
```

The bootstrap version solver is intentionally simple: it resolves
`"*"` constraints against the in-tree stubs, byte-stably hashes each
package directory, and writes the result back in alphabetical order so
the lockfile is reproducible. When the M37 publish workflow agent
lands, the solver will additionally consult `ori.sandbox_result.v1`
([`schemas/sandbox-result.schema.json`](../../schemas/sandbox-result.schema.json))
to fence each fetch step inside a sandbox that records `fs_reads`,
`fs_writes`, `env_reads`, and any `policy_violations`. The lockfile
shape itself does not change; the sandbox result is recorded as a
sibling envelope.

## 3. `ori audit` — capability + dependency findings

```bash
cargo run --release -p ori --quiet -- audit --json examples/publish_hello
```

```json
{
  "schema":   "ori.audit_report.v1",
  "summary":  { "pass": 3, "warn": 0, "fail": 0 },
  "findings": []
}
```

Three checks pass, no findings: the `[capabilities].declared` list is
empty and the module declares no effects, so there is no
`AUD0002`-style "declared but unused" finding to report. Compare with
the demo storefront, which today emits two `AUD0002` info-level
findings because its stub dependencies don't pull in the
`db.read` / `db.write` capabilities it declares.

## 4. `ori sbom` — software bill of materials

Three output formats are supported: the native `ori-native` shape,
SPDX, and CycloneDX.

```bash
cargo run --release -p ori --quiet -- sbom --format ori-native --json examples/publish_hello \
  | jq '{schema, format, root, components: (.components | length)}'
```

```json
{
  "schema":     "ori.sbom.v1",
  "format":     "ori-native",
  "root":       "publish_hello",
  "components": 3
}
```

```bash
cargo run --release -p ori --quiet -- sbom --format ori-native --json examples/publish_hello \
  | jq '.components[] | {name, version, checksum, capabilities}'
```

```json
{ "name": "core",          "version": "",      "checksum": "sha256-bootstrap:34dd875085d2e9df2778499b3b93fdef", "capabilities": [] }
{ "name": "publish_hello", "version": "0.1.0", "checksum": "sha256-bootstrap:8be450a608778567e68329bc2f7997f7", "capabilities": [] }
{ "name": "std",           "version": "",      "checksum": "sha256-bootstrap:cb64acc03c0fdebb3a77e18b99eaaf1b", "capabilities": [] }
```

Use `--format spdx` or `--format cyclonedx` to get the standardised
equivalents; both wrap the same checksum + capability data in the
respective community schemas.

## 5. `ori publish` — push to a local registry

The bootstrap publish flow targets a local directory registry. There is
no `--dry-run` flag today; for a no-side-effect preview, run
`ori package check` + `ori sbom` + `ori audit` and confirm they all
succeed before invoking publish.

Set up a registry directory and a tarball:

```bash
mkdir -p /tmp/ori-tutorial-reg
echo "publish_hello tarball contents (would be a real tar in CI)" > /tmp/publish_hello.tar
```

Publish:

```bash
cargo run --release -p ori --quiet -- publish \
  --registry /tmp/ori-tutorial-reg \
  --tarball  /tmp/publish_hello.tar \
  --json examples/publish_hello
```

```json
{
  "schema":                    "ori.publish_receipt.v1",
  "name":                      "publish_hello",
  "version":                   "0.1.0",
  "bytes":                     59,
  "checksum":                  "<16-hex-digits>",
  "published_at_unix_seconds": 1747356800
}
```

The receipt conforms to
[`schemas/publish-receipt.schema.json`](../../schemas/publish-receipt.schema.json).
The `checksum` is the 64-bit FNV1a of the tarball bytes, encoded as a
16-character lowercase hex string. The registry rejects empty tarballs
("refusing to publish zero bytes") so the receipt always carries
`bytes >= 1`.

## 6. `ori registry list` — what's in the registry

```bash
cargo run --release -p ori --quiet -- registry list --registry /tmp/ori-tutorial-reg --json
```

```json
{
  "schema":   "ori.registry_list.v1",
  "registry": "/tmp/ori-tutorial-reg",
  "packages": [
    {
      "name":     "publish_hello",
      "version":  "0.1.0",
      "bytes":    59,
      "checksum": "<16-hex-digits>",
      "yanked":   false
    }
  ]
}
```

Re-publishing the same version is rejected unless the previous receipt
has been yanked. This is the alpha equivalent of "immutable versions":
the registry simply refuses to overwrite an existing
`(name, version)` pair.

## 7. `ori registry yank` — withdrawing a published version

```bash
cargo run --release -p ori --quiet -- registry yank \
  --registry /tmp/ori-tutorial-reg \
  publish_hello@0.1.0 \
  --reason "Tutorial demonstration; replace with 0.1.1 in real workflows." \
  --json
```

```json
{
  "schema":  "ori.yank_receipt.v1",
  "name":    "publish_hello",
  "version": "0.1.0",
  "reason":  "Tutorial demonstration; replace with 0.1.1 in real workflows."
}
```

After yanking, `ori registry list` shows `yanked: true` on that
package. New consumers cannot resolve a yanked version; existing
lockfiles continue to point to it (the bytes themselves are not
deleted). Always provide a `--reason` long enough to explain to a
future maintainer why the version was withdrawn.

Clean up the test registry:

```bash
rm -rf /tmp/ori-tutorial-reg /tmp/publish_hello.tar
```

## 8. The version solver and `ori.sandbox_result.v1`

The bootstrap solver does the trivial thing: each `"*"` constraint
resolves against the in-tree stub, and the lockfile reflects the result
deterministically. The full solver — the one that will ship with the
M37 publish workflow agent — has three additional responsibilities:

1. Resolve semver ranges across the registry index.
2. Fence each fetch and unpack step inside an `ori.sandbox_result.v1`
   envelope so policy violations (file writes outside the staging
   directory, environment-variable reads, network access to a
   non-registry host) are surfaced as machine-checkable findings.
3. Write the final dependency graph to `ori.lockfile.v1` exactly as
   `ori package check` does today.

The sandbox-result schema names the kinds of policy violations the
solver will report: `FsRead`, `FsWrite`, `EnvRead`, `Network`, and
`Timeout`. Tooling that consumes the publish workflow output should be
ready to find one sandbox-result envelope per build step plus the final
lockfile envelope.

## 9. End-to-end smoke

```bash
set -euo pipefail
cargo run --release -p ori --quiet -- package check --json examples/publish_hello > /dev/null
cargo run --release -p ori --quiet -- audit         --json examples/publish_hello > /dev/null
cargo run --release -p ori --quiet -- sbom --format ori-native --json examples/publish_hello > /dev/null
mkdir -p /tmp/ori-tutorial-reg
echo "smoke" > /tmp/publish_hello.tar
cargo run --release -p ori --quiet -- publish \
  --registry /tmp/ori-tutorial-reg --tarball /tmp/publish_hello.tar \
  --json examples/publish_hello > /dev/null
cargo run --release -p ori --quiet -- registry list --registry /tmp/ori-tutorial-reg --json > /dev/null
cargo run --release -p ori --quiet -- registry yank --registry /tmp/ori-tutorial-reg \
  publish_hello@0.1.0 --reason "smoke" --json > /dev/null
rm -rf /tmp/ori-tutorial-reg /tmp/publish_hello.tar
echo "publish_hello: all gates pass"
```

## Common errors

| Symptom | Cause | Fix |
|---------|-------|-----|
| `publish failed: invalid registry input: tarball is empty; refusing to publish zero bytes` | Tarball is 0 bytes. | Build a non-empty tarball; this is a deliberate safety check. |
| Re-publishing returns `(name, version) already published` | Bootstrap registry rejects overwrites. | Bump the version or yank first. |
| `ori package check --json` reports `ok: false` | Manifest schema mismatch or unknown dependency. | Inspect `.manifest` for the failing field. |
| `ori sbom --format spdx` returns SPDX-shaped JSON | Working as designed; the SBOM is wrapped in the SPDX 2.3 envelope. | Use `--format ori-native` for the flat shape shown here. |

## Recap

- The bootstrap publishing surface is six commands: `package check`,
  `audit`, `sbom`, `publish`, `registry list`, `registry yank`.
- The lockfile (`ori.lockfile.v1`) is regenerated on every
  `ori package check`; it pins each dependency by name, version,
  source, and a stable 128-bit checksum.
- Publish receipts (`ori.publish_receipt.v1`) carry the byte count, the
  FNV1a-64 checksum, and a Unix timestamp; the bootstrap registry
  refuses to overwrite an existing version.
- The M37 publish workflow agent will fence each build step with
  `ori.sandbox_result.v1` envelopes; the schema is already pinned, so
  tooling can be written against it today.

## Next

[Chapter 14: Agent in the loop](./14-agent-in-loop.md) covers the
agent-facing surface end-to-end, including a runnable 3-iteration model
edit loop that emits `ori.model_loop_telemetry.v1`.
