//! Freshness gate for `docs/diagnostics/INDEX.md`.
//!
//! The diagnostic index is a machine-generated inventory of every stable
//! diagnostic ID emitted by the Orison toolchain. Drift between the
//! committed `docs/diagnostics/INDEX.md` and the current crate sources is
//! always a bug — either the index needs to be rebuilt, or a freshly
//! introduced diagnostic ID has not been wired into the index pipeline.
//!
//! This test shells out to the in-repo Python script
//! `scripts/build_diag_index.py` in `--check` mode. The script must:
//!   * walk every `crates/*/src/*.rs` file,
//!   * scan for canonical diagnostic ID literals, and
//!   * compare the resulting Markdown against the committed file.
//!
//! It exits `0` when the file is up to date and a non-zero status on
//! drift. We propagate that contract directly through `assert!`.
//!
//! The test deliberately requires Python 3.13 (the version available on
//! every supported developer toolchain and on the CI runners). If the
//! interpreter cannot be located the test is skipped so contributors who
//! do not have Python 3.13 on `PATH` can still iterate on the compiler
//! crate locally; CI installs a matching interpreter explicitly, so the
//! skip never masks drift in the gated environment.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Locate the workspace root by walking up from the crate manifest until
/// we find a directory that contains both `crates/` and `scripts/`.
fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut cur: &Path = manifest_dir.as_path();
    loop {
        if cur.join("crates").is_dir() && cur.join("scripts").is_dir() {
            return cur.to_path_buf();
        }
        match cur.parent() {
            Some(parent) => cur = parent,
            None => panic!(
                "could not locate Orison workspace root starting from {}",
                manifest_dir.display()
            ),
        }
    }
}

/// Probe a list of candidate Python interpreters and return the first
/// one whose `--version` output reports a 3.13.x release. Returns
/// `None` if no suitable interpreter is available.
fn find_python_3_13() -> Option<String> {
    let candidates = [
        "python3.13",
        "/opt/homebrew/bin/python3.13",
        "/usr/local/bin/python3.13",
        "/usr/bin/python3.13",
    ];
    for cand in candidates {
        let output = Command::new(cand).arg("--version").output();
        if let Ok(out) = output {
            if !out.status.success() {
                continue;
            }
            // `python --version` writes to stdout in 3.4+; older versions
            // wrote to stderr. Combine both so the check is robust.
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&out.stderr));
            if text.contains("Python 3.13") {
                return Some(cand.to_string());
            }
        }
    }
    None
}

#[test]
fn diag_index_matches_source_tree() {
    let python = match find_python_3_13() {
        Some(p) => p,
        None => {
            // No suitable interpreter; document the skip and bail. CI
            // pins Python 3.13 explicitly so this branch is never taken
            // there. Local skips are loud (printed in test output) but
            // do not fail the build.
            eprintln!(
                "diag_index_freshness: skipping — Python 3.13 not on PATH. \
                 Install python3.13 to run this gate locally."
            );
            return;
        }
    };

    let root = workspace_root();
    let script = root.join("scripts").join("build_diag_index.py");
    assert!(
        script.is_file(),
        "expected diagnostic index builder at {}",
        script.display()
    );

    let output = Command::new(python)
        .arg(&script)
        .arg("--check")
        .current_dir(&root)
        .output()
        .expect("failed to spawn diagnostic index builder");

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "docs/diagnostics/INDEX.md is out of date. \
             Run `python3.13 scripts/build_diag_index.py` to regenerate it.\n\
             --- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
        );
    }
}
