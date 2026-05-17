# Patch IR

Patch IR lets agents submit structural edits instead of rewriting whole files.

## Example

```json
{
  "schema": "ori.patch.v1",
  "intent": "Handle database error in user rendering",
  "operations": [
    {
      "op": "insert_match_arm",
      "target": "node:match:42",
      "pattern": "Err(Db(err))",
      "body": "render_db_error(err)"
    }
  ],
  "tests": {
    "run": ["ori test --changed"],
    "expected": "pass"
  }
}
```

## Operation types

```text
replace_node
insert_node
delete_node
rename_symbol
add_import
remove_import
change_signature
insert_match_arm
add_field
remove_field
add_protocol_impl
update_route
update_view
add_test
```

## Current `ori patch check` validation

The bootstrap validator checks:

- JSON parses successfully;
- root is an object;
- `schema` is `ori.patch.v1`;
- `intent` is a non-empty string;
- `operations` is a non-empty array;
- each operation is an object;
- each operation has a known `op`;
- each known operation contains its required fields;
- missing `tests` emits a warning.

It returns `ori.patch_check.v1` JSON.

## Future validation rules

- Target nodes must exist.
- Operation must be valid for node kind.
- Resulting code must parse.
- Resulting code must pass at least the affected checks.
- Public API changes require explicit intent.

## Implementation plan

1. Parse patch JSON.
2. Resolve target node IDs.
3. Apply operations to CST.
4. Reformat affected ranges.
5. Run affected compiler queries.
6. Return structured success/failure diagnostics.
