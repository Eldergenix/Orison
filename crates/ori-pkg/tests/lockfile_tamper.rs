//! Lockfile tamper detection contract.
//!
//! `build_lockfile` is deterministic for a given manifest + filesystem
//! layout (see `crates/ori-pkg/src/lockfile.rs`). That property gives the
//! supply chain a cheap tamper-detection primitive: rebuild from the
//! manifest, diff the serialised JSON against the committed lockfile, and
//! flag any divergence. This test demonstrates the contract by mutating a
//! single checksum field in the serialised lockfile and asserting that the
//! re-parsed document no longer matches a freshly built one.

use std::fs;
use std::path::{Path, PathBuf};

use ori_pkg::lockfile::{build_lockfile, Lockfile};
use ori_pkg::manifest::Manifest;

fn scratch(name: &str) -> PathBuf {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("ori_pkg_lockfile_tamper_{name}_{pid}_{nanos}"));
    let _ = fs::remove_dir_all(&dir);
    dir
}

#[allow(clippy::assertions_on_constants)]
fn write_manifest(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            assert!(false, "create_dir_all({}) failed: {err}", parent.display());
            return;
        }
    }
    if let Err(err) = fs::write(path, body) {
        assert!(false, "write({}) failed: {err}", path.display());
    }
}

#[allow(clippy::assertions_on_constants)]
fn write_synthetic_graph(tmp: &Path) {
    write_manifest(
        &tmp.join("root/ori.toml"),
        r#"schema = "ori.manifest.v1"
[package]
name = "tamper_root"
version = "0.1.0"
edition = "2027.1"
[capabilities]
declared = ["fs.read"]

[dependencies.leaf]
path = "../leaf"
"#,
    );
    write_manifest(
        &tmp.join("leaf/ori.toml"),
        r#"schema = "ori.manifest.v1"
[package]
name = "leaf"
version = "0.2.0"
edition = "2027.1"
[capabilities]
declared = ["fs.read"]
"#,
    );
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn rebuilt_lockfile_matches_pristine_serialisation() {
    // Sanity: without tampering the diff must be empty.
    let tmp = scratch("baseline");
    write_synthetic_graph(&tmp);

    let manifest = match Manifest::from_path(&tmp.join("root/ori.toml")) {
        Ok(m) => m,
        Err(err) => {
            assert!(false, "manifest parse failed: {err}");
            let _ = fs::remove_dir_all(&tmp);
            return;
        }
    };
    let pristine = match build_lockfile(&manifest, &tmp.join("root")) {
        Ok(l) => l,
        Err(err) => {
            assert!(false, "build_lockfile failed: {err}");
            let _ = fs::remove_dir_all(&tmp);
            return;
        }
    };
    let rebuilt = match build_lockfile(&manifest, &tmp.join("root")) {
        Ok(l) => l,
        Err(err) => {
            assert!(false, "second build_lockfile failed: {err}");
            let _ = fs::remove_dir_all(&tmp);
            return;
        }
    };
    assert_eq!(
        pristine, rebuilt,
        "build_lockfile must be deterministic across calls"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn checksum_tamper_is_detected_on_reparse() {
    let tmp = scratch("checksum");
    write_synthetic_graph(&tmp);

    let manifest = match Manifest::from_path(&tmp.join("root/ori.toml")) {
        Ok(m) => m,
        Err(err) => {
            assert!(false, "manifest parse failed: {err}");
            let _ = fs::remove_dir_all(&tmp);
            return;
        }
    };
    let pristine = match build_lockfile(&manifest, &tmp.join("root")) {
        Ok(l) => l,
        Err(err) => {
            assert!(false, "build_lockfile failed: {err}");
            let _ = fs::remove_dir_all(&tmp);
            return;
        }
    };

    // Serialise to JSON, then mutate the first non-empty checksum value via
    // serde without touching any non-checksum field.
    let mut value = match serde_json::to_value(&pristine) {
        Ok(v) => v,
        Err(err) => {
            assert!(false, "serialize pristine failed: {err}");
            let _ = fs::remove_dir_all(&tmp);
            return;
        }
    };

    let mut mutated_target: Option<String> = None;
    if let Some(packages) = value
        .get_mut("packages")
        .and_then(serde_json::Value::as_array_mut)
    {
        for pkg in packages.iter_mut() {
            // Choose the `leaf` package so the test does not depend on
            // root-vs-dep ordering inside the packages array.
            let name = pkg
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            if name.as_deref() == Some("leaf") {
                if let Some(sum_value) = pkg.get_mut("checksum") {
                    let original = sum_value.as_str().unwrap_or("").to_string();
                    // Flip the leading hex character so the digest is still a
                    // valid hex string but no longer matches the pristine.
                    let mut chars: Vec<char> = original.chars().collect();
                    assert!(!chars.is_empty(), "checksum was empty; nothing to mutate");
                    chars[0] = if chars[0] == 'f' { '0' } else { 'f' };
                    let new_value: String = chars.into_iter().collect();
                    assert!(
                        new_value != original,
                        "tampering produced identical checksum"
                    );
                    *sum_value = serde_json::Value::String(new_value);
                    mutated_target = Some(name.unwrap_or_default());
                    break;
                }
            }
        }
    }
    assert!(
        mutated_target.is_some(),
        "test setup must include a `leaf` package with a checksum field"
    );

    // Re-parse the tampered JSON via serde and compare with a fresh build.
    let tampered: Lockfile = match serde_json::from_value(value) {
        Ok(l) => l,
        Err(err) => {
            assert!(false, "re-parse tampered JSON failed: {err}");
            let _ = fs::remove_dir_all(&tmp);
            return;
        }
    };
    let fresh = match build_lockfile(&manifest, &tmp.join("root")) {
        Ok(l) => l,
        Err(err) => {
            assert!(false, "rebuild fresh lockfile failed: {err}");
            let _ = fs::remove_dir_all(&tmp);
            return;
        }
    };

    assert_ne!(
        tampered, fresh,
        "tampered lockfile must NOT equal a freshly built one"
    );

    // Locate the divergence: exactly one package by name must differ.
    let mut differing: Vec<&str> = Vec::new();
    for pkg_fresh in &fresh.packages {
        let matched = tampered
            .packages
            .iter()
            .find(|p| p.name == pkg_fresh.name && p.version == pkg_fresh.version);
        match matched {
            Some(p) if p.checksum != pkg_fresh.checksum => {
                differing.push(pkg_fresh.name.as_str());
            }
            Some(_) => {}
            None => {
                differing.push(pkg_fresh.name.as_str());
            }
        }
    }
    assert_eq!(
        differing.len(),
        1,
        "exactly one package checksum should differ, got {differing:?}"
    );
    assert_eq!(
        differing.first().copied(),
        Some("leaf"),
        "the diverging package must be the one we tampered"
    );

    // And critically: the schema header itself must be unchanged so the
    // diff tool can still recognise the document.
    assert_eq!(
        tampered.schema, fresh.schema,
        "tampering checksum must not change schema"
    );
    assert_eq!(
        tampered.format_version, fresh.format_version,
        "tampering checksum must not change format_version"
    );

    let _ = fs::remove_dir_all(&tmp);
}
