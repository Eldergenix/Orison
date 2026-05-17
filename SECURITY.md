# Orison Security Model

This document captures the security posture of the bootstrap and the
contracts the package manager + compiler enforce. The detailed
implementation lives in `crates/ori-compiler/src/effect_check.rs`,
`crates/ori-pkg/src/audit.rs`, and the security audit test suite under
`crates/{ori-compiler,ori-pkg}/tests/`.

## Threat model in scope

The bootstrap defends against the following classes of issue:

1. **Capability creep.** A dependency requires `fs.write` but the root
   package only declares `fs.read`. → `audit` finding `AUD0001` of
   severity `error`; `effect_check` raises `E0410`.
2. **Effect leak through call graph.** A function declares `uses
   db.read` but transitively calls a `uses db.write` callee. →
   `effect_propagate` raises `E0420` with a `change_signature` Patch IR
   fix appending the missing effect.
3. **Stale lockfile checksums.** Lockfile entries that don't match the
   freshly rebuilt lockfile signal tamper. → `lockfile_tamper` test
   asserts detection.
4. **Provenance spoofing.** Provenance JSON with an unrecognised
   signature is marked `verified: false` with notes. → `provenance_failure`
   test corpus.
5. **`unsafe` introduction.** Workspace scan asserts every Rust source
   under `crates/*/src/` is free of `unsafe fn / impl / trait / {`. →
   `unsafe_surface_report` test fails if any unsafe surface appears.
6. **Undeclared ambient I/O.** Source code that uses unknown effect
   names triggers `W0401`; with a non-empty package policy, it
   escalates to `E0410` via `effect_check::effect_diagnostics`.

## Threat model *not* in scope (bootstrap)

The bootstrap does **not** defend against the following — they are on
`docs/ROADMAP.md`:

- Sophisticated supply-chain attacks (the lockfile checksum is FNV-1a,
  not cryptographic).
- Malicious code in dependencies (no real sandboxing).
- Side-channel attacks (no constant-time guarantees in stdlib).
- Network MITM (no TLS verification — no real network stack at all).
- Privilege escalation at runtime (no capability runtime gating; the
  bootstrap is static-analysis-only).
- Memory safety bugs in `unsafe` Rust *inside the compiler itself*
  (the workspace forbids unsafe Rust — `validate_all.py` enforces).

## Capability lifecycle

```
[ori.toml]
[capabilities]
declared = ["http", "db.read", "db.write"]
            │
            │  read by ori-pkg/Manifest
            │
            ▼
[ori-pkg/audit::run_audit]
  ┌── declared - used     → AUD0002 info "unused capability"
  └── used - declared     → AUD0001 error "missing capability"
            │
            ▼
[ori-compiler/effect_check::effect_diagnostics]
  ┌── undeclared(used)    → E0410 error
  └── (with body parser)
       └── propagated     → E0420 error with Patch IR fix
            │
            ▼
[ori capability --json] → ori.capability.v1
[ori audit --json]      → ori.audit_report.v1
```

## Reporting issues

Use `.github/ISSUE_TEMPLATE/bug_report.md` for vulnerability reports
*until* the project has a dedicated security policy at `SECURITY.md`.
Do not post proof-of-concept exploit code in public issues until the
underlying fix is merged.

## Audit cadence

Every PR must:

1. Re-run the security test suite (`cargo test --workspace`).
2. Verify the `unsafe_surface_report` test still asserts zero.
3. Add a new test under `crates/ori-pkg/tests/` or
   `crates/ori-compiler/tests/` for any new capability check or effect
   rule.

CI workflows `static.yml`, `test.yml`, `sbom.yml` enforce the suite on
every push and the SBOM artefact on every release.
