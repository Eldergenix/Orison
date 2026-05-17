use ori_pkg::manifest::{Manifest, ManifestErrorKind};
use ori_pkg::toml_lite::TomlErrorKind;

#[test]
fn invalid_toml_reports_position() {
    let bad = "name = \"unterminated\n";
    let err = Manifest::parse(bad).expect_err("expected parse failure");
    match err.kind {
        ManifestErrorKind::Toml(TomlErrorKind::UnterminatedString) => {}
        other => panic!("unexpected error kind: {other:?}"),
    }
    assert_eq!(err.line, Some(1));
    assert!(err.column.is_some());
}

#[test]
fn missing_required_keys_are_reported() {
    let text = "[package]\nname = \"a\"\n";
    let err = Manifest::parse(text).expect_err("expected validation failure");
    match err.kind {
        ManifestErrorKind::MissingKey(_) => {}
        other => panic!("unexpected error kind: {other:?}"),
    }
}

#[test]
fn duplicate_keys_are_rejected() {
    let text = "[package]\nname = \"a\"\nname = \"b\"\nversion = \"0.1.0\"\nedition = \"x\"\n";
    let err = Manifest::parse(text).expect_err("expected duplicate");
    matches!(
        err.kind,
        ManifestErrorKind::Toml(TomlErrorKind::DuplicateKey(_))
    );
    assert_eq!(err.line, Some(3));
}

#[test]
fn unknown_section_is_rejected() {
    let err = Manifest::parse("[zzz]\n").expect_err("expected error");
    match err.kind {
        ManifestErrorKind::UnknownSection(s) => assert_eq!(s, "zzz"),
        other => panic!("unexpected error kind: {other:?}"),
    }
}
