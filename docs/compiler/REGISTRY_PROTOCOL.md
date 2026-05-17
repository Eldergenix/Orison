---
title: Registry Protocol (Planned)
status: design
milestone: M31
---

# Registry Protocol (Planned)

This document specifies the HTTPS registry protocol that the Orison package
manager will speak once the bootstrap dependency policy is relaxed enough to
allow real cryptographic and network crates. The interface is published here
so downstream tooling (mirrors, proxies, audit pipelines) can be designed in
parallel with the implementation.

Until the real implementation lands, `ori-pkg` ships a local-filesystem stub
(see `crates/ori-pkg/src/registry.rs`) that mimics the same shape against a
directory tree. Any client that targets the local stub today will be
compatible with the network protocol described below.

## Versioning

The protocol is versioned under `/api/v1/`. Each response carries a `schema`
field whose value matches one of the `ori.*.v1` identifiers defined in
`schemas/`. Breaking changes bump the path segment (`/api/v2/`) and the schema
constant in the same change set.

## Transport

* TLS 1.3 only. Plain HTTP is rejected.
* All bodies are UTF-8 JSON unless otherwise noted (the tarball endpoint
  returns `application/x-tar+zstd`).
* Pagination uses an opaque `cursor` query parameter and a `next_cursor` field
  in the response body. Clients MUST treat the cursor as opaque.

## Endpoints

### `GET /api/v1/packages/{name}/versions`

List every published version of `name`, sorted in descending SemVer order.

Response body:

```json
{
  "schema": "ori.registry_index.v1",
  "name": "app.users",
  "versions": [
    { "version": "0.2.1", "yanked": false, "published_at": "2026-05-16T12:00:00Z" },
    { "version": "0.2.0", "yanked": true,  "published_at": "2026-05-01T09:32:11Z" }
  ],
  "next_cursor": null
}
```

### `GET /api/v1/packages/{name}/{version}/manifest`

Return the canonical manifest document for `name @ version`. Shape matches
`schemas/manifest.schema.json`. Manifests are immutable once published.

### `GET /api/v1/packages/{name}/{version}/tarball`

Return the package tarball bytes (`application/x-tar+zstd`). The `ETag`
header is the SHA-256 of the body; the `Content-Length` and `Content-Type`
headers are mandatory. The body is byte-identical for every fetch of the
same `{name, version}`.

### `GET /api/v1/packages/{name}/{version}/provenance`

Return the Sigstore-signed provenance attestation. Shape matches
`schemas/provenance.schema.json` extended with a `signature` block whose
fields are the in-toto / DSSE envelope. Verification combines:

1. Sigstore Rekor inclusion proof against the public log.
2. Fulcio-issued short-lived certificate matching the signing identity in
   the package metadata.
3. The signed `subject.digest.sha256` value equals the SHA-256 of the tarball
   returned by `/tarball`.

### `POST /api/v1/packages`

Publish a new `{name, version}`. The request body is a multipart upload:

* `manifest` part: JSON, the manifest document.
* `tarball` part: bytes, the package archive.
* `provenance` part: JSON, the DSSE-wrapped Sigstore attestation.

Returns a `PublishReceipt` (`schemas/publish-receipt.schema.json`). Servers
MUST reject any publish whose provenance does not pass Sigstore verification.
Duplicate `{name, version}` returns `409 Conflict`.

### `POST /api/v1/packages/{name}/{version}/yank`

Mark a published `{name, version}` as yanked. Request body:

```json
{ "reason": "compromised key", "yanked_by": "alice@example.com" }
```

Yank is reversible via `POST .../unyank` only by a registry admin. Yanked
versions remain downloadable for reproducibility but are excluded from new
resolutions unless the client passes `--allow-yanked`.

## Wire formats summary

| Endpoint                                                | Method | Request body         | Response schema                                |
|---------------------------------------------------------|--------|----------------------|------------------------------------------------|
| `/api/v1/packages/{name}/versions`                      | GET    | -                    | `ori.registry_index.v1` (extended)             |
| `/api/v1/packages/{name}/{version}/manifest`            | GET    | -                    | `ori.manifest.v1`                              |
| `/api/v1/packages/{name}/{version}/tarball`             | GET    | -                    | `application/x-tar+zstd`                       |
| `/api/v1/packages/{name}/{version}/provenance`          | GET    | -                    | `ori.provenance.v1`                            |
| `/api/v1/packages`                                      | POST   | multipart            | `ori.publish_receipt.v1`                       |
| `/api/v1/packages/{name}/{version}/yank`                | POST   | JSON `{reason}`      | `204 No Content`                               |

## Sandbox roadmap

The build-script sandbox interface (`crates/ori-pkg/src/sandbox.rs`) ships in
bootstrap mode and emits a `SandboxResult` whose `stderr` contains the marker
`TODO(M32): real-sandbox-not-implemented`. Real enforcement requires:

* Linux: a `seccomp-bpf` syscall filter whose allow list is derived from the
  policy. File path enforcement is layered via Landlock LSM where available.
* macOS: a generated SBPL profile passed to the platform sandbox tool. The
  profile denies everything by default and adds `allow file-read*` and
  `allow file-write*` rules per allowed prefix.
* Windows: a Job Object with `JOB_OBJECT_LIMIT_ACTIVE_PROCESS` plus an
  AppContainer profile.

The `SandboxResult` schema (`schemas/sandbox-result.schema.json`) is stable
across all three implementations so the planned switch from "would-have-run"
to real enforcement is transparent to callers.

## Out-of-scope (bootstrap)

* Mirroring / federation.
* OAuth-based publish auth (the bootstrap will use a static API token).
* Search / discovery APIs.
* Per-package webhooks for new versions.

These are noted here so the v1 surface stays minimal and future RFCs do not
accidentally collide with the namespace.
