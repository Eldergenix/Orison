use ori_compiler::compiler::Compiler;
use ori_compiler::source::SourceFile;
use ori_lsp::diagnostics::{to_lsp_diagnostics, DIAGNOSTIC_SOURCE};
use ori_lsp::protocol::DiagnosticSeverity;

const NULL_SOURCE: &str =
    "module bad.null_example\n\nfn main() -> Unit:\n  let user = null\n  return Unit\n";
const TAB_SOURCE: &str = "module tabby\n\n\tfn main() -> Unit: return Unit\n";

#[test]
fn translates_null_error_diagnostic() {
    let result = Compiler::check_source(SourceFile::new("file:///bad.ori", NULL_SOURCE));
    let translated = to_lsp_diagnostics(&result.diagnostics);
    let error = translated
        .iter()
        .find(|d| d.code == "E0100")
        .expect("E0100 present");
    assert_eq!(error.severity, DiagnosticSeverity::Error);
    assert_eq!(error.source, DIAGNOSTIC_SOURCE);
    assert!(error.message.contains("null"));
    // Compiler span is line 4, columns 14..18 (1-based). LSP shift = -1.
    assert_eq!(error.range.start.line, 3);
    assert_eq!(error.range.start.character, 13);
    assert_eq!(error.range.end.line, 3);
    assert_eq!(error.range.end.character, 17);
    let data = error.data.as_ref().expect("encoded fixes");
    assert_eq!(data["schema"], "ori.lsp.fixes.v1");
    assert!(data["fixes"].as_array().is_some_and(|arr| !arr.is_empty()));
}

#[test]
fn translates_tab_warning_diagnostic() {
    let result = Compiler::check_source(SourceFile::new("file:///tabs.ori", TAB_SOURCE));
    let translated = to_lsp_diagnostics(&result.diagnostics);
    let warning = translated
        .iter()
        .find(|d| d.code == "W9001")
        .expect("W9001 present");
    assert_eq!(warning.severity, DiagnosticSeverity::Warning);
    // Tab is on the third line (line index 2 in 1-based, 2 in 0-based).
    assert_eq!(warning.range.start.line, 2);
    assert_eq!(warning.range.start.character, 0);
}

#[test]
fn zero_based_shift_does_not_underflow() {
    // Empty source still produces a module with a dummy span at line 1, col 1.
    // The translator must not panic and must clamp to zero.
    let result = Compiler::check_source(SourceFile::new("file:///empty.ori", ""));
    let _ = to_lsp_diagnostics(&result.diagnostics);
}
