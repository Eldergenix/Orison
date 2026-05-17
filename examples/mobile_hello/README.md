# Mobile Hello

A four-module mobile reference app:

- `src/domain.ori` declares `UserId`, `Username`, `Profile`, `FeedItem`
  and the `ProfileError` variant.
- `src/api.ori` declares the `MobileApi` service (`fetch_profile`,
  `fetch_feed`, `refresh_token`).
- `src/ui.ori` declares three typed views: `ProfileView`, `FeedView`,
  and `SettingsView` (the last requires the `auth` capability).
- `src/main.ori` declares `boot()`.

Walk through the build in
[`docs/tutorial/12-mobile-app.md`](../../docs/tutorial/12-mobile-app.md).

## Quick check

```bash
for f in examples/mobile_hello/src/*.ori; do
  cargo run --release -p ori --quiet -- check --json "$f"
done
```

All four should exit `0` with no stdout.
