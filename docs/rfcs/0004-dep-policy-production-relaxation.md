# RFC 0004: Production dependency policy — controlled relaxation

| Field           | Value                                                              |
| --------------- | ------------------------------------------------------------------ |
| RFC number      | 0004                                                               |
| Title           | Production dependency policy — controlled relaxation               |
| Authors         | Orison core (BDFL: Eldergenix)                                     |
| Status          | Draft                                                              |
| Pre-RFC issue   | https://github.com/Eldergenix/Orison/issues/XXXX                   |
| PR              | https://github.com/Eldergenix/Orison/pull/XXXX                     |
| Created         | 2026-05-17                                                         |
| FCP entered     |                                                                    |
| Merged          |                                                                    |
| Implemented     |                                                                    |
| Stabilised      |                                                                    |
| Supersedes      |                                                                    |
| Superseded by   |                                                                    |

---

## Table of contents

- [Summary](#summary)
- [Motivation](#motivation)
- [Detailed design](#detailed-design)
  - [The current rule](#the-current-rule)
  - [The proposed allow-list](#the-proposed-allow-list)
  - [Per-entry contract](#per-entry-contract)
  - [Initial entries](#initial-entries)
  - [How an addition is approved](#how-an-addition-is-approved)
  - [How `validate_all.py` is updated](#how-validate_allpy-is-updated)
  - [Per-dep ownership and audit cadence](#per-dep-ownership-and-audit-cadence)
- [Drawbacks](#drawbacks)
- [Alternatives considered](#alternatives-considered)
- [Prior art](#prior-art)
- [Unresolved questions](#unresolved-questions)
- [Future possibilities](#future-possibilities)
- [Acceptance criteria](#acceptance-criteria)
- [Compatibility impact](#compatibility-impact)
- [Adoption plan](#adoption-plan)
- [Rollback](#rollback)

---

## Summary

Replace the bootstrap rule "the only currently-approved workspace
dependencies are `serde` and `serde_json`" with a curated allow-list
that adds a small set of pure-Rust production dependencies, each
gated by a per-entry contract (purpose, transitive dep count,
license, audit owner, audit cadence). Phase in the change through
RFC sub-tickets, one per dep, so each addition is reviewed against
the security posture documented in
[`docs/security/THREAT_MODEL.md`](../security/THREAT_MODEL.md) and
the gap-list in
[`docs/security/INTERNAL_AUDIT.md`](../security/INTERNAL_AUDIT.md).

## Motivation

Several production-grade features cannot be implemented as honest
shippable code under the bootstrap policy:

1. **HTTPS for `ori publish` and `ori install`.** The current
   `LocalRegistry` path is in-process; there is no network code at
   all. Real package distribution needs TLS, which needs a TLS
   library. The relevant audit finding is `L-NET-001` in
   [`INTERNAL_AUDIT.md`](../security/INTERNAL_AUDIT.md) and the
   relevant threat is "Registry ↔ consumer" in
   [`THREAT_MODEL.md`](../security/THREAT_MODEL.md).
2. **Native ahead-of-time codegen.** Today the toolchain emits
   structured CLI envelopes and a text-IR; there is no machine
   code generation. Real performance work needs a code generator.
   The two production options on the table are LLVM (rejected:
   not pure-Rust, drags in C++ toolchains, multi-GB build times)
   and Cranelift (pure-Rust, scoped, used by `wasmtime` in
   production).
3. **Real event-driven async I/O.** Once HTTPS exists, the
   `std::thread`-per-connection model will become a bottleneck.
   The standard pure-Rust path is `mio` (the epoll/kqueue/iocp
   abstraction layer used by `tokio`).
4. **Future cryptographic signatures.** `M-PUB-001` documents the
   FNV-1a stub. The eventual fix needs an asymmetric crypto
   library. This RFC does not pre-decide that dep; it does decide
   the *process* by which the dep can be added.

The bootstrap-only policy ("`serde` plus `serde_json`, anything
else needs a CHANGELOG entry explaining the rationale") was the
right floor for waves 1-4: it forced the contract surface to be
shaped without leaning on convenience deps. With waves 1-4
shipped, the policy is now the binding constraint on every Layer 2
milestone that needs to interact with the outside world.

The relaxation must not become a slippery slope. Each addition has
to be deliberate, named, owned, and revertible. That is the design
this RFC encodes.

## Detailed design

### The current rule

`CONTRIBUTING.md` §"Source guardrails" says:

> No new third-party dependency without a `CHANGELOG.md` entry
> explaining the rationale. The only currently-approved deps are
> `serde` and `serde_json`.

`scripts/validate_all.py` enforces the workspace allow-list at the
static gate. The allow-list lives in the script's source and is
read by `--full` runs to fail any `Cargo.toml` that introduces a
forbidden dep.

### The proposed allow-list

A new section in `CONTRIBUTING.md` titled "Approved production
dependencies" replaces the inline allow-list with the table below.
The table is the authoritative source; `scripts/validate_all.py`
reads from it.

| Crate            | Purpose                                  | Approved for     | Audit owner   |
| ---------------- | ---------------------------------------- | ---------------- | ------------- |
| `serde`          | Typed (de)serialisation                  | All crates       | Orison core   |
| `serde_json`     | JSON envelope encoding/decoding          | All crates       | Orison core   |
| `rustls`         | TLS 1.2 / 1.3 client and server          | `ori-pkg` only   | Orison core   |
| `webpki-roots`   | Mozilla root certificate bundle for `rustls` | `ori-pkg` only | Orison core |
| `cranelift`      | Pure-Rust code generator                 | `ori-compiler`   | Orison core   |
| `mio`            | Epoll/kqueue/iocp abstraction            | `ori-pkg`        | Orison core   |

The "Approved for" column scopes the dep to a single crate so the
blast radius of a malicious-update event is bounded. A dep approved
for `ori-pkg` cannot appear in `ori-compiler` without a separate
RFC sub-ticket extending its scope.

### Per-entry contract

Every entry in the allow-list must declare the following fields in
its corresponding CHANGELOG and CONTRIBUTING entry:

1. **Purpose.** One sentence stating what the dep does and why it
   cannot be replaced with `std`.
2. **Total transitive dep count.** Measured by `cargo tree -p
   <crate>` at the version pinned in `Cargo.toml`. The count
   becomes a regression gate: a future minor-version bump that
   increases the count by more than 20% must be flagged in the
   PR description.
3. **License.** Must be one of MIT, Apache-2.0, BSD-2/3, ISC, or
   MPL-2.0. GPL/AGPL deps are not eligible.
4. **Security-audit status.** Either "audited externally on
   <date> by <auditor>" or "self-audited under the methodology
   in [`INTERNAL_AUDIT.md`](../security/INTERNAL_AUDIT.md)".
5. **Audit owner.** A specific maintainer (per `MAINTAINERS.md`)
   responsible for tracking advisories and proposing updates. The
   audit owner is named because deps die without owners.
6. **Update cadence.** Monthly minimum check against `cargo audit`
   advisories; quarterly review of upstream changelog.

The contract is enforced by `scripts/validate_all.py` reading the
CONTRIBUTING table and asserting every workspace dep has the six
fields populated.

### Initial entries

The first relaxation lands four production deps. Each is justified
below.

**`rustls`** (TLS 1.2 / 1.3, pure-Rust, no `openssl`).

- Purpose: TLS transport for `ori publish` and `ori install` HTTPS
  flows; closes audit finding `L-NET-001` once accepted.
- Replaces: nothing today (the current path is in-process
  `LocalRegistry`).
- Transitive count target: pinned at the version we land; bumping
  the count more than 20% is a flagged change.
- License: Apache-2.0 / MIT / ISC.
- Audit status: extensively audited upstream (used by Firefox via
  the `Rustls` Firefox crate; commercial audits in 2020 and 2023).
- Audit owner: Orison core.

**`webpki-roots`** (Mozilla root certificate bundle for `rustls`).

- Purpose: default trust anchors for `rustls`, regenerated from
  Mozilla's CA program.
- License: MPL-2.0.
- Audit status: data crate, no executable code; audit reduces to
  "does the bundle match Mozilla's published bundle?".
- Audit owner: Orison core.

**`cranelift`** (pure-Rust code generator, no LLVM).

- Purpose: native AOT codegen backend for `ori build --target
  native`. Cranelift is used in production by `wasmtime` and
  `wasmer`.
- Replaces: nothing today; the bootstrap codegen path emits
  text-IR only.
- License: Apache-2.0 with LLVM exception.
- Audit status: audited upstream as part of `wasmtime`'s release
  cadence; the relevant CVE history is well-known and the
  upstream advisory response time is short.
- Audit owner: Orison core. Update cadence: quarterly, aligned
  with `wasmtime` minor releases.

**`mio`** (epoll/kqueue/iocp abstraction, pure-Rust).

- Purpose: event-driven I/O for `ori-pkg` once HTTPS is the
  default. The thread-per-connection alternative is acceptable
  for a small number of concurrent uploads but does not scale to
  registry-server use cases.
- Replaces: nothing today.
- License: MIT.
- Audit status: audited as part of the `tokio` ecosystem; very
  small surface area (file descriptor abstraction).
- Audit owner: Orison core.

Optional fifth entry deferred to its own sub-RFC: an asymmetric
crypto library to back the production `ori publish` signature.
Candidates: `ring` (Rust + assembly), `ed25519-dalek` (pure-Rust).
The choice is intentionally deferred because the threat model for
package signing (key generation, key custody, key rotation,
revocation) is itself unsettled. See [Open questions](#unresolved-questions).

### How an addition is approved

1. A sub-RFC is opened referencing this RFC.
2. The sub-RFC fills the six fields of the per-entry contract.
3. The sub-RFC is reviewed under
   [`PROCESS.md`](./PROCESS.md) §8.1 (Cargo dependency additions)
   with the additional reviewer rotation rule from §8.3
   (security-sensitive changes).
4. On merge, `CONTRIBUTING.md`'s allow-list table is updated and
   `scripts/validate_all.py`'s static-gate allow-list is updated in
   the same PR.
5. The next-following CHANGELOG release notes the dep under a new
   "Production dependencies added" subsection.

### How `validate_all.py` is updated

The static gate currently has a hard-coded allow-list of `serde`,
`serde_json`. The relaxation replaces the hard-coded list with a
reader that parses the CONTRIBUTING table. The reader is
deliberately strict:

- The table must be a Markdown table under the heading "Approved
  production dependencies".
- Each row is `| <crate> | <purpose> | <approved for> | <audit
  owner> |`.
- A row with a missing field fails the gate.
- A workspace dep absent from the table fails the gate.
- A row referencing a non-existent workspace dep emits a warning
  (helps surface stale rows after a removal).

### Per-dep ownership and audit cadence

Every dep entry names an audit owner. The owner is responsible for:

- Subscribing to upstream security advisories (GitHub Security
  Advisories, RustSec, vendor mailing list).
- Running `cargo audit` against the workspace lockfile monthly.
- Proposing a minor-version bump within seven days of any
  affected advisory.
- Performing the quarterly upstream-changelog review.

If an owner steps down, the dep enters a 30-day grace period
during which a new owner must be named; otherwise the dep is
flagged for removal by the next RFC.

## Drawbacks

1. **Increased attack surface.** Every line of third-party code is
   a line not authored by the Orison maintainers. The audit cost
   grows linearly with the dep count.
2. **Longer build times.** Cranelift in particular is a multi-
   minute clean build. CI cost grows; cold-cache CI grows fastest.
3. **More SBOM entries.** `sbom.yml` will emit a larger artefact;
   downstream consumers running their own SBOM diffs will see a
   step change.
4. **Exposure to upstream breakage.** A bad release of `rustls` or
   `cranelift` becomes an Orison problem within hours. The pinned-
   version policy reduces but does not eliminate the risk.
5. **The allow-list is itself a target.** A reviewer who is
   convinced to add a dep with a hidden malicious transitive can
   defeat the policy. The two-reviewer rule under
   `PROCESS.md` §8.3 is the mitigation, and it is not infallible.
6. **The audit-owner role does not scale infinitely.** A small
   maintainer team that owns every dep is one resignation away
   from a stalled review queue. The 30-day grace period is the
   pressure-release valve; if it fires often, the team is too
   small for the dep count.

## Alternatives considered

- **Stay pure-std.** Keep the `serde` + `serde_json` rule
  indefinitely. Consequence: HTTPS, native codegen, and event-
  driven async stay out of bounds for years; the language ships
  as a self-contained toy. This was the right choice for waves
  1-4 and is the wrong choice for what comes next, because the
  threat model in
  [`THREAT_MODEL.md`](../security/THREAT_MODEL.md) has assets
  (registry tokens, signing keys) that cannot be protected by
  absent code paths.
- **Vendor a curated standard library.** Fork the relevant
  upstream crates into `vendor/` and own the maintenance. Higher
  audit budget per crate, lower upstream-breakage risk, much
  larger ongoing cost. Rejected on cost grounds; this RFC keeps
  the option open for a later "high-assurance" build profile.
- **Allow any dep but require RFC sub-tickets.** Same governance
  weight as the proposed design but without the curated list.
  Rejected because the list itself is a positive constraint:
  reviewers can refer to "is X on the list?" as a sufficient
  criterion at PR review time.
- **Allow a much larger initial set.** Add `tokio`, `hyper`,
  `reqwest`, etc. in one batch. Rejected as more change than the
  audit infrastructure can absorb at once; the phased plan keeps
  every addition reviewable.

## Prior art

- **Rust standard library scope policy.** The Rust project itself
  exercises restraint on what goes into `std`; the `extern crate`
  surface is small and additions are RFC-gated. The same
  philosophy applies here, with the inversion that Orison is
  curating *additions* to its workspace, not *removals* from a
  larger surface.
- **Bazel / Pants `requirement_set`.** Build systems for large
  monorepos curate a single requirement set that all targets
  consume. The allow-list here is the same idea applied to a
  language toolchain.
- **Debian / NixOS package selection.** Both projects have audit
  layers that decide which upstream packages enter the trusted
  set. The audit-owner role here is a lighter-weight version of
  the Debian package-maintainer role.
- **`wasmtime` and `rustls` dep curation.** Both projects publish
  explicit dep-policy docs. They were the most useful prior art
  for shaping the per-entry contract.

## Unresolved questions

1. **Vendoring strategy.** Should the workspace vendor the
   approved deps into `vendor/` for build reproducibility, or rely
   on `Cargo.lock` plus the registry mirror?
2. **Version pinning policy.** Pin to exact versions, or to
   compatible (`^x.y.z`) ranges? Exact pinning maximises
   reproducibility but costs more maintenance.
3. **Allow-list maintainership.** Who is allowed to propose
   additions to the allow-list — any contributor, only
   maintainers, or only the BDFL?
4. **Audit cadence frequency.** Monthly `cargo audit` plus
   quarterly changelog review is the proposal. Is monthly enough
   for `rustls` (TLS bugs land continuously)? Should `rustls`
   have a weekly cadence specifically?
5. **Supply-chain attack response playbook.** What does the
   project do in the first hour after a critical advisory in an
   approved dep? Who has authority to ship a yanked-and-replaced
   workspace? The playbook does not exist yet.
6. **Optional vs required deps.** Should approved deps be optional
   `[features]` (only built when the feature is enabled) or
   always-on? `cranelift` in particular suggests optional; a
   contributor who only wants the LSP should not pay the
   `cranelift` build cost.
7. **Build reproducibility.** Adding `cranelift` introduces native
   code generation paths that may not be byte-reproducible across
   build hosts. How does the SBOM verification policy account for
   that?
8. **Performance impact.** How much does each dep slow down a
   clean build, a warm build, and a CI run? Numbers will inform
   the next-batch decisions.
9. **Bundled root-CA freshness.** `webpki-roots` snapshots
   Mozilla's bundle. What is the maximum staleness the project
   accepts before bumping the bundle, and what is the response
   path if a root is removed by Mozilla mid-cycle?
10. **Cross-platform consistency.** Does `mio` give the same
    semantics on Linux (epoll), macOS (kqueue), and Windows
    (IOCP)? The project's testing matrix needs an answer before
    `mio` becomes load-bearing.
11. **Telemetry on dep use.** Should the toolchain emit telemetry
    on which approved deps are actually exercised at runtime? The
    privacy stance probably says no; the security stance probably
    says yes. The two have to be reconciled.
12. **Future asymmetric-crypto dep.** Choice between `ring` and
    `ed25519-dalek` is deferred to a sub-RFC. What evidence does
    that sub-RFC need to make the decision?

## Future possibilities

- **High-assurance build profile.** Vendor the deps, audit each
  release commit-by-commit, publish reproducible-build attestations.
- **Multi-tier allow-list.** A "core" tier (always allowed) and a
  "feature" tier (only allowed under a named cargo feature) so
  the default build stays minimal.
- **Automated dep-tree linting.** A pre-commit hook that runs
  `cargo tree` against the approved list and fails if a
  transitive dep appears that is not on the list. Stronger than
  the current top-level check.
- **Signed dep manifests.** Once the production signing path
  exists, sign the allow-list itself.

## Acceptance criteria

- [ ] `CONTRIBUTING.md` has a section titled "Approved production
      dependencies" with the table specified in
      [The proposed allow-list](#the-proposed-allow-list).
- [ ] `scripts/validate_all.py` reads the allow-list from
      `CONTRIBUTING.md` and fails the static gate when a workspace
      dep is not in the table.
- [ ] Each entry in the table cites the six fields of the per-
      entry contract in a follow-up CHANGELOG entry.
- [ ] The first four production deps (`rustls`, `webpki-roots`,
      `cranelift`, `mio`) are each shipped under their own
      RFC sub-ticket and not in this RFC's merge.
- [ ] An audit owner is named per dep in `CONTRIBUTING.md`.
- [ ] The two security-companion docs
      ([`THREAT_MODEL.md`](../security/THREAT_MODEL.md) and
      [`INTERNAL_AUDIT.md`](../security/INTERNAL_AUDIT.md)) are
      updated to reflect the closed and remaining findings after
      each sub-ticket lands.

## Compatibility impact

This RFC changes a policy, not a public surface. Source
compatibility, JSON contract compatibility, Rust API compatibility,
CLI compatibility, and agent ABI compatibility are all unchanged
by the policy itself. Each sub-ticket that adds a dep may have its
own compatibility implications (e.g. the `rustls` sub-ticket will
expose new CLI flags for `ori publish` certificate paths); those
implications are addressed in the sub-tickets, not here.

The static gate's behaviour changes:

- Before: any workspace dep outside `{serde, serde_json}` fails.
- After: any workspace dep absent from the CONTRIBUTING table
  fails.

This is a strict tightening for contributors who were relying on
the CHANGELOG-entry escape hatch; it is a strict loosening for the
project as a whole because the table can grow under sub-RFC
review.

## Adoption plan

The plan is phased so each phase can be reverted independently.

1. **Phase 1 (this RFC).** Land the policy. Update
   `CONTRIBUTING.md` with the table containing only `serde` and
   `serde_json`. Update `scripts/validate_all.py` to read from
   the table. No new deps yet.
2. **Phase 2 (sub-RFC 0004-A: `rustls`).** Add `rustls` +
   `webpki-roots` to the table; use them in `ori-pkg`'s HTTPS
   path. Close audit finding `L-NET-001`.
3. **Phase 3 (sub-RFC 0004-B: `cranelift`).** Add `cranelift`;
   use it behind a `native-codegen` cargo feature in
   `ori-compiler`. Establish the optional-dep pattern.
4. **Phase 4 (sub-RFC 0004-C: `mio`).** Add `mio` once Phase 2's
   thread-per-connection model is the bottleneck (measured,
   not assumed). Until measured, the addition is deferred.
5. **Phase 5 (sub-RFC 0004-D: cryptographic signature).** Choose
   between `ring` and `ed25519-dalek` and add the chosen dep.
   Close audit finding `M-PUB-001`.

Each phase is its own PR, its own merge, its own potential rollback.

## Rollback

Any added dep can be reverted before its first stable release if a
critical issue is found. The rollback procedure:

1. Remove the dep from `Cargo.toml`.
2. Remove the row from the `CONTRIBUTING.md` table.
3. Either reinstate the previous code path (thread-per-conn for
   `mio`, in-process `LocalRegistry` for `rustls`, text-IR-only
   for `cranelift`) or land a holding-pattern stub that emits an
   `unimplemented` error with a structured diagnostic.
4. Update `CHANGELOG.md` and re-open the corresponding audit
   finding.

Rollback is a normal RFC under
[`PROCESS.md`](./PROCESS.md), not an emergency-bypass mechanism.
The 30-day grace period for an unowned dep is the closest the
process gets to an automatic rollback trigger.
