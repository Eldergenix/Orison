//! Comprehensive iOS and Android permission catalogue for the mobile target.
//!
//! The bootstrap [`crate::mobile`] pass derives a coarse "permission key"
//! (`network`, `storage`, …) from declared effects. The real iOS and Android
//! permission catalogues are far richer: iOS uses Info.plist usage-description
//! strings, while Android uses `<uses-permission>` manifest entries that are
//! further graded by a protection level (Normal, Dangerous, Signature).
//!
//! This module exposes:
//!
//! * [`IosPermission`] — one entry per Info.plist usage-description key,
//!   carrying the canonical capability key, the plist key, and the
//!   `*UsageDescription` string Apple expects in the bundle.
//! * [`AndroidPermission`] — one entry per `<uses-permission>` AndroidManifest
//!   declaration, carrying the canonical capability key, the fully qualified
//!   `android.permission.*` name, and the OS [`ProtectionLevel`].
//! * [`ios_permission_for_capability`] / [`android_permission_for_capability`]
//!   — lookup helpers used by the mobile manifest generator and external
//!   tooling.
//!
//! ## Stability
//!
//! Both catalogues are *closed* in the bootstrap: new entries must be added
//! deliberately so downstream consumers (lockfiles, audit reports, SBOMs) see
//! a single source of truth. Keys are stable identifiers and must remain
//! globally unique — see the unit tests at the bottom of this module for the
//! enforcement gate.
//!
//! The module is panic-free: every fallible path returns [`Option`] / iterates
//! deterministically over `&'static` data, never via
//! `unwrap`/`expect`/`panic!`.

/// Android protection level mirroring the system-defined gradation.
///
/// * `Normal` — granted at install time, low risk.
/// * `Dangerous` — requires runtime user consent on Android 6.0+.
/// * `Signature` — granted only to apps signed with the same certificate as
///   the declaring app (typically a system-only permission).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectionLevel {
    /// Normal install-time permission.
    Normal,
    /// Dangerous runtime permission (Android 6.0+ prompts the user).
    Dangerous,
    /// Signature-only permission (system/sister-app use).
    Signature,
}

impl ProtectionLevel {
    /// Canonical, stable string used for JSON/CLI surfaces.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProtectionLevel::Normal => "normal",
            ProtectionLevel::Dangerous => "dangerous",
            ProtectionLevel::Signature => "signature",
        }
    }
}

/// One iOS Info.plist permission entry.
///
/// Apple ships permissions as plist keys whose **value** is a human-readable
/// usage description shown to the user at the consent prompt. The
/// `usage_description_key` and `plist_key` are equal today but are surfaced
/// separately so future iOS revisions that split presentation from
/// declaration can be modeled without a schema break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IosPermission {
    /// Canonical capability key used by [`crate::mobile`]
    /// (e.g. `camera`, `location.when_in_use`).
    pub key: &'static str,
    /// The plist key whose *value* is the usage-description string Apple
    /// displays in the consent prompt.
    pub usage_description_key: &'static str,
    /// The plist key written into `Info.plist`. Today this matches
    /// `usage_description_key`, but is kept as a distinct field so future
    /// presentation-only keys (e.g. companion `Always` variants) can be
    /// modelled without breaking the contract.
    pub plist_key: &'static str,
}

/// One Android `<uses-permission>` catalogue entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AndroidPermission {
    /// Canonical capability key used by [`crate::mobile`]
    /// (e.g. `camera`, `location.fine`).
    pub key: &'static str,
    /// Fully qualified Android manifest name (e.g.
    /// `android.permission.CAMERA`).
    pub manifest_name: &'static str,
    /// OS-defined protection level.
    pub protection_level: ProtectionLevel,
}

/// The full iOS permission catalogue. Ordered to keep diffs stable.
const IOS_CATALOGUE: &[IosPermission] = &[
    IosPermission {
        key: "camera",
        usage_description_key: "NSCameraUsageDescription",
        plist_key: "NSCameraUsageDescription",
    },
    IosPermission {
        key: "microphone",
        usage_description_key: "NSMicrophoneUsageDescription",
        plist_key: "NSMicrophoneUsageDescription",
    },
    IosPermission {
        key: "location.when_in_use",
        usage_description_key: "NSLocationWhenInUseUsageDescription",
        plist_key: "NSLocationWhenInUseUsageDescription",
    },
    IosPermission {
        key: "location.always",
        usage_description_key: "NSLocationAlwaysAndWhenInUseUsageDescription",
        plist_key: "NSLocationAlwaysAndWhenInUseUsageDescription",
    },
    IosPermission {
        key: "photos.read",
        usage_description_key: "NSPhotoLibraryUsageDescription",
        plist_key: "NSPhotoLibraryUsageDescription",
    },
    IosPermission {
        key: "photos.write",
        usage_description_key: "NSPhotoLibraryAddUsageDescription",
        plist_key: "NSPhotoLibraryAddUsageDescription",
    },
    IosPermission {
        key: "contacts",
        usage_description_key: "NSContactsUsageDescription",
        plist_key: "NSContactsUsageDescription",
    },
    IosPermission {
        key: "calendar",
        usage_description_key: "NSCalendarsUsageDescription",
        plist_key: "NSCalendarsUsageDescription",
    },
    IosPermission {
        key: "reminders",
        usage_description_key: "NSRemindersUsageDescription",
        plist_key: "NSRemindersUsageDescription",
    },
    IosPermission {
        key: "bluetooth",
        usage_description_key: "NSBluetoothAlwaysUsageDescription",
        plist_key: "NSBluetoothAlwaysUsageDescription",
    },
    IosPermission {
        key: "motion",
        usage_description_key: "NSMotionUsageDescription",
        plist_key: "NSMotionUsageDescription",
    },
    IosPermission {
        key: "face_id",
        usage_description_key: "NSFaceIDUsageDescription",
        plist_key: "NSFaceIDUsageDescription",
    },
    IosPermission {
        key: "health.read",
        usage_description_key: "NSHealthShareUsageDescription",
        plist_key: "NSHealthShareUsageDescription",
    },
    IosPermission {
        key: "health.write",
        usage_description_key: "NSHealthUpdateUsageDescription",
        plist_key: "NSHealthUpdateUsageDescription",
    },
    IosPermission {
        key: "speech_recognition",
        usage_description_key: "NSSpeechRecognitionUsageDescription",
        plist_key: "NSSpeechRecognitionUsageDescription",
    },
    IosPermission {
        key: "siri",
        usage_description_key: "NSSiriUsageDescription",
        plist_key: "NSSiriUsageDescription",
    },
    IosPermission {
        key: "local_network",
        usage_description_key: "NSLocalNetworkUsageDescription",
        plist_key: "NSLocalNetworkUsageDescription",
    },
    IosPermission {
        key: "tracking",
        usage_description_key: "NSUserTrackingUsageDescription",
        plist_key: "NSUserTrackingUsageDescription",
    },
    IosPermission {
        key: "nfc",
        usage_description_key: "NFCReaderUsageDescription",
        plist_key: "NFCReaderUsageDescription",
    },
];

/// The full Android permission catalogue. Ordered to keep diffs stable.
const ANDROID_CATALOGUE: &[AndroidPermission] = &[
    AndroidPermission {
        key: "internet",
        manifest_name: "android.permission.INTERNET",
        protection_level: ProtectionLevel::Normal,
    },
    AndroidPermission {
        key: "network_state",
        manifest_name: "android.permission.ACCESS_NETWORK_STATE",
        protection_level: ProtectionLevel::Normal,
    },
    AndroidPermission {
        key: "wifi_state",
        manifest_name: "android.permission.ACCESS_WIFI_STATE",
        protection_level: ProtectionLevel::Normal,
    },
    AndroidPermission {
        key: "wake_lock",
        manifest_name: "android.permission.WAKE_LOCK",
        protection_level: ProtectionLevel::Normal,
    },
    AndroidPermission {
        key: "vibrate",
        manifest_name: "android.permission.VIBRATE",
        protection_level: ProtectionLevel::Normal,
    },
    AndroidPermission {
        key: "foreground_service",
        manifest_name: "android.permission.FOREGROUND_SERVICE",
        protection_level: ProtectionLevel::Normal,
    },
    AndroidPermission {
        key: "post_notifications",
        manifest_name: "android.permission.POST_NOTIFICATIONS",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "camera",
        manifest_name: "android.permission.CAMERA",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "record_audio",
        manifest_name: "android.permission.RECORD_AUDIO",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "location.fine",
        manifest_name: "android.permission.ACCESS_FINE_LOCATION",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "location.coarse",
        manifest_name: "android.permission.ACCESS_COARSE_LOCATION",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "location.background",
        manifest_name: "android.permission.ACCESS_BACKGROUND_LOCATION",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "contacts.read",
        manifest_name: "android.permission.READ_CONTACTS",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "contacts.write",
        manifest_name: "android.permission.WRITE_CONTACTS",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "calendar.read",
        manifest_name: "android.permission.READ_CALENDAR",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "calendar.write",
        manifest_name: "android.permission.WRITE_CALENDAR",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "sms.read",
        manifest_name: "android.permission.READ_SMS",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "sms.send",
        manifest_name: "android.permission.SEND_SMS",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "phone.call",
        manifest_name: "android.permission.CALL_PHONE",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "phone.read_state",
        manifest_name: "android.permission.READ_PHONE_STATE",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "storage.read_media_images",
        manifest_name: "android.permission.READ_MEDIA_IMAGES",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "storage.read_media_audio",
        manifest_name: "android.permission.READ_MEDIA_AUDIO",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "storage.read_media_video",
        manifest_name: "android.permission.READ_MEDIA_VIDEO",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "bluetooth.connect",
        manifest_name: "android.permission.BLUETOOTH_CONNECT",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "bluetooth.scan",
        manifest_name: "android.permission.BLUETOOTH_SCAN",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "biometric",
        manifest_name: "android.permission.USE_BIOMETRIC",
        protection_level: ProtectionLevel::Normal,
    },
    AndroidPermission {
        key: "nfc",
        manifest_name: "android.permission.NFC",
        protection_level: ProtectionLevel::Normal,
    },
    AndroidPermission {
        key: "activity_recognition",
        manifest_name: "android.permission.ACTIVITY_RECOGNITION",
        protection_level: ProtectionLevel::Dangerous,
    },
    AndroidPermission {
        key: "body_sensors",
        manifest_name: "android.permission.BODY_SENSORS",
        protection_level: ProtectionLevel::Dangerous,
    },
];

/// Read-only view of the full iOS catalogue. Stable across releases.
pub fn ios_catalogue() -> &'static [IosPermission] {
    IOS_CATALOGUE
}

/// Read-only view of the full Android catalogue. Stable across releases.
pub fn android_catalogue() -> &'static [AndroidPermission] {
    ANDROID_CATALOGUE
}

/// Look up an iOS permission by capability key. Returns `None` for keys not
/// present in the catalogue (e.g. coarse Orison keys like `display`).
pub fn ios_permission_for_capability(key: &str) -> Option<IosPermission> {
    IOS_CATALOGUE.iter().find(|p| p.key == key).copied()
}

/// Look up an Android permission by capability key. Returns `None` for keys
/// not present in the catalogue.
pub fn android_permission_for_capability(key: &str) -> Option<AndroidPermission> {
    ANDROID_CATALOGUE.iter().find(|p| p.key == key).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn ios_catalogue_has_at_least_fifteen_entries() {
        assert!(
            ios_catalogue().len() >= 15,
            "ios catalogue has {} entries; expected >= 15",
            ios_catalogue().len()
        );
    }

    #[test]
    fn android_catalogue_has_at_least_twenty_entries() {
        assert!(
            android_catalogue().len() >= 20,
            "android catalogue has {} entries; expected >= 20",
            android_catalogue().len()
        );
    }

    #[test]
    fn every_ios_key_round_trips() {
        for entry in ios_catalogue() {
            let looked_up = ios_permission_for_capability(entry.key);
            assert_eq!(
                looked_up,
                Some(*entry),
                "iOS lookup mismatch for key `{}`",
                entry.key
            );
        }
    }

    #[test]
    fn every_android_key_round_trips() {
        for entry in android_catalogue() {
            let looked_up = android_permission_for_capability(entry.key);
            assert_eq!(
                looked_up,
                Some(*entry),
                "Android lookup mismatch for key `{}`",
                entry.key
            );
        }
    }

    #[test]
    fn unknown_capability_returns_none_ios() {
        assert!(ios_permission_for_capability("not-a-real-key").is_none());
    }

    #[test]
    fn unknown_capability_returns_none_android() {
        assert!(android_permission_for_capability("not-a-real-key").is_none());
    }

    #[test]
    fn ios_keys_are_unique() {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for entry in ios_catalogue() {
            assert!(
                seen.insert(entry.key),
                "duplicate iOS capability key: {}",
                entry.key
            );
        }
    }

    #[test]
    fn android_keys_are_unique() {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for entry in android_catalogue() {
            assert!(
                seen.insert(entry.key),
                "duplicate Android capability key: {}",
                entry.key
            );
        }
    }

    #[test]
    fn ios_plist_keys_are_unique() {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for entry in ios_catalogue() {
            assert!(
                seen.insert(entry.plist_key),
                "duplicate iOS plist key: {}",
                entry.plist_key
            );
        }
    }

    #[test]
    fn android_manifest_names_are_unique() {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for entry in android_catalogue() {
            assert!(
                seen.insert(entry.manifest_name),
                "duplicate Android manifest name: {}",
                entry.manifest_name
            );
        }
    }

    #[test]
    fn protection_level_string_is_stable() {
        assert_eq!(ProtectionLevel::Normal.as_str(), "normal");
        assert_eq!(ProtectionLevel::Dangerous.as_str(), "dangerous");
        assert_eq!(ProtectionLevel::Signature.as_str(), "signature");
    }

    #[test]
    fn camera_lookup_is_consistent_across_platforms() {
        let ios = ios_permission_for_capability("camera");
        let android = android_permission_for_capability("camera");
        assert!(ios.is_some());
        assert!(android.is_some());
        if let Some(android) = android {
            assert_eq!(android.protection_level, ProtectionLevel::Dangerous);
        }
    }
}
