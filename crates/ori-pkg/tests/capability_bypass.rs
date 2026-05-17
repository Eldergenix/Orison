//! Capability bypass audit tests.
//!
//! These exercise the two mismatch directions enforced by `run_audit`:
//!
//! * `AUD0001` (error) — a dependency requires a capability that the root
//!   manifest does NOT declare. This is the "bypass" path: a transitive
//!   package could exercise an undeclared trust surface unless the audit
//!   fails the build.
//! * `AUD0002` (info) — the root declares a capability that no dependency
//!   actually needs. This is the "over-declaration" path: the trust surface
//!   is wider than necessary.
//!
//! The graph is synthesised on disk in a per-test temp directory so the
//! resolver can walk path dependencies without depending on workspace state.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use ori_pkg::audit::{run_audit, AuditFinding, AuditSeverity};
use ori_pkg::manifest::Manifest;
use ori_pkg::resolver::resolve;

/// Per-test scratch directory. The PID + nanos suffix keeps parallel cargo
/// runs (within the same binary AND across processes) from racing on the
/// same path. The unique name also means we never need to clean stale
/// directories left behind by previous interrupted runs.
fn scratch(name: &str) -> PathBuf {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("ori_pkg_cap_bypass_{name}_{pid}_{nanos}"));
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
fn load(path: &Path) -> Manifest {
    match Manifest::from_path(path) {
        Ok(m) => m,
        Err(err) => {
            assert!(
                false,
                "Manifest::from_path({}) failed: {err}",
                path.display()
            );
            // Unreachable, but the type system needs a return.
            Manifest {
                schema: String::new(),
                package: ori_pkg::manifest::PackageMeta {
                    name: String::new(),
                    version: String::new(),
                    edition: String::new(),
                    description: None,
                    license: None,
                },
                capabilities: ori_pkg::manifest::CapabilityDecls::default(),
                dependencies: Default::default(),
                scripts: Default::default(),
            }
        }
    }
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn dependency_requires_undeclared_capability_emits_error() {
    let tmp = scratch("bypass_error");
    let root_manifest_path = tmp.join("root/ori.toml");

    // Root declares only `fs.read` but the synthetic child needs `fs.write`.
    write_manifest(
        &root_manifest_path,
        r#"schema = "ori.manifest.v1"
[package]
name = "root_app"
version = "0.1.0"
edition = "2027.1"
[capabilities]
declared = ["fs.read"]

[dependencies.writer_dep]
path = "../writer_dep"
"#,
    );
    write_manifest(
        &tmp.join("writer_dep/ori.toml"),
        r#"schema = "ori.manifest.v1"
[package]
name = "writer_dep"
version = "0.3.2"
edition = "2027.1"
[capabilities]
declared = ["fs.write"]
"#,
    );

    let manifest = load(&root_manifest_path);
    let graph = match resolve(&manifest, &tmp.join("root")) {
        Ok(g) => g,
        Err(err) => {
            assert!(false, "resolve failed: {err}");
            let _ = fs::remove_dir_all(&tmp);
            return;
        }
    };

    let report = run_audit(&manifest, &graph);

    let bypass: Vec<&AuditFinding> = report
        .findings
        .iter()
        .filter(|f| f.id == "AUD0001" && f.severity == AuditSeverity::Error)
        .collect();
    assert!(
        !bypass.is_empty(),
        "expected AUD0001 error for undeclared fs.write; report was {report:?}"
    );
    assert!(
        bypass.iter().any(|f| f.message.contains("fs.write")),
        "AUD0001 finding should name `fs.write`, got {bypass:?}"
    );
    assert!(
        bypass
            .iter()
            .any(|f| f.target == "package:writer_dep@0.3.2"),
        "AUD0001 target should identify the dependency by name@version, got {bypass:?}"
    );

    let bypass_count = bypass.len() as u32;
    assert_eq!(
        report.summary.fail, bypass_count,
        "summary.fail must include every AUD0001 finding"
    );
    assert!(
        report.summary.fail >= 1,
        "summary.fail must increment for the bypass case"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn declared_but_unused_capability_emits_info() {
    let tmp = scratch("bypass_info");
    let root_manifest_path = tmp.join("root/ori.toml");

    // Root declares `net.outbound` and `fs.write` but the dep only uses
    // `fs.write`, so `net.outbound` is over-declared.
    write_manifest(
        &root_manifest_path,
        r#"schema = "ori.manifest.v1"
[package]
name = "over_declared"
version = "0.1.0"
edition = "2027.1"
[capabilities]
declared = ["fs.write", "net.outbound"]

[dependencies.user]
path = "../user"
"#,
    );
    write_manifest(
        &tmp.join("user/ori.toml"),
        r#"schema = "ori.manifest.v1"
[package]
name = "user"
version = "0.1.0"
edition = "2027.1"
[capabilities]
declared = ["fs.write"]
"#,
    );

    let manifest = load(&root_manifest_path);
    let graph = match resolve(&manifest, &tmp.join("root")) {
        Ok(g) => g,
        Err(err) => {
            assert!(false, "resolve failed: {err}");
            let _ = fs::remove_dir_all(&tmp);
            return;
        }
    };

    let report = run_audit(&manifest, &graph);

    // No errors expected: every required cap is declared.
    let errors: Vec<&AuditFinding> = report
        .findings
        .iter()
        .filter(|f| f.severity == AuditSeverity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "expected no error findings, got {errors:?}"
    );
    assert_eq!(
        report.summary.fail, 0,
        "summary.fail must be zero when every required capability is declared"
    );

    let info: Vec<&AuditFinding> = report
        .findings
        .iter()
        .filter(|f| f.id == "AUD0002" && f.severity == AuditSeverity::Info)
        .collect();
    let info_targets: BTreeSet<&str> = info.iter().map(|f| f.target.as_str()).collect();
    assert!(
        info.iter().any(|f| f.message.contains("net.outbound")),
        "AUD0002 must call out the unused `net.outbound` capability, got {info:?}"
    );
    assert!(
        info_targets.contains("package:over_declared"),
        "AUD0002 target must point at the root package, got {info_targets:?}"
    );
    assert!(
        !info.iter().any(|f| f.message.contains("`fs.write`")),
        "AUD0002 must NOT flag `fs.write` because the dep uses it, got {info:?}"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn audit_report_is_byte_stable_across_runs() {
    // Determinism guard: identical inputs must produce identical reports so
    // CI diffing works regardless of filesystem entry ordering.
    let tmp = scratch("bypass_stable");
    write_manifest(
        &tmp.join("root/ori.toml"),
        r#"schema = "ori.manifest.v1"
[package]
name = "stable_root"
version = "0.1.0"
edition = "2027.1"
[capabilities]
declared = ["fs.read"]

[dependencies.alpha]
path = "../alpha"

[dependencies.beta]
path = "../beta"
"#,
    );
    write_manifest(
        &tmp.join("alpha/ori.toml"),
        r#"schema = "ori.manifest.v1"
[package]
name = "alpha"
version = "0.1.0"
edition = "2027.1"
[capabilities]
declared = ["net.outbound"]
"#,
    );
    write_manifest(
        &tmp.join("beta/ori.toml"),
        r#"schema = "ori.manifest.v1"
[package]
name = "beta"
version = "0.1.0"
edition = "2027.1"
[capabilities]
declared = ["fs.write"]
"#,
    );

    let manifest = load(&tmp.join("root/ori.toml"));
    let graph = match resolve(&manifest, &tmp.join("root")) {
        Ok(g) => g,
        Err(err) => {
            assert!(false, "resolve failed: {err}");
            let _ = fs::remove_dir_all(&tmp);
            return;
        }
    };

    let a = run_audit(&manifest, &graph);
    let b = run_audit(&manifest, &graph);
    let json_a = match serde_json::to_string_pretty(&a) {
        Ok(s) => s,
        Err(err) => {
            assert!(false, "serialise a failed: {err}");
            let _ = fs::remove_dir_all(&tmp);
            return;
        }
    };
    let json_b = match serde_json::to_string_pretty(&b) {
        Ok(s) => s,
        Err(err) => {
            assert!(false, "serialise b failed: {err}");
            let _ = fs::remove_dir_all(&tmp);
            return;
        }
    };
    assert_eq!(
        json_a, json_b,
        "audit report must be byte-stable across runs"
    );

    let _ = fs::remove_dir_all(&tmp);
}
