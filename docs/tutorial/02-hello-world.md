# Chapter 02: Hello world

**What you'll build.** Your first `.ori` source file, type-checked, run through
the bootstrap interpreter, and inspected as a semantic capsule. By the end of
this chapter you will understand the four core JSON envelopes you will see
hundreds of times in the rest of the tutorial:
`ori.diagnostic.v1`, `ori.run.v1`, `ori.capsule.v1`, and `ori.agent_map.v1`.

**Time:** ~5 minutes.

## 1. Write `hello.ori`

Create a working directory anywhere outside the repository tree:

```bash
mkdir -p ~/orison-tutorial && cd ~/orison-tutorial
```

Then write the smallest legal Orison program:

```ori
module hello

fn main() -> Unit:
  return Unit
```

Save it as `hello.ori`. Two things to notice:

1. Every `.ori` file starts with `module <dotted.name>`. Missing this header
   triggers diagnostic `E0001` (see [chapter 03](./03-types.md) for more on the
   `E00**` family).
2. The function body of the bootstrap is line-oriented. The colon at the end of
   `fn main() -> Unit:` opens a body block; everything indented two spaces below
   belongs to it. The body parser handles `let`, `return`, `if`, and `match` —
   see [`docs/language/REFERENCE.md`](../language/REFERENCE.md) for the
   authoritative list.

## 2. Run `ori check`

```bash
ori check hello.ori
```

Output:

```
ok: hello
```

The text mode prints `ok: <module-name>` on success. Switch to the JSON envelope
when you want machine-readable output:

```bash
ori check --json hello.ori
echo "exit=$?"
```

```
exit=0
```

When there are no diagnostics the JSON envelope is empty, so stdout is empty and
the exit code is 0. Any diagnostics print one JSON object per line, separated by
`\n`, and the exit code becomes 1.

## 3. Run the program

The bootstrap ships a tree-walking interpreter. Use `ori run` with the
`--json` flag for the structured envelope:

```bash
ori run --json hello.ori
```

```json
{
  "entry":  "main",
  "module": "hello",
  "schema": "ori.run.v1",
  "status": "ok",
  "value":  "Unit"
}
```

Field summary:

| Field    | Meaning                                                       |
|----------|---------------------------------------------------------------|
| `schema` | Always `ori.run.v1` for this command.                         |
| `module` | The dotted module name from your `module` header.             |
| `entry`  | The function actually invoked. Default `main`; overridable.   |
| `status` | `"ok"` if the function returned. Errors surface as `status`.  |
| `value`  | The textual form of the returned value (here `Unit`).         |

You can pick a different entry point by name:

```bash
ori run --entry main --json hello.ori
```

Same envelope as before.

## 4. Inspect the semantic capsule

`ori capsule` returns the per-module symbol table the agent ABI is built on.
Every chapter from here on uses it to introspect new types and functions.

```bash
ori capsule --json hello.ori | jq .
```

```json
{
  "schema": "ori.capsule.v1",
  "module": "hello",
  "path":   "hello.ori",
  "hash":   "fnv1a:d694ffda8410db16",
  "exports": [
    {
      "id":        "sym:hello.main",
      "kind":      "function",
      "name":      "main",
      "signature": "fn main() -> Unit",
      "effects":   [],
      "calls":     [],
      "tests":     [],
      "summary":   "function `main` declared in this module."
    }
  ],
  "imports":    [],
  "invariants": [
    "No null values; use Option[T].",
    "No exceptions; use Result[T, E]."
  ],
  "agent": {
    "token_summary":         "Module hello with 1 exported symbols and 0 imports.",
    "recommended_context": ["sym:hello.main"]
  }
}
```

Field summary:

| Field        | Purpose                                                        |
|--------------|----------------------------------------------------------------|
| `hash`       | Source-derived stable fingerprint of the module.               |
| `exports[].id` | The stable symbol id `sym:<module>.<name>` referenced by Patch IR. |
| `exports[].effects` | Effects declared via `uses` (empty here — `main` is pure). |
| `invariants` | Per-module promises the bootstrap enforces (no `null`, no `throw`). |
| `agent.token_summary` | One-line natural-language summary suited for agent prompts. |

The stable symbol id `sym:hello.main` is what you would reference in a Patch IR
operation later (chapter 08).

## 5. Inspect the agent map

The agent map is a budget-bounded view of the same data tuned for low-context
LLM inference. Even on a one-function file it is a useful smoke test that all
five compiler subsystems agree on the same view.

```bash
ori agent map --budget 500 --json hello.ori | jq .
```

```json
{
  "schema":           "ori.agent_map.v1",
  "module":           "hello",
  "budget":           500,
  "used_estimate":    148,
  "truncated":        false,
  "imports":          [],
  "symbols": [
    {
      "id":        "mod:hello",
      "kind":      "module",
      "name":      "hello",
      "signature": "module hello",
      "effects":   []
    },
    {
      "id":        "sym:hello.main",
      "kind":      "function",
      "name":      "main",
      "signature": "fn main() -> Unit",
      "effects":   []
    }
  ],
  "diagnostic_count": 0,
  "error_count":      0,
  "warning_count":    0
}
```

Field summary:

| Field           | Meaning                                                                 |
|-----------------|-------------------------------------------------------------------------|
| `budget`        | The byte budget you requested via `--budget`.                          |
| `used_estimate` | Approximate serialized bytes consumed by `symbols`.                    |
| `truncated`     | `true` if the budget forced the compiler to drop symbols.              |
| `imports`       | Modules imported by this file (empty for `hello`).                     |
| `symbols`       | Ordered list of module-scope items.                                    |

Try shrinking the budget so far that even one symbol does not fit:

```bash
ori agent map --budget 50 --json hello.ori | jq .truncated
```

```
true
```

The compiler always returns *some* symbols when possible, and reports
`truncated: true` so callers know the view is incomplete. Chapter 08 walks
through this property again with the multi-module storefront.

## 6. Try to break it

A good way to learn the diagnostic model is to inject a known error. Open
`hello.ori` and change the function body:

```ori
module hello

fn main() -> Unit:
  let user = null
  return Unit
```

Run check again:

```bash
ori check --json hello.ori | jq .
```

```json
{
  "schema":  "ori.diagnostic.v1",
  "id":      "E0100",
  "level":   "error",
  "message": "`null` is not part of Orison; use Option[T]",
  "span": {
    "file":  "hello.ori",
    "start": { "line": 4, "column": 14 },
    "end":   { "line": 4, "column": 18 }
  },
  "expected": ["Option[T]", "None", "Some(value)"],
  "found":    ["null"],
  "fixes": [
    {
      "kind":        "replace_null",
      "description": "Replace `null` with `None` or an explicit Option value.",
      "confidence":  0.82
    }
  ],
  "agent": {
    "summary":         "Replace null with Option semantics.",
    "minimal_context": [],
    "docs":            ["doc:types.option"]
  }
}
```

Exit code 1. The Orison promise is that you never silently get `null` — it is a
parse-time error with a structured fix attachment. Revert the file before
moving on:

```ori
module hello

fn main() -> Unit:
  return Unit
```

## 7. Try a slightly bigger program

Type signatures and effects show up in every envelope. Replace `hello.ori` with:

```ori
module greeter

fn greet(name: Str) -> Str:
  return name

fn main() -> Unit uses log:
  return Unit
```

The `uses log` clause declares an effect. The known-effect list is documented in
[`docs/language/REFERENCE.md`](../language/REFERENCE.md); `log` is not in that
list, so the compiler will emit `W0401` warning you that the name is unknown.
That's fine for this exercise — it shows you how warnings surface alongside the
capsule data.

```bash
ori check --json greeter.ori | jq .
```

```json
{
  "schema":  "ori.diagnostic.v1",
  "id":      "W0401",
  "level":   "warning",
  "message": "unknown effect or capability `log`",
  "span":    { "file": "greeter.ori", "start": { "line": 6, "column": 1 }, "end": { "line": 6, "column": 8 } },
  "symbol":  { "id": "sym:greeter.main" },
  "expected": ["fs.read", "fs.write", "net.inbound", "net.outbound", "db.read",
               "db.write", "env.read", "process.spawn", "crypto", "time",
               "random", "ui", "gpu", "unsafe", "http", "db", "fs", "net",
               "auth", "mail.send"],
  "found":   ["log"],
  "fixes":   [],
  "agent":   { "summary": "Declare a capability or use a known effect name.",
               "minimal_context": ["sym:greeter.main"],
               "docs": ["doc:effects.known-effects"] }
}
```

Note the exit code: warnings do not cause `ori check` to fail. Only `E*`-level
diagnostics push the exit code to 1.

```bash
ori check --json greeter.ori >/dev/null; echo "exit=$?"
```

```
exit=0
```

Fix the warning by using a real effect name (`time` is the closest to "log"):

```ori
module greeter

fn greet(name: Str) -> Str:
  return name

fn main() -> Unit uses time:
  return Unit
```

`ori capsule --json greeter.ori` now shows the function's declared effect:

```bash
ori capsule --json greeter.ori | jq '.exports[] | select(.name=="main")'
```

```json
{
  "id":        "sym:greeter.main",
  "kind":      "function",
  "name":      "main",
  "signature": "fn main() -> Unit uses time",
  "effects":   ["time"],
  "calls":     [],
  "tests":     [],
  "summary":   "function `main` declared in this module."
}
```

Chapter 04 covers effects in depth: where they propagate, how they are checked
against `ori.toml`, and how `E0410` / `E0420` surface them.

## Common errors

| Diagnostic | Cause | Fix |
|------------|-------|-----|
| `E0001` — missing module declaration | First non-comment line is not `module <name>`. | Add `module <name>` at the top. |
| `E0002` — module declaration requires a dotted module name | Trailing `.`, empty segment, or non-identifier. | Use `module app.name`; identifiers only. |
| `E0100` — `null` is not part of Orison | Literal `null` appeared outside a string or comment. | Use `Option[T]` (`None` / `Some(value)`). |
| `E0101` — exceptions are not part of Orison | Literal `throw` keyword appeared. | Return `Result[T, E]` (`Ok` / `Err`). |
| `W0401` — unknown effect or capability | `uses` clause names something not in the built-in list. | Use a known effect or declare `capability Name` (uppercase) in the module. |
| `W9001` — tabs are discouraged | Indentation contains tab characters. | Run `ori fmt <file>` to normalise. |

## Recap

- A minimal Orison module is `module <name>` plus a function. `Unit` is the
  zero-information return type, equivalent to `()` elsewhere.
- `ori check --json` emits one JSON diagnostic per line; success is silent and
  exits 0; warnings (`W*`) do not fail; errors (`E*`) do.
- `ori run --json` produces an `ori.run.v1` envelope; you read the result from
  the `value` field.
- `ori capsule --json` produces the per-module semantic capsule; symbol ids of
  the form `sym:<module>.<name>` are stable and used everywhere downstream.
- `ori agent map --json --budget N` returns a budget-bounded subset of the same
  view, tagged with `truncated: true` when the budget bites.

## Next

Continue with [chapter 03: Types](./03-types.md) to learn how newtypes, records,
variants, `Option`, and `Result` keep distinct domains from blurring into each
other. The chapter also walks you through `E0100` and a triggerable `E0540`
non-exhaustive-match scenario.

For the authoritative reference on every form the bootstrap parses, see
[`docs/language/REFERENCE.md`](../language/REFERENCE.md).
