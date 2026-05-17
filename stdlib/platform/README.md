# platform — target-specific shims

Platform-specific adapters: web (DOM bindings), mobile (notifications,
camera, sensors). Code in `platform` may depend on `core`, `std`,
and `app` — it sits above the application layer in the dependency
stack.

## Modules

| Module | Target | Purpose |
|--------|--------|---------|
| [`web.ori`](./web.ori) | wasm-web | DOM document + element + event bindings |
| [`mobile.ori`](./mobile.ori) | iOS + Android | Notifications, camera, sensors |

## Stability

Tier 3 (experimental) until M30 lands the mobile/desktop targets.
The interfaces are stable; the implementations are scaffolds.

## Capability mapping

`platform.web.get_document()` requires `uses ui`. `platform.mobile.open_camera()`
requires `uses ui`. The mobile manifest derived from these effect
declarations (see `crates/ori-compiler/src/mobile.rs`) maps `ui` →
iOS `NSCameraUsageDescription` + Android `android.permission.CAMERA`.

## Adding a module

A new `platform` module requires:

1. Parses clean via `ori check --json`.
2. The module declares which target it serves (`platform.web.*`,
   `platform.mobile.*`, etc.).
3. The corresponding manifest extractor knows about every effect the
   module exposes (extend `mobile_permissions.rs` for mobile,
   `wasm_component.rs` for web).
4. A `CHANGELOG.md` entry.
