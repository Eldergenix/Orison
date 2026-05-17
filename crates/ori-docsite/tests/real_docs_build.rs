//! Build the real `docs/` tree into `target/docsite/` and assert basic shape.

#![allow(clippy::assertions_on_constants)]

use std::fs;
use std::path::PathBuf;

use ori_docsite::build_site;

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    match manifest.parent().and_then(|p| p.parent()) {
        Some(p) => p.to_path_buf(),
        None => manifest,
    }
}

fn count_files(root: &PathBuf) -> usize {
    let mut count = 0usize;
    let mut stack = vec![root.clone()];
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
                count += 1;
            }
        }
    }
    count
}

#[test]
fn build_real_docs_into_target_docsite() {
    let root = workspace_root();
    let input = root.join("docs");
    if !input.is_dir() {
        // Repository layout missing; skip rather than fail spuriously.
        assert!(false, "docs/ directory missing at {}", input.display());
        return;
    }
    let out = root.join("target/docsite");
    if out.exists() {
        let _ = fs::remove_dir_all(&out);
    }
    let report = match build_site(&input, &out) {
        Ok(r) => r,
        Err(e) => {
            assert!(false, "build_site failed on real docs: {e}");
            return;
        }
    };
    assert!(report.pages > 30, "expected >30 pages, got {}", report.pages);
    assert_eq!(report.assets, 1);
    let total = count_files(&out);
    assert!(total > 30, "expected >30 output files, got {total}");
    let css = out.join("style.css");
    assert!(css.is_file(), "style.css missing in real docs build");
}
