# Chapter 09: Testing and benchmarks

**What you'll build.** A smoke test added to the demo storefront, exercised
through `ori test` and `ori coverage`. Then you will run `ori bench --json`
twice and feed the two reports into `scripts/compare_bench.py` to gate on
performance regression.

**Time:** ~5 minutes.

## 1. The shape of a test

A test in Orison today is just a function named `test_<something>` in a file
under a `tests/` directory. There is no special `#[test]` annotation. The
bootstrap discovery rule is:

- Any function whose symbol id is `sym:<...>.tests.<...>.test_<name>` is a
  candidate test.
- `ori test --json <root>` walks every `.ori` file under `<root>` and lists
  every test it finds.
- `ori test --changed --json <root>` only selects tests that mention symbols
  the agent has reported as changed (see `ori agent changed`); skipped tests
  are reported with a `reason` field.

The bootstrap does not yet *execute* tests directly — the runner contract
lands in M37e. Today `ori test` returns a deterministic selection list that a
shell driver can pipe into `ori run`.

## 2. Write a smoke test

Use the demo storefront as the working project:

```bash
cd /path/to/Orison
ls examples/demo_store/tests/
# store_smoke.ori
```

Open the existing `store_smoke.ori`. Add one more test at the bottom:

```ori
fn test_money_zero_is_usd_zero() -> Unit:
  let m = money_zero()
  return Unit
```

`money_zero` is the smart constructor on the `Money` record from
[`examples/demo_store/src/domain.ori`](../../examples/demo_store/src/domain.ori).
The test does not assert anything yet — the assertion DSL lands with the body
parser — but the function is recognised as a test and it exercises a real
import path.

Verify the parse is clean:

```bash
ori check --json examples/demo_store/tests/store_smoke.ori; echo "exit=$?"
```

```
exit=0
```

## 3. Discover tests with `ori test`

```bash
ori test --json examples/demo_store | jq .
```

```json
{
  "schema":      "ori.agent_tests.v1",
  "root":        ".",
  "total_tests": 5,
  "selected":    [],
  "skipped": [
    {
      "id":     "sym:demo_store.tests.store_smoke.test_cart_empty_total_is_zero",
      "name":   "test_cart_empty_total_is_zero",
      "file":   "examples/demo_store/tests/store_smoke.ori",
      "reason": "no changed symbol referenced"
    },
    {
      "id":     "sym:demo_store.tests.store_smoke.test_add_line_increases_total",
      "name":   "test_add_line_increases_total",
      "file":   "examples/demo_store/tests/store_smoke.ori",
      "reason": "no changed symbol referenced"
    },
    {
      "id":     "sym:demo_store.tests.store_smoke.test_invalid_cart_returns_err",
      "name":   "test_invalid_cart_returns_err",
      "file":   "examples/demo_store/tests/store_smoke.ori",
      "reason": "no changed symbol referenced"
    },
    {
      "id":     "sym:demo_store.tests.store_smoke.test_get_product_not_found_returns_err",
      "name":   "test_get_product_not_found_returns_err",
      "file":   "examples/demo_store/tests/store_smoke.ori",
      "reason": "no changed symbol referenced"
    },
    {
      "id":     "sym:demo_store.tests.store_smoke.test_money_zero_is_usd_zero",
      "name":   "test_money_zero_is_usd_zero",
      "file":   "examples/demo_store/tests/store_smoke.ori",
      "reason": "no changed symbol referenced"
    }
  ]
}
```

By default `ori test` runs in change-aware mode: with no changes detected,
every test is `skipped` with `reason: "no changed symbol referenced"`. The
`selected` array is the subset the runner should actually invoke; the
`skipped` array documents *why* each candidate was held back.

## 4. Select tests affected by a changed symbol

The `ori agent tests --affected --changed-name <name>` variant takes the names
of changed symbols explicitly. Use it after you have edited a real source file
to filter the test list:

```bash
ori agent tests --affected --changed-name fetch_product --json examples/demo_store \
  | jq '{total_tests, selected: (.selected | length), skipped: (.skipped | length)}'
```

```json
{ "total_tests": 5, "selected": 5, "skipped": 0 }
```

When a changed symbol matches every test's transitive context (or the
selection logic falls back to "run everything"), all tests are selected.
You can pass `--changed-name` multiple times to narrow the set.

## 5. Coverage

`ori coverage --json <root>` walks every `tests/*.ori` file and matches every
function name and symbol id it finds inside test bodies against the surface
of the project. The result is a list of *covered* and *uncovered* exported
functions, suitable for a CI gate.

```bash
ori coverage --json examples/demo_store \
  | jq '{total_functions, covered_count: (.covered | length), uncovered_count: (.uncovered | length)}'
```

```json
{
  "total_functions": 12,
  "covered_count":   5,
  "uncovered_count": 7
}
```

The `covered` array lists per-symbol detail:

```bash
ori coverage --json examples/demo_store | jq '.covered[]'
```

```json
{
  "id":                  "sym:demo_store.cart.add_line",
  "name":                "add_line",
  "kind":                "function",
  "tests_referencing": ["examples/demo_store/tests/store_smoke.ori"]
}
{
  "id":                  "sym:demo_store.cart.total",
  ...
}
```

A CI gate that requires every public function to be touched by at least one
test reads `.uncovered | length == 0`.

## 6. Run `ori bench` and capture a baseline

```bash
ori bench --samples 30 --json > /tmp/bench_baseline.json
```

`--samples` controls how many measurements per metric the benchmark loop
takes. The bootstrap ships 32 suites today, covering every measurement listed
in [`BENCHMARKS.md`](../../BENCHMARKS.md). The total runtime at `--samples 30`
is about three seconds on Apple Silicon.

Inspect the envelope:

```bash
jq '{schema, generated_at, suite_count: (.suites | length)}' /tmp/bench_baseline.json
```

```json
{
  "schema":       "ori.benchmark.v1",
  "generated_at": "@unix:1778993096",
  "suite_count":  32
}
```

Every suite carries one or more `metrics`, each with `mean`, `p50`, `p95`,
`max`, `min`, and the number of `samples`:

```bash
jq '.suites[0]' /tmp/bench_baseline.json
```

```json
{
  "name": "cold_check_latency",
  "metrics": [
    {
      "key":     "check_small_ns",
      "unit":    "ns",
      "samples": 30,
      "mean":    4030.0,
      "p50":     2200.0,
      "p95":     6400.0,
      "max":     7200.0,
      "min":     1800.0
    }
  ]
}
```

## 7. Run a second `ori bench` and compare

```bash
ori bench --samples 30 --json > /tmp/bench_current.json
```

Now compare the two runs with the regression gate:

```bash
python3.13 scripts/compare_bench.py \
  --baseline /tmp/bench_baseline.json \
  --current  /tmp/bench_current.json
```

The script prints a Markdown table to stdout. The default regression threshold
is **20% on p50**: any metric whose `p50` is more than 20% above the baseline
flips the row's `Status` column to `regression` and the script exits 1.

A typical run on the same hardware looks like this (truncated):

```
# Bench regression report (threshold: ±20.0% on p50)

| Suite                          | Metric             | Baseline p50 | Current p50 | Δ%    | Status |
|--------------------------------|--------------------|--------------|-------------|-------|--------|
| agent_map_budget_levels        | agent_map_budget_2000_ns | 21,083 | 19,667 | -6.72% | ok |
| agent_map_token_density        | agent_map_medium_ns | 24,500 | 20,750 | -15.31% | ok |
| capability_manifest_latency    | capability_medium_ns | 92,500 | 21,000 | -77.30% | improvement |
| capsule_generation_latency     | capsule_medium_ns | 37,500 | 25,709 | -31.44% | improvement |
| cold_check_latency             | check_small_ns | 4,458 | 5,042 | +13.10% | ok |
| ...                            | ...                | ...    | ...    | ...    | ...    |
```

Statuses:

| Status         | Meaning                                                              |
|----------------|----------------------------------------------------------------------|
| `ok`           | Within ±20% of the baseline. Exit 0.                                 |
| `improvement`  | More than 20% below the baseline. Exit 0 (improvements never block). |
| `regression`   | More than 20% above the baseline. Exit 1 (the gate fails).           |

Override the threshold with `--threshold <pct>`. Write the table to a file
with `--markdown <path>`:

```bash
python3.13 scripts/compare_bench.py \
  --baseline /tmp/bench_baseline.json \
  --current  /tmp/bench_current.json \
  --threshold 25 \
  --markdown /tmp/bench_report.md
```

## 8. Use it in CI

A typical CI job lays out as:

```yaml
- name: Benchmark baseline
  run: |
    cargo build --release -p ori
    target/release/ori bench --samples 100 --json > bench_pr.json

- name: Compare against main
  run: |
    git show main:BENCHMARKS.results.json > bench_main.json
    python3.13 scripts/compare_bench.py \
      --baseline bench_main.json \
      --current  bench_pr.json \
      --threshold 20 \
      --markdown $GITHUB_STEP_SUMMARY
```

The repository's own committed `BENCHMARKS.results.json` is the
reference baseline. Bumping it on intentional improvements is part of the
release checklist.

## 9. A note on benchmark hygiene

The bootstrap's benchmark numbers are deterministic across runs *on the same
hardware*. They are not portable across CPUs, OS versions, or thermal states.
Treat the comparison as a same-machine regression gate, not as a portable
performance claim. The README's benchmark table is captured on Apple Silicon
(M-series) under steady state; numbers on other targets will differ.

## Common errors

| Symptom | Likely cause | Fix |
|--------|--------------|-----|
| `total_tests: 0` from `ori test` | No file matches `tests/**/*.ori` under the requested root, or every function is missing the `test_` prefix. | Place tests under `tests/` and prefix functions with `test_`. |
| Every test in `skipped` with `reason: "no changed symbol referenced"` | Default `ori test` is change-aware. | Pass `--changed-name <symbol>` to force selection, or wait until you have edits with a real `ori agent changed` baseline. |
| `compare_bench: file not found: ...` | One of `--baseline` / `--current` paths does not exist. | Re-run `ori bench --json > <path>` first; check spelling. |
| `compare_bench: is not an ori.benchmark.v1 document` | The file is empty or malformed JSON. | Confirm the file is exactly the stdout of `ori bench --json`; do not append other text. |
| `Status: regression` | A metric's p50 is more than `--threshold` percent above baseline. | Investigate the offending suite. Either fix the regression or update the baseline if the regression is intentional and approved. |
| `coverage_count == 0` despite tests existing | Tests do not mention any exported symbol by name. | Add a real `let x = some_fn(...)` line inside the test body so the coverage scanner can match. |

## Recap

- Tests are functions named `test_<something>` in `.ori` files under a
  `tests/` directory. The bootstrap discovers them via `ori test --json`.
- `ori test` is change-aware by default; `ori agent tests --affected --changed-name`
  lets you force selection by symbol name.
- `ori coverage --json` reports per-function coverage from the symbol-id
  references found in test bodies.
- `ori bench --json` emits an `ori.benchmark.v1` envelope with 32 suites,
  every metric carrying `mean`, `p50`, `p95`, `min`, `max`, `samples`.
- `scripts/compare_bench.py` compares two bench runs and gates on a default
  `±20%` p50 threshold, exiting 1 on regression and 0 on improvement.

## Next

Continue with [chapter 10: Shipping the demo storefront](./10-shipping-the-demo-storefront.md).
You will walk every `.ori` file in `examples/demo_store/src/`, run the full
`check / capsule / openapi / ui / capability / wasm / run` chain on each, then
dry-run the canonical agent patch end-to-end.

For the long-form benchmark methodology see
[`BENCHMARKS.md`](../../BENCHMARKS.md); for the test runner roadmap see
[`docs/ROADMAP.md`](../ROADMAP.md).
