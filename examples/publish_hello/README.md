# Publish Hello

A one-module library used by the publishing tutorial:

- `src/greeter.ori` declares `Greeting` (newtype) and two pure functions:
  `greet` and `greet_loud`.

Walk through the publish lifecycle in
[`docs/tutorial/13-publishing.md`](../../docs/tutorial/13-publishing.md).

## Quick check

```bash
cargo run --release -p ori --quiet -- check --json examples/publish_hello/src/greeter.ori
cargo run --release -p ori --quiet -- package check --json examples/publish_hello
cargo run --release -p ori --quiet -- sbom --format ori-native --json examples/publish_hello
```
