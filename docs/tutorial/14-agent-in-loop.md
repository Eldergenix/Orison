# Chapter 14: Agent in the loop

**What you'll build.** A complete walkthrough of the agent-facing CLI
surface — `ori agent map`, `ori agent diagnose`, `ori agent telemetry`,
and `ori patch apply --dry-run` — paired with a tiny shell script that
simulates a 3-iteration model-in-loop edit session. The session emits
`ori.model_loop_telemetry.v1` at the end so any agent harness can ingest
and aggregate the result.

**Time:** ~10 minutes.

## 1. The agent contract

Every agent-facing subcommand returns a single JSON envelope whose
`schema` field names the contract. The four envelopes you will use in
this chapter:

| Envelope                            | Producer                          | Purpose                                              |
|-------------------------------------|-----------------------------------|------------------------------------------------------|
| `ori.agent_map.v1`                  | `ori agent map`                   | Budget-bounded symbol table for context windows.     |
| `ori.agent_diagnose.v1`             | `ori agent diagnose`              | Status + top repair candidates.                      |
| `ori.patch_apply.v1`                | `ori patch apply --dry-run`       | Preview the post-patch source without writing disk.  |
| `ori.model_loop_telemetry.v1`       | `ori agent telemetry --in <path>` | Recompute and validate session totals.               |

The full schema set lives under [`schemas/`](../../schemas). The
contracts the model edits to satisfy — Patch IR
([`schemas/patch.schema.json`](../../schemas/patch.schema.json)) and
the diagnostic envelope
([`schemas/diagnostic.schema.json`](../../schemas/diagnostic.schema.json))
— were introduced in chapters 02 and 08.

## 2. `ori agent map` — bounded symbol tables

Agents read the whole repository once, but their context window is
finite. `ori agent map --budget N` returns a symbol table that fits
within `N` bytes of estimated token usage, truncating the longest
entries first if the budget is too tight.

```bash
cargo run --release -p ori --quiet -- agent map --budget 2000 --json examples/demo_store/src/api.ori \
  | jq '{module, used_estimate, truncated, symbol_count: (.symbols | length), imports}'
```

```json
{
  "module":        "demo_store.api",
  "used_estimate": 873,
  "truncated":     false,
  "symbol_count":  5,
  "imports":       ["demo_store.domain", "demo_store.catalog", "demo_store.cart"]
}
```

Drop the budget to 300 bytes and the map truncates:

```bash
cargo run --release -p ori --quiet -- agent map --budget 300 --json examples/demo_store/src/api.ori \
  | jq '{used_estimate, truncated}'
```

The `used_estimate` is the agent's source-of-truth for how many bytes
of context the map consumed; the agent should treat the value as an
opaque accounting unit and never recompute it.

## 3. `ori agent diagnose` — status + repair candidates

```bash
cargo run --release -p ori --quiet -- agent diagnose --json examples/demo_store/src/api.ori
```

```json
{
  "schema":                "ori.agent_diagnose.v1",
  "module":                "demo_store.api",
  "overall_status":        "ok",
  "errors":                0,
  "warnings":              0,
  "diagnostics":           [],
  "top_repair_candidates": []
}
```

When a module has unresolved diagnostics, `top_repair_candidates`
returns the highest-confidence fixes from the per-diagnostic `fixes`
array. Confidence is a float in `[0.0, 1.0]`; an agent harness that
applies fixes automatically should set a threshold (e.g. only apply at
`>= 0.85`) and surface the rest to a human.

## 4. `ori patch apply --dry-run` — preview without disk writes

Patch IR is structural-edit JSON (chapter 08). The contract is in
[`schemas/patch.schema.json`](../../schemas/patch.schema.json). The
canonical model-edit cycle is: model emits Patch IR ->
`ori patch check` validates shape -> `ori patch apply --dry-run`
captures `before` / `after` -> `ori check` on `after` text -> commit
(without `--dry-run`) or feed diagnostics back to the model.

The dry-run envelope is `ori.patch_apply.v1`:

```bash
cargo run --release -p ori --quiet -- patch apply --dry-run --json \
  examples/demo_store/contracts/agent_patch_add_product_search.json \
  examples/demo_store/src/catalog.ori \
  | jq '{schema, applied, dry_run, operations_attempted, operations_applied, diagnostic_count: (.diagnostics | length)}'
```

```json
{
  "schema":               "ori.patch_apply.v1",
  "applied":              true,
  "dry_run":              true,
  "operations_attempted": 3,
  "operations_applied":   2,
  "diagnostic_count":     1
}
```

Two of three ops landed on the catalog file; the third (`P1010`)
targeted a different file and was skipped without aborting the patch.
The harness should record both the diagnostic and the
`operations_applied < operations_attempted` mismatch as a follow-up
todo for the model.

## 5. `ori agent telemetry` — closing the loop

After the edit session ends, the harness writes a single
`ori.model_loop_telemetry.v1` document and feeds it to
`ori agent telemetry`. The CLI recomputes the `totals` from the
`iterations` array — callers cannot smuggle inconsistent aggregates —
and emits the canonical envelope. The schema is at
[`schemas/model-loop-telemetry.schema.json`](../../schemas/model-loop-telemetry.schema.json).

A single iteration record carries `iteration`, `started_at`,
`completed_at`, `edits_proposed`, `edits_accepted`, `edits_rejected`,
`tokens_in`, `tokens_out`, `budget_remaining`, and the
`diagnostics_before` / `diagnostics_after` deltas. See the example
block in [`schemas/model-loop-telemetry.schema.json`](../../schemas/model-loop-telemetry.schema.json)
for the canonical shape. Pass `--in -` to read from stdin; if the
totals are inconsistent, the CLI rejects the input.

## 6. A 3-iteration edit loop in one shell script

Save the following as `/tmp/edit_loop.sh`. At each iteration it reads
the agent map, asks `ori agent diagnose` for the status (in a real
harness, forwards to the model), validates and dry-runs the patch the
model would produce, then emits one iteration record. The final
`ori.model_loop_telemetry.v1` envelope is recomputed by
`ori agent telemetry --in -`.

```bash
cat > /tmp/edit_loop.sh <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
SOURCE=examples/demo_store/src/catalog.ori
PATCH=examples/demo_store/contracts/agent_patch_add_product_search.json
SESSION_ID="edit-$(date +%s)"
MODEL_ID="${ORI_MODEL_ID:-claude-opus-4-7}"
BUDGET=10000
ITER_FILE=$(mktemp)
trap "rm -f $ITER_FILE" EXIT
echo "[]" > "$ITER_FILE"
WALL_START=0
START=0
for i in 1 2 3; do
  END=$(( START + 1000 ))
  # 1. Map the symbol surface (budgeted at 2000 estimated bytes).
  cargo run --release -p ori --quiet -- agent map --budget 2000 --json "$SOURCE" > /dev/null
  # 2. Diagnose (currently zero diagnostics; would be the agent's prompt).
  DIAG_BEFORE=$(cargo run --release -p ori --quiet -- agent diagnose --json "$SOURCE" | jq '.errors + .warnings')
  # 3. Validate + dry-run the patch the model would have produced.
  cargo run --release -p ori --quiet -- patch check --json "$PATCH" > /dev/null
  APPLY=$(cargo run --release -p ori --quiet -- patch apply --dry-run --json "$PATCH" "$SOURCE")
  ATTEMPTED=$(echo "$APPLY" | jq '.operations_attempted')
  APPLIED=$(echo "$APPLY"   | jq '.operations_applied')
  REJECTED=$(( ATTEMPTED - APPLIED ))
  DIAG_AFTER=$(echo "$APPLY" | jq '.diagnostics | length')
  TOKENS_IN=$(( 400 + i * 100 ))
  TOKENS_OUT=$(( 80 + i * 20 ))
  BUDGET=$(( BUDGET - TOKENS_IN - TOKENS_OUT ))
  ITER=$(jq -n \
    --argjson i        "$i" \
    --argjson sa       "$START" \
    --argjson en       "$END" \
    --argjson prop     "$ATTEMPTED" \
    --argjson acc      "$APPLIED" \
    --argjson rej      "$REJECTED" \
    --argjson tin      "$TOKENS_IN" \
    --argjson tout     "$TOKENS_OUT" \
    --argjson budget   "$BUDGET" \
    --argjson dbefore  "$DIAG_BEFORE" \
    --argjson dafter   "$DIAG_AFTER" \
    '{ iteration: $i, started_at: $sa, completed_at: $en,
       edits_proposed: $prop, edits_accepted: $acc, edits_rejected: $rej,
       tokens_in: $tin, tokens_out: $tout, budget_remaining: $budget,
       diagnostics_before: $dbefore, diagnostics_after: $dafter }')
  jq --argjson e "$ITER" '. + [$e]' "$ITER_FILE" > "$ITER_FILE.tmp" && mv "$ITER_FILE.tmp" "$ITER_FILE"
  START=$END
done
WALL_MS=$START
# Aggregate totals across the iterations.
TOTALS=$(jq '{
  iterations: length,
  wall_ms: (last.completed_at - first.started_at),
  edits_proposed: (map(.edits_proposed) | add),
  edits_accepted: (map(.edits_accepted) | add),
  edits_rejected: (map(.edits_rejected) | add),
  tokens_in: (map(.tokens_in) | add),
  tokens_out: (map(.tokens_out) | add),
  diagnostics_resolved: (first.diagnostics_before - last.diagnostics_after)
}' "$ITER_FILE")
jq -n \
  --arg sid "$SESSION_ID" \
  --arg mid "$MODEL_ID" \
  --slurpfile iters "$ITER_FILE" \
  --argjson totals "$TOTALS" \
  '{ schema: "ori.model_loop_telemetry.v1",
     session_id: $sid, model_id: $mid,
     iterations: $iters[0], totals: $totals }' \
  | cargo run --release -p ori --quiet -- agent telemetry --in - --json
EOF
chmod +x /tmp/edit_loop.sh
bash /tmp/edit_loop.sh | jq '{totals, iter_count: (.iterations | length)}'
```

Expected output shape:

```json
{
  "totals": {
    "iterations":           3,
    "wall_ms":              3000,
    "edits_proposed":       9,
    "edits_accepted":       6,
    "edits_rejected":       3,
    "tokens_in":            1800,
    "tokens_out":           360,
    "diagnostics_resolved": -1
  },
  "iter_count": 3
}
```

The `tokens_in` / `tokens_out` numbers are fake (a real harness gets
them from the model SDK). `diagnostics_resolved` is negative because
the `P1010` per-op skip from the cross-file patch counts as a new
diagnostic — exactly the kind of regression telemetry the harness is
supposed to expose.

## 7. Patch-IR-driven repair

The harness composes four envelopes per iteration:
`ori agent diagnose` -> picks the top repair candidate ->
asks the model to produce a Patch IR document ->
`ori patch check` -> `ori patch apply --dry-run` -> re-run
`ori agent diagnose` against the `after` text -> commit with
`ori patch apply` (without `--dry-run`) -> record the iteration in
`ori.model_loop_telemetry.v1`. The harness never edits source by
string substitution; every edit flows through Patch IR, which is the
property that makes the loop auditable.

## Common errors

| Symptom | Cause | Fix |
|---------|-------|-----|
| `ori agent map` truncates aggressively | Budget too low for the symbol set. | Raise `--budget`; the per-symbol cost is roughly `signature.len() * 2`. |
| `ori patch apply --dry-run` returns `applied: true` with `operations_applied < operations_attempted` | Cross-file patch; per-op stale ids surface as `P1010`. | Expected; the other ops still land. |
| `ori agent telemetry` rejects the input | The `totals` don't match the `iterations` sum. | Let the CLI recompute; do not pre-fill `totals`. |
| The 3-iteration script prints `diagnostics_resolved: 0` | The source was already clean. | Point at a broken source file to see a positive value. |

## Recap

- The agent-facing surface is four envelopes:
  `ori.agent_map.v1`, `ori.agent_diagnose.v1`, `ori.patch_apply.v1`,
  and `ori.model_loop_telemetry.v1`. All four are pinned schemas.
- `ori agent map --budget N` is the contract for fitting a symbol table
  inside a fixed token budget; the CLI returns `truncated: true` when
  it had to drop entries.
- The repair loop never edits source by string substitution: every
  edit is a Patch IR document validated by `ori patch check` and
  previewed by `ori patch apply --dry-run`.
- `ori agent telemetry --in` recomputes the session totals from the
  per-iteration array, so a harness cannot smuggle inconsistent
  aggregates.

## Next

You have finished the extended tutorial set. For the in-tree reference
material see [`CHEATSHEET.md`](./CHEATSHEET.md); for the long-form
intended language see
[`docs/language/SPECIFICATION.md`](../language/SPECIFICATION.md); for
the road map see [`docs/ROADMAP.md`](../ROADMAP.md).
