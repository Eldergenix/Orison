# Mobile Framework

Mobile support is a platform adapter, not a separate language.

```ori
mobile App:
  screen "/":
    HomeView()

  permission camera:
    reason "Scan QR codes for account setup"

  permission notifications:
    reason "Send order updates"
```

## Mobile capabilities

```text
mobile.camera
mobile.location
mobile.notifications
mobile.storage
mobile.contacts
mobile.clipboard
mobile.sensors
```

Permissions must be explicit and manifest-generated.
