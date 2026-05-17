# Effects and Capabilities

Effects are part of Orison's type-level security and agent-context model.

## Effect syntax

```ori
fn load(path: Path) -> Result[Str, FsErr] uses fs.read:
  return fs.read_text(path)
```

## Built-in effects

```text
fs.read
fs.write
net.inbound
net.outbound
db.read
db.write
env.read
process.spawn
crypto
time
random
ui
gpu
unsafe
```

## Capabilities

Capabilities bundle effects with constraints.

```ori
capability StripeApi:
  net.outbound = ["api.stripe.com"]
  secrets = ["STRIPE_API_KEY"]
```

## Propagation

A function calling another function must either:

- declare the called function's effects;
- satisfy them via a capability;
- handle them through an explicitly passed context.

## Agent value

Effects reduce token consumption. An agent can inspect a symbol card to know whether a function touches the network, database, filesystem, UI, or unsafe code without reading the full body.
