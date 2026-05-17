use std::fs;

use ori_pkg::manifest::Manifest;
use ori_pkg::resolver::{resolve, ResolveErrorKind};

#[test]
fn detects_two_node_cycle() {
    // Unique scratch dir per test invocation to avoid parallel-run collisions.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("ori_pkg_cycle_{}_{}", std::process::id(), nanos));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(tmp.join("a")).expect("mkdir a");
    fs::create_dir_all(tmp.join("b")).expect("mkdir b");

    fs::write(
        tmp.join("a/ori.toml"),
        r#"schema = "ori.manifest.v1"
[package]
name = "a"
version = "0.1.0"
edition = "2027.1"
[dependencies.b]
path = "../b"
"#,
    )
    .expect("write a manifest");

    fs::write(
        tmp.join("b/ori.toml"),
        r#"schema = "ori.manifest.v1"
[package]
name = "b"
version = "0.1.0"
edition = "2027.1"
[dependencies.a]
path = "../a"
"#,
    )
    .expect("write b manifest");

    let root_manifest = Manifest::from_path(&tmp.join("a/ori.toml")).expect("parse a");
    let err = resolve(&root_manifest, &tmp.join("a")).expect_err("cycle should be reported");
    match err.kind {
        ResolveErrorKind::Cycle(path) => {
            assert!(path.contains(&"a".to_string()));
            assert!(path.contains(&"b".to_string()));
            assert_eq!(
                path.first(),
                path.last(),
                "cycle path should start and end with the same node"
            );
        }
        other => panic!("expected cycle, got {other:?}"),
    }

    let _ = fs::remove_dir_all(&tmp);
}
