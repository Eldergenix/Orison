//! Design-token enforcement pass (M14 bootstrap).
//!
//! Walks every `view` declaration in a [`Module`] and surfaces two
//! classes of finding:
//!
//! * **Unknown tokens** (`D0010`) — the source references
//!   `tokens.<category>.<key>` but `<key>` is not declared in the
//!   configured [`TokenSet`].
//! * **Raw color literals** (`D0020`) — the source contains hex
//!   color literals (`"#abc"` / `"#abcdef"`) or `rgb(...)` /
//!   `rgba(...)` strings, which should be expressed through the token
//!   system instead.
//!
//! Both diagnostics carry an `agent_summary` and a `docs:design.tokens`
//! reference so downstream agents know how to act.
//!
//! ## Detection strategy
//!
//! The bootstrap item parser does not lower view bodies into the
//! expression IR (only `fn` symbols are body-parsed today). To find
//! design-token usages inside a view body we:
//!
//! 1. Re-lex the source file at [`Module::path`] when readable. The
//!    body is identified as the run of tokens between the view header
//!    line and the next top-level item keyword (mirroring the body
//!    extraction logic in [`crate::body`]).
//! 2. When the source file is unreachable (e.g. the module came from
//!    in-memory text in a unit test) we fall back to scanning the
//!    `signature` string. Signatures are single-line so this only
//!    catches inline token uses, which is fine for the bootstrap.
//!
//! Only dotted access (`tokens.colors.primary`) is recognised today.
//! Bracketed access (`tokens["colors"]["primary"]`) is a *documented
//! gap*: agents that emit views should prefer the dotted form, which
//! is also what the formatter normalises to.
//!
//! ## Determinism guarantees
//!
//! * Per-view `used_tokens`, `unknown_tokens`, and `raw_color_literals`
//!   are de-duplicated via [`BTreeSet`] and emitted in sorted order so
//!   the JSON output is byte-stable across runs.
//! * The view list itself preserves source order (mirrors
//!   [`UiManifest`](crate::ui_check::UiManifest) so cross-pass
//!   correlation stays simple).
//! * Diagnostics are sorted by `(id, view-name, message)`.

use crate::ast::{Module, SymbolKind};
use crate::diagnostic::Diagnostic;
use crate::json::to_json;
use crate::lexer::{lex, Token, TokenKind};
use crate::source::{SourceFile, Span};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Catalogue of design tokens loaded from a TOML-subset file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TokenSet {
    /// `[colors]` table — each key/value pair is a token name and its
    /// resolved value (typically a hex string).
    pub colors: BTreeMap<String, String>,
    /// `[spacings]` table.
    pub spacings: BTreeMap<String, String>,
    /// `[fonts]` table.
    pub fonts: BTreeMap<String, String>,
}

impl TokenSet {
    /// `true` if every category is empty. Used as the "no policy
    /// configured" signal — in that mode every token reference is
    /// reported as unknown so agents can see what the views currently
    /// depend on.
    pub fn is_empty(&self) -> bool {
        self.colors.is_empty() && self.spacings.is_empty() && self.fonts.is_empty()
    }

    /// Look up a token by category and key. Returns `None` for
    /// unknown categories.
    pub fn contains(&self, category: &str, key: &str) -> bool {
        match category {
            "colors" => self.colors.contains_key(key),
            "spacings" => self.spacings.contains_key(key),
            "fonts" => self.fonts.contains_key(key),
            _ => false,
        }
    }

    /// Parse a TOML-subset string into a [`TokenSet`].
    ///
    /// Accepted syntax:
    ///
    /// ```text
    /// [colors]
    /// primary = "#3366ff"
    /// danger  = "#cc0033"
    ///
    /// [spacings]
    /// s = "4px"
    ///
    /// [fonts]
    /// body = "Inter"
    /// ```
    ///
    /// Anything more exotic (arrays, nested tables, multi-line
    /// strings) is rejected with a positional [`TokenError`].
    pub fn from_toml_subset(text: &str) -> Result<TokenSet, TokenError> {
        parse_toml_subset(text)
    }
}

/// Aggregate report produced by [`check_module`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TokenCheckReport {
    /// Schema tag — `"ori.design_tokens_report.v1"`.
    pub schema: &'static str,
    /// Per-view findings, in source order.
    pub views: Vec<ViewTokenFindings>,
}

impl TokenCheckReport {
    /// Render the report as a single-line JSON document compatible
    /// with `schemas/design-tokens-report.schema.json`.
    pub fn to_json(&self) -> String {
        to_json(self)
    }
}

/// Per-view token usage. All lists are sorted and de-duplicated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ViewTokenFindings {
    /// View name (`Symbol::name`).
    pub view: String,
    /// Tokens referenced through `tokens.<category>.<key>`.
    pub used_tokens: Vec<String>,
    /// Subset of `used_tokens` whose key is not declared in the
    /// configured [`TokenSet`].
    pub unknown_tokens: Vec<String>,
    /// Raw color literals (hex or `rgb(...)`/`rgba(...)`) found in
    /// the view body. The original text is preserved verbatim so the
    /// agent can rewrite it in place.
    pub raw_color_literals: Vec<String>,
}

/// Positional error from [`TokenSet::from_toml_subset`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenError {
    /// Human-readable message describing the parse failure.
    pub message: String,
    /// 1-indexed line where the error was detected.
    pub line: usize,
    /// 1-indexed column where the error was detected.
    pub column: usize,
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at line {} column {}",
            self.message, self.line, self.column
        )
    }
}

impl std::error::Error for TokenError {}

// ---------------------------------------------------------------------------
// Core checker
// ---------------------------------------------------------------------------

/// Walk every `view` symbol in `module` and produce a deterministic
/// [`TokenCheckReport`]. Source text is read from `module.path` on a
/// best-effort basis; when the file cannot be read we fall back to
/// scanning each view's `signature` so the bootstrap pass still
/// returns *something* useful in test harnesses.
pub fn check_module(module: &Module, tokens: &TokenSet) -> TokenCheckReport {
    let source_text = fs::read_to_string(&module.path).ok();
    let lexed: Option<Vec<Token>> = source_text.as_ref().map(|text| {
        let source = SourceFile::new(module.path.clone(), text.clone());
        lex(&source)
    });

    let view_lines: Vec<usize> = collect_view_header_lines(lexed.as_deref());

    let mut views: Vec<ViewTokenFindings> = Vec::new();
    for symbol in module.symbols.iter().filter(|s| s.kind == SymbolKind::View) {
        let mut used: BTreeSet<String> = BTreeSet::new();
        let mut unknown: BTreeSet<String> = BTreeSet::new();
        let mut raw_colors: BTreeSet<String> = BTreeSet::new();

        // Prefer the body-token scan over the signature scan when the
        // source is available, but always run both — inline uses on
        // the header line still need to be picked up.
        scan_signature(
            &symbol.signature,
            tokens,
            &mut used,
            &mut unknown,
            &mut raw_colors,
        );

        if let Some(tokens_vec) = lexed.as_deref() {
            let header_line = symbol.span.start.line;
            let next_header = view_lines
                .iter()
                .copied()
                .find(|line| *line > header_line)
                .unwrap_or(usize::MAX);
            // Defence-in-depth: also stop at the next *non-view* item
            // keyword on a fresh line.
            let body_end_line = first_item_line_after(tokens_vec, header_line, next_header);
            scan_token_range(
                tokens_vec,
                header_line,
                body_end_line,
                tokens,
                &mut used,
                &mut unknown,
                &mut raw_colors,
            );
        }

        views.push(ViewTokenFindings {
            view: symbol.name.clone(),
            used_tokens: used.into_iter().collect(),
            unknown_tokens: unknown.into_iter().collect(),
            raw_color_literals: raw_colors.into_iter().collect(),
        });
    }

    TokenCheckReport {
        schema: "ori.design_tokens_report.v1",
        views,
    }
}

/// Convert a [`TokenCheckReport`] into a deterministic vector of
/// diagnostics (`D0010` / `D0020`). Diagnostics are sorted by
/// `(id, view, message)` so repeated runs over the same input produce
/// identical output.
pub fn report_to_diagnostics(module: &Module, report: &TokenCheckReport) -> Vec<Diagnostic> {
    let span_for_view: BTreeMap<&str, Span> = module
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::View)
        .map(|s| (s.name.as_str(), s.span.clone()))
        .collect();

    let mut out: Vec<Diagnostic> = Vec::new();
    for view in &report.views {
        let span = span_for_view
            .get(view.view.as_str())
            .cloned()
            .unwrap_or_else(|| Span::dummy(module.path.clone()));
        for tok in &view.unknown_tokens {
            let msg = format!("view `{}` uses unknown design token `{}`", view.view, tok);
            out.push(
                Diagnostic::warning("D0010", msg, span.clone())
                    .with_agent_summary(
                        "Add this key to your design tokens file or rename the reference \
                         to an existing token.",
                    )
                    .with_docs(vec!["doc:design.tokens".to_string()]),
            );
        }
        for lit in &view.raw_color_literals {
            let msg = format!(
                "view `{}` uses raw color literal `{}` instead of design token",
                view.view, lit
            );
            out.push(
                Diagnostic::warning("D0020", msg, span.clone())
                    .with_agent_summary(
                        "Replace this raw color literal with a `tokens.colors.<name>` reference \
                         from the design tokens file.",
                    )
                    .with_docs(vec!["doc:design.tokens".to_string()]),
            );
        }
    }
    out.sort_by(|a, b| {
        a.id.cmp(&b.id)
            .then_with(|| a.message.cmp(&b.message))
            .then_with(|| a.span.start.line.cmp(&b.span.start.line))
    });
    out
}

// ---------------------------------------------------------------------------
// Internal scanning helpers
// ---------------------------------------------------------------------------

fn collect_view_header_lines(tokens: Option<&[Token]>) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    let Some(tokens) = tokens else {
        return out;
    };
    for (idx, t) in tokens.iter().enumerate() {
        if t.kind != TokenKind::Keyword {
            continue;
        }
        if !is_top_level_item_keyword(&t.lexeme) {
            continue;
        }
        if !is_at_line_start(tokens, idx) {
            continue;
        }
        out.push(t.span.start.line);
    }
    out.sort_unstable();
    out
}

fn first_item_line_after(tokens: &[Token], header_line: usize, hint_line: usize) -> usize {
    let mut best = hint_line;
    for (idx, t) in tokens.iter().enumerate() {
        if t.kind != TokenKind::Keyword {
            continue;
        }
        if !is_top_level_item_keyword(&t.lexeme) {
            continue;
        }
        if !is_at_line_start(tokens, idx) {
            continue;
        }
        if t.span.start.line > header_line && t.span.start.line < best {
            best = t.span.start.line;
        }
    }
    best
}

fn is_top_level_item_keyword(kw: &str) -> bool {
    matches!(
        kw,
        "fn" | "type"
            | "service"
            | "view"
            | "actor"
            | "query"
            | "migration"
            | "capability"
            | "module"
            | "import"
            | "protocol"
            | "impl"
    )
}

fn is_at_line_start(tokens: &[Token], idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let prev = &tokens[idx - 1];
    let cur = &tokens[idx];
    cur.span.start.line > prev.span.start.line
}

/// Scan a contiguous token range (inclusive on `start_line`, exclusive
/// on `end_line`) for design-token usages and raw color literals.
fn scan_token_range(
    tokens: &[Token],
    start_line: usize,
    end_line: usize,
    set: &TokenSet,
    used: &mut BTreeSet<String>,
    unknown: &mut BTreeSet<String>,
    raw_colors: &mut BTreeSet<String>,
) {
    let mut i = 0usize;
    while i < tokens.len() {
        let t = &tokens[i];
        if t.span.start.line < start_line {
            i += 1;
            continue;
        }
        if t.span.start.line >= end_line {
            break;
        }
        // tokens.<category>.<key> dotted path detection.
        if t.kind == TokenKind::Ident && t.lexeme == "tokens" {
            if let Some((category, key, consumed)) = read_dotted_two(tokens, i + 1) {
                record_token_use(&category, &key, set, used, unknown);
                i += 1 + consumed;
                continue;
            }
        }
        // String-literal color detection.
        if t.kind == TokenKind::String {
            for lit in extract_raw_colors(&t.lexeme) {
                raw_colors.insert(lit);
            }
        }
        i += 1;
    }
}

/// Read the next two `.ident.ident` segments following a `tokens`
/// keyword. Returns `(category, key, tokens_consumed_after_tokens)`
/// on success.
fn read_dotted_two(tokens: &[Token], start: usize) -> Option<(String, String, usize)> {
    if start + 3 >= tokens.len() {
        return None;
    }
    let dot1 = tokens.get(start)?;
    let cat = tokens.get(start + 1)?;
    let dot2 = tokens.get(start + 2)?;
    let key = tokens.get(start + 3)?;
    if !is_symbol(dot1, ".") || !is_symbol(dot2, ".") {
        return None;
    }
    if cat.kind != TokenKind::Ident || key.kind != TokenKind::Ident {
        return None;
    }
    Some((cat.lexeme.clone(), key.lexeme.clone(), 4))
}

fn is_symbol(t: &Token, sym: &str) -> bool {
    t.kind == TokenKind::Symbol && t.lexeme == sym
}

fn record_token_use(
    category: &str,
    key: &str,
    set: &TokenSet,
    used: &mut BTreeSet<String>,
    unknown: &mut BTreeSet<String>,
) {
    let label = format!("tokens.{category}.{key}");
    used.insert(label.clone());
    // Empty config = "no policy yet" — treat every reference as
    // unknown so agents can see the surface area they depend on.
    if set.is_empty() || !set.contains(category, key) {
        unknown.insert(label);
    }
}

/// Walk a signature string for the same patterns as
/// [`scan_token_range`]. Signatures are byte slices, not token
/// streams, so detection is regex-free string scanning.
fn scan_signature(
    signature: &str,
    set: &TokenSet,
    used: &mut BTreeSet<String>,
    unknown: &mut BTreeSet<String>,
    raw_colors: &mut BTreeSet<String>,
) {
    // tokens.<cat>.<key>
    let bytes = signature.as_bytes();
    let needle = b"tokens.";
    let mut idx = 0usize;
    while idx + needle.len() <= bytes.len() {
        if &bytes[idx..idx + needle.len()] == needle {
            let after = idx + needle.len();
            if let Some((category, after_cat)) = read_ident_at(signature, after) {
                if after_cat < signature.len() && bytes[after_cat] == b'.' {
                    if let Some((key, _)) = read_ident_at(signature, after_cat + 1) {
                        record_token_use(&category, &key, set, used, unknown);
                    }
                }
            }
            idx = after;
            continue;
        }
        idx += 1;
    }
    // String literals are not surfaced as a separate token in
    // signatures, so look at quoted substrings.
    for lit in extract_string_literals(signature) {
        for color in extract_raw_colors(&lit) {
            raw_colors.insert(color);
        }
    }
}

fn read_ident_at(s: &str, start: usize) -> Option<(String, usize)> {
    let bytes = s.as_bytes();
    if start >= bytes.len() {
        return None;
    }
    let first = bytes[start];
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return None;
    }
    let mut end = start;
    while end < bytes.len() {
        let b = bytes[end];
        if b.is_ascii_alphanumeric() || b == b'_' {
            end += 1;
        } else {
            break;
        }
    }
    Some((s[start..end].to_string(), end))
}

fn extract_string_literals(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b'"' {
                j += 1;
            }
            if j <= bytes.len() && j > start {
                // Slice on byte indices is safe because `"` is ASCII.
                out.push(s[start..j].to_string());
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// Pull every hex/`rgb()`/`rgba()` color literal out of one string
/// literal's contents. Returns the *verbatim* matched substrings so
/// callers can reproduce the original text in fix suggestions.
fn extract_raw_colors(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'#' {
            let start = i;
            let mut j = i + 1;
            while j < bytes.len() && is_ascii_hex_digit(bytes[j]) {
                j += 1;
            }
            let hex_len = j - i - 1;
            if hex_len == 3 || hex_len == 6 || hex_len == 4 || hex_len == 8 {
                out.push(text[start..j].to_string());
            }
            i = j;
            continue;
        }
        // case-insensitive match for "rgb(" or "rgba("
        if starts_with_ci(&bytes[i..], b"rgba(") {
            if let Some(end) = find_close_paren(&bytes[i..]) {
                out.push(text[i..i + end + 1].to_string());
                i += end + 1;
                continue;
            }
        }
        if starts_with_ci(&bytes[i..], b"rgb(") {
            if let Some(end) = find_close_paren(&bytes[i..]) {
                out.push(text[i..i + end + 1].to_string());
                i += end + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn is_ascii_hex_digit(b: u8) -> bool {
    b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || (b'A'..=b'F').contains(&b)
}

fn starts_with_ci(haystack: &[u8], needle: &[u8]) -> bool {
    if haystack.len() < needle.len() {
        return false;
    }
    for (i, n) in needle.iter().enumerate() {
        if !haystack[i].eq_ignore_ascii_case(n) {
            return false;
        }
    }
    true
}

fn find_close_paren(bytes: &[u8]) -> Option<usize> {
    let mut depth = 0i32;
    for (idx, b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// TOML-subset parser
// ---------------------------------------------------------------------------

fn parse_toml_subset(text: &str) -> Result<TokenSet, TokenError> {
    let mut set = TokenSet::default();
    let mut current_section: Option<String> = None;

    for (line_idx, raw_line) in text.split('\n').enumerate() {
        let line_no = line_idx + 1;
        let line = strip_comment(raw_line);
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix('[') {
            let close = match rest.find(']') {
                Some(c) => c,
                None => {
                    return Err(TokenError {
                        message: "unterminated section header".to_string(),
                        line: line_no,
                        column: leading_ws(raw_line) + 1,
                    });
                }
            };
            let name = rest[..close].trim();
            if name.is_empty() {
                return Err(TokenError {
                    message: "empty section header".to_string(),
                    line: line_no,
                    column: leading_ws(raw_line) + 1,
                });
            }
            if !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(TokenError {
                    message: format!("invalid section header `{name}`"),
                    line: line_no,
                    column: leading_ws(raw_line) + 2,
                });
            }
            // Trailing characters after `]` would be ambiguous.
            let tail = rest[close + 1..].trim();
            if !tail.is_empty() {
                return Err(TokenError {
                    message: format!("unexpected `{tail}` after section header"),
                    line: line_no,
                    column: leading_ws(raw_line) + close + 2,
                });
            }
            current_section = Some(name.to_string());
            continue;
        }

        // key = "value"
        let eq = match line.find('=') {
            Some(eq) => eq,
            None => {
                return Err(TokenError {
                    message: "expected `=` in key/value line".to_string(),
                    line: line_no,
                    column: leading_ws(raw_line) + 1,
                });
            }
        };
        let key = line[..eq].trim();
        if key.is_empty() {
            return Err(TokenError {
                message: "missing key before `=`".to_string(),
                line: line_no,
                column: leading_ws(raw_line) + 1,
            });
        }
        if !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(TokenError {
                message: format!("invalid key `{key}`"),
                line: line_no,
                column: leading_ws(raw_line) + 1,
            });
        }
        let value_part = line[eq + 1..].trim();
        let value = match value_part
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
        {
            Some(v) => v.to_string(),
            None => {
                return Err(TokenError {
                    message: "values must be double-quoted strings".to_string(),
                    line: line_no,
                    column: line.find('=').map(|p| p + 2).unwrap_or(1),
                });
            }
        };

        let section = match current_section.as_deref() {
            Some(s) => s,
            None => {
                return Err(TokenError {
                    message: format!("key `{key}` outside any [section] header"),
                    line: line_no,
                    column: leading_ws(raw_line) + 1,
                });
            }
        };

        let target = match section {
            "colors" => &mut set.colors,
            "spacings" => &mut set.spacings,
            "fonts" => &mut set.fonts,
            other => {
                return Err(TokenError {
                    message: format!(
                        "unknown section `[{other}]`; expected one of \
                         [colors], [spacings], [fonts]"
                    ),
                    line: line_no,
                    column: leading_ws(raw_line) + 2,
                });
            }
        };
        if target.contains_key(key) {
            return Err(TokenError {
                message: format!("duplicate key `{key}` in [{section}]"),
                line: line_no,
                column: leading_ws(raw_line) + 1,
            });
        }
        target.insert(key.to_string(), value);
    }

    Ok(set)
}

fn strip_comment(line: &str) -> &str {
    // The TOML subset has no string literals that may contain `#`
    // outside of values; the value parser already strips a trailing
    // quoted string before this matters. To stay safe, only strip
    // `#` that appears outside of double quotes.
    let bytes = line.as_bytes();
    let mut in_str = false;
    let mut escape = false;
    for (i, b) in bytes.iter().enumerate() {
        if in_str {
            if escape {
                escape = false;
            } else if *b == b'\\' {
                escape = true;
            } else if *b == b'"' {
                in_str = false;
            }
        } else if *b == b'"' {
            in_str = true;
        } else if *b == b'#' {
            return &line[..i];
        }
    }
    line
}

fn leading_ws(line: &str) -> usize {
    line.bytes()
        .take_while(|b| *b == b' ' || *b == b'\t')
        .count()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_source;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn unique_tmp_path(label: &str) -> std::path::PathBuf {
        let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "ori_design_tokens_{label}_{pid}_{n}.ori",
            pid = std::process::id()
        ))
    }

    fn write_tmp(label: &str, body: &str) -> std::path::PathBuf {
        let path = unique_tmp_path(label);
        if let Err(err) = fs::write(&path, body) {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "failed to write tmp file {}: {err}", path.display());
            }
        }
        path
    }

    fn parse_module_from_disk(path: &std::path::Path) -> Module {
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "failed to read tmp file: {err}");
                }
                String::new()
            }
        };
        let source = SourceFile::new(path.to_string_lossy().to_string(), text);
        parse_source(&source).module
    }

    fn sample_tokens() -> TokenSet {
        let text = "[colors]\nprimary = \"#3366ff\"\ndanger = \"#cc0033\"\n\
                    [spacings]\ns = \"4px\"\n[fonts]\nbody = \"Inter\"\n";
        match TokenSet::from_toml_subset(text) {
            Ok(set) => set,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "sample tokens failed to parse: {err}");
                }
                TokenSet::default()
            }
        }
    }

    #[test]
    fn unknown_token_yields_d0010() {
        let src = "module demo\n\
                   view Hero(title: Str) -> Html uses ui:\n\
                   \x20\x20text(color: tokens.colors.brandiose)\n";
        let path = write_tmp("d0010", src);
        let module = parse_module_from_disk(&path);
        let report = check_module(&module, &sample_tokens());
        let diags = report_to_diagnostics(&module, &report);
        let _ = fs::remove_file(&path);
        let has_d0010 = diags.iter().any(|d| d.id == "D0010");
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(has_d0010, "expected D0010, got {diags:?}");
        }
        let hero = report.views.iter().find(|v| v.view == "Hero");
        let unknown = hero.map(|v| v.unknown_tokens.clone()).unwrap_or_default();
        assert!(
            unknown.iter().any(|t| t == "tokens.colors.brandiose"),
            "expected unknown=[tokens.colors.brandiose], got {unknown:?}"
        );
    }

    #[test]
    fn raw_hex_literal_yields_d0020() {
        let src = "module demo\n\
                   view Hero(title: Str) -> Html uses ui:\n\
                   \x20\x20text(color: \"#ABCDEF\")\n";
        let path = write_tmp("d0020", src);
        let module = parse_module_from_disk(&path);
        let report = check_module(&module, &sample_tokens());
        let diags = report_to_diagnostics(&module, &report);
        let _ = fs::remove_file(&path);
        let has_d0020 = diags.iter().any(|d| d.id == "D0020");
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(has_d0020, "expected D0020 for uppercase hex, got {diags:?}");
        }
        let hero = report.views.iter().find(|v| v.view == "Hero");
        let raw = hero
            .map(|v| v.raw_color_literals.clone())
            .unwrap_or_default();
        assert!(
            raw.iter().any(|l| l == "#ABCDEF"),
            "expected #ABCDEF in raw literals, got {raw:?}"
        );
    }

    #[test]
    fn rgb_literal_yields_d0020() {
        let src = "module demo\n\
                   view Hero(title: Str) -> Html uses ui:\n\
                   \x20\x20text(color: \"rgba(0, 0, 0, 0.5)\")\n";
        let path = write_tmp("rgb", src);
        let module = parse_module_from_disk(&path);
        let report = check_module(&module, &sample_tokens());
        let _ = fs::remove_file(&path);
        let raw = report
            .views
            .iter()
            .find(|v| v.view == "Hero")
            .map(|v| v.raw_color_literals.clone())
            .unwrap_or_default();
        assert!(
            raw.iter().any(|l| l == "rgba(0, 0, 0, 0.5)"),
            "expected rgba literal, got {raw:?}"
        );
    }

    #[test]
    fn known_token_is_quiet() {
        let src = "module demo\n\
                   view Hero(title: Str) -> Html uses ui:\n\
                   \x20\x20text(color: tokens.colors.primary)\n";
        let path = write_tmp("known", src);
        let module = parse_module_from_disk(&path);
        let report = check_module(&module, &sample_tokens());
        let diags = report_to_diagnostics(&module, &report);
        let _ = fs::remove_file(&path);
        assert!(
            diags.is_empty(),
            "expected no diagnostics for known token, got {diags:?}"
        );
        let hero = report
            .views
            .iter()
            .find(|v| v.view == "Hero")
            .cloned()
            .unwrap_or(ViewTokenFindings {
                view: "Hero".into(),
                used_tokens: vec![],
                unknown_tokens: vec![],
                raw_color_literals: vec![],
            });
        assert_eq!(hero.used_tokens, vec!["tokens.colors.primary".to_string()]);
        assert!(hero.unknown_tokens.is_empty());
        assert!(hero.raw_color_literals.is_empty());
    }

    #[test]
    fn multiple_views_checked_independently() {
        let src = "module demo\n\
                   view A(x: Str) -> Html uses ui:\n\
                   \x20\x20text(color: tokens.colors.primary)\n\
                   view B(x: Str) -> Html uses ui:\n\
                   \x20\x20text(color: tokens.colors.nope)\n";
        let path = write_tmp("multi", src);
        let module = parse_module_from_disk(&path);
        let report = check_module(&module, &sample_tokens());
        let _ = fs::remove_file(&path);
        assert_eq!(report.views.len(), 2);
        let a = match report.views.iter().find(|v| v.view == "A") {
            Some(v) => v.clone(),
            None => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "missing view A");
                }
                return;
            }
        };
        let b = match report.views.iter().find(|v| v.view == "B") {
            Some(v) => v.clone(),
            None => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "missing view B");
                }
                return;
            }
        };
        assert!(a.unknown_tokens.is_empty(), "view A should be clean");
        assert_eq!(b.unknown_tokens, vec!["tokens.colors.nope".to_string()]);
        // Cross-view bleed: A's `primary` reference must NOT show up
        // in B.
        assert!(!b.used_tokens.iter().any(|t| t == "tokens.colors.primary"));
    }

    #[test]
    fn empty_tokens_config_treats_everything_as_unknown() {
        let src = "module demo\n\
                   view Hero(x: Str) -> Html uses ui:\n\
                   \x20\x20text(color: tokens.colors.anything)\n";
        let path = write_tmp("empty_set", src);
        let module = parse_module_from_disk(&path);
        let report = check_module(&module, &TokenSet::default());
        let _ = fs::remove_file(&path);
        let hero = report.views.iter().find(|v| v.view == "Hero");
        let unknown = hero.map(|v| v.unknown_tokens.clone()).unwrap_or_default();
        assert_eq!(unknown, vec!["tokens.colors.anything".to_string()]);
    }

    #[test]
    fn toml_parse_error_returns_positional_error() {
        // Missing `]` on the section header → unterminated.
        let err = match TokenSet::from_toml_subset("[colors\nprimary = \"#fff\"\n") {
            Ok(_) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected TOML parse error");
                }
                return;
            }
            Err(err) => err,
        };
        assert_eq!(err.line, 1);
        assert!(
            err.message.contains("unterminated"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn deterministic_ordering_and_dedup() {
        let src = "module demo\n\
                   view Hero(x: Str) -> Html uses ui:\n\
                   \x20\x20text(color: tokens.colors.nope)\n\
                   \x20\x20text(color: tokens.colors.nope)\n\
                   \x20\x20text(color: \"#fff\")\n\
                   \x20\x20text(color: \"#fff\")\n";
        let path = write_tmp("dedup", src);
        let module = parse_module_from_disk(&path);
        let r1 = check_module(&module, &sample_tokens());
        let r2 = check_module(&module, &sample_tokens());
        let _ = fs::remove_file(&path);
        assert_eq!(r1, r2, "non-deterministic output between runs");
        let hero = match r1.views.iter().find(|v| v.view == "Hero") {
            Some(v) => v.clone(),
            None => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "missing view Hero");
                }
                return;
            }
        };
        assert_eq!(hero.unknown_tokens.len(), 1);
        assert_eq!(hero.raw_color_literals, vec!["#fff".to_string()]);
    }

    #[test]
    fn idempotent_across_runs() {
        let src = "module demo\n\
                   view Hero(x: Str) -> Html uses ui:\n\
                   \x20\x20text(color: tokens.colors.primary)\n\
                   \x20\x20text(color: \"#abc\")\n";
        let path = write_tmp("idem", src);
        let module = parse_module_from_disk(&path);
        let set = sample_tokens();
        let a = check_module(&module, &set);
        let b = check_module(&module, &set);
        let c = check_module(&module, &set);
        let _ = fs::remove_file(&path);
        assert_eq!(a, b);
        assert_eq!(b, c);
    }

    #[test]
    fn three_char_and_six_char_hex_both_detected() {
        // Sanity check both common forms in a single view.
        let src = "module demo\n\
                   view Hero(x: Str) -> Html uses ui:\n\
                   \x20\x20text(color: \"#abc\")\n\
                   \x20\x20text(color: \"#aabbcc\")\n";
        let path = write_tmp("hex_lens", src);
        let module = parse_module_from_disk(&path);
        let report = check_module(&module, &sample_tokens());
        let _ = fs::remove_file(&path);
        let raw = report
            .views
            .iter()
            .find(|v| v.view == "Hero")
            .map(|v| v.raw_color_literals.clone())
            .unwrap_or_default();
        assert!(raw.iter().any(|l| l == "#abc"));
        assert!(raw.iter().any(|l| l == "#aabbcc"));
    }
}
