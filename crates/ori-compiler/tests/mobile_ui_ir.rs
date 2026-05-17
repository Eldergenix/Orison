//! Integration tests for the native UI IR.
//!
//! Exercises the public surface of [`ori_compiler::mobile_ui_ir`] including
//! per-target prop mappings, JSON serialisation, and the optional embedded
//! `native_ui_manifest` field on the mobile manifest.

use ori_compiler::mobile::build_mobile_manifest_with_ui;
use ori_compiler::mobile_ui_ir::{build_native_ui_manifest, derive_native_ui_specs, NativeUiKind};
use ori_compiler::parser::parse_source;
use ori_compiler::source::SourceFile;

fn module_for(text: &str) -> ori_compiler::ast::Module {
    parse_source(&SourceFile::new("/t.ori", text)).module
}

#[test]
fn derives_two_specs_for_each_target() {
    let module =
        module_for("module demo\nview ProductDetail(product: Product)\nview UserCard(name: Str)");
    for kind in [
        NativeUiKind::iOS_UIKit,
        NativeUiKind::iOS_SwiftUI,
        NativeUiKind::Android_Compose,
        NativeUiKind::Android_View,
    ] {
        let specs = derive_native_ui_specs(&module, kind);
        assert_eq!(
            specs.len(),
            2,
            "expected 2 specs for target `{}`, got {}",
            kind.as_str(),
            specs.len()
        );
    }
}

#[test]
fn manifest_json_contains_stable_schema_and_target() {
    let module = module_for("module demo\nview ProductDetail(product: Product)");
    let json = build_native_ui_manifest(&module, NativeUiKind::Android_Compose).to_json();
    assert!(json.contains("\"schema\":\"ori.native_ui_manifest.v1\""));
    assert!(json.contains("\"target\":\"android-compose\""));
    assert!(json.contains("\"view_name\":\"ProductDetail\""));
}

#[test]
fn uikit_prop_mapping_targets_view_data_source() {
    let module = module_for("module demo\nview ProductDetail(product: Product)");
    let specs = derive_native_ui_specs(&module, NativeUiKind::iOS_UIKit);
    let first = specs.into_iter().next();
    assert!(first.is_some(), "expected at least one spec");
    if let Some(spec) = first {
        let mapped = spec.props_mapping.get("product").cloned();
        assert_eq!(
            mapped.as_deref(),
            Some("UIViewController.productDataSource")
        );
    }
}

#[test]
fn cli_string_round_trips_for_every_target() {
    for kind in [
        NativeUiKind::iOS_UIKit,
        NativeUiKind::iOS_SwiftUI,
        NativeUiKind::Android_Compose,
        NativeUiKind::Android_View,
    ] {
        let s = kind.as_str();
        assert_eq!(NativeUiKind::from_cli_str(s), Some(kind));
    }
    assert!(NativeUiKind::from_cli_str("totally-bogus").is_none());
}

#[test]
fn mobile_manifest_with_ui_embeds_native_ui_manifest() {
    let module = module_for("module demo\nview ProductDetail(product: Product)");
    let manifest = build_mobile_manifest_with_ui(
        &module,
        "com.example.demo",
        &["ios"],
        Some(NativeUiKind::iOS_SwiftUI),
    );
    assert!(manifest.native_ui_manifest.is_some());
    let json = manifest.to_json();
    assert!(json.contains("\"native_ui_manifest\""));
    assert!(json.contains("\"target\":\"ios-swiftui\""));
}

#[test]
fn mobile_manifest_without_ui_omits_native_field() {
    let module = module_for("module demo\nview ProductDetail(product: Product)");
    let manifest = build_mobile_manifest_with_ui(&module, "com.example.demo", &["ios"], None);
    assert!(manifest.native_ui_manifest.is_none());
    let json = manifest.to_json();
    assert!(!json.contains("native_ui_manifest"));
}
