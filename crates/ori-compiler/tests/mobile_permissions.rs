//! Integration tests for the iOS / Android permission catalogue.
//!
//! These tests exercise the public surface of
//! [`ori_compiler::mobile_permissions`] from outside the crate so the
//! catalogue is wired through the published module path.

use ori_compiler::mobile_permissions::{
    android_catalogue, android_permission_for_capability, ios_catalogue,
    ios_permission_for_capability, ProtectionLevel,
};

#[test]
fn catalogues_meet_minimum_size_bar() {
    assert!(
        ios_catalogue().len() >= 15,
        "iOS catalogue has {} entries; expected >= 15",
        ios_catalogue().len()
    );
    assert!(
        android_catalogue().len() >= 20,
        "Android catalogue has {} entries; expected >= 20",
        android_catalogue().len()
    );
}

#[test]
fn ios_lookup_for_camera_matches_apple_plist_key() {
    let entry = ios_permission_for_capability("camera");
    assert!(
        entry.is_some(),
        "expected `camera` to be in the iOS catalogue"
    );
    if let Some(entry) = entry {
        assert_eq!(entry.plist_key, "NSCameraUsageDescription");
        assert_eq!(entry.usage_description_key, "NSCameraUsageDescription");
    }
}

#[test]
fn android_lookup_for_internet_is_normal_protection() {
    let entry = android_permission_for_capability("internet");
    assert!(entry.is_some());
    if let Some(entry) = entry {
        assert_eq!(entry.manifest_name, "android.permission.INTERNET");
        assert_eq!(entry.protection_level, ProtectionLevel::Normal);
    }
}

#[test]
fn android_lookup_for_fine_location_is_dangerous() {
    let entry = android_permission_for_capability("location.fine");
    assert!(entry.is_some());
    if let Some(entry) = entry {
        assert_eq!(
            entry.manifest_name,
            "android.permission.ACCESS_FINE_LOCATION"
        );
        assert_eq!(entry.protection_level, ProtectionLevel::Dangerous);
    }
}

#[test]
fn unknown_capability_returns_none_on_both_platforms() {
    assert!(ios_permission_for_capability("definitely-not-real").is_none());
    assert!(android_permission_for_capability("definitely-not-real").is_none());
}

#[test]
fn every_catalogue_entry_round_trips_through_lookup() {
    for entry in ios_catalogue() {
        let looked_up = ios_permission_for_capability(entry.key);
        assert_eq!(
            looked_up,
            Some(*entry),
            "iOS round-trip mismatch for `{}`",
            entry.key
        );
    }
    for entry in android_catalogue() {
        let looked_up = android_permission_for_capability(entry.key);
        assert_eq!(
            looked_up,
            Some(*entry),
            "Android round-trip mismatch for `{}`",
            entry.key
        );
    }
}
