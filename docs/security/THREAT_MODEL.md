# Orison Threat Model

This document is the formal threat-model companion to
[`SECURITY.md`](../../SECURITY.md). It enumerates the assets the
toolchain protects, the trust boundaries the system spans, the
adversaries we explicitly defend against, and — per asset — the
threats classified using a STRIDE-style table. Mitigations
distinguish what is shipping in the current bootstrap from what is
deferred to later milestones.

Where a mitigation cites a path, that path is the authoritative
implementation. Where a mitigation cites a roadmap item, the work is
deferred and the threat is currently unmitigated unless the
"Mitigations" column says otherwise.

## Table of contents

- [Assets](#assets)
- [Trust boundaries](#trust-boundaries)
- [Adversaries](#adversaries)
- [Threats per asset (STRIDE)](#threats-per-asset-stride)
- [Mitigations shipped today](#mitigations-shipped-today)
- [Mitigations deferred](#mitigations-deferred)
- [Out of scope](#out-of-scope)
- [Maintenance](#maintenance)

---

## Assets

The following assets are in scope. Each asset has a defined owner
(the principal whose intent the asset reflects) and a defined trust
class (the level of trust a downstream consumer should extend to it).

1. **Source code.** The `.ori` files an author commits to a package.
   Owner: the package author. Trust class: untrusted by the
   compiler, treated as input that must pass parsing, type checking,
   effect checking, and capability auditing before any tool will
   emit a derived artefact from it.
2. **Lockfile.** `ori.lock` produced by `crates/ori-pkg/src/lockfile.rs`.
   Owner: the package author, reproducible from `ori.toml` plus
   resolver output. Trust class: integrity-critical — consumers
   re-derive the lockfile and compare against the committed one to
   detect tamper.
3. **Registry tokens.** The credentials used by `ori publish` to
   write to a registry. In the bootstrap there is only a
   `LocalRegistry`, so the "token" is filesystem write permission;
   in production this will be an out-of-band capability with a
   distinct revocation lifecycle. Owner: the publishing principal.
   Trust class: secret.
4. **Capability tokens.** The `CapabilitySet`,
   `DelegationChainToken`, and `RevocationList` values defined in
   `crates/ori-compiler/src/capability_runtime.rs`. Owner: the
   principal that holds the token. Trust class: confidential
   (knowledge of the token's bytes is equivalent to authority to
   exercise it in v1; v2 tightens with attenuation).
5. **Signing keys.** The keys that will eventually back package
   provenance. Bootstrap status: not yet generated — provenance
   today carries a deterministic FNV-1a stub signature (see RFC 0004
   §"Drawbacks" for the upgrade path). Owner: the publishing
   principal. Trust class: secret in production.
6. **Build artefacts.** The compiled outputs of `ori build`, the
   release binaries of `ori` itself, and the JSON envelopes the CLI
   emits. Owner: the build operator. Trust class: integrity-critical
   for any consumer (CI, downstream tooling, agents).
7. **Dev-machine credentials.** SSH keys, GitHub tokens, registry
   tokens, and cloud credentials present on a maintainer's machine.
   Owner: the maintainer. Trust class: secret. This asset is
   adjacent to but not stored by the Orison toolchain; the toolchain
   defends against escalation paths that would expose it.

## Trust boundaries

Each boundary is a place where authority is reduced or remapped as
data crosses it. A boundary is named by the two principals on either
side.

1. **Source author ↔ compiler.** The compiler treats source as
   untrusted input. Parsing, type checking, effect checking, and
   the static gate (`scripts/validate_all.py`) run on every commit.
   The compiler never executes source as part of the parse pipeline;
   only `ori run` or `ori interp` evaluate, and they do so through
   the tree-walking interpreter at
   `crates/ori-compiler/src/interp_exec.rs`, which has a closed
   value lattice and a `MAX_CALL_DEPTH` cap.
2. **Compiler ↔ runtime.** The compiler emits declared effects on
   every function; the runtime enforces those declarations via the
   capability guard (`guard_call_at` in `capability_runtime.rs`)
   when capabilities are present. A function calling an effect it
   did not declare is a compile-time error (`E0420`); a function
   exercising an effect the runtime has not granted is a guard
   denial (`CAP0001`–`CAP0006`).
3. **Runtime ↔ host OS.** The bootstrap runtime never reaches the
   host OS for production effects. There is no real `fs`, `net`,
   `process`, `mail`, or `websocket` implementation — the stdlib
   modules at `stdlib/std/{mail,process,websocket}.ori` are
   declarations only. The interpreter is pure with respect to the
   host except for stdout/stderr writes from the CLI driver.
4. **Package author ↔ registry.** The registry stores published
   packages. In the bootstrap `LocalRegistry`, the "registry" is a
   directory and the "publish" is a `serde_json` write. Production
   registries will mediate authority with tokens; the bootstrap
   surface is intentionally minimal so the production surface can
   be designed deliberately.
5. **Registry ↔ consumer.** A consumer reads a package, validates
   its lockfile against the registry index, and downloads tarballs
   by FNV-1a digest. The boundary today is in-process; production
   will move it over HTTPS, requiring the TLS dependency proposed
   by RFC 0004.
6. **Principal ↔ capability token.** A capability token grants a
   principal authority over an effect. The boundary is the
   distinction between *holding* the token (presence in the
   `CapabilitySet`) and *exercising* it (a call site that requires
   the effect). Revocation flips the boundary's allow/deny verdict
   without revoking the token's bytes — the
   `RevocationList::check_revoked` lookup at every guard call
   ensures revoked principals cannot exercise the effect even if
   their token still parses.

## Adversaries

The model defends against the following adversaries. Each one is
described by capability and motivation, not identity.

1. **Malicious package author.** Publishes a package that declares
   benign effects in `ori.toml` but exercises broader effects in
   source. Capability: full control over the package payload.
   Motivation: data exfiltration, supply-chain reach.
2. **Registry compromise.** An attacker who controls a registry can
   serve modified package payloads, modified lockfile entries, or
   spoofed provenance documents. Capability: arbitrary substitution
   at fetch time.
3. **MITM on registry fetch.** An on-path attacker between consumer
   and registry. Capability: read, drop, or rewrite traffic. Relevant
   once `ori publish` and `ori install` move over HTTPS; today the
   `LocalRegistry` path is in-process so this threat is not currently
   live, but it becomes live the moment RFC 0004's `rustls` dep
   lands.
4. **Malicious code in a dependency.** A direct or transitive
   dependency that the author trusts but should not. Capability:
   full control over the dependency's exported surface; bounded by
   the effect-check and audit passes if the consumer policy is
   tight.
5. **Compromised dev machine.** An attacker who has executed code
   on a maintainer's workstation. Capability: read local
   credentials, modify source pre-commit, publish on the
   maintainer's behalf. Largely an out-of-band concern; the
   toolchain reduces blast radius by treating every commit as
   untrusted input on the CI side.
6. **Malicious model in agent loop.** An LLM that produces
   structurally well-formed Patch IR but with adversarial intent
   (delete-everything operations, capability-creep operations,
   exfiltration via test scaffolding). Capability: produces patches
   the toolchain will accept structurally. Mitigated by the closed
   Patch IR operation taxonomy (RFC 0003) and by the effect-check
   pass running on every applied patch before any subsequent build.
7. **Host OS escalation.** A local attacker who already has user-
   level access escalates to root via the Orison toolchain.
   Capability: read/write of files the user can see. The toolchain
   exposes no setuid binary and uses no `unsafe` Rust
   (`unsafe_surface_report` test enforces); the attack surface
   reduces to "what the user could already do".

## Threats per asset (STRIDE)

The table below summarises threats by asset using the STRIDE
classification (Spoofing, Tampering, Repudiation, Information
disclosure, Denial of service, Elevation of privilege).

| Asset                | S | T | R | I | D | E |
| -------------------- | - | - | - | - | - | - |
| Source code          | M | H | L | L | M | M |
| Lockfile             | M | H | L | L | L | L |
| Registry tokens      | H | M | M | H | L | H |
| Capability tokens    | H | H | M | M | M | H |
| Signing keys         | H | H | M | H | L | H |
| Build artefacts      | M | H | L | L | M | M |
| Dev-machine creds    | M | M | M | H | L | H |

Severity legend: H = high, M = medium, L = low.

Concrete threats per cell, with mitigation pointer:

- **Source code, Tampering (H).** An attacker who can write the
  repo can edit `.ori` files to broaden effects. Mitigation: the
  effect-check pass (`crates/ori-compiler/src/effect_check.rs`)
  raises `E0410`/`E0420`; the audit pass raises `AUD0001`. Both
  fail the build before any artefact is produced.
- **Lockfile, Tampering (H).** An attacker swaps a checksum to
  point at a malicious tarball. Mitigation: the consumer re-derives
  the lockfile from `ori.toml` and compares (`lockfile_tamper`
  test). Limitation: the digest is FNV-1a, not cryptographic — see
  the Internal Audit's `M-PUB-001` finding.
- **Registry tokens, Spoofing (H) / Elevation (H).** An attacker
  who steals the token can publish in the owner's name. Mitigation
  in bootstrap: filesystem permissions on the `LocalRegistry`
  directory. Production mitigation pending.
- **Capability tokens, Spoofing (H) / Elevation (H).** An attacker
  who fabricates a token grants themselves an effect. Mitigation:
  the v2 chain (`DelegationChainToken` in `capability_runtime.rs`)
  binds tokens to principals; the revocation list (`RevocationList`)
  invalidates exercised authority on demand. Limitation: tokens are
  in-process; cross-registry revocation propagation is not yet
  shipping.
- **Signing keys, Tampering / Information disclosure (H).**
  Compromise of the signing key lets the attacker publish any
  payload as the project. Mitigation: not yet — the bootstrap
  signature is a stub. RFC 0004 unblocks the dependency the real
  signature will need (`ring` or `ed25519-dalek` once a follow-up
  RFC sub-ticket accepts it).
- **Build artefacts, Tampering (H).** An attacker swaps the
  shipped `ori` binary. Mitigation: SBOM artefact on every release
  (`sbom.yml`), reproducible-build invariants in
  `scripts/validate_all.py --full`, and downstream consumers should
  verify SBOM digests. Limitation: digest is informational, not
  signed.
- **Dev-machine credentials, Information disclosure (H).** The
  toolchain does not store these but can be a vector if a
  malicious plugin exfiltrates them. Mitigation: the toolchain has
  no plugin system; extensions live under `extensions/` and are
  reviewed source-in-tree.

## Mitigations shipped today

The mitigations enumerated below are live in the codebase at the
file paths cited. They are the floor of the security posture; the
deferred list extends them.

- **Capability creep detection.** Audit pass raises `AUD0001` for
  any used effect not declared in `ori.toml`. Path:
  `crates/ori-pkg/src/audit.rs`. Companion compile-time check
  `E0410` lives in `crates/ori-compiler/src/effect_check.rs`.
- **Effect propagation.** `E0420` rises through the call graph so
  that a function declaring `db.read` cannot transitively invoke a
  `db.write` callee. Path:
  `crates/ori-compiler/src/effect_propagate.rs`.
- **Capability runtime guard.** `guard_call_at` and
  `guard_call_with_chain` evaluate capability presence, delegation
  chain age, and revocation per call. Path:
  `crates/ori-compiler/src/capability_runtime.rs`. Threaded into
  the interpreter at `crates/ori-compiler/src/interp_exec.rs`
  through `ExecState` and the eval_call code path.
- **Revocation list.** `RevocationList::check_revoked` enforces
  effect-scoped denial keyed by principal id. Path:
  `crates/ori-compiler/src/capability_runtime.rs`.
- **Lockfile integrity.** `lockfile_signature` deterministically
  re-derives the lockfile and asserts equality; tamper is detected
  by the `lockfile_tamper` test. Path:
  `crates/ori-pkg/src/lockfile.rs`.
- **Provenance scaffolding.** `ori publish` emits a
  `PublishReceipt` carrying a deterministic FNV-1a stub signature
  so the contract surface is in place even though the cryptographic
  signature is deferred. Path: `crates/ori-pkg/src/publish.rs`.
  Provenance verification raises a structured `verified: false` for
  unrecognised algorithm prefixes. Path:
  `crates/ori-pkg/src/provenance.rs`.
- **`unsafe` exclusion.** `unsafe_surface_report` test asserts that
  no Rust source under `crates/*/src/` contains `unsafe fn`,
  `unsafe impl`, `unsafe trait`, or `unsafe { ... }`. Enforced by
  `scripts/validate_all.py`.
- **Production-source guardrails.** `.unwrap()`, `.expect()`,
  `panic!`, `todo!`, `unimplemented!`, and `dbg!` are banned in
  production sources. Enforced by `scripts/validate_all.py`.
- **Dependency allow-list (bootstrap).** Workspace deps are limited
  to `serde` and `serde_json`. Enforced by
  `scripts/validate_all.py`. RFC 0004 proposes the controlled
  relaxation needed for production work.
- **Bounded recursion in the interpreter.** `MAX_CALL_DEPTH = 64`
  cap aborts deeply nested execution with `R0005` before the OS
  stack is exhausted. Path:
  `crates/ori-compiler/src/interp_exec.rs`.

## Mitigations deferred

The following mitigations are not yet in the codebase but are on the
roadmap. The Internal Audit tracks each as an open finding with
severity and target.

- **Cryptographic signature on `ori publish` receipts.** Replaces
  the FNV-1a stub. Blocked on RFC 0004 (production dep policy) plus
  a follow-up RFC selecting the algorithm.
- **HTTPS transport for registry fetches.** Blocked on RFC 0004
  accepting `rustls` + `webpki-roots`.
- **Cross-registry revocation propagation.** The local
  `RevocationList` is in-process; production needs a propagation
  protocol that distributes revocations to mirrors and caches.
- **Hardware-bounded recursion / memory limit.** The current
  `MAX_CALL_DEPTH` is a frame-count soft cap. A real memory limit
  needs accounting against the heap, not just the frame count.
- **Real production stdlib bodies for `mail`, `process`,
  `websocket`.** Today these are declarations only; safe by
  vacuity (nothing reaches the host), but not useful in
  production.
- **TLS verification policy.** Once `rustls` lands, the policy
  decision is "which root CAs do we trust?" — `webpki-roots` is
  the default but consumers may want a pinned set.

## Out of scope

The following classes of attack are explicitly out of scope for the
Orison toolchain. They are real concerns; they are simply not what
this codebase defends against. Acknowledging them keeps the
threat-model claims honest.

- **Timing side channels.** The interpreter and compiler are not
  written to be constant-time. A program processing secret data
  may leak timing information that an adversary can observe. If
  constant-time execution is required, the program must be
  compiled with that constraint and run in an environment that
  preserves it; the Orison toolchain makes no such guarantee.
- **Hardware fault injection.** Rowhammer-class, voltage-glitching,
  and electromagnetic-pulse attacks fall to the hardware and OS
  vendor.
- **Supply-chain attacks on the Rust toolchain itself.** A malicious
  `rustc` or a malicious `cargo` could produce a malicious `ori`
  binary regardless of any policy the project enforces. Mitigation
  is upstream (rust-lang's signing and reproducibility work) and
  out of band of this codebase. Consumers concerned about this
  should verify their `rustc` install via `rustup`'s signature
  chain.
- **OS-level escalation outside the toolchain.** A kernel bug, a
  setuid binary on the host, or a malicious driver are outside
  the Orison threat model.
- **Physical access to dev machines.** Disk encryption, screen
  locking, and physical security are owned by the user.
- **Social engineering of maintainers.** A reviewer who is fooled
  into approving a malicious patch defeats every technical
  mitigation. The project's defence here is process: the two-
  reviewer rule for security-sensitive changes (`docs/rfcs/PROCESS.md`
  §8.3) and the rate at which maintainers rotate (per
  `MAINTAINERS.md`).

## Maintenance

This document is updated whenever any of the following occurs:

1. A new asset appears in the codebase. (Example: when real signing
   keys land, the "Signing keys" row's "shipped today" mitigations
   change.)
2. A trust boundary moves. (Example: when `ori publish` moves over
   HTTPS, the "Registry ↔ consumer" boundary's threat surface
   grows to include MITM.)
3. A new adversary becomes plausible. (Example: when the agent
   loop ships its first network-facing model adapter, the
   "Malicious model in agent loop" adversary's capability grows.)
4. An RFC accepted under [`docs/rfcs/PROCESS.md`](../rfcs/PROCESS.md)
   §8.3 modifies the security posture. The RFC's "Compatibility
   impact" section is the changelog source.

The document's two companions are
[`SECURITY.md`](../../SECURITY.md) (the user-facing summary and the
reporting policy) and
[`INTERNAL_AUDIT.md`](./INTERNAL_AUDIT.md) (the rolling list of
findings with severities). Treat the three as a triangle: this
document is the model, the audit is the gap-list, and the security
policy is the public commitment.
