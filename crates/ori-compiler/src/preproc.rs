//! Tiny, safe text-level pre-processor for `.ori` source files.
//!
//! This module deliberately implements a very narrow surface:
//!
//! - `${ENV_NAME}` markers expand to the value of an environment variable,
//!   but only when `ENV_NAME` is present in `PreprocessConfig::allow_env`
//!   and the variable is set in the process environment.
//! - `@orison/<const>` markers expand to a string constant declared in
//!   `PreprocessConfig::constants`.
//!
//! Two hard safety bars are enforced by the implementation:
//!
//! 1. **String-literal aware.** Text between double-quote characters is
//!    preserved verbatim — markers are never substituted inside string
//!    literals. Escaped quotes (`\"`) do **not** terminate the string.
//!    Discovering an unsubstituted marker inside a string literal yields a
//!    `PRE0030` diagnostic so the caller can see why the marker was skipped.
//! 2. **Allow-list gated environment reads.** `std::env::var` is only ever
//!    called for names that appear in `allow_env`. A `${X}` marker with no
//!    allow-list entry will never read the process environment and will
//!    instead emit `PRE0010`.
//!
//! Determinism: each call to [`preprocess`] uses a fresh per-call cache, so
//! repeated occurrences of `${X}` in one input read the environment once and
//! always substitute the same value. Two calls on identical input and config
//! produce byte-identical output (assuming the underlying environment did not
//! change between calls).

use crate::diagnostic::Diagnostic;
use crate::source::Span;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

/// Configuration for [`preprocess`]. Built up by the caller (CLI, build
/// system, IDE, …) and passed by reference so it can be reused across many
/// files without cloning.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PreprocessConfig {
    /// Names of environment variables that the caller has explicitly
    /// allow-listed. `${X}` markers whose name is not in this list are
    /// **never** read from the process environment.
    pub allow_env: Vec<String>,
    /// Compile-time string constants addressable as `@orison/<name>`.
    pub constants: BTreeMap<String, String>,
}

impl PreprocessConfig {
    /// Empty config: no allowed env vars, no constants.
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a compile-time constant. Returns `self` for builder-style use.
    pub fn with_const(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.constants.insert(key.into(), value.into());
        self
    }

    /// Add an environment variable name to the allow-list. Duplicates are
    /// silently de-duplicated to keep the public surface forgiving.
    pub fn allow_env_var(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        if !self.allow_env.iter().any(|existing| existing == &name) {
            self.allow_env.push(name);
        }
        self
    }
}

/// Per-call scanner state. Tracks the byte offset into the input as well as
/// the 1-based line/column position so diagnostics can point at the right
/// place in the source.
struct Scanner<'a> {
    chars: &'a [char],
    pos: usize,
    line: usize,
    col: usize,
}

impl<'a> Scanner<'a> {
    fn new(chars: &'a [char]) -> Self {
        Self {
            chars,
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(ch)
    }
}

/// Output of [`preprocess`]: the substituted text plus any diagnostics
/// produced along the way. Diagnostics are always returned (never returned
/// via `Err`) so callers can decide whether to treat them as fatal.
pub fn preprocess(text: &str, config: &PreprocessConfig) -> (String, Vec<Diagnostic>) {
    let chars: Vec<char> = text.chars().collect();
    let mut scanner = Scanner::new(&chars);
    let mut out = String::with_capacity(text.len());
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    // Per-call environment cache. Always fresh — never shared across calls
    // so two runs cannot leak values from each other and so the cache cannot
    // grow unboundedly across long-running processes.
    let mut env_cache: BTreeMap<String, Option<String>> = BTreeMap::new();

    while scanner.pos < scanner.chars.len() {
        let ch = match scanner.peek(0) {
            Some(c) => c,
            None => break,
        };

        // --- String literal: copy verbatim, watch for escaped quotes. -----
        if ch == '"' {
            copy_string_literal(&mut scanner, &mut out, &mut diagnostics);
            continue;
        }

        // --- Line comment: copy verbatim up to the newline. ---------------
        if ch == '/' && scanner.peek(1) == Some('/') {
            copy_line_comment(&mut scanner, &mut out);
            continue;
        }

        // --- ${ENV_NAME} marker. ------------------------------------------
        if ch == '$'
            && scanner.peek(1) == Some('{')
            && try_expand_env(
                &mut scanner,
                &mut out,
                &mut diagnostics,
                config,
                &mut env_cache,
            )
        {
            continue;
        }

        // --- @orison/<name> marker. ---------------------------------------
        if ch == '@'
            && starts_with_at(&scanner, "@orison/")
            && try_expand_const(&mut scanner, &mut out, &mut diagnostics, config)
        {
            continue;
        }

        // Plain character: copy across and move on.
        if let Some(c) = scanner.advance() {
            out.push(c);
        }
    }

    (out, diagnostics)
}

/// Read `path` from disk and pre-process it. Returns `Err` for I/O failures
/// (including invalid UTF-8); on success returns the substituted text and
/// any diagnostics generated by [`preprocess`].
pub fn preprocess_file(
    path: &Path,
    config: &PreprocessConfig,
) -> io::Result<(String, Vec<Diagnostic>)> {
    let bytes = fs::read(path)?;
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(err) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: file is not valid UTF-8: {}", path.display(), err),
            ));
        }
    };
    Ok(preprocess(&text, config))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Copy a `"..."` string literal verbatim into `out`, respecting backslash
/// escapes. If the literal is unterminated we emit a `PRE0040` diagnostic so
/// the caller can see where the trouble started, but the copy still consumes
/// the rest of the input (matching the lexer's recovery shape).
fn copy_string_literal(
    scanner: &mut Scanner<'_>,
    out: &mut String,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Remember where the literal started for diagnostic spans.
    let start_line = scanner.line;
    let start_col = scanner.col;

    // Consume the opening quote.
    if let Some(c) = scanner.advance() {
        out.push(c);
    }

    let mut last_escape = false;
    let mut found_marker_inside = false;
    let mut marker_line = start_line;
    let mut marker_col = start_col;
    let mut marker_text = String::new();

    while scanner.pos < scanner.chars.len() {
        let ch = match scanner.peek(0) {
            Some(c) => c,
            None => break,
        };

        if !last_escape && ch == '"' {
            if let Some(c) = scanner.advance() {
                out.push(c);
            }
            if found_marker_inside {
                diagnostics.push(
                    Diagnostic::warning(
                        "P3030",
                        format!(
                            "preprocessor refused to expand body inside string literal `\"...{marker_text}...\"` to prevent escape"
                        ),
                        Span::new("<preprocess>", marker_line, marker_col, scanner.line, scanner.col),
                    )
                    .with_agent_summary(
                        "Move the marker outside of the string literal, or inject the value through a typed binding.",
                    ),
                );
            }
            return;
        }

        // Detect markers inside the string for a diagnostic — but never
        // substitute them. We only record the first one to keep the output
        // small; further markers are simply preserved.
        if !last_escape && !found_marker_inside {
            if ch == '$' && scanner.peek(1) == Some('{') {
                let snippet = capture_env_marker_snippet(scanner);
                if !snippet.is_empty() {
                    found_marker_inside = true;
                    marker_line = scanner.line;
                    marker_col = scanner.col;
                    marker_text = snippet;
                }
            } else if ch == '@' && starts_with_at(scanner, "@orison/") {
                let snippet = capture_const_marker_snippet(scanner);
                if !snippet.is_empty() {
                    found_marker_inside = true;
                    marker_line = scanner.line;
                    marker_col = scanner.col;
                    marker_text = snippet;
                }
            }
        }

        if let Some(c) = scanner.advance() {
            out.push(c);
        }

        // Track escapes: a backslash toggles `last_escape` so the next
        // character is treated literally — crucially this means `\"` does
        // not close the string. A second backslash (`\\`) resets the flag.
        last_escape = ch == '\\' && !last_escape;
    }
}

/// Copy a `// ...` line comment verbatim into `out` up to (but not
/// including) the terminating newline. The newline itself is left for the
/// main loop to handle so line counters stay correct.
fn copy_line_comment(scanner: &mut Scanner<'_>, out: &mut String) {
    while let Some(ch) = scanner.peek(0) {
        if ch == '\n' {
            break;
        }
        if let Some(c) = scanner.advance() {
            out.push(c);
        }
    }
}

/// Attempt to expand a `${NAME}` marker at the current scanner position.
///
/// Returns `true` if the marker was recognised (and either expanded or
/// reported via a diagnostic). Returns `false` if the bytes at the cursor
/// did not actually form a valid `${NAME}` shape — in which case the caller
/// falls back to copying `$` literally.
fn try_expand_env(
    scanner: &mut Scanner<'_>,
    out: &mut String,
    diagnostics: &mut Vec<Diagnostic>,
    config: &PreprocessConfig,
    env_cache: &mut BTreeMap<String, Option<String>>,
) -> bool {
    let start_line = scanner.line;
    let start_col = scanner.col;
    let Some(name) = parse_brace_name(scanner) else {
        return false;
    };

    if !config.allow_env.iter().any(|allowed| allowed == &name) {
        // SAFETY BAR: a `${X}` whose name is not allow-listed must never
        // read the process environment.
        diagnostics.push(
            Diagnostic::warning(
                "P3010",
                format!("env reference `${{{name}}}` is not in the allow-list"),
                Span::new("<preprocess>", start_line, start_col, scanner.line, scanner.col),
            )
            .with_agent_summary(
                "Add the env var name to PreprocessConfig::allow_env or pass --allow-env on the CLI.",
            ),
        );
        out.push_str(&format!("${{{name}}}"));
        return true;
    }

    // Lazy, deterministic env read.
    let value = match env_cache.get(&name) {
        Some(v) => v.clone(),
        None => {
            let v = std::env::var(&name).ok();
            env_cache.insert(name.clone(), v.clone());
            v
        }
    };

    match value {
        Some(v) => out.push_str(&v),
        None => {
            diagnostics.push(
                Diagnostic::warning(
                    "P3010",
                    format!(
                        "env reference `${{{name}}}` is not in the allow-list (variable not set in the environment)"
                    ),
                    Span::new("<preprocess>", start_line, start_col, scanner.line, scanner.col),
                )
                .with_agent_summary(
                    "Export the variable before invoking the preprocessor.",
                ),
            );
            out.push_str(&format!("${{{name}}}"));
        }
    }
    true
}

/// Attempt to expand an `@orison/<name>` marker at the current cursor.
fn try_expand_const(
    scanner: &mut Scanner<'_>,
    out: &mut String,
    diagnostics: &mut Vec<Diagnostic>,
    config: &PreprocessConfig,
) -> bool {
    let start_line = scanner.line;
    let start_col = scanner.col;
    let Some(name) = parse_const_name(scanner) else {
        return false;
    };

    match config.constants.get(&name) {
        Some(value) => out.push_str(value),
        None => {
            diagnostics.push(
                Diagnostic::warning(
                    "P3020",
                    format!("constant `@orison/{name}` is not declared"),
                    Span::new("<preprocess>", start_line, start_col, scanner.line, scanner.col),
                )
                .with_agent_summary(
                    "Declare the constant via PreprocessConfig::with_const or pass --const name=value on the CLI.",
                ),
            );
            out.push_str(&format!("@orison/{name}"));
        }
    }
    true
}

/// Parse `${NAME}` at the current cursor. On success advances the scanner
/// past the closing brace and returns `NAME`. On a malformed marker
/// (missing `{`, missing `}`, empty body, illegal characters) the scanner
/// is left unchanged and `None` is returned.
fn parse_brace_name(scanner: &mut Scanner<'_>) -> Option<String> {
    let saved_pos = scanner.pos;
    let saved_line = scanner.line;
    let saved_col = scanner.col;

    if scanner.peek(0) != Some('$') || scanner.peek(1) != Some('{') {
        return None;
    }
    // Consume `${`.
    scanner.advance();
    scanner.advance();

    let mut name = String::new();
    let mut closed = false;
    while let Some(ch) = scanner.peek(0) {
        if ch == '}' {
            scanner.advance();
            closed = true;
            break;
        }
        if !is_ident_char(ch) {
            break;
        }
        if let Some(c) = scanner.advance() {
            name.push(c);
        }
    }

    if !closed || name.is_empty() {
        // Roll back so the caller can emit `$` literally and re-scan.
        scanner.pos = saved_pos;
        scanner.line = saved_line;
        scanner.col = saved_col;
        return None;
    }
    Some(name)
}

/// Parse `@orison/<name>` at the current cursor. Returns the bare `<name>`
/// on success (without the prefix) and advances the scanner past it. On a
/// malformed marker the scanner is rolled back and `None` is returned.
fn parse_const_name(scanner: &mut Scanner<'_>) -> Option<String> {
    let saved_pos = scanner.pos;
    let saved_line = scanner.line;
    let saved_col = scanner.col;

    let prefix = ['@', 'o', 'r', 'i', 's', 'o', 'n', '/'];
    for (offset, expected) in prefix.iter().enumerate() {
        if scanner.peek(offset) != Some(*expected) {
            return None;
        }
    }
    for _ in 0..prefix.len() {
        scanner.advance();
    }

    let mut name = String::new();
    while let Some(ch) = scanner.peek(0) {
        if !is_const_name_char(ch) {
            break;
        }
        if let Some(c) = scanner.advance() {
            name.push(c);
        }
    }

    if name.is_empty() {
        scanner.pos = saved_pos;
        scanner.line = saved_line;
        scanner.col = saved_col;
        return None;
    }
    Some(name)
}

fn is_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn is_const_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' || ch == '/'
}

/// Peek-only: is the current cursor sitting on the literal string `needle`?
fn starts_with_at(scanner: &Scanner<'_>, needle: &str) -> bool {
    for (offset, expected) in needle.chars().enumerate() {
        if scanner.peek(offset) != Some(expected) {
            return false;
        }
    }
    true
}

/// Peek-only: capture the `${NAME}` substring at the current cursor for a
/// diagnostic message, without advancing the scanner. Returns an empty
/// string if the marker is malformed (the caller treats that as "no
/// marker").
fn capture_env_marker_snippet(scanner: &Scanner<'_>) -> String {
    if scanner.peek(0) != Some('$') || scanner.peek(1) != Some('{') {
        return String::new();
    }
    let mut snippet = String::from("${");
    let mut offset = 2usize;
    let mut found_close = false;
    let mut name_len = 0usize;
    while let Some(ch) = scanner.peek(offset) {
        if ch == '}' {
            snippet.push('}');
            found_close = true;
            break;
        }
        if !is_ident_char(ch) {
            break;
        }
        snippet.push(ch);
        name_len += 1;
        offset += 1;
    }
    if !found_close || name_len == 0 {
        return String::new();
    }
    snippet
}

/// Peek-only counterpart to [`capture_env_marker_snippet`] for `@orison/`
/// constants.
fn capture_const_marker_snippet(scanner: &Scanner<'_>) -> String {
    if !starts_with_at(scanner, "@orison/") {
        return String::new();
    }
    let mut snippet = String::from("@orison/");
    let mut offset = "@orison/".len();
    let mut name_len = 0usize;
    while let Some(ch) = scanner.peek(offset) {
        if !is_const_name_char(ch) {
            break;
        }
        snippet.push(ch);
        name_len += 1;
        offset += 1;
    }
    if name_len == 0 {
        return String::new();
    }
    snippet
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique env var names per test to keep the global process env from
    /// being contended by parallel test runs.
    fn unique_env_name(suffix: &str) -> String {
        format!(
            "ORI_PREPROC_TEST_{}_{}_{}",
            suffix.to_uppercase(),
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        )
    }

    fn cfg() -> PreprocessConfig {
        PreprocessConfig::new()
    }

    // --- 1. Constant substitution works. ---------------------------------
    #[test]
    fn constant_substitution_replaces_marker() {
        let config = cfg().with_const("version", "1.2.3");
        let (text, diags) = preprocess("ver = @orison/version", &config);
        assert_eq!(text, "ver = 1.2.3");
        assert!(diags.is_empty());
    }

    // --- 2. Env var substitution works when allowed. ---------------------
    #[test]
    fn env_substitution_replaces_when_allowed() {
        let name = unique_env_name("env_basic");
        std::env::set_var(&name, "hello-env");
        let config = cfg().allow_env_var(&name);
        let input = format!("greet = ${{{name}}}");
        let (text, diags) = preprocess(&input, &config);
        assert_eq!(text, "greet = hello-env");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        std::env::remove_var(&name);
    }

    // --- 3. Disallowed env emits PRE0010. --------------------------------
    #[test]
    fn disallowed_env_emits_pre0010() {
        // Even if the env var IS set in the environment, an entry that's
        // not in the allow-list must never be expanded.
        let name = unique_env_name("env_disallowed");
        std::env::set_var(&name, "should-not-appear");
        let config = cfg(); // no allow_env entries
        let input = format!("x = ${{{name}}}");
        let (text, diags) = preprocess(&input, &config);
        assert_eq!(text, input, "input must be preserved verbatim");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].id, "P3010");
        assert!(!text.contains("should-not-appear"));
        std::env::remove_var(&name);
    }

    // --- 4. Missing const emits PRE0020. ---------------------------------
    #[test]
    fn missing_const_emits_pre0020() {
        let (text, diags) = preprocess("x = @orison/missing", &cfg());
        assert_eq!(text, "x = @orison/missing");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].id, "P3020");
        assert!(diags[0].message.contains("@orison/missing"));
    }

    // --- 5. String literals are not touched (even when env is set). ------
    #[test]
    fn string_literals_are_preserved_verbatim() {
        let name = unique_env_name("env_string");
        std::env::set_var(&name, "INJECTED");
        let config = cfg().allow_env_var(&name);
        let input = format!("let s = \"hello ${{{name}}} world\";");
        let (text, diags) = preprocess(&input, &config);
        assert_eq!(text, input, "string contents must be byte-identical");
        assert!(!text.contains("INJECTED"));
        // We expect a PRE0030 string-literal-escape diagnostic to surface
        // so the caller learns why the marker was skipped.
        assert!(diags.iter().any(|d| d.id == "P3030"));
        std::env::remove_var(&name);
    }

    // --- 5b. Escaped quotes inside a string literal are respected. -------
    #[test]
    fn escaped_quote_does_not_terminate_string() {
        let config = cfg().with_const("v", "VV");
        // The string literal contains an escaped quote and an `@orison/v`
        // marker that must NOT be substituted. After the closing quote the
        // same marker outside should expand normally.
        let input = "let s = \"a \\\" @orison/v b\"; let t = @orison/v;";
        let (text, _diags) = preprocess(input, &config);
        assert!(text.contains("\"a \\\" @orison/v b\""), "got: {text}");
        assert!(text.ends_with("let t = VV;"));
    }

    // --- 6. Comment-internal markers are not touched. --------------------
    #[test]
    fn comment_markers_are_preserved_verbatim() {
        let name = unique_env_name("env_comment");
        std::env::set_var(&name, "NEVER");
        let config = cfg().allow_env_var(&name).with_const("ver", "9.9.9");
        let input =
            format!("// keep this: ${{{name}}} and @orison/ver verbatim\nlet v = @orison/ver;");
        let (text, _diags) = preprocess(&input, &config);
        let comment_line = text.lines().next().unwrap_or("");
        assert!(
            comment_line.contains(&format!("${{{name}}}")) && comment_line.contains("@orison/ver"),
            "comment must be untouched, got: {comment_line}"
        );
        assert!(text.contains("let v = 9.9.9;"));
        std::env::remove_var(&name);
    }

    // --- 7. Multiple markers on one line all substitute. -----------------
    #[test]
    fn multiple_markers_on_one_line_all_substitute() {
        let name = unique_env_name("env_multi");
        std::env::set_var(&name, "EV");
        let config = cfg().allow_env_var(&name).with_const("k", "KV");
        let input = format!("a=${{{name}}} b=@orison/k c=${{{name}}} d=@orison/k");
        let (text, diags) = preprocess(&input, &config);
        assert_eq!(text, "a=EV b=KV c=EV d=KV");
        assert!(diags.is_empty());
        std::env::remove_var(&name);
    }

    // --- 8. Determinism: identical inputs + config -> identical output. --
    #[test]
    fn determinism_two_runs_match() {
        let name = unique_env_name("env_det");
        std::env::set_var(&name, "DETERMINISTIC");
        let config = cfg().allow_env_var(&name).with_const("c", "CV");
        let input = format!("x=${{{name}}}, y=@orison/c, z=${{{name}}}");
        let (a, _) = preprocess(&input, &config);
        let (b, _) = preprocess(&input, &config);
        assert_eq!(a, b);
        std::env::remove_var(&name);
    }

    // --- 9. Idempotence: substituted text is a no-op on re-run. ----------
    #[test]
    fn idempotence_already_substituted_is_noop() {
        let config = cfg().with_const("v", "1.0");
        let (once, _) = preprocess("x = @orison/v;", &config);
        let (twice, diags) = preprocess(&once, &config);
        assert_eq!(once, twice);
        assert!(diags.is_empty());
    }

    // --- 10. Empty file is fine. -----------------------------------------
    #[test]
    fn empty_input_is_empty_output() {
        let (text, diags) = preprocess("", &cfg());
        assert!(text.is_empty());
        assert!(diags.is_empty());
    }

    // --- 11. Allow-list gate is honoured even when var is set in env. ----
    #[test]
    fn allow_list_is_load_bearing() {
        // This is the key safety invariant: an unset allow-list must mean
        // the env var is never read. We can only assert behaviour, but the
        // assertion is meaningful: the substituted text must not contain
        // the env var's value.
        let name = unique_env_name("env_gate");
        std::env::set_var(&name, "LEAKED-SECRET");
        let config = cfg();
        let input = format!("x = ${{{name}}}");
        let (text, _) = preprocess(&input, &config);
        assert!(
            !text.contains("LEAKED-SECRET"),
            "allow-list gate failed: {text}"
        );
        std::env::remove_var(&name);
    }

    // --- 12. Env cache: repeated ${X} reads yield identical values. -----
    #[test]
    fn env_value_is_cached_per_call() {
        let name = unique_env_name("env_cache");
        std::env::set_var(&name, "FIRST");
        let config = cfg().allow_env_var(&name);
        let input = format!("a=${{{name}}} b=${{{name}}}");
        // Note: we can't observe the cache directly, but we can observe
        // that both expansions yield the same value within a call.
        let (text, _) = preprocess(&input, &config);
        assert_eq!(text, "a=FIRST b=FIRST");
        std::env::remove_var(&name);
    }

    // --- 13. Malformed markers are passed through. -----------------------
    #[test]
    fn malformed_markers_pass_through() {
        // `${` without a closing brace is not a marker.
        let (text, diags) = preprocess("x = ${unclosed and $ alone", &cfg());
        assert_eq!(text, "x = ${unclosed and $ alone");
        assert!(diags.is_empty());
    }

    // --- 14. Multi-line input keeps line counters honest. ----------------
    #[test]
    fn diagnostic_span_tracks_line_number() {
        let input = "line1\nline2 ${UNSET}\nline3";
        let (_, diags) = preprocess(input, &cfg());
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].span.start.line, 2);
    }

    // --- 15. preprocess_file round-trips through the filesystem. ---------
    #[test]
    fn preprocess_file_reads_and_substitutes() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "ori_preproc_{}_{}.ori",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::write(&path, "x = @orison/v").ok();
        let config = cfg().with_const("v", "OK");
        let (text, diags) = preprocess_file(&path, &config).unwrap_or((String::new(), Vec::new()));
        let _ = std::fs::remove_file(&path);
        assert_eq!(text, "x = OK");
        assert!(diags.is_empty());
    }
}
