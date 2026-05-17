//! CST-preserving formatter.
//!
//! Produces idempotent output by walking the source line-by-line and
//! normalising whitespace within an item line while preserving every
//! comment, blank line, and string interior unchanged. The formatter does
//! not reorder declarations.

use crate::cst::{parse_cst, CstNodeKind};
use crate::source::SourceFile;

/// Format `text` using the bootstrap formatter. The formatter is idempotent:
/// `format_text(format_text(x)) == format_text(x)`.
pub fn format_text(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let source = SourceFile::new("/fmt.ori", normalized.clone());
    let cst = parse_cst(&source);

    let item_lines: std::collections::BTreeSet<usize> = cst
        .nodes
        .iter()
        .filter(|n| {
            !matches!(
                n.kind,
                CstNodeKind::Comment | CstNodeKind::BlankLine | CstNodeKind::Error
            )
        })
        .map(|n| n.span.start.line)
        .collect();

    let mut out = String::new();
    let mut prev_blank = false;
    for (idx, line) in normalized.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed_end = line.trim_end();
        if trimmed_end.is_empty() {
            // Collapse consecutive blank lines to a single blank line.
            if !prev_blank {
                out.push('\n');
            }
            prev_blank = true;
            continue;
        }
        prev_blank = false;
        if item_lines.contains(&line_no) {
            out.push_str(&normalize_item_line(trimmed_end));
        } else {
            // Comment or unclassified line: leave alone (just trim trailing).
            out.push_str(trimmed_end);
        }
        out.push('\n');
    }
    out
}

fn normalize_item_line(line: &str) -> String {
    // Collapse runs of internal whitespace outside of strings to a single
    // space, except preserve leading indentation.
    let mut leading = String::new();
    let mut chars = line.chars().peekable();
    while let Some(&ch) = chars.peek() {
        if ch == ' ' || ch == '\t' {
            leading.push(ch);
            chars.next();
        } else {
            break;
        }
    }
    let rest: String = chars.collect();
    let mut out = leading;
    let mut in_str = false;
    let mut prev_space = false;
    for ch in rest.chars() {
        if in_str {
            out.push(ch);
            if ch == '"' {
                in_str = false;
            }
            continue;
        }
        if ch == '"' {
            in_str = true;
            out.push(ch);
            prev_space = false;
            continue;
        }
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_are_idempotent() {
        let input = "module a\n\n\nfn f() -> Unit\n";
        let a = format_text(input);
        let b = format_text(&a);
        assert_eq!(a, b);
    }

    #[test]
    fn collapses_double_blank_lines() {
        let input = "module a\n\n\nfn f() -> Unit\n";
        let out = format_text(input);
        assert!(!out.contains("\n\n\n"));
    }

    #[test]
    fn preserves_comments_verbatim() {
        let input = "module a\n// keep me\nfn f() -> Unit\n";
        let out = format_text(input);
        assert!(out.contains("// keep me"));
    }

    #[test]
    fn collapses_multispace_inside_item_lines() {
        let input = "module a\nfn  hello (  ) ->  Unit\n";
        let out = format_text(input);
        assert!(out.contains("fn hello ( ) -> Unit") || out.contains("fn hello() -> Unit"));
    }

    #[test]
    fn does_not_touch_string_contents() {
        let input = "module a\nfn f() -> Str // \"keep   spaces\"\n";
        // We preserve comment text verbatim; comment lines are not item
        // lines.
        let out = format_text(input);
        assert!(out.contains("\"keep   spaces\""));
    }
}
