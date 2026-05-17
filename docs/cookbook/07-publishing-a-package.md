# Recipe 07: Publish a package from manifest to tagged release

**Goal.** Take a one-module library through the full publish lifecycle:
manifest validation, dry-run publish, real publish to a local registry,
git tag, and release announcement. Every step uses a CLI command that
is documented in `ori --help` and validated against a real example
package (`examples/publish_hello/`).

**Prerequisites.** A working `ori` binary, a local directory you will
use as a registry, and `git`.

**Time:** ~20 minutes.

## 1. Anatomy of a publishable package

A publishable Orison package is a directory with `ori.toml` (the
manifest), one or more `src/<module>.ori` modules (each starts with
`module <dotted.name>` and the file name follows the module path),
and a `README.md` (recommended; required for public packages).

The reference package is `examples/publish_hello/`:

```ori
module publish_hello.greeter

type Greeting wraps Str

fn greet(name: Str) -> Greeting:
  return Greeting { value: "hello" }

fn greet_loud(name: Str) -> Greeting:
  return Greeting { value: "HELLO" }
```

with this manifest:

```toml
[package]
name        = "publish_hello"
version     = "0.1.0"
edition     = "2027.1"
description = "Tiny one-module library."
license     = "Apache-2.0"

[capabilities]
declared = []

[dependencies]
core = "*"
std  = "*"
```

The manifest's `[capabilities].declared` is empty because the module
is pure (no `uses` clauses). Any dependency on a capability — say
`http` or `db.write` — must be declared here, or `ori package check`
emits `AUD0001`.

## 2. `ori package check` — validate the manifest and lockfile

```bash
ori package check --json . | jq '{ok, schema, lockfile_packages: (.lockfile.packages | length)}'
```

```json
{
  "ok":                true,
  "schema":            "ori.package_check.v1",
  "lockfile_packages": 3
}
```

`ori package check` runs the version solver, generates a deterministic
lockfile (`ori.lockfile.v1`), and reports any manifest problems. The
three lockfile entries are `core`, `std`, and `publish_hello` itself.
Each entry is byte-stable: the `checksum` field is a hash of the
package directory contents, and the order is alphabetical.

If `ok: false`, the response includes diagnostics. Common codes:
`PUB0001` (missing manifest field), `PUB0002` (invalid semver),
`PUB0003` (malformed dependency name), `AUD0001` (effect used but
not declared in `[capabilities].declared`). The lockfile is
reproducible: two developers running `ori package check` on the
same source produce byte-identical lockfiles.

## 3. `ori audit` — capability and dependency findings

```bash
ori audit --json . | jq '{schema, findings: (.findings | length)}'
```

`ori audit` enumerates the capability surface, the transitive
dependency tree, and supply-chain provenance. For a clean package,
`findings` is empty. Real findings carry `AUD****` codes: `AUD0001`
(undeclared effect), `AUD0002` (transitive effect leak), `AUD0003`
(lockfile-manifest drift). Errors block publish; warnings are
advisory.

## 4. `ori sbom` — software bill of materials

```bash
ori sbom --json --format ori-native . | jq '.packages | length'
```

Three formats are supported: `ori-native` (the native envelope),
`spdx` (SPDX 2.3 JSON), and `cyclonedx` (CycloneDX 1.5 JSON). The
publish workflow ships an SBOM with every release; pick the format
that matches your security tooling.

## 5. `ori publish --dry-run` — preview the publish

Before you publish for real, run the dry-run. It walks every step the
real publish takes, but stops before writing to the registry:

```bash
mkdir -p /tmp/local-registry
ori publish --dry-run --registry /tmp/local-registry --json .
```

The reply is an `ori.publish_receipt.v1` envelope with `applied:
false` and a `would_write` field listing the artefacts that would be
created in the registry directory. Read it carefully — if `applied`
is `false` and `diagnostics` is non-empty, the real publish would
fail, and the dry-run is your chance to fix the issues without
polluting the registry.

The dry-run also runs the sandbox fence
(`ori.sandbox_result.v1`) that the M37 publish workflow agent uses to
record `fs_reads`, `fs_writes`, `env_reads`, and any
`policy_violations`. If the sandbox flags an unexpected file write,
the publish refuses to proceed.

## 6. `ori publish` — the real thing

When the dry-run is clean:

```bash
ori publish --registry /tmp/local-registry --json .
```

The envelope returns `applied: true` and a receipt:

```json
{
  "schema":   "ori.publish_receipt.v1",
  "applied":  true,
  "package":  "publish_hello",
  "version":  "0.1.0",
  "registry": "/tmp/local-registry",
  "checksum": "8be450a608778567e68329bc2f7997f7"
}
```

The checksum is the package digest from step 2; it will appear in
downstream lockfiles when consumers depend on this version.

Confirm the registry now lists the package:

```bash
ori registry list --registry /tmp/local-registry --json | jq .
```

You will see `publish_hello@0.1.0` in the listing. If you publish a
second time without bumping the version, the registry refuses with
`PUB0010`; the only way to replace a published version is to yank it
and republish.

## 7. Tag the release in git

```bash
git add ori.toml CHANGELOG.md
git commit -m "Release publish_hello 0.1.0"
git tag -a v0.1.0 -m "publish_hello 0.1.0"
git push origin v0.1.0
```

Prefix the tag with `v`; many CI tools and release-note generators
expect it. Match the tag message to the manifest version exactly so
`git describe --tags` is reliable.

## 8. Yank if something is wrong

If a published version has a security issue, yank rather than
delete. Yanking marks the version unavailable for *new* dependents
but leaves existing lockfiles intact:

```bash
ori registry yank \
  --registry /tmp/local-registry \
  --package publish_hello \
  --version 0.1.0 \
  --reason "security: CAP0003 leak"
```

`ori registry list --json` now shows the version with `yanked: true`
and the reason. The next `ori package check` against a manifest that
names a yanked version produces `PUB0020`. The production registry
(M31b) replicates the lifecycle to an HTTP backend with signed
receipts.

## 9. Wiring the whole flow into CI

A release workflow runs these gates in order, and refuses to proceed
if any one fails:

```
1. ori check               (all source parses)
2. ori fmt | diff -        (formatter is a no-op)
3. ori package check       (manifest + lockfile valid)
4. ori audit               (no AUD**** errors)
5. ori sbom                (SBOM generates cleanly)
6. ori publish --dry-run   (publish would succeed)
7. ori publish             (only on tag push)
8. git tag verify          (tag matches manifest version)
```

The workflow lives in `.github/workflows/release-publish.yml` in this
repo for reference. The `ori.publish_receipt.v1` envelope from step 7
becomes a release-notes artefact; the SBOM from step 5 attaches to
the GitHub release.
