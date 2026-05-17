# Chapter 04: Effects and capabilities

**What you'll build.** A small two-function module that declares effects,
contrasted with an `ori.toml` package policy. You will see how `uses` clauses
attach to functions, how the policy diff surfaces in the `ori.capability.v1`
envelope, and how diagnostic codes `W0401` (unknown effect), `E0410` (effect
not in the package policy), and `E0420` (transitive effect missing from a
caller) describe the three failure modes.

**Time:** ~10 minutes.

## 1. What an effect is

An *effect* is a named permission a function asks for at its signature. Orison's
core promise is that the set of effects a package can use is bounded at
compile time and visible in the manifest. The compiler knows the following
well-known effect names (defined in
[`crates/ori-compiler/src/effects.rs`](../../crates/ori-compiler/src/effects.rs)):

```
fs.read   fs.write   net.inbound  net.outbound
db.read   db.write   env.read     process.spawn
crypto    time       random       ui
gpu       unsafe     http         db
fs        net        auth         mail.send
```

Any identifier that begins with a lowercase letter and is *not* in that list is
flagged with `W0401`. Any identifier that begins with an uppercase letter is
treated as a user-declared capability (declare it with `capability Name` in the
same module). Effects on a function are listed after a `uses` keyword in the
signature:

```ori
fn list_active() -> List[Product] uses db.read:
  return []
```

## 2. Start a tiny module

```bash
mkdir -p ~/orison-tutorial/effects && cd ~/orison-tutorial/effects
```

Save the following as `service_demo.ori`:

```ori
module shop.svc

type Product = {
  sku:  Str,
  name: Str
}

fn list_active() -> List[Product] uses db.read:
  return []

fn write_audit() -> Unit uses fs.write:
  return Unit
```

Check it:

```bash
ori check --json service_demo.ori; echo "exit=$?"
```

```
exit=0
```

Capsule:

```bash
ori capsule --json service_demo.ori | jq '.exports[] | {id, effects}'
```

```json
{ "id": "sym:shop.svc.Product",     "effects": [] }
{ "id": "sym:shop.svc.list_active", "effects": ["db.read"] }
{ "id": "sym:shop.svc.write_audit", "effects": ["fs.write"] }
```

Every effect declared in a `uses` clause shows up in the symbol's `effects`
array. Effects propagate through every downstream view: the agent map, the
capability manifest, the openapi report, the wasm component manifest.

## 3. The capability manifest

`ori capability --json` collects the effects of every symbol and returns the
union, sorted, with the symbol ids that own each one:

```bash
ori capability --json service_demo.ori | jq .
```

```json
{
  "schema": "ori.capability.v1",
  "module": "shop.svc",
  "effects": [
    { "name": "db.read",  "uses": ["sym:shop.svc.list_active"] },
    { "name": "fs.write", "uses": ["sym:shop.svc.write_audit"] }
  ],
  "policy": {
    "declared":   [],
    "undeclared": ["db.read", "fs.write"],
    "unused":     []
  }
}
```

When `--policy` is omitted the `declared` list is empty and every observed
effect is reported as `undeclared`. That is the right default for an
unconfigured module: the compiler tells you exactly which capabilities you
would need to declare.

## 4. Add a package policy

Real Orison projects declare their capabilities in `ori.toml`. Create one in
the same directory:

```bash
cat > ori.toml <<'EOF'
[package]
name        = "shop_service"
version     = "0.1.0"
edition     = "2027.1"
description = "Effects tutorial."
license     = "Apache-2.0"

[capabilities]
declared = ["db.read", "fs.write"]
EOF
```

The capability manifest envelope reads the policy from `--policy` (a CLI flag,
useful for one-off checks and CI gates). Re-run with the same effect set:

```bash
ori capability --policy "db.read,fs.write" --json service_demo.ori | jq .policy
```

```json
{
  "declared":   ["db.read", "fs.write"],
  "undeclared": [],
  "unused":     []
}
```

The policy now matches the observed effects exactly. Both lists are empty,
which is the desired steady state.

## 5. Add an effect that exceeds the policy

Add a third function that uses `net.outbound`:

```ori
module shop.svc

type Product = {
  sku:  Str,
  name: Str
}

fn list_active() -> List[Product] uses db.read:
  return []

fn write_audit() -> Unit uses fs.write:
  return Unit

fn fetch_remote() -> Bytes uses net.outbound:
  return []
```

Re-check the capability diff with the same policy:

```bash
ori capability --policy "db.read,fs.write" --json service_demo.ori | jq .policy
```

```json
{
  "declared":   ["db.read", "fs.write"],
  "undeclared": ["net.outbound"],
  "unused":     []
}
```

`net.outbound` shows up in `undeclared`. The compiler's effect checker emits an
`E0410` diagnostic for each offending symbol when the policy is fed in via the
library. The diagnostic envelope looks like this:

```json
{
  "schema":  "ori.diagnostic.v1",
  "id":      "E0410",
  "level":   "error",
  "message": "effect `net.outbound` is used by `sym:shop.svc.fetch_remote` but is not in the package capability policy",
  "symbol":  { "id": "sym:shop.svc.fetch_remote" },
  "expected": ["declare `net.outbound` in [capabilities].declared"],
  "found":    ["net.outbound"],
  "agent":   { "summary": "Add the missing capability to ori.toml or remove the effect usage.",
               "docs":    ["doc:effects.policy"] }
}
```

You fix `E0410` in one of two ways:

1. **Add the effect to the policy.** Update `ori.toml`:

   ```toml
   [capabilities]
   declared = ["db.read", "fs.write", "net.outbound"]
   ```

   Re-run with the wider policy:

   ```bash
   ori capability --policy "db.read,fs.write,net.outbound" --json service_demo.ori | jq .policy
   ```

   ```json
   { "declared": ["db.read", "fs.write", "net.outbound"],
     "undeclared": [], "unused": [] }
   ```

2. **Remove the effect from the function.** If `fetch_remote` should not be
   making outbound calls, change the signature back to something the policy
   permits or refactor the call into a separate, explicitly-authorised module.

The capability manifest also reports `unused`: declarations in `ori.toml` that
no symbol actually exercises. They are not errors but they widen the trust
surface needlessly. Demonstrate this by declaring a capability nothing uses:

```bash
ori capability --policy "db.read,fs.write,net.outbound,crypto" --json service_demo.ori | jq .policy
```

```json
{
  "declared":   ["db.read", "fs.write", "net.outbound", "crypto"],
  "undeclared": [],
  "unused":     ["crypto"]
}
```

The package audit (`ori audit --json`) folds this into `AUD0002` for info-level
findings; we cover the audit envelope in [chapter 10](./10-shipping-the-demo-storefront.md).

## 6. Unknown effect names (`W0401`)

If you mistype an effect name, the compiler will warn — not error. Unknown
names are treated as user capabilities only if they start uppercase; lowercase
identifiers are almost certainly typos:

```ori
fn boot() -> Unit uses loging:
  return Unit
```

`ori check --json` (replace your module's `boot` if needed):

```bash
ori check --json service_demo.ori | jq 'select(.id=="W0401")'
```

```json
{
  "schema":  "ori.diagnostic.v1",
  "id":      "W0401",
  "level":   "warning",
  "message": "unknown effect or capability `loging`",
  "symbol":  { "id": "sym:shop.svc.boot" },
  "expected": ["fs.read", "fs.write", "net.inbound", "net.outbound", "db.read",
               "db.write", "env.read", "process.spawn", "crypto", "time",
               "random", "ui", "gpu", "unsafe", "http", "db", "fs", "net",
               "auth", "mail.send"],
  "found":   ["loging"],
  "agent":   { "summary": "Declare a capability or use a known effect name.",
               "minimal_context": ["sym:shop.svc.boot"],
               "docs": ["doc:effects.known-effects"] }
}
```

`W0401` does not change the exit code. Repair by either:

- Picking the right name (`logging` is not in the list; `time` is the closest
  near-match for "I want to record when something happened").
- Declaring a real user capability and naming it: `capability Logging` then
  `uses Logging`.

The bootstrap permits the second pattern but does not yet treat user
capabilities specially beyond name validation.

## 7. Transitive effect propagation (`E0420`)

If `caller` invokes `leaf` and `leaf` declares `fs.read`, then `caller` must
also declare `fs.read` (or a superset). The compiler's
[`effect_propagate.rs`](../../crates/ori-compiler/src/effect_propagate.rs)
pass walks the call graph and emits `E0420` per missing effect, attaching a
`change_signature` Patch IR fix that proposes the new signature line.

Minimal trigger source:

```ori
module shop.chain

fn leaf() -> Int uses fs.read:
  return 0

fn caller() -> Int:
  return leaf()
```

The propagation pass is exercised by `cargo test -p ori-compiler effect_propagate`.
The diagnostic envelope it produces has this shape:

```json
{
  "schema":  "ori.diagnostic.v1",
  "id":      "E0420",
  "level":   "error",
  "message": "function `caller` requires effect `fs.read` transitively through calls to `leaf` but does not declare it",
  "symbol":  { "id": "sym:shop.chain.caller" },
  "expected": ["fs.read"],
  "found":    [],
  "fixes": [
    {
      "kind":        "change_signature",
      "description": "Add `fs.read` to `caller`'s effect list.",
      "confidence":  0.9,
      "patch": {
        "schema": "ori.patch.v1",
        "intent": "Propagate effect `fs.read` from `leaf` to `caller`",
        "operations": [
          { "op":     "change_signature",
            "target": "sym:shop.chain.caller",
            "signature": "fn caller() -> Int uses fs.read" }
        ]
      }
    }
  ],
  "agent": { "summary": "Add the missing effect to the caller's signature.",
             "docs":    ["doc:effects.propagation"] }
}
```

The fix encodes the exact signature string the agent (or the LSP code action
handler) should write. As of the bootstrap CLI this pass is not yet wired into
`ori check`; the diagnostic surfaces through library callers and the
`ori-lsp` `textDocument/codeAction` handler. The wiring lands as part of M37b
once the body parser settles.

For now, you can verify the call-graph propagation contract by running:

```bash
cargo test -p ori-compiler effect_propagate -- --nocapture 2>&1 | tail -15
```

All tests in that module assert on the exact envelope shape shown above.

## 8. The full effect picture

Three diagnostic IDs cover the entire static effect story. Memorise them:

| ID      | Meaning                                                       | Where it shows up                                                |
|---------|---------------------------------------------------------------|------------------------------------------------------------------|
| `W0401` | The named effect is unknown to the compiler.                   | `ori check --json`                                              |
| `E0410` | A declared effect is not in the package's `[capabilities].declared`. | `ori capability --policy ...` (manifest); library effect_diagnostics. |
| `E0420` | A function transitively needs an effect it has not declared.   | Library `effect_propagate`; LSP code actions; wired into `ori check` in M37b. |

The `policy.undeclared` field of `ori.capability.v1` is the surface every CI
gate should script against. A package is policy-compliant iff
`policy.undeclared` is empty:

```bash
test "$(ori capability --policy "$(jq -r '.capabilities.declared | join(",")' ori.toml)" \
        --json service_demo.ori | jq '.policy.undeclared | length')" = 0
```

(Substitute your manifest path. Real CI scripts walk every `.ori` file under
`src/`.)

## Common errors

| Diagnostic | Cause | Fix |
|------------|-------|-----|
| `W0401` — unknown effect or capability | An effect name not in the bootstrap's known set was used in `uses`. | Use one of the listed effects, or declare a `capability Name` (uppercase) in the module and reference it by that name. |
| `E0410` — effect not in package policy | A declared effect on a symbol is missing from `[capabilities].declared`. | Either add the effect to `ori.toml` or remove the effect from the function. |
| `E0420` — effect required transitively | A caller forgot an effect that one of its callees needs. | Add the effect to the caller's `uses` clause. Apply the attached `change_signature` Patch IR for an automated fix. |
| `AUD0002` — declared but unused capability | A capability in `ori.toml` is not used by any symbol. | Remove the declaration to shrink the trust surface. Info-level only. |

## Recap

- Functions declare effects with `uses <name1>, <name2>` after the return type.
  The compiler knows ~20 built-in effect names; uppercase identifiers are
  treated as user-declared capabilities.
- `ori capability --policy a,b,c --json <file>` returns an `ori.capability.v1`
  envelope whose `policy.undeclared` and `policy.unused` lists are the entire
  policy contract.
- `W0401` is a warning for typos. `E0410` is the error for missing-from-policy
  declarations. `E0420` is the error for transitive holes in the call graph.
- The `change_signature` Patch IR fix attached to `E0420` lets an agent (or the
  LSP) apply the propagated signature without rewriting the file.

## Next

Continue with [chapter 05: Functions and services](./05-functions-and-services.md).
You will declare HTTP routes, generate an OpenAPI 3.1 document directly from
source, and watch the `service` keyword tie a set of routes to a single
capability budget.

For the long-form description of the effect model see
[`docs/language/EFFECTS_AND_CAPABILITIES.md`](../language/EFFECTS_AND_CAPABILITIES.md);
for the diagnostic envelope shape see
[`docs/compiler/DIAGNOSTICS.md`](../compiler/DIAGNOSTICS.md).
