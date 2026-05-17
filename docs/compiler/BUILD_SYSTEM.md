# Build System

The Orison build system is integrated into the `ori` tool.

## Commands

```bash
ori check
ori run
ori test
ori test --changed
ori build --dev
ori build --release
ori build --target wasm-component
ori build --target mobile
```

## Safety invariant

Dev and release modes use the same safety checks. Only optimization and backend strategy differ.

## Bundle-size strategy

- No reflection by default.
- No import side effects.
- Tree-shaken standard modules.
- Capability-aware linking.
- Dead route elimination.
- Dead UI component elimination.
- Typed asset bundling.
