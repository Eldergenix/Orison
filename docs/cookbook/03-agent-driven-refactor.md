# Recipe 03: Agent-driven rename across a workspace

**Goal.** Use `ori agent map` to get a budget-bounded view of a module,
then construct a Patch IR document that renames a symbol, validate it
with `ori patch check`, preview it with `ori patch apply --dry-run`,
and finally commit it with `ori patch apply`. The same workflow is what
an LLM-driven agent harness runs every iteration.

**Prerequisites.** A working `ori` binary and familiarity with
[tutorial 08](../tutorial/08-patches-and-agents.md). `jq` for inspecting
JSON.

**Time:** ~20 minutes.

## 1. Set up a workspace to refactor

```bash
mkdir -p ledger/src && cd ledger
cat > ori.toml <<'TOML'
[package]
name        = "ledger"
version     = "0.1.0"
edition     = "2027.1"
description = "Small ledger module to refactor."
license     = "Apache-2.0"

[capabilities]
declared = ["db.read", "db.write"]
TOML
```

Save this as `src/ledger.ori`. We are going to rename `tally` to
`compute_balance`.

```ori
module ledger.core

type Amount wraps Int
type Entry = {
  amount: Amount,
  note: Str
}

fn tally(entries: List[Entry]) -> Amount uses db.read:
  return Amount { value: 0 }

fn record(entry: Entry) -> Result[Unit, Str] uses db.write:
  return Ok(Unit)

fn report() -> Amount uses db.read:
  return tally([])
```

The interesting structural fact: `report` calls `tally`. A naive `sed`
rename would risk modifying string literals or comments; the Patch IR
flow walks the parsed structure.

Check:

```bash
ori check --json src/ledger.ori; echo "exit=$?"
```

Expected: empty stdout, `exit=0`.

## 2. `ori agent map` — get a bounded symbol table

The agent surface starts with `ori agent map`. It returns a JSON
envelope that names every symbol in the module, its kind, signature,
and effect set, capped at the byte budget you choose. The cap matters
because LLM context windows are finite; the bootstrap returns the
densest information first and truncates the longest entries when it
runs out of room.

```bash
ori agent map --budget 2000 --json src/ledger.ori | jq '{module, used_estimate, truncated, symbol_count: (.symbols | length)}'
```

```json
{
  "module":        "ledger.core",
  "used_estimate": 480,
  "truncated":     false,
  "symbol_count":  5
}
```

Drop the budget to 200 bytes and `truncated` flips to `true`. The
`used_estimate` is the agent's source-of-truth for how much context
the map consumed; treat it as opaque accounting and do not recompute
it client-side.

To see what `tally` looks like in detail:

```bash
ori agent explain tally --json src/ledger.ori | jq .
```

The reply is an `ori.symbol_card.v1` envelope with the signature,
effects, callers (`report` will appear here), and a small docstring
slot. This is the data an agent uses to plan the rename: it can see
exactly which other symbols mention `tally` before it writes any
patch.

## 3. Write the Patch IR by hand

A Patch IR document is a JSON object that conforms to
`schemas/patch.schema.json`. For this rename we need two operations:

- `change_signature` on `sym:ledger.core.tally` — rewrites the
  signature line.
- `replace_node` on the call site inside `report` — rewrites the call
  expression.

Save this as `rename.json`:

```json
{
  "schema":  "ori.patch.v1",
  "intent":  "Rename tally to compute_balance for clarity.",
  "operations": [
    {
      "op":        "change_signature",
      "target":    "sym:ledger.core.tally",
      "signature": "fn compute_balance(entries: List[Entry]) -> Amount uses db.read"
    },
    {
      "op":     "replace_node",
      "target": "sym:ledger.core.report",
      "text":   "fn report() -> Amount uses db.read:\n  return compute_balance([])\n"
    }
  ],
  "tests": {
    "run":      ["sym:ledger.tests.test_compute_balance"],
    "expected": "pass"
  }
}
```

Validate the shape only:

```bash
ori patch check --json rename.json | jq .
```

```json
{ "schema": "ori.patch_check.v1", "valid": true, "diagnostics": [] }
```

`ori patch check` does not load any `.ori` source. It validates that
the document is a well-formed Patch IR: required fields present, op
names known, each operation's required args present, intent non-empty,
tests declared. The byte-stable response means CI gates can hash it.

## 4. Dry-run the apply

Now run the patch against the source file with `--dry-run`. The
toolchain captures the before/after text, runs every op in order,
and refuses to write disk.

```bash
ori patch apply --dry-run --json rename.json src/ledger.ori \
  | jq '{applied, dry_run, operations_attempted, operations_applied, diagnostics_count: (.diagnostics | length)}'
```

```json
{
  "applied":              true,
  "dry_run":              true,
  "operations_attempted": 2,
  "operations_applied":   2,
  "diagnostics_count":    0
}
```

Both ops landed. The envelope includes the full `after` text under the
`result.after` field. Pipe it through `jq -r '.result.after'` to see
the post-rename source:

```bash
ori patch apply --dry-run --json rename.json src/ledger.ori \
  | jq -r '.result.after'
```

You will see `tally` rewritten to `compute_balance` in both the
signature and the call site, with whitespace preserved.

## 5. Check the post-patch source for new diagnostics

The agent loop is: emit patch -> check shape -> dry-run -> type-check
the after -> commit if green, otherwise feed diagnostics back to the
model.

```bash
ori patch apply --dry-run --json rename.json src/ledger.ori \
  | jq -r '.result.after' > /tmp/after.ori
ori check --json /tmp/after.ori; echo "exit=$?"
```

Expected: empty stdout, `exit=0`. If there were a leftover call site
to `tally`, the type checker would emit `W0531` ("call target `tally`
is not a known function in this module") and the agent would loop.

## 6. Commit the rename

When you are confident, drop `--dry-run`:

```bash
ori patch apply --json rename.json src/ledger.ori
```

The CLI writes the new text to `src/ledger.ori` atomically. Verify:

```bash
ori check --json src/ledger.ori; echo "exit=$?"
```

Re-run `ori agent map` to confirm the new symbol table:

```bash
ori agent map --budget 2000 --json src/ledger.ori | jq '.symbols[].name'
```

`tally` is gone; `compute_balance` is in its place.

## 7. Scaling up to a workspace

Patch IR `target` ids are stable across files —
`sym:ledger.core.tally` resolves wherever `ledger.core` lives. To
rename across a workspace: run `ori agent map --budget N` on every
file (or `ori agent symbols --changed` for the dirty set); find
every file that mentions the old symbol; generate one Patch IR
document with one `change_signature` op for the definition and one
`replace_node` per call site; `ori patch check` for shape; dry-run
per file collecting diagnostics; apply for real only when every
dry-run is clean. Every `target` references a structural id, not a
line number, so whitespace edits, comment changes, and reorderings
cannot move the target out from under the patch.
