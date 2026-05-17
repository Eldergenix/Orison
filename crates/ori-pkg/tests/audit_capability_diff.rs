use std::fs;

use ori_pkg::audit::{run_audit, AuditSeverity};
use ori_pkg::manifest::Manifest;
use ori_pkg::resolver::resolve;

#[test]
fn missing_root_capability_produces_error_finding() {
    // Unique temp dir per test run to avoid parallel collisions.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("ori_pkg_audit_{}_{}", std::process::id(), nanos));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(tmp.join("root")).expect("mkdir root");
    fs::create_dir_all(tmp.join("child")).expect("mkdir child");

    fs::write(
        tmp.join("root/ori.toml"),
        r#"schema = "ori.manifest.v1"
[package]
name = "root"
version = "0.1.0"
edition = "2027.1"
[capabilities]
declared = ["fs.read"]

[dependencies.child]
path = "../child"
"#,
    )
    .expect("write root manifest");

    fs::write(
        tmp.join("child/ori.toml"),
        r#"schema = "ori.manifest.v1"
[package]
name = "child"
version = "0.1.0"
edition = "2027.1"
[capabilities]
declared = ["net.fetch"]
"#,
    )
    .expect("write child manifest");

    let manifest = Manifest::from_path(&tmp.join("root/ori.toml")).expect("parse");
    let graph = resolve(&manifest, &tmp.join("root")).expect("resolve");
    let report = run_audit(&manifest, &graph);

    // Expect at least one error finding for net.fetch.
    let errors: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.severity == AuditSeverity::Error)
        .collect();
    assert!(
        !errors.is_empty(),
        "expected error findings, got {:?}",
        report
    );
    assert!(errors.iter().any(|f| f.id == "AUD0001"));
    assert!(errors.iter().any(|f| f.message.contains("net.fetch")));
    assert_eq!(report.summary.fail, errors.len() as u32);

    // Also expect an info finding for fs.read which is declared but unused.
    let info: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.severity == AuditSeverity::Info)
        .collect();
    assert!(info.iter().any(|f| f.message.contains("fs.read")));

    let _ = fs::remove_dir_all(&tmp);
}
