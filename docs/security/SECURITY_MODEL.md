# Security Model

Orison security combines memory safety, capability effects, package integrity, and supply-chain provenance.

## Safe code guarantees

Safe Orison code must not:

- dereference invalid memory;
- cause data races;
- use after free;
- double free;
- read uninitialized memory;
- silently access undeclared filesystem, network, process, database, or environment capabilities;
- perform unchecked FFI.

## Unsafe code

Unsafe code exists but is narrow and explicit.

```ori
unsafe fn from_raw(ptr: Ptr[UInt8], len: Int) -> Bytes:
  ...
```

Unsafe effects propagate into:

- diagnostics;
- capsules;
- package manifests;
- audit reports.

## Package controls

The package manager should support:

- content-addressed package resolution;
- lockfiles;
- signed packages;
- SBOM output;
- provenance metadata;
- capability diffing;
- build script permission declarations.

## Build scripts

No arbitrary build scripts by default. A build script must declare capabilities.

```ori
build_script GenerateIcons uses fs.read, fs.write:
  ...
```
