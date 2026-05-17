# Chapter 08: Patches and agents

**What you'll build.** A hand-written Patch IR document, validated and applied
in dry-run mode. You will observe how `P0**` codes flag structural problems
in the patch itself, how `P1010` skips stale-target ops without aborting the
whole patch, and how `ori agent map --budget N` returns progressively richer
symbol tables at 500, 2000, and 4000-byte budgets.

**Time:** ~10 minutes.

## 1. The Patch IR contract

Patch IR is the structural-edit format Orison uses instead of textual diffs.
The schema is [`schemas/patch.schema.json`](../../schemas/patch.schema.json);
the runtime contract is documented at
[`docs/compiler/PATCH_IR.md`](../compiler/PATCH_IR.md). The high-level shape
is:

```json
{
  "schema": "ori.patch.v1",
  "intent": "Plain-English description of the change.",
  "operations": [
    { "op": "<op_name>", "target": "<symbol or node id>", ...args... }
  ],
  "tests": {
    "run":      ["<test symbol id>"],
    "expected": "pass"
  }
}
```

The op kinds the bootstrap recognises today are:

| Op                  | Args                                         | Meaning                                              |
|---------------------|----------------------------------------------|------------------------------------------------------|
| `insert_node`       | `target`, `position` (`before:<id>` / `after:<id>`), `text` | Insert a new top-level item.                      |
| `replace_node`      | `target`, `text`                             | Replace the item at `target`.                       |
| `delete_node`       | `target`                                     | Delete the item at `target`.                        |
| `change_signature`  | `target`, `signature`                        | Rewrite the signature line of a function or service. |
| `insert_match_arm`  | `target`, `pattern`, `body`                  | Add a missing variant arm in a `match`.             |

Every `target` is a stable symbol id (`sym:<module>.<name>`) or a stable node
id (`node:<module>.<kind>.<name>.<discriminant>`). Both ids survive
whitespace-only and comment-only edits because they are derived from a
structural fingerprint.

## 2. Set up

Use the canonical storefront example as the source under test — its symbol ids
are stable across edits and its module structure is real-world-shaped.

```bash
cd /path/to/Orison           # the repository root
```

```bash
ls examples/demo_store/contracts/
# agent_patch_add_product_search.json   change_manifest_checkout.json
```

`agent_patch_add_product_search.json` ships as a reference Patch IR.

## 3. Validate a known-good patch

```bash
ori patch check --json examples/demo_store/contracts/agent_patch_add_product_search.json
```

```json
{ "schema": "ori.patch_check.v1", "valid": true, "diagnostics": [] }
```

`ori patch check` validates the JSON shape only — it does not load any
`.ori` source. The result is byte-stable so CI gates can hash it.

## 4. Validate a known-bad patch

Save the minimal failing example:

```bash
cat > /tmp/bad_patch.json <<'EOF'
{
  "schema":     "ori.patch.v1",
  "operations": []
}
EOF
ori patch check --json /tmp/bad_patch.json
echo "exit=$?"
```

```json
{
  "schema": "ori.patch_check.v1",
  "valid":  false,
  "diagnostics": [
    {
      "schema": "ori.diagnostic.v1",
      "id":     "P0004",
      "level":  "error",
      "message": "patch file must include a non-empty intent",
      "expected": ["intent: non-empty string"],
      "found":    [],
      "agent":   { "summary": "Describe the intended change so humans and agents can audit it.",
                   "docs":    ["doc:patch.intent"] }
    },
    {
      "schema": "ori.diagnostic.v1",
      "id":     "P0002",
      "level":  "error",
      "message": "patch file must include at least one operation",
      "expected": ["operations: non-empty array"],
      "found":    [],
      "agent":   { "summary": "Add at least one structural operation for the patch to do useful work.",
                   "docs":    ["doc:patch.operations"] }
    },
    {
      "schema": "ori.diagnostic.v1",
      "id":     "P0100",
      "level":  "warning",
      "message": "patch file does not declare validation tests",
      "expected": ["tests.run"],
      "found":    [],
      "agent":   { "summary": "Declare the tests expected to validate this patch.",
                   "docs":    ["doc:patch.tests"] }
    }
  ]
}
```

```
exit=1
```

The `P0***` family covers structural problems with the Patch IR document
itself. The full list:

| ID     | Trigger                                                          |
|--------|------------------------------------------------------------------|
| `P0000` | `schema` field missing or wrong value.                          |
| `P0001` | An operation has an unknown `op` name.                          |
| `P0002` | `operations` is missing or empty.                               |
| `P0003` | A required argument for an operation is missing.                |
| `P0004` | `intent` is missing or empty.                                   |
| `P0100` | `tests.run` is absent (warning, not error).                     |

## 5. Write your first Patch IR by hand

Save the following as `~/my_patch.json`. It changes one thing: it adds a new
function `search` after `list_active` in the catalog module.

```bash
cat > ~/my_patch.json <<'EOF'
{
  "schema": "ori.patch.v1",
  "intent": "Add a product search function to the catalog module.",
  "operations": [
    {
      "op":       "insert_node",
      "target":   "sym:demo_store.catalog",
      "position": "after:sym:demo_store.catalog.list_active",
      "text":     "fn search(query: Str) -> Result[List[Product], CatalogError] uses db.read:\n  return Ok([])\n"
    }
  ],
  "tests": {
    "run":      ["sym:demo_store.tests.store_smoke.test_get_product_not_found_returns_err"],
    "expected": "pass"
  }
}
EOF
```

Validate:

```bash
ori patch check --json ~/my_patch.json
```

```json
{ "schema": "ori.patch_check.v1", "valid": true, "diagnostics": [] }
```

## 6. Dry-run the patch

`ori patch dry-run` applies the patch in memory and returns the resulting
source. No disk is written. The envelope is `ori.patch_apply.v1` with
`dry_run: true`.

```bash
ori patch dry-run --json ~/my_patch.json examples/demo_store/src/catalog.ori | jq '{applied, dry_run, operations_attempted, operations_applied}'
```

```json
{
  "applied":              true,
  "dry_run":              true,
  "operations_attempted": 1,
  "operations_applied":   1
}
```

The envelope also carries `before` and `after` strings with the full source.
Show the diff manually:

```bash
ori patch dry-run --json ~/my_patch.json examples/demo_store/src/catalog.ori \
  | jq -r .after \
  | diff -u examples/demo_store/src/catalog.ori -
```

The resulting patch insertion is appended near the targeted `list_active`
function.

## 7. Observe `P1010` skipping a stale-target op

The reference `agent_patch_add_product_search.json` carries three operations:

1. Insert a `search` function after `list_active` in `demo_store.catalog`.
2. Add a `SearchUnavailable` arm to `demo_store.catalog.CatalogError`.
3. Insert a search form before the `ProductList` view in `demo_store.ui`.

When you dry-run the patch against `catalog.ori` alone, operation 3 cannot
resolve because `demo_store.ui.ProductList` lives in a different file:

```bash
ori patch dry-run --json examples/demo_store/contracts/agent_patch_add_product_search.json examples/demo_store/src/catalog.ori \
  | jq '{applied, operations_attempted, operations_applied, diagnostics: (.diagnostics | map({id, message}))}'
```

```json
{
  "applied":              true,
  "operations_attempted": 3,
  "operations_applied":   2,
  "diagnostics": [
    {
      "id":      "P1010",
      "message": "operation 2 references unknown node id `sym:demo_store.ui.ProductList`"
    }
  ]
}
```

Observe four things:

1. `applied: true` — the patch did partial-apply: two of three ops landed.
2. `operations_applied: 2 < operations_attempted: 3` — the count reports the
   partial result.
3. The skipped op surfaces as `P1010` with the offending id in the message.
4. The exit code is `0` for partial-apply success; structural failures (the
   `P10**` fatal family — `P1000` invalid op, `P1001` invalid arg, `P1002`
   conflicting edit, `P1003` malformed source) abort the entire patch and
   produce exit `1`.

The partial-apply behaviour is what makes Patch IR robust to whitespace edits
and unrelated changes: an agent can ship a multi-file patch and the runtime
will land what it can while reporting exactly which targets need attention.

## 8. Apply the patch for real

`ori patch apply` writes the result to disk. Always run `dry-run` first.

```bash
cp examples/demo_store/src/catalog.ori /tmp/catalog.backup.ori
ori patch apply --json ~/my_patch.json examples/demo_store/src/catalog.ori \
  | jq '{applied, operations_applied}'
```

```json
{ "applied": true, "operations_applied": 1 }
```

Verify the file changed, then re-check:

```bash
ori check --json examples/demo_store/src/catalog.ori; echo "exit=$?"
```

```
exit=0
```

Restore the backup before moving on:

```bash
mv /tmp/catalog.backup.ori examples/demo_store/src/catalog.ori
```

## 9. `ori agent map` at three budgets

The agent map is the single best primer for an LLM working on a module. Run
the same command at three budget levels to see how the compiler honours the
ceiling:

```bash
ori agent map --budget 500  --json examples/demo_store/src/api.ori \
  | jq '{budget, used_estimate, truncated, symbol_count: (.symbols | length)}'
```

```json
{ "budget": 500, "used_estimate": 476, "truncated": true, "symbol_count": 4 }
```

```bash
ori agent map --budget 2000 --json examples/demo_store/src/api.ori \
  | jq '{budget, used_estimate, truncated, symbol_count: (.symbols | length)}'
```

```json
{ "budget": 2000, "used_estimate": 820, "truncated": false, "symbol_count": 6 }
```

```bash
ori agent map --budget 4000 --json examples/demo_store/src/api.ori \
  | jq '{budget, used_estimate, truncated, symbol_count: (.symbols | length)}'
```

```json
{ "budget": 4000, "used_estimate": 820, "truncated": false, "symbol_count": 6 }
```

Observations:

- At budget **500**, four of six symbols fit (`used_estimate: 476`) and the
  envelope reports `truncated: true`. The agent knows the view is partial.
- At budget **2000**, all six symbols fit (`used_estimate: 820`) and
  `truncated` flips to `false`. The agent has the complete module view.
- At budget **4000**, the result is identical to 2000 — there is nothing more
  to add. The compiler does not pad output to fill the budget; it reports the
  honest cost.

The symbol order is deterministic (alphabetic within each kind), so the
budget-truncated subset is stable across runs.

## 10. `ori agent diagnose`

For a higher-level "is this module healthy" check, use:

```bash
ori agent diagnose --json examples/demo_store/src/api.ori | jq .
```

```json
{
  "schema":         "ori.agent_diagnose.v1",
  "module":         "demo_store.api",
  "overall_status": "ok",
  "errors":         0,
  "warnings":       0,
  "diagnostics":    [],
  "top_repair_candidates": []
}
```

`top_repair_candidates` becomes non-empty when there are diagnostics with
attached `fixes` — agents loop on this field to choose what to repair next.

## 11. `ori patch explain`

For a one-line summary of a patch (useful in PR descriptions):

```bash
ori patch explain --json examples/demo_store/contracts/agent_patch_add_product_search.json | jq .
```

```json
{
  "schema":          "ori.patch_explain.v1",
  "intent":          "Add product search to the catalog module, extend CatalogError with a SearchUnavailable arm, and insert a search box into the ProductList view.",
  "operation_count": 3,
  "advice":          "Run `ori patch dry-run` to preview the resulting source before applying."
}
```

## Common errors

| Diagnostic | Cause | Fix |
|------------|-------|-----|
| `P0000` — patch schema invalid | Missing or wrong `schema` field. | Use `"schema": "ori.patch.v1"`. |
| `P0001` — unknown op | An operation specifies an op name the runtime does not know. | Use one of the documented ops (`insert_node`, `replace_node`, `delete_node`, `change_signature`, `insert_match_arm`). |
| `P0002` — empty operations | `operations` is missing or empty. | Add at least one operation. |
| `P0003` — missing required arg | An op is missing one of its required arguments (e.g. `text` on `insert_node`). | Add the required argument. The schema lists them per op. |
| `P0004` — empty intent | `intent` is missing or an empty string. | Set a human-readable intent string. |
| `P0100` — no tests declared (warning) | `tests.run` is missing. | Add a list of test symbol ids to validate the patch. |
| `P1000` — structural apply failure | The runtime could not parse the source after applying. | Inspect the `before`/`after` strings and fix the malformed insertion. Whole patch aborts. |
| `P1001` — conflicting overlapping op | Two ops in the same patch target the same node in incompatible ways. | Split into two patches or merge the ops. Whole patch aborts. |
| `P1002` — source mutation race | The source was changed between dry-run and apply. | Re-run dry-run, re-resolve target ids, then re-apply. Whole patch aborts. |
| `P1003` — runtime cannot edit binary file | The `target` resolves outside the .ori CST. | Patch only `.ori` files. Whole patch aborts. |
| `P1010` — stale target id (per-op) | A symbol or node id in the operation does not exist in the current CST. | Re-resolve the id; or accept the partial-apply behaviour and patch the rest. The other ops still land. |

## Recap

- Patch IR (`ori.patch.v1`) is the structural-edit format. Every op targets a
  stable `sym:` or `node:` id; ids survive whitespace edits.
- `ori patch check` validates the JSON shape. `P0**` errors block; `P0100`
  warns on missing `tests`.
- `ori patch dry-run` applies in memory and returns `before`/`after` source
  plus per-op diagnostics. `ori patch apply` writes to disk.
- `P1010` per-op stale-target ops are skipped (partial-apply succeeds);
  `P1000`–`P1003` structural failures abort the whole patch.
- `ori agent map --budget N` returns budget-bounded symbol tables with
  `truncated: true` when the budget bites. The compiler does not pad.
- `ori agent diagnose` summarises a module's health for an agent loop;
  `ori patch explain` summarises a patch for a PR description.

## Next

Continue with [chapter 09: Testing and benchmarks](./09-testing-and-benchmarks.md).
You will write a smoke test, exercise `ori coverage`, run `ori bench`, and
compare two benchmark runs with `scripts/compare_bench.py`.

For the long-form Patch IR contract see
[`docs/compiler/PATCH_IR.md`](../compiler/PATCH_IR.md); for the agent context
ABI see [`docs/compiler/AGENT_CONTEXT_ABI.md`](../compiler/AGENT_CONTEXT_ABI.md).
