# Recipe 04: Capability tokens — issue, delegate, attenuate, revoke

**Goal.** Walk the four operations that every capability token undergoes
during its lifetime, using `ori capability check --dry-run` to simulate
each one. By the end you will know how the `CAP****` diagnostic family
maps to denial reasons, and you will have a reproducible script that
exercises every transition.

**Prerequisites.** A working `ori` binary and familiarity with
[tutorial 04](../tutorial/04-effects.md). The capability runtime is
documented in `crates/ori-compiler/src/capability_runtime.rs` and the
threat model section of `docs/security/THREAT_MODEL.md`.

**Time:** ~20 minutes.

## 1. What a capability token is

A capability token is the runtime artefact that lets a principal
perform an effect. The compile-time `uses` clause says *which*
effects a function needs; the runtime token says *whether* the
calling principal is authorised. Both checks must agree for the
call to proceed.

A token (`CapabilityToken` in `capability_runtime.rs`) has three
fields: `effect` (the effect name), `issued_to` (the principal id),
`expires_at` (optional Unix seconds). A principal holds a
`CapabilitySet`: a map of effect names to tokens plus a denial
allowlist. The guard (`guard_call_at`) returns one of:

| Code     | Meaning                                                       |
|----------|---------------------------------------------------------------|
| `CAP0001` | No token in the set for the requested effect.                |
| `CAP0002` | Token exists but expired.                                    |
| `CAP0003` | Token exists but `issued_to` does not match the principal.   |
| `CAP0004` | Effect is on the denial list, even though a token exists.    |
| `allowed` | Present, unexpired, owned, not denied.                       |

## 2. Set up a module to guard

Save this as `src/billing.ori`:

```ori
module billing.svc

type InvoiceId wraps Str

service Billing uses db.read, db.write

fn get_invoice(id: InvoiceId) -> Result[InvoiceId, Str] uses db.read:
  return Ok(id)

fn post_invoice(id: InvoiceId) -> Result[InvoiceId, Str] uses db.write:
  return Ok(id)

fn delete_invoice(id: InvoiceId) -> Result[Unit, Str] uses db.write:
  return Ok(Unit)
```

Check:

```bash
ori check --json src/billing.ori; echo "exit=$?"
```

Empty stdout, exit 0. The module declares three routes with split
effects: one read, two writes.

## 3. Issue: alice gets db.read

The first lifecycle step is *issue*. A capability authority (in
production: the package owner or a designated operator; in the
bootstrap: whoever runs the CLI) decides which principal gets which
effect. To simulate alice with `db.read` only:

```bash
ori capability check --dry-run \
  --module src/billing.ori \
  --principal alice \
  --has db.read
```

The envelope (`ori.capability_runtime.v1`) returns one outcome per
guarded call:

```json
{
  "schema": "ori.capability_runtime.v1",
  "outcomes": [
    {"index": 0, "outcome": {"outcome": "allowed"}},
    {"index": 1, "outcome": {"outcome": "denied",
      "code": "CAP0001",
      "reason": "principal `alice` is missing a capability token for effect `db.write`"}},
    {"index": 2, "outcome": {"outcome": "denied",
      "code": "CAP0001",
      "reason": "principal `alice` is missing a capability token for effect `db.write`"}}
  ]
}
```

Alice can call `get_invoice` (read) but not the two write routes.
`CAP0001` is the missing-token denial; the reason string names the
specific effect she lacks.

## 4. Delegate: bob inherits from alice

A delegation chain (`DelegationChainToken` in the v2 capability
runtime) lets a holder hand a subset of their authority to another
principal. In the bootstrap simulation, delegation is modelled by
issuing tokens with matching `issued_to` fields. To simulate alice
delegating `db.read` to bob:

```bash
ori capability check --dry-run \
  --module src/billing.ori \
  --principal bob \
  --has db.read
```

Bob now sees the same outcome shape as alice did. The delegation
chain preserves alice's attenuation: bob cannot exercise anything
alice could not. The chain check happens inside
`guard_call_with_chain`; the bootstrap refuses to mint a child
token with an effect not present in the parent's set.

## 5. Attenuate: carol gets read-only, time-bounded

Attenuation narrows a token. Two ways to narrow:

- **Effect set**: hand out fewer effects than you hold.
- **Expiration**: set `expires_at` to a near-future Unix time.

The CLI simulation models the effect-set form directly. Carol gets a
read-only token (the same shape as alice's), but in production she
would also have an `expires_at` set on the token record:

```bash
ori capability check --dry-run \
  --module src/billing.ori \
  --principal carol \
  --has db.read
```

If the runtime later evaluates carol's token after `expires_at`, the
outcome is `CAP0002`:

```json
{
  "outcome": "denied",
  "code":    "CAP0002",
  "reason":  "capability token for effect `db.read` issued to `carol` expired at <unix_ts>"
}
```

`CAP0002` is one of the two "still in your set but no longer valid"
denials. The other is `CAP0003`, which fires when the token's
`issued_to` does not match the caller — that is the wire-tap or
token-theft signal.

## 6. Revoke: bob loses his token

Revocation is the only path that destroys a token. The runtime keeps
a `RevocationList`; before allowing a call, the guard checks both
that the token is present and that the principal is not on the list.
A revoked principal sees `CAP0004` even if their token set is
otherwise intact:

```json
{
  "outcome": "denied",
  "code":    "CAP0004",
  "reason":  "effect `db.read` is denied by policy for principal `bob`"
}
```

Model revocation in the simulation by omitting the effect from
`--has`; every route returns `CAP0001` for bob. The distinction
matters operationally: `CAP0001` is a routine missing-permission
case, while `CAP0004` indicates an active security event (token
exists but the principal is on the deny list).

## 7. Reproducible script

Put the four steps in a shell script for documentation and CI use:

```bash
#!/usr/bin/env bash
set -euo pipefail

MOD=src/billing.ori
for PRINCIPAL in alice bob carol; do
  for HAS in "" "db.read" "db.read,db.write"; do
    echo "=== principal=$PRINCIPAL has=[$HAS] ==="
    ori capability check --dry-run \
      --module "$MOD" \
      --principal "$PRINCIPAL" \
      --has "$HAS" \
      | jq '.outcomes[] | {index, code: .outcome.code, outcome: .outcome.outcome}'
  done
done
```

This produces a stable matrix of (principal, capability-set, route) ->
outcome rows. Diff it across PRs to catch any drift in your effect
plumbing.

## 8. What the bootstrap does not yet ship

The bootstrap implements `guard_call`, `guard_call_at`, and
`guard_call_with_audit`. The full v2 surface (`DelegationChainToken`
persistence, on-disk `RevocationList`, audit-entry stream sink)
lands in milestone M35. Until then, the CLI is a complete and
accurate model of the policy check; only persistence and the audit
log target differ. The codes (`CAP0001`–`CAP0005`), the envelope
(`ori.capability_runtime.v1`), and call semantics are tier-1 stable
from 1.0.
