# Memory Model

Orison safe code is memory-safe by construction.

## Ownership

- Values are owned by default.
- Assignment and parameter passing move non-copy values.
- Copy types are explicitly classified.

## Borrowing

Public APIs may use explicit borrows:

```ori
fn checksum(data: borrow Bytes) -> UInt64:
  ...
```

Mutable borrows are exclusive:

```ori
fn normalize(data: mut Bytes) -> Unit:
  ...
```

## Allocation strategies

- Stack allocation.
- Owned heap allocation.
- Arena allocation.
- Reference-counted shared ownership.
- Weak references for cycles.

## Garbage collection

There is no mandatory global GC. Platform adapters may provide managed-host interop, but core Orison semantics should not depend on a global collector.

## Agent diagnostics

Ownership diagnostics must include:

- moved symbol ID;
- invalid use span;
- original move span;
- suggested borrow or clone repair;
- minimal context symbols.
