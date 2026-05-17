use std::path::PathBuf;

use ori_pkg::manifest::Manifest;

fn repo_manifest_path() -> PathBuf {
    // CARGO_MANIFEST_DIR for ori-pkg is `<repo>/crates/ori-pkg`.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("ori.toml");
    p
}

#[test]
fn parses_repo_manifest() {
    let path = repo_manifest_path();
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read manifest {}: {err}", path.display()));
    let manifest = Manifest::parse(&text).expect("parse manifest");
    assert_eq!(manifest.package.name, "orison.bootstrap");
    assert_eq!(manifest.package.version, "0.1.1");
    assert_eq!(manifest.schema, "ori.manifest.v1");
}

#[test]
fn serialize_then_deserialize_roundtrips() {
    let path = repo_manifest_path();
    let text = std::fs::read_to_string(&path).expect("read manifest");
    let manifest = Manifest::parse(&text).expect("parse manifest");
    let json = serde_json::to_string(&manifest).expect("serialize");
    let again: Manifest = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(manifest, again);
}
