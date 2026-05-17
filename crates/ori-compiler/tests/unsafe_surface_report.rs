//! Unsafe-surface report.
//!
//! The bootstrap forbids `unsafe` Rust everywhere under `crates/*/src/` (see
//! the `FORBIDDEN_SOURCE_PATTERNS` table in `scripts/validate_all.py`). This
//! test walks the same source tree with a stdlib `read_dir` recursion (no
//! `walkdir` dep — see workspace dep policy in `MEMORY.md` D002),
//! synthesises a JSON report under the same schema shape the audit tooling
//! consumes, and asserts that `unsafe_count == 0`.
//!
//! When a future commit introduces an approved `unsafe` block, this test
//! will need an explicit exception list updated alongside MEMORY.md.

use std::fs;
use std::path::{Path, PathBuf};

/// Walk the workspace `crates/` tree, then return every `.rs` file under any
/// `<crate>/src/` subdirectory.
fn collect_src_rs_files(crates_dir: &Path) -> Vec<PathBuf> {
    let mut acc: Vec<PathBuf> = Vec::new();
    let entries = match fs::read_dir(crates_dir) {
        Ok(it) => it,
        Err(_) => return acc,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let src_dir = path.join("src");
        if src_dir.is_dir() {
            walk_rs(&src_dir, &mut acc);
        }
    }
    // Sort so the report is deterministic regardless of filesystem ordering.
    acc.sort();
    acc
}

fn walk_rs(dir: &Path, acc: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(it) => it,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_rs(&path, acc);
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            acc.push(path);
        }
    }
}

/// Strip `//` and `/* ... */` runs so a comment that mentions
/// `unsafe fn` does not show up in the surface report. The implementation is
/// intentionally tiny — it only needs to be precise enough to give a
/// stable count for the bootstrap codebase.
fn strip_comments_and_strings(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        let next = if i + 1 < bytes.len() {
            bytes[i + 1] as char
        } else {
            '\0'
        };
        if c == '/' && next == '/' {
            while i < bytes.len() && bytes[i] as char != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && next == '*' {
            i += 2;
            let mut depth = 1;
            while i + 1 < bytes.len() && depth > 0 {
                if bytes[i] as char == '/' && bytes[i + 1] as char == '*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] as char == '*' && bytes[i + 1] as char == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if c == '"' {
            out.push(' ');
            i += 1;
            while i < bytes.len() {
                let ch = bytes[i] as char;
                if ch == '\\' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                if ch == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Returns the count of `unsafe fn|impl|trait|{` occurrences inside `text`
/// after comment/string elision. Matches the same regex
/// (`\bunsafe\s+(fn|impl|trait|\{)`) used by `scripts/validate_all.py`.
fn count_unsafe_surface(text: &str) -> usize {
    let scrubbed = strip_comments_and_strings(text);
    let mut count = 0usize;
    let bytes = scrubbed.as_bytes();
    let needle = b"unsafe";
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            // Word-boundary before.
            let boundary_left = i == 0 || !is_word(bytes[i - 1] as char);
            // Whitespace(s) then one of fn|impl|trait|{.
            let mut j = i + needle.len();
            let mut saw_ws = false;
            while j < bytes.len() {
                let cj = bytes[j] as char;
                if cj.is_whitespace() {
                    saw_ws = true;
                    j += 1;
                } else {
                    break;
                }
            }
            if boundary_left && saw_ws && j < bytes.len() {
                let rest = &bytes[j..];
                let hits_fn = rest.starts_with(b"fn") && is_kw_boundary(rest, 2);
                let hits_impl = rest.starts_with(b"impl") && is_kw_boundary(rest, 4);
                let hits_trait = rest.starts_with(b"trait") && is_kw_boundary(rest, 5);
                let hits_brace = rest.first() == Some(&b'{');
                if hits_fn || hits_impl || hits_trait || hits_brace {
                    count += 1;
                    i = j;
                    continue;
                }
            }
        }
        i += 1;
    }
    count
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn is_kw_boundary(rest: &[u8], kw_len: usize) -> bool {
    match rest.get(kw_len) {
        None => true,
        Some(b) => !is_word(*b as char),
    }
}

fn workspace_crates_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p
}

#[derive(Debug)]
struct UnsafeReport {
    schema: &'static str,
    scanned_file_count: usize,
    unsafe_count: usize,
    offenders: Vec<(String, usize)>,
}

fn report_to_json(report: &UnsafeReport) -> serde_json::Value {
    let offenders: Vec<serde_json::Value> = report
        .offenders
        .iter()
        .map(|(path, count)| {
            serde_json::json!({
                "path": path,
                "unsafe_count": count,
            })
        })
        .collect();
    serde_json::json!({
        "schema": report.schema,
        "scanned_file_count": report.scanned_file_count,
        "unsafe_count": report.unsafe_count,
        "offenders": offenders,
    })
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn workspace_has_zero_unsafe_surface() {
    let crates_dir = workspace_crates_dir();
    let files = collect_src_rs_files(&crates_dir);
    assert!(
        !files.is_empty(),
        "found no .rs source files under {}",
        crates_dir.display()
    );

    let mut offenders: Vec<(String, usize)> = Vec::new();
    let mut total = 0usize;
    for file in &files {
        let text = match fs::read_to_string(file) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let count = count_unsafe_surface(&text);
        if count > 0 {
            let rel = file
                .strip_prefix(&crates_dir)
                .unwrap_or(file.as_path())
                .to_string_lossy()
                .into_owned();
            offenders.push((rel, count));
            total += count;
        }
    }
    offenders.sort();

    let report = UnsafeReport {
        schema: "ori.unsafe_surface.v1",
        scanned_file_count: files.len(),
        unsafe_count: total,
        offenders,
    };
    let json = report_to_json(&report);
    let serialised = match serde_json::to_string_pretty(&json) {
        Ok(s) => s,
        Err(err) => {
            assert!(false, "serialise unsafe report failed: {err}");
            return;
        }
    };

    // Report shape must be stable: the diagnostic tooling consumes these
    // fields by name.
    assert_eq!(
        json.get("schema").and_then(|v| v.as_str()),
        Some("ori.unsafe_surface.v1"),
        "schema header must be stable"
    );
    assert!(
        json.get("scanned_file_count").is_some(),
        "report must publish scanned_file_count"
    );
    assert!(
        json.get("offenders").and_then(|v| v.as_array()).is_some(),
        "report must publish an offenders array"
    );

    assert_eq!(
        report.unsafe_count, 0,
        "bootstrap forbids `unsafe`; report was:\n{serialised}"
    );
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn counter_recognises_known_unsafe_forms() {
    // Defensive: the test above is only meaningful if the counter actually
    // catches what `scripts/validate_all.py` would catch.
    let cases = [
        ("unsafe fn raw() {}", 1usize),
        ("unsafe impl Send for X {}", 1),
        ("unsafe trait Marker {}", 1),
        ("unsafe { 0 }", 1),
        ("let s = \"unsafe fn in string\";", 0),
        ("// unsafe fn in comment\n", 0),
        ("/* unsafe fn block */ fn x() {}", 0),
        ("unsafely fn x() {}", 0),
        ("fn unsafe_helper() {}", 0),
    ];
    for (text, want) in cases {
        let got = count_unsafe_surface(text);
        assert_eq!(
            got, want,
            "count_unsafe_surface({text:?}) returned {got}, want {want}"
        );
    }
}
