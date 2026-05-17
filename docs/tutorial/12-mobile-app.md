# Chapter 12: Building a mobile app

**What you'll build.** A four-module mobile reference app — three typed
views and one network service — built for both iOS (SwiftUI) and Android
(Compose) through `ori build --target mobile`. Unlike the desktop
builder (chapter 11), the mobile pipeline is fully wired today: every
build emits a `ori.mobile_manifest.v1` with a populated
`native_ui_manifest`, an explicit permissions list, and the union of
capabilities required across the entry-point set.

**Time:** ~10 minutes.

## 1. The mobile build surface

The mobile target shipped with the bootstrap and is documented in
`ori build --help`:

```bash
cargo run --release -p ori -- build
```

```
usage: ori build [--target dev|release|wasm-component|llvm-text|mobile]
                 [--app-id <id>] [--platforms ios,android]
                 [--ui-kind ios-uikit|ios-swiftui|android-compose|android-view]
                 [--json] <file.ori>
```

Four `--ui-kind` values are recognised today:

| `--ui-kind`        | Target framework                                    | View component name      |
|--------------------|-----------------------------------------------------|--------------------------|
| `ios-uikit`        | UIKit (`UIView` / `UIViewController`)               | `View`                   |
| `ios-swiftui`      | SwiftUI declarative `View`                          | `View`                   |
| `android-compose`  | Jetpack Compose `@Composable fun`                   | `ComposableFunction`     |
| `android-view`     | Android `android.view.View` hierarchy               | `View`                   |

The result is an `ori.mobile_manifest.v1` envelope. Read the schema
([`schemas/mobile-manifest.schema.json`](../../schemas/mobile-manifest.schema.json))
to see the full shape.

## 2. The reference app

[`examples/mobile_hello/`](../../examples/mobile_hello) contains four
modules:

```bash
ls examples/mobile_hello/src/
# api.ori   domain.ori   main.ori   ui.ori
```

[`domain.ori`](../../examples/mobile_hello/src/domain.ori) is pure
values:

```ori
module mobile_hello.domain

type UserId wraps Str
type Username wraps Str

type Profile = {
  id: UserId,
  username: Username,
  avatar_url: Str,
  follower_count: Int
}

type FeedItem = {
  id: Str,
  author: Username,
  text: Str,
  posted_at_unix_seconds: Int
}

variant ProfileError =
  | NotFound(id: UserId)
  | NetworkFailed(reason: Str)
  | Unauthorised
```

[`api.ori`](../../examples/mobile_hello/src/api.ori) declares the
network service:

```ori
module mobile_hello.api

import mobile_hello.domain

service MobileApi uses http, net.outbound

fn fetch_profile(id: UserId) -> Result[Profile, ProfileError] uses http, net.outbound
fn fetch_feed(id: UserId) -> Result[List[FeedItem], ProfileError] uses http, net.outbound
fn refresh_token() -> Result[Unit, ProfileError] uses http, net.outbound
```

[`ui.ori`](../../examples/mobile_hello/src/ui.ori) declares three views,
one of which gates on the `auth` capability:

```ori
module mobile_hello.ui

import mobile_hello.domain

view ProfileView(profile: Profile) -> Html uses ui:
  card:
    heading(level: 1, text: profile.username.value)
    text(profile.avatar_url)

view FeedView(items: List[FeedItem]) -> Html uses ui:
  list:
    for item in items:
      heading(level: 3, text: item.author.value)
      text(item.text)

view SettingsView(profile: Profile) -> Html uses ui, auth:
  form:
    heading(level: 2, text: "Settings")
    text(profile.username.value)
```

And [`main.ori`](../../examples/mobile_hello/src/main.ori) is the entry
point:

```ori
module mobile_hello.main

import mobile_hello.api
import mobile_hello.ui

fn boot() -> Unit uses http, net.outbound, ui, auth:
  return Unit
```

## 3. Check, capsule, capability

Run the per-file gate before building:

```bash
for f in examples/mobile_hello/src/*.ori; do
  cargo run --release -p ori --quiet -- check --json "$f"
  echo "exit=$?"
done
```

All four exit `0`. The UI module's effects:

```bash
cargo run --release -p ori --quiet -- capability --json examples/mobile_hello/src/ui.ori \
  | jq '.effects | map(.name)'
```

```json
["auth", "ui"]
```

`auth` shows up because `SettingsView` declares `uses ui, auth`. Compare
with `api.ori`:

```bash
cargo run --release -p ori --quiet -- capability --json examples/mobile_hello/src/api.ori \
  | jq '.effects | map(.name)'
```

```json
["http", "net.outbound"]
```

`ori ui` returns the typed view manifest, which is the input to the
mobile build:

```bash
cargo run --release -p ori --quiet -- ui --json examples/mobile_hello/src/ui.ori \
  | jq '.views[] | {name, prop_count: (.props | length)}'
```

```json
{ "name": "ProfileView",  "prop_count": 1 }
{ "name": "FeedView",     "prop_count": 1 }
{ "name": "SettingsView", "prop_count": 1 }
```

## 4. Build for iOS SwiftUI

```bash
cargo run --release -p ori --quiet -- build \
  --target mobile \
  --ui-kind ios-swiftui \
  --app-id com.example.mobile_hello \
  --platforms ios \
  --json examples/mobile_hello/src/ui.ori \
  | jq '.outputs[0].manifest | {schema, app_id, platforms, capabilities, permissions, view_count: (.native_ui_manifest.views | length)}'
```

```json
{
  "schema":       "ori.mobile_manifest.v1",
  "app_id":       "com.example.mobile_hello",
  "platforms":    ["ios"],
  "capabilities": ["auth", "ui"],
  "permissions":  [
    { "key": "display",         "justification": "auto-derived from declared effect ui" },
    { "key": "authentication",  "justification": "auto-derived from declared effect auth" }
  ],
  "view_count":   3
}
```

The `permissions` list is auto-derived — you do not write it by hand.
The native UI manifest pins one entry per view:

```bash
cargo run --release -p ori --quiet -- build \
  --target mobile --ui-kind ios-swiftui --app-id com.example.mobile_hello --platforms ios \
  --json examples/mobile_hello/src/ui.ori \
  | jq '.outputs[0].manifest.native_ui_manifest.views[] | {view_name, component, kind, props: (.props_mapping | keys)}'
```

```json
{ "view_name": "ProfileView",  "component": "View", "kind": "ios-swiftui", "props": ["profile"] }
{ "view_name": "FeedView",     "component": "View", "kind": "ios-swiftui", "props": ["items"] }
{ "view_name": "SettingsView", "component": "View", "kind": "ios-swiftui", "props": ["profile"] }
```

The build writes its manifest to `examples/mobile_hello/src/ui.ori.mobile.json`.
Delete it when you are done:

```bash
rm -f examples/mobile_hello/src/ui.ori.mobile.json
```

## 5. Build for Android Compose

Same input file, different `--ui-kind`:

```bash
cargo run --release -p ori --quiet -- build \
  --target mobile \
  --ui-kind android-compose \
  --app-id com.example.mobile_hello \
  --platforms android \
  --json examples/mobile_hello/src/ui.ori \
  | jq '.outputs[0].manifest.native_ui_manifest.views[] | {view_name, component, kind}'
```

```json
{ "view_name": "ProfileView",  "component": "ComposableFunction", "kind": "android-compose" }
{ "view_name": "FeedView",     "component": "ComposableFunction", "kind": "android-compose" }
{ "view_name": "SettingsView", "component": "ComposableFunction", "kind": "android-compose" }
```

The native shell receives a list of composable functions whose
parameters map 1:1 to the Orison view props. Delete the artefact:

```bash
rm -f examples/mobile_hello/src/ui.ori.mobile.json
```

## 6. The combined platforms build

You can request both platforms at once:

```bash
cargo run --release -p ori --quiet -- build \
  --target mobile --ui-kind ios-swiftui \
  --app-id com.example.mobile_hello \
  --platforms ios,android \
  --json examples/mobile_hello/src/main.ori \
  | jq '.outputs[0].manifest | {platforms, entrypoints, capabilities}'
```

```json
{
  "platforms":    ["ios", "android"],
  "entrypoints":  ["sym:mobile_hello.main.boot"],
  "capabilities": ["auth", "http", "net.outbound", "ui"]
}
```

When you build off `main.ori`, the `entrypoints` array names the boot
function and the capability set is the union of every effect the entry
point and its transitive imports declare. The `native_ui_manifest.views`
list is empty because `main.ori` itself declares no views — to get the
populated view list, build off `ui.ori` as in step 4.

## 7. The package manifest

[`examples/mobile_hello/ori.toml`](../../examples/mobile_hello/ori.toml)
declares the capability superset:

```toml
[capabilities]
declared = ["http", "net.outbound", "ui", "auth"]
```

`ori package check` confirms:

```bash
cargo run --release -p ori --quiet -- package check --json examples/mobile_hello | jq '{ok}'
```

```json
{ "ok": true }
```

## 8. Permissions table

The bootstrap derives the platform permission keys from the Orison
effect set using a fixed table:

| Effect          | Permission key      | Notes                                   |
|-----------------|---------------------|-----------------------------------------|
| `http`          | `network`           | NSAppTransportSecurity / `INTERNET`     |
| `net.outbound`  | `network`           | Coalesces with `http`.                  |
| `net.inbound`   | `network_server`    | Listening sockets.                      |
| `ui`            | `display`           | Foreground UI presentation.             |
| `auth`          | `authentication`    | Biometrics / keychain access.           |
| `db.read`       | `database`          | App-local store.                        |
| `db.write`      | `database`          | Coalesces with `db.read`.               |
| `fs.read`       | `file_read`         | Document picker / shared container.     |
| `fs.write`      | `file_write`        | Coalesces with `fs.read` when both.     |

Every entry in `permissions` carries a `justification` string that
points back to the effect that derived it; the field is intended for
inclusion in the App Store / Play Store privacy disclosures.

## 9. One-shell smoke

```bash
set -euo pipefail
for f in examples/mobile_hello/src/*.ori; do
  cargo run --release -p ori --quiet -- check --json "$f" > /dev/null
done
cargo run --release -p ori --quiet -- ui --json examples/mobile_hello/src/ui.ori > /dev/null
cargo run --release -p ori --quiet -- build --target mobile --ui-kind ios-swiftui \
  --app-id com.example.mobile_hello --platforms ios \
  --json examples/mobile_hello/src/ui.ori > /dev/null
cargo run --release -p ori --quiet -- build --target mobile --ui-kind android-compose \
  --app-id com.example.mobile_hello --platforms android \
  --json examples/mobile_hello/src/ui.ori > /dev/null
rm -f examples/mobile_hello/src/ui.ori.mobile.json
echo "mobile_hello: all gates pass"
```

## Common errors

| Symptom | Cause | Fix |
|---------|-------|-----|
| `MOB0001` | The `--app-id` doesn't match the bundle-id regex. | Lowercase + at least one dot. |
| `MOB0002` | `--ui-kind` omitted but the source has `view` declarations. | Add `--ui-kind ios-swiftui` (or any of the four). |
| `MOB0003` | Effect with no permission mapping. | Open a tracking issue; bootstrap maps the nine effects above only. |
| Build report `outputs[0].byte_count` is 0 | The manifest serialised empty — almost always means the source has no exports. | Build off the module that actually contains views or services. |

## Recap

- `ori build --target mobile` is fully wired and emits
  `ori.mobile_manifest.v1` with a `native_ui_manifest`, a permissions
  list, and a capability union.
- Four UI kinds ship today: `ios-uikit`, `ios-swiftui`,
  `android-compose`, `android-view`. The `view` -> native component
  mapping is fixed per kind.
- The permissions list is auto-derived from the declared effects via
  the nine-row table in section 8.
- The reference app under `examples/mobile_hello/` exercises every UI
  kind across three views and one service.

## Next

[Chapter 13: Publishing](./13-publishing.md) covers the full package
lifecycle — `ori package check`, `ori sbom`, `ori publish`,
`ori registry list`, and `ori registry yank`.
