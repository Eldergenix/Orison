//! Tests for each markdown feature supported by `ori_docsite::render_markdown`.

#![allow(clippy::assertions_on_constants)]

use ori_docsite::render_markdown;

#[test]
fn atx_headers_emit_h_tags() {
    let html = render_markdown("# Hello\n## Sub\n### Three\n###### Six\n");
    assert!(html.contains("<h1>Hello</h1>"), "h1 missing in: {html}");
    assert!(html.contains("<h2>Sub</h2>"), "h2 missing in: {html}");
    assert!(html.contains("<h3>Three</h3>"), "h3 missing in: {html}");
    assert!(html.contains("<h6>Six</h6>"), "h6 missing in: {html}");
}

#[test]
fn unordered_lists_dash_and_star() {
    let html = render_markdown("- one\n- two\n\n* alpha\n* beta\n");
    assert!(html.contains("<ul>"), "ul missing");
    assert!(html.contains("<li>one</li>"), "first item missing");
    assert!(html.contains("<li>alpha</li>"), "star item missing");
    // Two separate lists separated by a blank line.
    assert!(
        html.matches("<ul>").count() >= 2,
        "expected two lists in: {html}"
    );
}

#[test]
fn ordered_lists_emit_ol() {
    let html = render_markdown("1. first\n2. second\n3. third\n");
    assert!(html.contains("<ol>"), "ol missing");
    assert!(html.contains("<li>first</li>"));
    assert!(html.contains("<li>third</li>"));
}

#[test]
fn fenced_code_block_with_language_tag() {
    let html = render_markdown("```rust\nfn main() {}\n```\n");
    assert!(
        html.contains("<pre><code class=\"language-rust\">"),
        "missing language class: {html}"
    );
    assert!(html.contains("fn main() {}"), "code body missing");
    assert!(html.contains("</code></pre>"));
}

#[test]
fn fenced_code_block_html_escaped() {
    let html = render_markdown("```\n<script>alert(1)</script>\n```\n");
    assert!(
        html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
        "code not escaped: {html}"
    );
    assert!(!html.contains("<script>"), "raw script tag leaked");
}

#[test]
fn inline_code_renders_code_tag() {
    let html = render_markdown("use `cargo build` to compile.\n");
    assert!(html.contains("<code>cargo build</code>"));
}

#[test]
fn bold_and_italic_render_strong_and_em() {
    let html = render_markdown("This is **bold** and *italic*.\n");
    assert!(html.contains("<strong>bold</strong>"), "strong missing");
    assert!(html.contains("<em>italic</em>"), "em missing");
}

#[test]
fn links_render_anchor_tags() {
    let html = render_markdown("See [Orison](https://example.com/orison).\n");
    assert!(
        html.contains("<a href=\"https://example.com/orison\">Orison</a>"),
        "anchor missing: {html}"
    );
}

#[test]
fn tables_render_table_tags() {
    let html = render_markdown("| h1 | h2 |\n|----|----|\n| a  | b  |\n| c  | d  |\n");
    assert!(html.contains("<table>"), "table tag missing");
    assert!(html.contains("<th>h1</th>"), "th missing");
    assert!(html.contains("<td>a</td>"), "td a missing");
    assert!(html.contains("<td>d</td>"), "td d missing");
}

#[test]
fn paragraphs_separated_by_blank_lines() {
    let html = render_markdown("First para.\n\nSecond para.\n");
    assert!(html.contains("<p>First para.</p>"));
    assert!(html.contains("<p>Second para.</p>"));
}

#[test]
fn horizontal_rule_renders_hr() {
    let html = render_markdown("intro\n\n---\n\noutro\n");
    assert!(html.contains("<hr />"), "hr missing: {html}");
}

#[test]
fn raw_html_special_chars_are_escaped_outside_code() {
    let html = render_markdown("a < b & c > d in plain text.\n");
    assert!(html.contains("a &lt; b &amp; c &gt; d"));
    assert!(!html.contains(" < b "), "raw `<` leaked into output");
}

#[test]
fn empty_input_produces_empty_output() {
    assert_eq!(render_markdown(""), "");
}

#[test]
fn unterminated_link_falls_back_to_literal() {
    let html = render_markdown("look [here without url\n");
    // The literal `[` must render as `[` (not a broken anchor), and the rest
    // is escaped text. Most importantly: no `<a` tag should be emitted.
    assert!(!html.contains("<a "), "should not emit anchor: {html}");
    assert!(html.contains("[here without url"), "expected literal bracket: {html}");
}
