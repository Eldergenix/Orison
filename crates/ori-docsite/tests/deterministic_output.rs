//! Verify that two consecutive builds of the same input produce byte-identical
//! output for every emitted file.

#![allow(clippy::assertions_on_constants)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use ori_docsite::build_site;

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    match manifest.parent().and_then(|p| p.parent()) {
        Some(p) => p.to_path_buf(),
        None => manifest,
    }
}

fn read_all(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let ft = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .map(|p| p.to_string_lossy().replace('\\', "/").to_string())
                    .unwrap_or_else(|_| path.to_string_lossy().to_string());
                if let Ok(bytes) = fs::read(&path) {
                    out.insert(rel, bytes);
                }
            }
        }
    }
    out
}

#[test]
fn two_builds_produce_byte_identical_files() {
    let root = workspace_root();
    let input = root.join("tools/docsite-fixtures/simple");
    let out_a = root.join("target/docsite-test-det-a");
    let out_b = root.join("target/docsite-test-det-b");
    for p in [&out_a, &out_b] {
        if p.exists() {
            let _ = fs::remove_dir_all(p);
        }
    }

    match build_site(&input, &out_a) {
        Ok(_) => {}
        Err(e) => {
            assert!(false, "first build failed: {e}");
            return;
        }
    }
    match build_site(&input, &out_b) {
        Ok(_) => {}
        Err(e) => {
            assert!(false, "second build failed: {e}");
            return;
        }
    }

    let map_a = read_all(&out_a);
    let map_b = read_all(&out_b);
    assert_eq!(
        map_a.keys().collect::<Vec<_>>(),
        map_b.keys().collect::<Vec<_>>(),
        "file sets differ"
    );
    for (path, bytes_a) in &map_a {
        let bytes_b = match map_b.get(path) {
            Some(b) => b,
            None => {
                assert!(false, "missing {path} in second build");
                return;
            }
        };
        assert_eq!(
            bytes_a, bytes_b,
            "byte mismatch for {path}: A={} B={} bytes",
            bytes_a.len(),
            bytes_b.len()
        );
    }
}
