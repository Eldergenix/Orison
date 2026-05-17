# Orison Demo Walkthrough

This document walks a developer through the Orison bootstrap toolchain
using the canonical `examples/demo_store/` app. Every command produces a
schema-versioned JSON contract; pasting the outputs into a downstream
agent should be reliable.

> Prerequisite: build the release CLI once.
>
> ```bash
> cargo build --release -p ori
> alias ori="$PWD/target/release/ori"
> ```

## 1. Health check

```bash
ori doctor
```

Returns `ori.doctor.v1` with the compiler version, host toolchain, and the
17 stable contract versions the bootstrap knows about. Pipe to `jq` for a
human-readable view.

## 2. Single-file check

```bash
ori check --json examples/demo_store/src/domain.ori
```

No diagnostics → exit 0. Each diagnostic line is one `ori.diagnostic.v1`
JSON object.

## 3. Module capsule

```bash
ori capsule --json examples/demo_store/src/api.ori
```

`ori.capsule.v1`: exports + imports + invariants + agent token summary +
recommended-context symbol ids. Use this when an agent needs a single
shot of "what does this module promise".

## 4. Budget-aware agent map

```bash
ori agent map --budget 3000 --json examples/demo_store/src/api.ori
```

`ori.agent_map.v1`: every symbol that fits inside the budget, with
`used_estimate` and `truncated` flags. Tune `--budget` until the output
fits the model context window of your downstream agent.

## 5. Symbol explanation

```bash
ori agent explain sym:demo_store.api.post_checkout --json \
  examples/demo_store/src/api.ori
```

`ori.symbol_card.v1`: the single-symbol expansion with signature,
effects, span, and short summary.

## 6. Structured diagnose

```bash
ori agent diagnose --json examples/demo_store/src/api.ori
```

`ori.agent_diagnose.v1`: overall status, error/warning counts, and the
top-confidence repair candidates extracted from diagnostic fixes.

## 7. OpenAPI generation

```bash
ori openapi --json examples/demo_store/src/api.ori
```

`ori.openapi_report.v1`: derived routes (method + path + params +
response type + effects) from any fn carrying the `http` effect.

## 8. UI manifest + accessibility

```bash
ori ui --json examples/demo_store/src/ui.ori
```

`ori.ui_manifest.v1`: every `view <Name>(props)` plus baseline
accessibility findings. The bootstrap surfaces an info-level hint for
the `CheckoutForm` view (missing `submit_label`).

## 9. Capability manifest + policy diff

```bash
ori capability --policy "http,db.read,db.write" --json \
  examples/demo_store/src/api.ori
```

`ori.capability.v1`: every effect used by the module's symbols, mapped
back to the symbol ids that declare them. `policy.undeclared` /
`policy.unused` is the diff against the declared policy. A clean match
yields empty arrays.

## 10. Wasm component manifest

```bash
ori wasm --json examples/demo_store/src/api.ori
```

`ori.wasm_component.v1`: exports, imports, capability union, and a
proposed `<module>-world` name. Future codegen passes consume this.

## 11. Build a real wasm artefact

```bash
ori build --target wasm-component --json examples/hello.ori
```

Writes `examples/hello.ori.wasm` (37 bytes) and reports it under the
`outputs[]` array of the build report.

## 12. Patch IR round trip

Validate the demo patch:

```bash
ori patch check --json examples/demo_store/contracts/agent_patch_add_product_search.json
```

Dry-run the apply against the catalog module:

```bash
ori patch dry-run --json \
  examples/demo_store/contracts/agent_patch_add_product_search.json \
  examples/demo_store/src/catalog.ori
```

The dry-run reports 2 of 3 ops applied — the third targets a different
file (`ui.ori`), which the bootstrap apply engine correctly skips with a
`P1010` stale-target diagnostic. The `after` field shows the updated
catalog source.

## 13. Run the demo entrypoint

```bash
ori run examples/hello.ori
```

The bootstrap interpreter reports `status: ok` with the entry function
name and observed effects. (Body-level execution is wave 3 work.)

## 14. Affected tests

```bash
ori agent tests --affected --changed-name search --json examples/demo_store
```

`ori.agent_tests.v1`: returns the per-file set of tests that reference
the `search` identifier.

## 15. Document the module set

```bash
ori docs --format agent --budget 1500 examples/demo_store/src
```

Budget-aware markdown listing every module, every symbol id, effects,
and dependency edges — the form designed to feed an AI agent's context
window.

## 16. Plan an edition migration

```bash
ori migrate --from 2027.1 --to 2028.1 --dry-run --json examples/demo_store/src
```

`ori.migration_report.v1`: candidate rewrites for the target edition
(the demo storefront has no candidates because its existing source is
already 2028.1-compatible).

## 17. Run the benchmark suite

```bash
ori bench --samples 50 --json > /tmp/bench.json
```

`ori.benchmark.v1`: eight suites with mean / p50 / p95 / max / min for
each metric. See `BENCHMARKS.md` for the table form.

## 18. Package check / audit / SBOM

```bash
ori package check --json
ori audit --json
ori sbom --json --format ori-native
```

All three round-trip the repo's own `ori.toml` through the typed
package-manager pipeline.

## 19. Run the LSP server

```bash
ori lsp --stdio
```

The server accepts standard LSP base-protocol traffic on stdin/stdout
(initialize, didOpen, didChange, hover, completion, code actions,
rename, shutdown).

## 20. Run the full quality gate

```bash
python3 scripts/validate_all.py --full
```

Final line: **validation passed**.
