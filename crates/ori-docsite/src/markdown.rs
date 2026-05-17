//! Hand-written markdown to HTML converter.
//!
//! Supports the subset documented in the crate-level docs:
//!
//! * ATX headers `#` through `######`
//! * Unordered lists `-` and `*`
//! * Ordered lists `1.` etc.
//! * Fenced code blocks ` ``` ` with optional language tag (HTML escaped, no
//!   syntax highlighting)
//! * Inline code with backticks
//! * Bold `**...**` and italic `*...*`
//! * Links `[text](url)`
//! * GFM-style tables with header separator row `|---|---|`
//! * Paragraphs separated by blank lines
//! * Horizontal rules `---` on a line by themselves
//! * Everything else is HTML-escaped
//!
//! Output is deterministic and does not depend on global state. The converter
//! works in two phases: a block-level pass that classifies each line into a
//! `Block`, then a render pass that emits HTML and walks inline runs.

/// Render the given markdown source to an HTML fragment.
///
/// The result is a UTF-8 string of HTML body content suitable for embedding
/// inside a page template. The function never panics on malformed input;
/// unparseable constructs are escaped and emitted as plain text.
pub fn render_markdown(md: &str) -> String {
    let blocks = parse_blocks(md);
    render_blocks(&blocks)
}

/// A block-level element produced by [`parse_blocks`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum Block {
    Heading { level: u8, text: String },
    Paragraph(String),
    UnorderedList(Vec<String>),
    OrderedList(Vec<String>),
    CodeBlock { lang: Option<String>, body: String },
    Table { header: Vec<String>, rows: Vec<Vec<String>> },
    HorizontalRule,
    Blank,
}

fn parse_blocks(md: &str) -> Vec<Block> {
    // Normalise CRLF to LF up front so the rest of the pipeline can assume LF.
    let normalised = md.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalised.split('\n').collect();
    let mut blocks: Vec<Block> = Vec::new();
    let mut i = 0usize;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_end();

        // Blank line: emit a blank marker (used to flush paragraphs).
        if trimmed.trim().is_empty() {
            blocks.push(Block::Blank);
            i += 1;
            continue;
        }

        // Fenced code block.
        if let Some(rest) = trimmed.strip_prefix("```") {
            let lang = if rest.trim().is_empty() {
                None
            } else {
                Some(rest.trim().to_string())
            };
            let mut body = String::new();
            i += 1;
            while i < lines.len() {
                let inner = lines[i];
                if inner.trim_end().starts_with("```") {
                    i += 1;
                    break;
                }
                body.push_str(inner);
                body.push('\n');
                i += 1;
            }
            blocks.push(Block::CodeBlock { lang, body });
            continue;
        }

        // Horizontal rule. `---` (or `***`, `___`) on a line by itself,
        // with at least three characters and only that character + spaces.
        if is_horizontal_rule(trimmed) {
            blocks.push(Block::HorizontalRule);
            i += 1;
            continue;
        }

        // ATX header.
        if let Some((level, text)) = parse_atx_header(trimmed) {
            blocks.push(Block::Heading {
                level,
                text: text.to_string(),
            });
            i += 1;
            continue;
        }

        // Table: a header row followed by a separator row of the form
        // `| --- | :---: |` (with the same number of cells).
        if looks_like_table_header(trimmed) && i + 1 < lines.len()
            && is_table_separator(lines[i + 1].trim_end())
        {
            let header = split_table_row(trimmed);
            let header_cols = header.len();
            let separator_cols = split_table_row(lines[i + 1].trim_end()).len();
            if header_cols > 0 && header_cols == separator_cols {
                let mut rows: Vec<Vec<String>> = Vec::new();
                i += 2;
                while i < lines.len() {
                    let row_line = lines[i].trim_end();
                    if row_line.trim().is_empty() || !row_line.contains('|') {
                        break;
                    }
                    let row = split_table_row(row_line);
                    if row.is_empty() {
                        break;
                    }
                    // Pad / truncate to header column count so render is safe.
                    let mut padded = row;
                    while padded.len() < header_cols {
                        padded.push(String::new());
                    }
                    padded.truncate(header_cols);
                    rows.push(padded);
                    i += 1;
                }
                blocks.push(Block::Table { header, rows });
                continue;
            }
        }

        // Unordered list.
        if is_unordered_list_item(trimmed) {
            let mut items: Vec<String> = Vec::new();
            while i < lines.len() {
                let l = lines[i].trim_end();
                if l.trim().is_empty() {
                    break;
                }
                if let Some(text) = strip_unordered_marker(l) {
                    items.push(text.to_string());
                } else {
                    break;
                }
                i += 1;
            }
            blocks.push(Block::UnorderedList(items));
            continue;
        }

        // Ordered list.
        if is_ordered_list_item(trimmed) {
            let mut items: Vec<String> = Vec::new();
            while i < lines.len() {
                let l = lines[i].trim_end();
                if l.trim().is_empty() {
                    break;
                }
                if let Some(text) = strip_ordered_marker(l) {
                    items.push(text.to_string());
                } else {
                    break;
                }
                i += 1;
            }
            blocks.push(Block::OrderedList(items));
            continue;
        }

        // Paragraph: accumulate consecutive non-blank, non-block-starting lines.
        let mut buf = String::new();
        while i < lines.len() {
            let l = lines[i].trim_end();
            if l.trim().is_empty() {
                break;
            }
            if l.starts_with("```")
                || parse_atx_header(l).is_some()
                || is_horizontal_rule(l)
                || is_unordered_list_item(l)
                || is_ordered_list_item(l)
            {
                break;
            }
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(l);
            i += 1;
        }
        if !buf.is_empty() {
            blocks.push(Block::Paragraph(buf));
        }
    }

    blocks
}

fn parse_atx_header(line: &str) -> Option<(u8, &str)> {
    let bytes = line.as_bytes();
    let mut level = 0u8;
    while level < 6 && (level as usize) < bytes.len() && bytes[level as usize] == b'#' {
        level += 1;
    }
    if level == 0 {
        return None;
    }
    // Header must be followed by a space (or be the bare `#` line, which we
    // treat as an empty header).
    let rest = &line[level as usize..];
    if rest.is_empty() {
        return Some((level, ""));
    }
    if !rest.starts_with(' ') {
        return None;
    }
    let text = rest.trim_start_matches(' ').trim_end_matches('#').trim_end();
    Some((level, text))
}

fn is_horizontal_rule(line: &str) -> bool {
    let t = line.trim();
    if t.len() < 3 {
        return false;
    }
    let c = match t.chars().next() {
        Some(ch) if ch == '-' || ch == '_' || ch == '*' => ch,
        _ => return false,
    };
    t.chars().all(|ch| ch == c || ch == ' ')
        && t.chars().filter(|ch| *ch == c).count() >= 3
}

fn is_unordered_list_item(line: &str) -> bool {
    let t = line.trim_start();
    (t.starts_with("- ") || t.starts_with("* ") || t == "-" || t == "*")
        && !is_horizontal_rule(line)
}

fn strip_unordered_marker(line: &str) -> Option<&str> {
    let t = line.trim_start();
    if let Some(rest) = t.strip_prefix("- ") {
        Some(rest)
    } else if let Some(rest) = t.strip_prefix("* ") {
        Some(rest)
    } else if t == "-" || t == "*" {
        Some("")
    } else {
        None
    }
}

fn is_ordered_list_item(line: &str) -> bool {
    strip_ordered_marker(line).is_some()
}

fn strip_ordered_marker(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let mut digits = 0usize;
    for ch in t.chars() {
        if ch.is_ascii_digit() {
            digits += 1;
        } else {
            break;
        }
    }
    if digits == 0 {
        return None;
    }
    let after_digits = &t[digits..];
    if let Some(rest) = after_digits.strip_prefix(". ") {
        Some(rest)
    } else if after_digits == "." {
        Some("")
    } else {
        None
    }
}

fn looks_like_table_header(line: &str) -> bool {
    // A table header must contain at least one `|` and not start with a list
    // marker / code fence / heading hash.
    line.contains('|')
        && !line.trim_start().starts_with("```")
        && parse_atx_header(line).is_none()
}

fn is_table_separator(line: &str) -> bool {
    let cells = split_table_row(line);
    if cells.is_empty() {
        return false;
    }
    cells.iter().all(|cell| {
        let c = cell.trim();
        let body = c
            .trim_start_matches(':')
            .trim_end_matches(':');
        !body.is_empty() && body.chars().all(|ch| ch == '-')
    })
}

fn split_table_row(line: &str) -> Vec<String> {
    // Trim a single leading / trailing pipe and split on `|`. Pipes escaped
    // with backslash are kept as literal characters.
    let trimmed = line.trim();
    let inner = trimmed
        .strip_prefix('|')
        .unwrap_or(trimmed)
        .strip_suffix('|')
        .unwrap_or_else(|| {
            trimmed.strip_prefix('|').unwrap_or(trimmed)
        });
    let mut cells: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(&next) = chars.peek() {
                if next == '|' {
                    current.push('|');
                    let _ = chars.next();
                    continue;
                }
            }
            current.push(ch);
        } else if ch == '|' {
            cells.push(current.trim().to_string());
            current = String::new();
        } else {
            current.push(ch);
        }
    }
    cells.push(current.trim().to_string());
    cells
}

fn render_blocks(blocks: &[Block]) -> String {
    let mut out = String::new();
    for block in blocks {
        match block {
            Block::Blank => {}
            Block::Heading { level, text } => {
                let level = (*level).clamp(1, 6);
                out.push_str(&format!(
                    "<h{lvl}>{body}</h{lvl}>\n",
                    lvl = level,
                    body = render_inline(text)
                ));
            }
            Block::Paragraph(text) => {
                out.push_str("<p>");
                out.push_str(&render_inline(text));
                out.push_str("</p>\n");
            }
            Block::UnorderedList(items) => {
                out.push_str("<ul>\n");
                for item in items {
                    out.push_str("  <li>");
                    out.push_str(&render_inline(item));
                    out.push_str("</li>\n");
                }
                out.push_str("</ul>\n");
            }
            Block::OrderedList(items) => {
                out.push_str("<ol>\n");
                for item in items {
                    out.push_str("  <li>");
                    out.push_str(&render_inline(item));
                    out.push_str("</li>\n");
                }
                out.push_str("</ol>\n");
            }
            Block::CodeBlock { lang, body } => {
                match lang {
                    Some(l) => out.push_str(&format!(
                        "<pre><code class=\"language-{}\">",
                        escape_html(l)
                    )),
                    None => out.push_str("<pre><code>"),
                }
                out.push_str(&escape_html(body));
                out.push_str("</code></pre>\n");
            }
            Block::Table { header, rows } => {
                out.push_str("<table>\n  <thead>\n    <tr>\n");
                for cell in header {
                    out.push_str("      <th>");
                    out.push_str(&render_inline(cell));
                    out.push_str("</th>\n");
                }
                out.push_str("    </tr>\n  </thead>\n  <tbody>\n");
                for row in rows {
                    out.push_str("    <tr>\n");
                    for cell in row {
                        out.push_str("      <td>");
                        out.push_str(&render_inline(cell));
                        out.push_str("</td>\n");
                    }
                    out.push_str("    </tr>\n");
                }
                out.push_str("  </tbody>\n</table>\n");
            }
            Block::HorizontalRule => {
                out.push_str("<hr />\n");
            }
        }
    }
    out
}

/// Render inline markdown (links, code, bold, italic) inside a single block.
///
/// Inline parsing is a single left-to-right scan. Each construct that begins
/// with a delimiter falls back to literal-with-escaping if it does not find a
/// closing marker on the same input, so malformed input still renders as
/// readable HTML.
fn render_inline(input: &str) -> String {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut out = String::new();
    let mut i = 0usize;
    while i < len {
        let b = bytes[i];
        match b {
            b'\\' if i + 1 < len => {
                // Backslash escape for any of the inline-active chars.
                let next = bytes[i + 1] as char;
                if matches!(
                    next,
                    '\\' | '`' | '*' | '_' | '[' | ']' | '(' | ')' | '|' | '#' | '<' | '>' | '&'
                ) {
                    out.push_str(&escape_char(next));
                    i += 2;
                    continue;
                }
                out.push_str(&escape_char('\\'));
                i += 1;
            }
            b'`' => {
                // Inline code spans up to the next backtick on the same input.
                if let Some(end) = find_byte(bytes, b'`', i + 1) {
                    out.push_str("<code>");
                    out.push_str(&escape_html(&input[i + 1..end]));
                    out.push_str("</code>");
                    i = end + 1;
                } else {
                    out.push_str("&#96;");
                    i += 1;
                }
            }
            b'[' => {
                if let Some((text, url, consumed)) = parse_link(&input[i..]) {
                    out.push_str("<a href=\"");
                    out.push_str(&escape_attr(&url));
                    out.push_str("\">");
                    out.push_str(&render_inline(&text));
                    out.push_str("</a>");
                    i += consumed;
                } else {
                    out.push_str("[");
                    i += 1;
                }
            }
            b'*' => {
                // `**` bold; single `*` italic.
                if i + 1 < len && bytes[i + 1] == b'*' {
                    if let Some(end) = find_double_star(bytes, i + 2) {
                        out.push_str("<strong>");
                        out.push_str(&render_inline(&input[i + 2..end]));
                        out.push_str("</strong>");
                        i = end + 2;
                        continue;
                    }
                    out.push_str("**");
                    i += 2;
                    continue;
                }
                if let Some(end) = find_single_star(bytes, i + 1) {
                    out.push_str("<em>");
                    out.push_str(&render_inline(&input[i + 1..end]));
                    out.push_str("</em>");
                    i = end + 1;
                } else {
                    out.push_str("*");
                    i += 1;
                }
            }
            b'\n' => {
                // Soft break: collapse newlines inside a paragraph to a space.
                out.push(' ');
                i += 1;
            }
            _ => {
                // Multi-byte UTF-8 safe: copy the full code point.
                let ch_end = utf8_char_end(bytes, i);
                let slice = &input[i..ch_end];
                if slice == "<" {
                    out.push_str("&lt;");
                } else if slice == ">" {
                    out.push_str("&gt;");
                } else if slice == "&" {
                    out.push_str("&amp;");
                } else if slice == "\"" {
                    out.push_str("&quot;");
                } else {
                    out.push_str(slice);
                }
                i = ch_end;
            }
        }
    }
    out
}

fn utf8_char_end(bytes: &[u8], start: usize) -> usize {
    if start >= bytes.len() {
        return bytes.len();
    }
    let b = bytes[start];
    let width = if b < 0x80 {
        1
    } else if b & 0xE0 == 0xC0 {
        2
    } else if b & 0xF0 == 0xE0 {
        3
    } else if b & 0xF8 == 0xF0 {
        4
    } else {
        1
    };
    (start + width).min(bytes.len())
}

fn find_byte(bytes: &[u8], target: u8, from: usize) -> Option<usize> {
    for (offset, b) in bytes.iter().enumerate().skip(from) {
        if *b == target {
            return Some(offset);
        }
    }
    None
}

fn find_double_star(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'*' && bytes[i + 1] == b'*' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_single_star(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == b'*' {
            // Reject `**` as the closing marker (it is bold, not italic).
            if i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                i += 2;
                continue;
            }
            return Some(i);
        }
        i += 1;
    }
    None
}

fn parse_link(input: &str) -> Option<(String, String, usize)> {
    // input starts with '['. Find the matching ']' allowing escapes, then
    // require an immediate '(...)' for the URL.
    let bytes = input.as_bytes();
    if bytes.is_empty() || bytes[0] != b'[' {
        return None;
    }
    let mut i = 1usize;
    let mut depth = 1i32;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() => i += 2,
            b'[' => {
                depth += 1;
                i += 1;
            }
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    if depth != 0 || i >= bytes.len() {
        return None;
    }
    let text_end = i;
    // Require '(' right after ']'.
    if i + 1 >= bytes.len() || bytes[i + 1] != b'(' {
        return None;
    }
    let url_start = i + 2;
    let mut j = url_start;
    while j < bytes.len() && bytes[j] != b')' {
        if bytes[j] == b'\\' && j + 1 < bytes.len() {
            j += 2;
            continue;
        }
        j += 1;
    }
    if j >= bytes.len() {
        return None;
    }
    let text = input[1..text_end].to_string();
    let url = input[url_start..j].trim().to_string();
    let consumed = j + 1;
    Some((text, url, consumed))
}

/// HTML-escape a string for use inside element content.
pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// HTML-escape a string for use inside an attribute value.
pub fn escape_attr(input: &str) -> String {
    escape_html(input)
}

fn escape_char(ch: char) -> String {
    match ch {
        '<' => "&lt;".to_string(),
        '>' => "&gt;".to_string(),
        '&' => "&amp;".to_string(),
        '"' => "&quot;".to_string(),
        '\'' => "&#39;".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;

    #[test]
    fn renders_basic_paragraph() {
        let html = render_markdown("Hello world.\n");
        assert!(html.contains("<p>Hello world.</p>"));
    }

    #[test]
    fn escapes_html_in_paragraph() {
        let html = render_markdown("a < b & c > d\n");
        assert!(html.contains("a &lt; b &amp; c &gt; d"));
    }

    #[test]
    fn unclosed_inline_code_does_not_panic() {
        let html = render_markdown("an `unterminated code span\n");
        assert!(html.contains("&#96;"));
    }
}
