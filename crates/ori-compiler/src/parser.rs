//! Top-level declaration parser. Produces a [`Module`] and an accompanying
//! diagnostic stream from a [`SourceFile`].

use crate::ast::{Module, Symbol, SymbolKind};
use crate::diagnostic::{Diagnostic, Fix};
use crate::effects::is_known_effect_or_capability;
use crate::lexer::{lex, Token, TokenKind};
use crate::source::{SourceFile, Span};
use std::collections::HashSet;

/// Output of [`parse_source`]: the (possibly partial) module and any
/// diagnostics emitted during parsing.
#[derive(Debug, Clone)]
pub struct ParseOutput {
    /// Parsed module (always present, even when diagnostics are non-empty).
    pub module: Module,
    /// Diagnostics produced during parsing.
    pub diagnostics: Vec<Diagnostic>,
}

/// Parse a single source file into a [`ParseOutput`].
pub fn parse_source(source: &SourceFile) -> ParseOutput {
    let tokens = lex(source);
    let mut diagnostics = Vec::new();
    let module_name = parse_module_name(source, &tokens, &mut diagnostics);
    let mut module = Module::new(module_name.clone(), source.path.clone());
    module.imports = parse_imports(&tokens, &mut diagnostics);

    let mut seen = HashSet::new();
    for symbol in parse_symbols(&module_name, &tokens, source, &mut diagnostics) {
        if !seen.insert(symbol.id.clone()) {
            diagnostics.push(
                Diagnostic::error(
                    "E0201",
                    format!("duplicate symbol `{}`", symbol.name),
                    symbol.span.clone(),
                )
                .with_symbol(symbol.id.clone())
                .with_agent_summary("Rename or remove one duplicate declaration.")
                .with_minimal_context(vec![symbol.id.clone()])
                .with_docs(vec!["doc:names.duplicates".to_string()]),
            );
            continue;
        }
        for effect in &symbol.effects {
            if !is_known_effect_or_capability(effect) {
                diagnostics.push(
                    Diagnostic::warning(
                        "W0401",
                        format!("unknown effect or capability `{effect}`"),
                        symbol.span.clone(),
                    )
                    .with_symbol(symbol.id.clone())
                    .with_expected(
                        crate::effects::KNOWN_EFFECTS
                            .iter()
                            .map(|name| (*name).to_string())
                            .collect(),
                    )
                    .with_found(vec![effect.clone()])
                    .with_agent_summary("Declare a capability or use a known effect name.")
                    .with_minimal_context(vec![symbol.id.clone()])
                    .with_docs(vec!["doc:effects.known-effects".to_string()]),
                );
            }
        }
        if symbol.kind == SymbolKind::Function && !symbol.signature.contains("->") {
            diagnostics.push(
                Diagnostic::warning(
                    "W0301",
                    format!("function `{}` has no explicit return type", symbol.name),
                    symbol.span.clone(),
                )
                .with_symbol(symbol.id.clone())
                .with_fix(Fix::new(
                    "add_return_type",
                    "Add `-> Unit` or the correct return type.",
                    0.74,
                ))
                .with_agent_summary("Public functions should use explicit return types.")
                .with_minimal_context(vec![symbol.id.clone()])
                .with_docs(vec!["doc:types.public-signatures".to_string()]),
            );
        }
        module.symbols.push(symbol);
    }

    scan_for_reserved_runtime_hazards(&tokens, &mut diagnostics);

    ParseOutput {
        module,
        diagnostics,
    }
}

fn parse_module_name(
    source: &SourceFile,
    tokens: &[Token],
    diagnostics: &mut Vec<Diagnostic>,
) -> String {
    for (idx, token) in tokens.iter().enumerate() {
        if token.kind == TokenKind::Keyword && token.lexeme == "module" {
            let mut name = String::new();
            let mut expect_ident = true;
            let mut j = idx + 1;
            while j < tokens.len() {
                let t = &tokens[j];
                match (t.kind, t.lexeme.as_str(), expect_ident) {
                    (TokenKind::Ident, _, true) => {
                        name.push_str(&t.lexeme);
                        expect_ident = false;
                    }
                    (TokenKind::Symbol, ".", false) => {
                        name.push('.');
                        expect_ident = true;
                    }
                    _ => break,
                }
                j += 1;
            }
            if name.is_empty() || name.ends_with('.') {
                diagnostics.push(
                    Diagnostic::error(
                        "E0002",
                        "module declaration requires a dotted module name",
                        token.span.clone(),
                    )
                    .with_expected(vec!["module app.name".to_string()])
                    .with_fix(Fix::new(
                        "replace_module_name",
                        "Use `module app.name`.",
                        0.85,
                    ))
                    .with_agent_summary("Fix the module declaration name.")
                    .with_docs(vec!["doc:modules.declaration".to_string()]),
                );
                return module_name_from_path(&source.path);
            }
            return name;
        }
    }

    diagnostics.push(
        Diagnostic::error(
            "E0001",
            "missing module declaration",
            Span::dummy(source.path.clone()),
        )
        .with_expected(vec!["module <name>".to_string()])
        .with_fix(Fix::new(
            "insert_module",
            "Insert `module <name>` at the top of the file.",
            0.90,
        ))
        .with_agent_summary("Add a module declaration at the top of the file.")
        .with_docs(vec!["doc:modules.declaration".to_string()]),
    );
    module_name_from_path(&source.path)
}

fn parse_imports(tokens: &[Token], diagnostics: &mut Vec<Diagnostic>) -> Vec<String> {
    let mut imports = Vec::new();
    for (idx, token) in tokens.iter().enumerate() {
        if token.kind == TokenKind::Keyword && token.lexeme == "import" {
            let line = token.span.start.line;
            let mut j = idx + 1;
            let mut name = String::new();
            let mut expect_ident = true;
            while j < tokens.len() && tokens[j].span.start.line == line {
                let t = &tokens[j];
                match (t.kind, t.lexeme.as_str(), expect_ident) {
                    (TokenKind::Ident, _, true) => {
                        name.push_str(&t.lexeme);
                        expect_ident = false;
                    }
                    (TokenKind::Symbol, ".", false) => {
                        name.push('.');
                        expect_ident = true;
                    }
                    _ => break,
                }
                j += 1;
            }
            if name.is_empty() || name.ends_with('.') {
                diagnostics.push(
                    Diagnostic::error(
                        "E0003",
                        "import declaration requires a dotted module path",
                        token.span.clone(),
                    )
                    .with_expected(vec!["import std.json".to_string()])
                    .with_agent_summary("Fix the import path.")
                    .with_docs(vec!["doc:modules.imports".to_string()]),
                );
            } else {
                imports.push(name);
            }
        }
    }
    imports
}

fn module_name_from_path(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = file.strip_suffix(".ori").unwrap_or(file);
    stem.replace('-', "_")
}

fn parse_symbols(
    module_name: &str,
    tokens: &[Token],
    source: &SourceFile,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Symbol> {
    let mut out = Vec::new();
    let mut idx = 0usize;
    while idx < tokens.len() {
        let token = &tokens[idx];
        let kind = match (token.kind, token.lexeme.as_str()) {
            (TokenKind::Keyword, "fn") => Some(SymbolKind::Function),
            (TokenKind::Keyword, "type") => Some(SymbolKind::Type),
            (TokenKind::Keyword, "service") => Some(SymbolKind::Service),
            (TokenKind::Keyword, "view") => Some(SymbolKind::View),
            (TokenKind::Keyword, "actor") => Some(SymbolKind::Actor),
            (TokenKind::Keyword, "query") => Some(SymbolKind::Query),
            (TokenKind::Keyword, "migration") => Some(SymbolKind::Migration),
            (TokenKind::Keyword, "capability") => Some(SymbolKind::Capability),
            _ => None,
        };
        if let Some(kind) = kind {
            if let Some(name_token) = tokens.get(idx + 1) {
                if matches!(name_token.kind, TokenKind::Ident) {
                    let line = token.span.start.line;
                    let signature = collect_signature(tokens, idx, line);
                    let effects = collect_effects(tokens, idx, line);
                    let id = format!("sym:{module_name}.{}", name_token.lexeme);
                    out.push(Symbol {
                        id,
                        name: name_token.lexeme.clone(),
                        kind,
                        signature,
                        effects,
                        span: Span::new(
                            source.path.clone(),
                            line,
                            token.span.start.column,
                            line,
                            name_token.span.end.column,
                        ),
                    });
                } else {
                    diagnostics.push(
                        Diagnostic::error(
                            "E0200",
                            format!("{} declaration requires a name", token.lexeme),
                            token.span.clone(),
                        )
                        .with_expected(vec![format!("{} Name", token.lexeme)])
                        .with_agent_summary("Add an identifier after the declaration keyword.")
                        .with_docs(vec!["doc:names.declarations".to_string()]),
                    );
                }
            }
        }
        idx += 1;
    }
    out
}

fn collect_signature(tokens: &[Token], start_idx: usize, line: usize) -> String {
    let mut parts = Vec::new();
    let mut idx = start_idx;
    while idx < tokens.len()
        && tokens[idx].span.start.line == line
        && tokens[idx].kind != TokenKind::Eof
    {
        parts.push(tokens[idx].lexeme.clone());
        idx += 1;
    }
    if matches!(parts.last().map(String::as_str), Some(":")) {
        parts.pop();
    }
    compact_signature(&parts)
}

fn compact_signature(parts: &[String]) -> String {
    let mut out = String::new();
    for part in parts {
        let no_space_before = matches!(part.as_str(), ")" | "]" | "," | ":" | "." | "(" | "[");
        let no_space_after_prev = out.ends_with('(') || out.ends_with('[') || out.ends_with('.');
        if !out.is_empty() && !no_space_before && !no_space_after_prev {
            out.push(' ');
        }
        out.push_str(part);
    }
    out
}

fn collect_effects(tokens: &[Token], start_idx: usize, line: usize) -> Vec<String> {
    let mut effects = Vec::new();
    let mut idx = start_idx;
    while idx < tokens.len() && tokens[idx].span.start.line == line {
        if tokens[idx].lexeme == "uses" {
            idx += 1;
            let mut current = String::new();
            while idx < tokens.len() && tokens[idx].span.start.line == line {
                let t = &tokens[idx];
                if t.lexeme == ":" || t.lexeme == "->" {
                    break;
                }
                if t.lexeme == "," {
                    if !current.is_empty() {
                        effects.push(current.clone());
                        current.clear();
                    }
                } else if t.lexeme == "." {
                    current.push('.');
                } else if matches!(t.kind, TokenKind::Ident | TokenKind::Keyword) {
                    current.push_str(&t.lexeme);
                }
                idx += 1;
            }
            if !current.is_empty() {
                effects.push(current);
            }
            break;
        }
        idx += 1;
    }
    effects
}

fn scan_for_reserved_runtime_hazards(tokens: &[Token], diagnostics: &mut Vec<Diagnostic>) {
    for token in tokens {
        if token.kind == TokenKind::Ident && token.lexeme == "null" {
            diagnostics.push(
                Diagnostic::error(
                    "E0100",
                    "`null` is not part of Orison; use Option[T]",
                    token.span.clone(),
                )
                .with_expected(vec![
                    "Option[T]".to_string(),
                    "None".to_string(),
                    "Some(value)".to_string(),
                ])
                .with_found(vec!["null".to_string()])
                .with_fix(Fix::new(
                    "replace_null",
                    "Replace `null` with `None` or an explicit Option value.",
                    0.82,
                ))
                .with_agent_summary("Replace null with Option semantics.")
                .with_docs(vec!["doc:types.option".to_string()]),
            );
        }
        if token.lexeme == "throw" {
            diagnostics.push(
                Diagnostic::error(
                    "E0101",
                    "exceptions are not part of Orison; return Result[T, E]",
                    token.span.clone(),
                )
                .with_expected(vec!["Result[T, E]".to_string(), "Err(value)".to_string()])
                .with_found(vec!["throw".to_string()])
                .with_fix(Fix::new(
                    "replace_throw",
                    "Return `Err(...)` from a Result-returning function.",
                    0.76,
                ))
                .with_agent_summary("Replace exception-style control flow with Result.")
                .with_docs(vec!["doc:errors.result".to_string()]),
            );
        }
    }
}
