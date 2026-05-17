# Debugging Playbook for Agents

1. Reproduce with the smallest command.
2. Capture JSON diagnostics when available.
3. Validate JSON with `jq` or `serde_json` before interpreting it.
4. Identify affected symbol IDs.
5. Request or generate a symbol card.
6. Make a minimal patch.
7. Run affected checks or tests.
8. Run full tests when compiler architecture changes.
9. Update docs if behavior changed.

Do not rewrite unrelated files.
