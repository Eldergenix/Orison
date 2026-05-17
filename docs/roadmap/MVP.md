# MVP Definition

The MVP should prove the agent-native compiler loop.

## MVP scope

- Parser and formatter.
- Type checker for core functions, records, variants, Option, Result.
- Effect declarations.
- JSON diagnostics.
- Semantic capsules.
- Agent maps and symbol cards.
- Patch IR validation.
- Test runner scaffold.
- HTTP/router framework prototype.
- JSON and validation standard modules.

## Explicitly out of MVP

- Mature mobile adapter.
- Full ML/autodiff stack.
- Production package registry.
- Full optimizing compiler.
- Complete UI renderer.

## MVP acceptance test

An agent should be able to:

1. inspect a small full-stack Orison app with `ori agent map`;
2. add a typed backend route;
3. update a UI view;
4. run affected tests;
5. keep diagnostics clean;
6. use less context than whole-file prompting.
