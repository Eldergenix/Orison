use std::path::PathBuf;

use ori_pkg::lockfile::build_lockfile;
use ori_pkg::manifest::Manifest;

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

#[test]
fn lockfile_is_byte_identical_across_runs() {
    let root = repo_root();
    let text = std::fs::read_to_string(root.join("ori.toml")).expect("read manifest");
    let manifest = Manifest::parse(&text).expect("parse manifest");
    let lock_a = build_lockfile(&manifest, &root).expect("build a");
    let lock_b = build_lockfile(&manifest, &root).expect("build b");
    let json_a = serde_json::to_string_pretty(&lock_a).expect("serialise a");
    let json_b = serde_json::to_string_pretty(&lock_b).expect("serialise b");
    assert_eq!(json_a, json_b, "lockfile JSON must be deterministic");

    // Hashing the body must also match.
    let hash_a = simple_hash(&json_a);
    let hash_b = simple_hash(&json_b);
    assert_eq!(hash_a, hash_b);

    // Schema and format_version sanity.
    assert_eq!(lock_a.schema, "ori.lockfile.v1");
    assert_eq!(lock_a.format_version, 1);
    // Root package must be present.
    assert!(lock_a.packages.iter().any(|p| p.name == "orison.bootstrap"));
}

fn simple_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}
