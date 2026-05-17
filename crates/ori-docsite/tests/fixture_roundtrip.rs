//! Full-site round-trip on the small fixture in `tools/docsite-fixtures/simple/`.

#![allow(clippy::assertions_on_constants)]

use std::fs;
use std::path::PathBuf;

use ori_docsite::build_site;

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR points at crates/ori-docsite/; walk up twice.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = match manifest.parent().and_then(|p| p.parent()) {
        Some(p) => p.to_path_buf(),
        None => manifest,
    };
    root
}

#[test]
fn fixture_builds_into_temp_dir() {
    let root = workspace_root();
    let input = root.join("tools/docsite-fixtures/simple");
    let out = root.join("target/docsite-test-fixture");
    if out.exists() {
        let _ = fs::remove_dir_all(&out);
    }
    let report = match build_site(&input, &out) {
        Ok(r) => r,
        Err(e) => {
            assert!(false, "build_site failed: {e}");
            return;
        }
    };
    assert_eq!(report.pages, 3, "expected 3 pages, got {}", report.pages);
    assert_eq!(report.assets, 1);
    assert!(report.bytes_written > 0);

    // Verify each expected file exists.
    let index_html = out.join("index.html");
    assert!(index_html.is_file(), "missing {}", index_html.display());
    let about_html = out.join("about.html");
    assert!(about_html.is_file(), "missing {}", about_html.display());
    let nested = out.join("sub/page.html");
    assert!(nested.is_file(), "missing {}", nested.display());
    let css = out.join("style.css");
    assert!(css.is_file(), "missing {}", css.display());

    // The nested page should reference `../style.css`.
    let nested_html = match fs::read_to_string(&nested) {
        Ok(s) => s,
        Err(e) => {
            assert!(false, "could not read nested page: {e}");
            return;
        }
    };
    assert!(
        nested_html.contains("href=\"../style.css\""),
        "expected ../style.css link in nested page: {nested_html}"
    );

    // The top-level page should reference `style.css`.
    let top_html = match fs::read_to_string(&index_html) {
        Ok(s) => s,
        Err(e) => {
            assert!(false, "could not read index: {e}");
            return;
        }
    };
    assert!(
        top_html.contains("href=\"style.css\""),
        "expected style.css link in top page"
    );
    // Body content from index.md should be present in some form.
    assert!(top_html.contains("Fixture Home"), "missing title content");
    assert!(top_html.contains("<table>"), "table not rendered");
    assert!(top_html.contains("<pre><code"), "code block not rendered");
}
