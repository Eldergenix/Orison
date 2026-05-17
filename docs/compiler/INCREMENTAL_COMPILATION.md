# Incremental Compilation

Fast builds are a primary product requirement.

## Query model

Every compiler phase should be a pure or explicitly invalidated query.

Example queries:

```text
parse(file_hash) -> CST
declarations(module_id) -> DeclList
resolve(symbol_id) -> ResolvedSymbol
type_of(expr_id) -> TypeRef
effects_of(symbol_id) -> EffectSet
borrow_graph(symbol_id) -> BorrowGraph
mir(symbol_id) -> MirBody
capsule(module_id) -> Capsule
```

## Invalidation

A source edit should invalidate the smallest possible set of queries.

## Agent advantage

The same dependency graph used for incremental compilation powers:

- affected tests;
- minimal context slices;
- patch validation;
- API impact reports.
