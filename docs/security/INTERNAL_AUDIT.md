# Orison Internal Security Audit

This is the rolling internal-audit log for the Orison toolchain. It
sits beside [`THREAT_MODEL.md`](./THREAT_MODEL.md) (the model) and
[`SECURITY.md`](../../SECURITY.md) (the policy). The audit's job is
to keep an honest tally of where the codebase departs from the
threat model's intent, classify those gaps by severity, and link
each one to a fix or to a "won't fix in v1" decision.

The audit is updated:

- After every wave of Layer 1 / Layer 2 work that touches a
  security-relevant subsystem.
- After every accepted RFC under
  [`docs/rfcs/PROCESS.md`](../rfcs/PROCESS.md) §8.3
  (security-sensitive changes).
- When a maintainer running the methodology below opens a finding.

## Table of contents

- [Methodology](#methodology)
- [Severity rubric](#severity-rubric)
- [Findings](#findings)
- [Notes on individual findings](#notes-on-individual-findings)
- [Process for adding a finding](#process-for-adding-a-finding)

---

## Methodology

A reviewer walking the codebase looking for security issues follows
seven passes. Each pass has a clearly defined scope and a clearly
defined exit criterion. Passes are independent and may be done in
any order, but the recommended order is breadth-first.

### Pass 1: capability bypasses

Goal: confirm every effectful call site goes through the capability
guard when capabilities are present.

- Read `crates/ori-compiler/src/capability_runtime.rs` end to end.
  Public surface: `CallContext`, `CapabilitySet`, `GuardOutcome`,
  `guard_call_at`, `guard_call_with_audit_at`,
  `guard_call_with_chain`, `RevocationList`, `check_revoked`,
  `revoke`.
- Grep for `guard_call` and confirm every interpreter dispatch
  point flows through it: `interp_exec.rs` (via `ExecState` and
  `eval_call`), `bench.rs` (synthetic benchmark).
- For each hit, confirm the call context's `principal_id` comes
  from the runtime state, not user-supplied input.
- Exit: every call site is through the guard or under
  `#[cfg(test)]`.

### Pass 2: schema-drift exploits

Goal: ensure no public JSON envelope can be forged or mis-parsed
into a higher-authority shape.

- For each schema under `schemas/*.schema.json`, locate the emitter
  struct in `crates/*/src/`. Confirm the struct has a hard-coded
  `schema: &'static str` field and that downstream consumers
  reject mismatching ids.
- Confirm there is at least one round-trip conformance test that
  encodes the struct, decodes the JSON, and asserts the canonical
  shape (`crates/*/tests/`).
- Look for any path that constructs JSON by string concatenation
  rather than `serde`. There should be none in production sources
  (CONTRIBUTING.md §"JSON contract rules").
- Exit: every schema has a typed emitter, every consumer rejects
  unknown ids.

### Pass 3: registry-trust assumptions

Goal: surface places where the toolchain trusts data from a
registry beyond what its threat class warrants.

- Read `crates/ori-pkg/src/{registry,lockfile,provenance,publish}.rs`
  in order.
- For each function consuming registry-supplied bytes, confirm
  the bytes are checksum-verified before parse.
- For each function producing registry-bound bytes, confirm the
  producer is the only writer (no TOCTOU between digest and
  consumption).
- Audit FNV-1a usage explicitly: `fnv1a_hex` in `registry.rs`,
  `fnv1a_64` in `lockfile.rs`, the `fnv1a:` prefix in
  `publish.rs`. Each appearance must carry a "bootstrap stub, not
  cryptographic" comment.
- Exit: every registry boundary documented as trusted (and why)
  or untrusted (and where verification happens).

### Pass 4: unsafe constructs

Goal: ensure no `unsafe` Rust has crept into production sources.

- Run `python3 scripts/validate_all.py --full` and confirm the
  `unsafe_surface_report` test passes with zero matches.
- Grep `crates/*/src/` for `unsafe ` (with the trailing space) and
  confirm the only matches are doc comments or string literals.
- Confirm no new third-party dependency has been added without an
  entry in `CHANGELOG.md` under the workspace-dependency
  allow-list (currently `serde`, `serde_json`).
- Exit: zero unsafe surface, allow-list intact.

### Pass 5: unbounded-recursion / DoS

Goal: identify control-flow paths that could exhaust memory or
CPU before failing cleanly.

- Read `MAX_CALL_DEPTH` in `interp_exec.rs` and confirm the
  `depth: usize` counter is correctly threaded through
  `call_function` and `eval_expr`.
- For each AST walker (`crates/ori-compiler/src/{cst,parser,
  effect_propagate,type_infer,exhaustive}.rs`), confirm a
  termination invariant or depth cap. Today only the interpreter
  has an explicit cap; static-analysis walkers rely on a finite,
  parsed CST.
- For each JSON envelope decoder, confirm a size cap before
  deserialisation. `serde_json` has none; callers must enforce.
- Exit: every walker terminates or has a cap; every decoder has
  a size limit.

### Pass 6: panic surface

Goal: surface places where a malicious input could panic the
process.

- Confirm `scripts/validate_all.py` enforces the "no `.unwrap()`
  / `.expect()` / `panic!`" rule and is wired into CI.
- For each `?` operator in production sources, confirm the
  surrounding function returns a structured error, not a panic.
- Exit: no panic on parser-accepted adversarial input.

### Pass 7: agent-loop integrity

Goal: ensure the Patch IR apply path cannot be coerced into an
out-of-vocabulary edit by a malicious model.

- Read `crates/ori-compiler/src/patch.rs` and `patch_apply.rs`.
  Confirm `KNOWN_OPERATIONS` is closed and `op` values outside
  it are rejected with `P1002`.
- Confirm partial-apply correctness: fatal errors abort the patch;
  stale targets only skip the affected op.
- Exit: closed vocabulary holds; partial-apply semantics match
  RFC 0003.

## Severity rubric

Severity reflects the cost of exploitation, not the difficulty of
the fix. A "Low" finding may take a multi-quarter rewrite; a
"Critical" finding may be a one-line patch.

- **Critical.** A remote unauthenticated adversary can read or
  write data outside the principal's authority, or can execute
  arbitrary code on a consumer machine. Zero open at the time of
  writing.
- **High.** A local adversary who can author a package or model
  output can read or write data outside their authority, or can
  cause a consumer's build to produce a wrong artefact
  silently. Zero open at the time of writing.
- **Medium.** A local adversary can defeat a stated security
  property under conditions that are plausible in production
  (e.g. collision on a 64-bit FNV digest). Several open; see the
  findings table.
- **Low.** A stated security property has a known limitation that
  is documented and gated, or the affected surface is not yet
  reachable in a production deployment.
- **Info.** A discovery about the codebase that is not a security
  problem but is worth recording because future work depends on
  it.

## Findings

| Id          | Severity | Area                    | Title                                                                 | Status   | Link                                            |
| ----------- | -------- | ----------------------- | --------------------------------------------------------------------- | -------- | ----------------------------------------------- |
| M-INT-001   | Medium   | Interpreter             | `MAX_CALL_DEPTH = 64` is a frame-count cap, not a memory cap          | Open     | [#m-int-001](#m-int-001)                        |
| M-PUB-001   | Medium   | Publish / provenance    | `ori publish` receipt uses FNV-1a, not a cryptographic signature      | WontFix  | [#m-pub-001](#m-pub-001)                        |
| M-INT-002   | Medium   | Interpreter             | `capability_runtime::guard_call` not threaded into `interp_exec`      | Fixed    | [#m-int-002](#m-int-002)                        |
| M-CAP-001   | Medium   | Capability runtime      | Capability tokens have no revocation propagation across registries    | Open     | [#m-cap-001](#m-cap-001)                        |
| L-NET-001   | Low      | Network / publish       | `ori serve --dry-run` is the only HTTP mode; production HTTPS missing | Open     | [#l-net-001](#l-net-001)                        |
| L-STD-001   | Low      | Stdlib                  | `mail`, `process`, `websocket` bodies are still declarations          | Open     | [#l-std-001](#l-std-001)                        |
| I-DEP-001   | Info     | Dependency policy       | Bootstrap dep allow-list blocks production-grade features             | Open     | [#i-dep-001](#i-dep-001)                        |
| I-AUD-001   | Info     | Audit infrastructure    | No machine-readable audit envelope yet                                | Open     | [#i-aud-001](#i-aud-001)                        |

## Notes on individual findings

### M-INT-001

**`MAX_CALL_DEPTH = 64` is a frame-count cap, not a memory cap.**

The constant at `crates/ori-compiler/src/interp_exec.rs`:37 bounds
nested function frames before returning `R0005`. The cap is
conservative because every frame carries a cloned `Env` plus the
recursive `eval_expr` stack underneath.

It does not guarantee against memory exhaustion: a program that
constructs deeply nested data within one frame (a long `List` or
`Record` chain) can allocate without bound. The interpreter has
no heap-accounting layer.

Fix path: an `AllocBudget` on `ExecState` accumulating `Value`
byte cost, aborting with a new `R0006` at a configurable ceiling.
Tracked against future Layer 2; out of scope this wave.

### M-PUB-001

**`ori publish` receipt uses FNV-1a, not a cryptographic signature.**

`SIGNATURE_PREFIX = "fnv1a:"` at
`crates/ori-pkg/src/publish.rs`:54 marks the bootstrap stub. The
"signature" is an FNV-1a digest of `manifest_hash || version || name`
and trivially forgeable. The production path will use an asymmetric
algorithm (working assumption: `ed25519`); the choice is deferred
to a sub-RFC after RFC 0004 unlocks the dependency budget.

Status is "WontFix" because the stub is by-design pre-1.0: the
contract surface (`PublishReceipt`, the `signature` field, the
`verified: false` semantics for unrecognised prefixes) ships now;
the cryptographic content is deferred to a superseding RFC.

### M-INT-002

**`capability_runtime::guard_call` is now threaded into `interp_exec`.**

This was open through wave 3. The L1-INTERP work in the current
wave introduced `ExecState` at
`crates/ori-compiler/src/interp_exec.rs`:221 and routes
`eval_call` through `guard_call_at` whenever the runtime carries a
non-empty `CapabilitySet`. A denial is propagated as an `EvalFlow::Err`
carrying the guard's stable `CAP####` code, so the call site is
visible in the runtime error.

Verification:

```text
$ grep -n "guard_call_at" crates/ori-compiler/src/interp_exec.rs
579:    // `capability_runtime::guard_call_at`. A denial halts the evaluation
590:            match guard_call_at(&ctx, state.clock_now) {
```

Status is "Fixed" effective this wave. The closing artefact is the
fact that the guard is the only path: there is no longer a code
path that dispatches a user-defined function while ignoring an
attached capability set.

### M-CAP-001

**Capability tokens have no revocation propagation across registries.**

`RevocationList` at
`crates/ori-compiler/src/capability_runtime.rs`:348 makes
revocation enforceable inside a single process — a guard call
consults the list before allowing the effect. That is the
L2-CAP-EXT local deliverable, shipping this wave.

The cross-registry case is not shipping. If principal P holds a
token redeemable at registries A and B, revoking at A does not
propagate to B. Propagation is a distributed-systems problem (cache
TTL, gossip, push-vs-pull, signed revocation envelopes) that needs
its own RFC and depends on RFC 0004 accepting TLS and async-I/O.

### L-NET-001

**`ori serve --dry-run` is the only HTTP mode; production HTTPS
requires a TLS dep currently outside the policy.**

`cmd_serve` at `crates/ori-cli/src/main.rs`:2096 emits a route table
and exits. There is no socket open, no listener bound. This is safe
by vacuity: nothing can be exploited because nothing is listening.

The follow-up is gated on RFC 0004 plus its `rustls` allow-list
entry. Until then, the "production HTTPS" surface is documented as
absent in this audit and in [`SECURITY.md`](../../SECURITY.md);
consumers should not assume `ori serve` is production-ready.

### L-STD-001

**Stdlib bodies for `mail`, `process`, `websocket` are still
declarations.**

The relevant files:

- `stdlib/std/mail.ori` (single declaration: `fn send(message:
  Message) -> Result[Unit, MailError] uses mail.send`).
- `stdlib/std/process.ori` (process-spawning declarations).
- `stdlib/std/websocket.ori` (websocket declarations).

A consumer who imports one of these modules and calls the
declared function will get an unimplemented-builtin error at
runtime, not a successful effect. This is safe by vacuity for the
same reason as L-NET-001: the call cannot reach the host.

When the implementations land, the effect surface gets larger and
new findings should be opened against each one (untrusted-input
validation on `Message.body_text`, command-line escaping on
`process.spawn`, frame-size limits on `websocket`).

### I-DEP-001

**Bootstrap dep allow-list blocks production-grade features.**

`scripts/validate_all.py` enforces the `serde` + `serde_json`
allow-list. Correct for waves 1-4; now blocking HTTPS, native AOT
codegen, and real async I/O.

RFC 0004 proposes the relaxation. Filed as "Info" because it is the
absence of a feature, not a vulnerability — included so the audit
reader sees the full picture.

### I-AUD-001

**No machine-readable audit envelope yet.**

This document is human-readable Markdown. A long-term goal is for
the audit itself to ship as an `ori.audit.v1` envelope so that
automated tooling can ingest it (track resolution times, compute
mean-time-to-fix per severity, etc.). Open as an Info finding so
the work is on the books.

## Process for adding a finding

1. Pick the next free id in the relevant area:
   `M-INT-002`, `L-CAP-003`, etc. Area codes in use: `INT`
   (interpreter), `PUB` (publish), `CAP` (capability), `NET`
   (network), `STD` (stdlib), `DEP` (dependency), `AUD` (audit).
2. Add a row to the findings table and an `### Id` section below.
3. Cite the file path and line range that the finding refers to.
4. Set status to `Open` unless the finding is being filed
   alongside its fix.
5. If severity is `Medium` or higher, link the issue to an RFC if
   one is required to fix it.
6. Re-run `python3 scripts/validate_all.py --full` to confirm the
   markdown still parses under the static gate (the gate
   currently checks Markdown file presence and code-block
   balance; the audit must remain valid Markdown).
