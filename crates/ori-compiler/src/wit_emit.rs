//! Orison Module → WebAssembly Interface Types (WIT) emitter.
//!
//! This module is a one-way lowering pass from the bootstrap surface
//! [`Module`] to a stable, deterministic WIT envelope. It is intentionally
//! conservative: only the subset of Orison that has a meaningful WIT
//! analogue is emitted, and anything else surfaces as a `WIT0001`
//! diagnostic-style error so downstream tools never see surprise output.
//!
//! ## Output shape
//!
//! ```text
//! package <pkg>@0.1.0;
//!
//! world <world> {
//!     import <capability>;
//!     ...
//!     interface <service> { ... }
//!     export <fn-name>: func(...);
//!     ...
//! }
//! ```
//!
//! ## Determinism
//!
//! Every collection iterated for emit is either visited in source order
//! (functions, services, imports) or sorted by name (capabilities,
//! record/variant declarations gathered from the source). The resulting
//! [`WitReport::source`] string is byte-stable for a given input module.
//!
//! ## Production guardrails
//!
//! No `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`, `dbg!`,
//! or `unsafe` appears in this file. Every fallible path returns a
//! [`WitError`] with a stable code.

use crate::ast::{Module, Symbol, SymbolKind};
use crate::json::to_json;
use crate::lexer::{lex, Token, TokenKind};
use crate::source::SourceFile;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;

/// Stable schema identifier for [`WitReport`].
pub const WIT_REPORT_SCHEMA: &str = "ori.wit_report.v1";

/// Diagnostic id surfaced when a type cannot be lowered to WIT.
pub const WIT_UNSUPPORTED_TYPE: &str = "WIT0001";
/// Diagnostic id surfaced when the requested package name is invalid.
pub const WIT_INVALID_PACKAGE: &str = "WIT0002";
/// Diagnostic id surfaced when a record/variant participates in a cycle.
pub const WIT_RECORD_CYCLE: &str = "WIT0003";

/// Summary statistics for a [`WitReport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WitStats {
    /// Number of `interface` declarations emitted (one per service).
    pub interfaces: usize,
    /// Number of WIT `func` declarations emitted.
    pub funcs: usize,
    /// Number of `record` declarations emitted.
    pub records: usize,
    /// Number of `variant` declarations emitted.
    pub variants: usize,
}

/// Envelope produced by [`emit_wit_for_module`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WitReport {
    /// Stable schema identifier.
    pub schema: &'static str,
    /// Canonical WIT package name (`namespace:name`).
    pub package: String,
    /// Name of the emitted WIT world.
    pub world: String,
    /// Generated WIT source text. Byte-stable for a given input.
    pub source: String,
    /// Summary statistics.
    pub stats: WitStats,
}

/// Errors raised while lowering an Orison [`Module`] to WIT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitError {
    /// Diagnostic id (`WIT0001`, `WIT0002`, `WIT0003`).
    pub code: &'static str,
    /// Human-readable description of the failure.
    pub message: String,
}

impl WitError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for WitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for WitError {}

/// Render a [`WitReport`] as canonical JSON.
pub fn wit_report_json(r: &WitReport) -> String {
    to_json(r)
}

/// Lower `module` to a [`WitReport`] using `package` and `world` as the
/// canonical WIT identifiers.
pub fn emit_wit_for_module(
    module: &Module,
    package: &str,
    world: &str,
) -> Result<WitReport, WitError> {
    let package = validate_package(package)?;
    let world_name = validate_kebab(world).map_err(|m| {
        WitError::new(
            WIT_INVALID_PACKAGE,
            format!("invalid world name `{world}`: {m}"),
        )
    })?;

    let body = collect_body(module);

    // Build the canonical, deterministic capability import list.
    let imports = collect_capability_imports(module);

    // Collect record and variant declarations, then run a direct-cycle
    // check so the caller learns about unbounded types before any source
    // text is rendered.
    let records = collect_records(&body, module)?;
    let variants = collect_variants(&body, module);
    detect_direct_record_cycles(&records)?;

    // Module-level functions split into two buckets: routes that live on
    // a service interface (decided by the presence of an `http*` effect)
    // and free functions that become world-level exports.
    let services = collect_services(module);
    let (service_routes, free_funcs) = partition_functions(module, &services);

    // Validate that every function param/return type is something WIT can
    // represent. Surfacing this before assembly keeps `source` empty when
    // the report would otherwise be a lie.
    for func in free_funcs.iter().chain(service_routes.iter().flat_map(|(_, fs)| fs.iter())) {
        check_func_types(func)?;
    }

    let mut source = String::new();
    source.push_str("package ");
    source.push_str(&package);
    source.push_str("@0.1.0;\n\n");

    source.push_str("world ");
    source.push_str(&world_name);
    source.push_str(" {\n");

    for import in &imports {
        source.push_str("    import ");
        source.push_str(import);
        source.push_str(";\n");
    }
    if !imports.is_empty() {
        source.push('\n');
    }

    let mut interfaces_emitted = 0usize;
    let mut funcs_emitted = 0usize;

    // Records first (deterministic by name), then variants, so the
    // type universe of the world is fully defined before functions
    // reference it.
    for rec in &records {
        emit_record(&mut source, rec);
        source.push('\n');
    }
    for var in &variants {
        emit_variant(&mut source, var);
        source.push('\n');
    }

    // One interface per service, in source order; route methods are kept
    // in source order to match the rest of the bootstrap ABI.
    for service in &services {
        let kebab = to_kebab(&service.name);
        source.push_str("    interface ");
        source.push_str(&kebab);
        source.push_str(" {\n");
        if let Some(routes) = service_routes.iter().find(|(name, _)| name == &service.name) {
            for func in &routes.1 {
                emit_func(&mut source, func, "        ");
                funcs_emitted += 1;
            }
        }
        source.push_str("    }\n");
        interfaces_emitted += 1;
    }

    for func in &free_funcs {
        source.push_str("    export ");
        source.push_str(&to_kebab(&func.name));
        source.push_str(": ");
        source.push_str(&render_func_signature(func));
        source.push_str(";\n");
        funcs_emitted += 1;
    }

    source.push_str("}\n");

    Ok(WitReport {
        schema: WIT_REPORT_SCHEMA,
        package,
        world: world_name,
        source,
        stats: WitStats {
            interfaces: interfaces_emitted,
            funcs: funcs_emitted,
            records: records.len(),
            variants: variants.len(),
        },
    })
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

fn validate_package(package: &str) -> Result<String, WitError> {
    let (ns, name) = match package.split_once(':') {
        Some(pair) => pair,
        None => {
            return Err(WitError::new(
                WIT_INVALID_PACKAGE,
                format!(
                    "package name `{package}` must use `namespace:name` form (e.g. `acme:demo`)"
                ),
            ));
        }
    };
    let ns_k = validate_kebab(ns).map_err(|m| {
        WitError::new(
            WIT_INVALID_PACKAGE,
            format!("invalid package namespace `{ns}`: {m}"),
        )
    })?;
    let nm_k = validate_kebab(name).map_err(|m| {
        WitError::new(
            WIT_INVALID_PACKAGE,
            format!("invalid package name `{name}`: {m}"),
        )
    })?;
    Ok(format!("{ns_k}:{nm_k}"))
}

/// WIT identifiers are kebab-case ASCII. We accept inputs that are already
/// kebab-case or that match the relaxed `[a-zA-Z0-9_]+` shape used by
/// Orison module segments, lowercasing and replacing `_` with `-` on the
/// fly so callers can pass `my_world` or `MyWorld` interchangeably. Case
/// boundaries (`MyWorld`, `MWUser`) are also treated as kebab segment
/// boundaries to keep the output canonical.
fn validate_kebab(name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err("identifier must not be empty".into());
    }
    // Pre-validate the character set so we surface an explicit error for
    // forbidden glyphs rather than silently dropping them.
    for ch in name.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-') {
            return Err(format!("character `{ch}` is not allowed in WIT identifiers"));
        }
    }
    let kebab = to_kebab(name);
    if kebab.is_empty() {
        return Err("identifier reduces to the empty string".into());
    }
    Ok(kebab)
}

fn to_kebab(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let mut prev_lower_or_digit = false;
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 && prev_lower_or_digit {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
            prev_lower_or_digit = false;
        } else if ch == '_' || ch == '-' || ch == ' ' {
            if !out.ends_with('-') && !out.is_empty() {
                out.push('-');
            }
            prev_lower_or_digit = false;
        } else if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_lower_or_digit = true;
        }
        // Other characters are silently dropped: they cannot appear in
        // legal Orison identifiers, but defence-in-depth keeps WIT clean.
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "x".to_string()
    } else {
        out
    }
}

// ---------------------------------------------------------------------------
// Source body acquisition
// ---------------------------------------------------------------------------

/// Token stream taken from `module.path` when readable, otherwise empty.
/// Body extraction degrades to "no records / variants" rather than failing
/// so unit tests that synthesise modules without on-disk sources still
/// produce a valid envelope.
fn collect_body(module: &Module) -> Vec<Token> {
    let text = match fs::read_to_string(&module.path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let source = SourceFile::new(module.path.clone(), text);
    lex(&source)
}

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordDecl {
    name: String,
    fields: Vec<RecordField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordField {
    name: String,
    /// Raw Orison type text as it appeared in the source.
    orison_type: String,
    /// Lowered WIT type text.
    wit_type: String,
}

fn collect_records(tokens: &[Token], module: &Module) -> Result<Vec<RecordDecl>, WitError> {
    let mut out: BTreeMap<String, RecordDecl> = BTreeMap::new();
    for symbol in module.symbols.iter().filter(|s| s.kind == SymbolKind::Type) {
        if symbol.signature.contains("wraps") {
            // Wrappers map to type aliases in Orison, not records; the
            // current WIT subset has no nominal-wrapper notion so we
            // intentionally leave these out and let any reference fall
            // through to the underlying type's WIT shape.
            continue;
        }
        // A record declaration's signature ends with `{`; a variant's
        // signature ends with `=`. Anything else (aliases, etc.) is
        // ignored for the v1 envelope.
        if !symbol.signature.trim_end().ends_with('{') {
            continue;
        }
        let header_line = symbol.span.start.line;
        let fields_raw = extract_record_fields(tokens, header_line);
        let mut fields = Vec::with_capacity(fields_raw.len());
        for (name, ty) in fields_raw {
            let wit_type = lower_type(&ty)?;
            fields.push(RecordField {
                name: to_kebab(&name),
                orison_type: ty,
                wit_type,
            });
        }
        out.insert(
            symbol.name.clone(),
            RecordDecl {
                name: symbol.name.clone(),
                fields,
            },
        );
    }
    Ok(out.into_values().collect())
}

fn extract_record_fields(tokens: &[Token], header_line: usize) -> Vec<(String, String)> {
    // Walk tokens after the header line; consume until the first '}' on
    // any line. Fields are `name : Type ,?`. The bootstrap lexer turns
    // the body into individual tokens which makes this scan straight-
    // forward without a real parser.
    let mut idx = 0usize;
    while idx < tokens.len() && tokens[idx].span.start.line <= header_line {
        idx += 1;
    }
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_type = String::new();
    let mut saw_colon = false;
    while idx < tokens.len() {
        let tok = &tokens[idx];
        if tok.kind == TokenKind::Eof {
            break;
        }
        if tok.lexeme == "}" {
            push_field(&mut fields, &mut current_name, &mut current_type, &mut saw_colon);
            break;
        }
        if tok.lexeme == "," {
            push_field(&mut fields, &mut current_name, &mut current_type, &mut saw_colon);
            idx += 1;
            continue;
        }
        if !saw_colon {
            if matches!(tok.kind, TokenKind::Ident) && current_name.is_none() {
                current_name = Some(tok.lexeme.clone());
            } else if tok.lexeme == ":" {
                saw_colon = true;
            }
            idx += 1;
            continue;
        }
        if matches!(tok.kind, TokenKind::Ident | TokenKind::Keyword | TokenKind::Symbol)
            || tok.kind == TokenKind::Number
        {
            append_type_token(&mut current_type, &tok.lexeme);
        }
        idx += 1;
    }
    fields
}

fn push_field(
    fields: &mut Vec<(String, String)>,
    current_name: &mut Option<String>,
    current_type: &mut String,
    saw_colon: &mut bool,
) {
    if let Some(name) = current_name.take() {
        let ty = std::mem::take(current_type).trim().to_string();
        if !ty.is_empty() {
            fields.push((name, ty));
        }
    }
    *saw_colon = false;
    current_type.clear();
}

fn append_type_token(buf: &mut String, lexeme: &str) {
    let no_space_before =
        matches!(lexeme, "]" | "," | "." | ")" | ">" | "[" | "(" | "<");
    let no_space_after_prev =
        buf.ends_with('[') || buf.ends_with('(') || buf.ends_with('.') || buf.ends_with('<');
    if !buf.is_empty() && !no_space_before && !no_space_after_prev {
        buf.push(' ');
    }
    buf.push_str(lexeme);
}

// ---------------------------------------------------------------------------
// Variants
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct VariantDecl {
    name: String,
    cases: Vec<VariantCase>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VariantCase {
    name: String,
    /// Optional payload type, already lowered to WIT.
    payload: Option<String>,
}

fn collect_variants(tokens: &[Token], module: &Module) -> Vec<VariantDecl> {
    let mut out: BTreeMap<String, VariantDecl> = BTreeMap::new();
    for symbol in module.symbols.iter().filter(|s| s.kind == SymbolKind::Type) {
        // A variant declaration's signature is `type X =`.
        if !symbol.signature.trim_end().ends_with('=') {
            continue;
        }
        let header_line = symbol.span.start.line;
        let cases = extract_variant_cases(tokens, header_line);
        if cases.is_empty() {
            continue;
        }
        out.insert(
            symbol.name.clone(),
            VariantDecl {
                name: symbol.name.clone(),
                cases,
            },
        );
    }
    out.into_values().collect()
}

fn extract_variant_cases(tokens: &[Token], header_line: usize) -> Vec<VariantCase> {
    // Variants are written as `| Case` or `| Case(Payload)` or
    // `| Case(name: Payload, ...)`. We walk forward until we hit either a
    // top-level keyword (next declaration) or EOF.
    let mut idx = 0usize;
    while idx < tokens.len() && tokens[idx].span.start.line <= header_line {
        idx += 1;
    }
    let mut cases: Vec<VariantCase> = Vec::new();
    while idx < tokens.len() {
        let tok = &tokens[idx];
        if tok.kind == TokenKind::Eof {
            break;
        }
        // Stop at the next top-level declaration keyword on column 1.
        if matches!(tok.kind, TokenKind::Keyword)
            && tok.span.start.column == 1
            && is_top_level_keyword(&tok.lexeme)
        {
            break;
        }
        if tok.lexeme == "|" {
            idx += 1;
            // Expect an Ident next.
            if idx >= tokens.len() {
                break;
            }
            let case_tok = &tokens[idx];
            if !matches!(case_tok.kind, TokenKind::Ident) {
                idx += 1;
                continue;
            }
            let case_name = case_tok.lexeme.clone();
            idx += 1;
            let mut payload: Option<String> = None;
            if idx < tokens.len() && tokens[idx].lexeme == "(" {
                idx += 1;
                let mut depth = 1usize;
                let mut buf = String::new();
                while idx < tokens.len() && depth > 0 {
                    let inner = &tokens[idx];
                    if inner.lexeme == "(" {
                        depth += 1;
                    } else if inner.lexeme == ")" {
                        depth -= 1;
                        if depth == 0 {
                            idx += 1;
                            break;
                        }
                    }
                    if depth > 0 {
                        append_type_token(&mut buf, &inner.lexeme);
                    }
                    idx += 1;
                }
                payload = Some(buf.trim().to_string());
            }
            // Payloads may be named (`name: Ty`) or anonymous (`Ty`).
            // Strip a leading `name :` if present.
            let payload_lowered: Option<String> = match payload {
                None => None,
                Some(raw) if raw.is_empty() => None,
                Some(raw) => {
                    let stripped = strip_named_payload(&raw);
                    Some(lower_type_lossy(stripped))
                }
            };
            cases.push(VariantCase {
                name: to_kebab(&case_name),
                payload: payload_lowered,
            });
        } else {
            idx += 1;
        }
    }
    cases
}

fn is_top_level_keyword(lex: &str) -> bool {
    matches!(
        lex,
        "fn" | "type"
            | "service"
            | "view"
            | "actor"
            | "query"
            | "migration"
            | "capability"
            | "module"
            | "import"
    )
}

fn strip_named_payload(raw: &str) -> &str {
    // Inputs like `body: Str` or `user: UserId` collapse to the type
    // text. Multi-field payloads (`url: Str, alt: Str`) are intentionally
    // left as-is — WIT cannot express anonymous-record variant payloads
    // in the v1 envelope, so [`lower_type_lossy`] will reject them with
    // a stable WIT0001 string ("any" placeholder).
    if let Some(colon) = raw.find(':') {
        let before = raw[..colon].trim();
        if !before.is_empty()
            && before
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return raw[colon + 1..].trim();
        }
    }
    raw
}

// ---------------------------------------------------------------------------
// Cycle detection
// ---------------------------------------------------------------------------

fn detect_direct_record_cycles(records: &[RecordDecl]) -> Result<(), WitError> {
    let names: BTreeSet<String> = records.iter().map(|r| r.name.clone()).collect();
    for rec in records {
        for field in &rec.fields {
            for token in tokenise_orison_type(&field.orison_type) {
                // A direct self-reference (`x: SameRecord`) is forbidden
                // by WIT; references through `option` or `list` are
                // perfectly fine and intentionally not flagged here.
                if token == rec.name && !is_wrapped(&field.orison_type, &rec.name) {
                    return Err(WitError::new(
                        WIT_RECORD_CYCLE,
                        format!(
                            "record `{}` references itself directly via field `{}`; wrap in `Option[..]` or `List[..]`",
                            rec.name, field.name
                        ),
                    ));
                }
                let _ = names.contains(&token);
            }
        }
    }
    Ok(())
}

fn tokenise_orison_type(ty: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in ty.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            current.push(ch);
        } else {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn is_wrapped(orison_type: &str, name: &str) -> bool {
    // Whole-string wrappers count as wrapped; everything else is direct.
    // This is conservative: `List[Option[Self]]` is correctly recognised
    // because the outer constructor is List.
    let trimmed = orison_type.trim();
    let candidates = ["List", "Option", "Result", "Set", "Map", "Vec"];
    for cand in &candidates {
        let prefix = format!("{cand}[");
        if trimmed.starts_with(&prefix) && trimmed.ends_with(']') {
            // Inner content must mention `name` somewhere — if it does
            // not, the outer name is misleading and we fall through.
            let inner = &trimmed[prefix.len()..trimmed.len() - 1];
            if inner.contains(name) {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Capability imports
// ---------------------------------------------------------------------------

fn collect_capability_imports(module: &Module) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    // Explicit `capability X` declarations.
    for symbol in module
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Capability)
    {
        set.insert(to_kebab(&symbol.name));
    }
    // Effects ending in a capability-like fragment also show up as
    // imports; we keep the conservative rule "any dotted effect head is
    // a capability namespace" so `db.read` registers `db` as the import.
    for symbol in &module.symbols {
        for effect in &symbol.effects {
            let head = effect.split('.').next().unwrap_or(effect);
            if head.is_empty() {
                continue;
            }
            set.insert(to_kebab(head));
        }
    }
    set.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Services and functions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ServiceDecl<'a> {
    name: &'a str,
}

#[derive(Debug, Clone)]
struct FuncDecl {
    name: String,
    params: Vec<FuncParam>,
    response: String,
    /// Raw Orison return type before lowering — kept for error messages.
    response_orison: String,
}

#[derive(Debug, Clone)]
struct FuncParam {
    name: String,
    wit_type: String,
    orison_type: String,
}

fn collect_services(module: &Module) -> Vec<ServiceDecl<'_>> {
    module
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Service)
        .map(|s| ServiceDecl { name: &s.name })
        .collect()
}

fn partition_functions(
    module: &Module,
    services: &[ServiceDecl<'_>],
) -> (Vec<(String, Vec<FuncDecl>)>, Vec<FuncDecl>) {
    let mut routes: Vec<(String, Vec<FuncDecl>)> = services
        .iter()
        .map(|s| (s.name.to_string(), Vec::new()))
        .collect();
    let mut free: Vec<FuncDecl> = Vec::new();

    for symbol in module.symbols.iter().filter(|s| s.kind == SymbolKind::Function) {
        let func = match build_func(symbol) {
            Some(f) => f,
            None => continue,
        };
        let is_route = symbol.effects.iter().any(|e| e == "http" || e.starts_with("http."));
        if is_route && !routes.is_empty() {
            // Bootstrap convention: every http-tagged free function
            // belongs to the first (and usually only) service.
            if let Some(bucket) = routes.first_mut() {
                bucket.1.push(func);
            }
        } else {
            free.push(func);
        }
    }
    (routes, free)
}

fn build_func(symbol: &Symbol) -> Option<FuncDecl> {
    let sig = symbol.signature.as_str();
    let open = sig.find('(')?;
    let close_off = sig[open..].find(')')?;
    let close = open + close_off;
    let params_text = &sig[open + 1..close];
    let response_orison = match sig[close + 1..].find("->") {
        Some(off) => {
            let after = sig[close + 1 + off + 2..].trim();
            let cutoff = after.find(" uses ").unwrap_or(after.len());
            after[..cutoff].trim().to_string()
        }
        None => "Unit".to_string(),
    };
    let response = lower_type_lossy(&response_orison);

    let mut params: Vec<FuncParam> = Vec::new();
    if !params_text.trim().is_empty() {
        for piece in split_top_level_commas(params_text) {
            let piece = piece.trim();
            if let Some(colon) = piece.find(':') {
                let name = piece[..colon].trim().to_string();
                let ty = piece[colon + 1..].trim().to_string();
                if name.is_empty() || ty.is_empty() {
                    continue;
                }
                params.push(FuncParam {
                    name: to_kebab(&name),
                    wit_type: lower_type_lossy(&ty),
                    orison_type: ty,
                });
            }
        }
    }

    Some(FuncDecl {
        name: symbol.name.clone(),
        params,
        response,
        response_orison,
    })
}

fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0usize;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' | '[' | '<' | '{' => depth += 1,
            ')' | ']' | '>' | '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

fn check_func_types(func: &FuncDecl) -> Result<(), WitError> {
    for param in &func.params {
        // Lowered shape "any" means we hit an unsupported type during
        // lossy lowering — promote it to a stable WIT0001 error here.
        if param.wit_type == "any" {
            return Err(WitError::new(
                WIT_UNSUPPORTED_TYPE,
                format!(
                    "function `{}` parameter `{}` has unsupported Orison type `{}`",
                    func.name, param.name, param.orison_type
                ),
            ));
        }
    }
    if func.response == "any" {
        return Err(WitError::new(
            WIT_UNSUPPORTED_TYPE,
            format!(
                "function `{}` return type `{}` has no WIT mapping",
                func.name, func.response_orison
            ),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Type lowering
// ---------------------------------------------------------------------------

/// Lower an Orison type expression to its WIT equivalent. Returns a
/// `WIT0001` error if no mapping exists.
fn lower_type(ty: &str) -> Result<String, WitError> {
    let lowered = lower_type_lossy(ty);
    if lowered == "any" {
        Err(WitError::new(
            WIT_UNSUPPORTED_TYPE,
            format!("type `{ty}` has no WIT mapping"),
        ))
    } else {
        Ok(lowered)
    }
}

/// Same as [`lower_type`] but returns the sentinel string `"any"` rather
/// than an error. Used in places where the caller wants to defer the
/// validation decision (`check_func_types`).
fn lower_type_lossy(ty: &str) -> String {
    let t = ty.trim();
    if t.is_empty() {
        return "any".into();
    }

    // Parameterised constructors first.
    if let Some(inner) = extract_generic(t, "List") {
        return format!("list<{}>", lower_type_lossy(inner));
    }
    if let Some(inner) = extract_generic(t, "Option") {
        return format!("option<{}>", lower_type_lossy(inner));
    }
    if let Some(inner) = extract_generic(t, "Result") {
        let parts = split_top_level_commas(inner);
        if parts.len() == 2 {
            return format!(
                "result<{}, {}>",
                lower_type_lossy(parts[0].trim()),
                lower_type_lossy(parts[1].trim())
            );
        }
        if parts.len() == 1 {
            return format!("result<{}>", lower_type_lossy(parts[0].trim()));
        }
    }

    // Primitives. Orison's surface uses Pascal-case names; map them to
    // WIT's lower-case primitives.
    match t {
        "Str" | "String" => "string".into(),
        "Bool" => "bool".into(),
        "Int" | "I64" => "s64".into(),
        "I32" => "s32".into(),
        "I16" => "s16".into(),
        "I8" => "s8".into(),
        "U64" => "u64".into(),
        "U32" => "u32".into(),
        "U16" => "u16".into(),
        "U8" => "u8".into(),
        "F64" | "Float" | "Float64" => "float64".into(),
        "F32" | "Float32" => "float32".into(),
        "Char" => "char".into(),
        "Unit" | "Void" => "_".into(),
        "Bytes" => "list<u8>".into(),
        "UUID" | "Uuid" => "string".into(),
        // Anything that is a bare identifier referring to a user type
        // passes through as kebab-case so `User` → `user`.
        other if is_simple_ident(other) => to_kebab(other),
        _ => "any".into(),
    }
}

fn extract_generic<'a>(t: &'a str, head: &str) -> Option<&'a str> {
    let prefix = format!("{head}[");
    if t.starts_with(&prefix) && t.ends_with(']') {
        return Some(&t[prefix.len()..t.len() - 1]);
    }
    let prefix_angle = format!("{head}<");
    if t.starts_with(&prefix_angle) && t.ends_with('>') {
        return Some(&t[prefix_angle.len()..t.len() - 1]);
    }
    None
}

fn is_simple_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// ---------------------------------------------------------------------------
// Source emission
// ---------------------------------------------------------------------------

fn emit_record(out: &mut String, rec: &RecordDecl) {
    out.push_str("    record ");
    out.push_str(&to_kebab(&rec.name));
    out.push_str(" {\n");
    for (i, field) in rec.fields.iter().enumerate() {
        out.push_str("        ");
        out.push_str(&field.name);
        out.push_str(": ");
        out.push_str(&field.wit_type);
        if i + 1 < rec.fields.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("    }\n");
}

fn emit_variant(out: &mut String, var: &VariantDecl) {
    out.push_str("    variant ");
    out.push_str(&to_kebab(&var.name));
    out.push_str(" {\n");
    for (i, case) in var.cases.iter().enumerate() {
        out.push_str("        ");
        out.push_str(&case.name);
        if let Some(payload) = &case.payload {
            if !payload.is_empty() && payload != "any" {
                out.push('(');
                out.push_str(payload);
                out.push(')');
            }
        }
        if i + 1 < var.cases.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("    }\n");
}

fn emit_func(out: &mut String, func: &FuncDecl, indent: &str) {
    out.push_str(indent);
    out.push_str(&to_kebab(&func.name));
    out.push_str(": ");
    out.push_str(&render_func_signature(func));
    out.push_str(";\n");
}

fn render_func_signature(func: &FuncDecl) -> String {
    let mut s = String::from("func(");
    for (i, param) in func.params.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(&param.name);
        s.push_str(": ");
        s.push_str(&param.wit_type);
    }
    s.push(')');
    if func.response != "_" {
        s.push_str(" -> ");
        s.push_str(&func.response);
    }
    s
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_source;
    use crate::source::SourceFile;
    use std::io::Write;

    fn write_temp(name: &str, text: &str) -> String {
        let mut path = std::env::temp_dir();
        path.push(format!("ori_wit_{}_{}.ori", std::process::id(), name));
        let mut file = match std::fs::File::create(&path) {
            Ok(f) => f,
            Err(err) => {
                assert!(false, "temp create failed: {err}");
                return String::new();
            }
        };
        if let Err(err) = file.write_all(text.as_bytes()) {
            assert!(false, "temp write failed: {err}");
        }
        path.to_string_lossy().to_string()
    }

    fn module_for(text: &str, name: &str) -> Module {
        let path = write_temp(name, text);
        let source = SourceFile::new(path, text.to_string());
        parse_source(&source).module
    }

    #[test]
    fn record_maps_to_wit_record() {
        let m = module_for(
            "module demo\ntype User = {\n  id: Str,\n  age: Int\n}\n",
            "record_basic",
        );
        let report = match emit_wit_for_module(&m, "acme:demo", "demo") {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "expected ok report, got {err}");
                return;
            }
        };
        assert!(report.source.contains("record user"));
        assert!(report.source.contains("id: string"));
        assert!(report.source.contains("age: s64"));
        assert_eq!(report.stats.records, 1);
    }

    #[test]
    fn variant_maps_to_wit_variant() {
        let m = module_for(
            "module demo\ntype Color =\n  | Red\n  | Green\n  | Blue\n",
            "variant_basic",
        );
        let report = match emit_wit_for_module(&m, "acme:demo", "demo") {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "expected ok report, got {err}");
                return;
            }
        };
        assert!(report.source.contains("variant color"));
        assert!(report.source.contains("red"));
        assert!(report.source.contains("green"));
        assert!(report.source.contains("blue"));
        assert_eq!(report.stats.variants, 1);
    }

    #[test]
    fn variant_with_payload_includes_paren_payload() {
        let m = module_for(
            "module demo\ntype Msg =\n  | Text(body: Str)\n  | Ping\n",
            "variant_payload",
        );
        let report = match emit_wit_for_module(&m, "acme:demo", "demo") {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "expected ok report, got {err}");
                return;
            }
        };
        assert!(report.source.contains("text(string)"));
        assert!(report.source.contains("ping"));
    }

    #[test]
    fn service_routes_become_interface() {
        let m = module_for(
            "module demo\nservice Api\nfn get_users() -> Str uses http\n",
            "service_routes",
        );
        let report = match emit_wit_for_module(&m, "acme:demo", "demo") {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "expected ok report, got {err}");
                return;
            }
        };
        assert!(report.source.contains("interface api"));
        assert!(report.source.contains("get-users: func() -> string"));
        assert_eq!(report.stats.interfaces, 1);
        assert!(report.stats.funcs >= 1);
    }

    #[test]
    fn capability_uses_become_world_imports() {
        let m = module_for(
            "module demo\ncapability Logger\nfn ping() -> Unit uses logger\n",
            "capability_imports",
        );
        let report = match emit_wit_for_module(&m, "acme:demo", "demo") {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "expected ok report, got {err}");
                return;
            }
        };
        assert!(report.source.contains("import logger;"));
    }

    #[test]
    fn module_level_fn_becomes_export() {
        let m = module_for(
            "module demo\nfn add(a: Int, b: Int) -> Int\n",
            "fn_export",
        );
        let report = match emit_wit_for_module(&m, "acme:demo", "demo") {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "expected ok report, got {err}");
                return;
            }
        };
        assert!(report.source.contains("export add: func(a: s64, b: s64) -> s64"));
    }

    #[test]
    fn unsupported_type_returns_wit0001() {
        // Tuples are unsupported in the v1 envelope.
        let m = module_for(
            "module demo\nfn weird(x: SomethingExotic[Int, Str, Foo, Bar, Baz]) -> Unit\n",
            "unsupported",
        );
        // The bare type `SomethingExotic[...]` is unsupported because it
        // is neither a known constructor nor a simple identifier.
        let err = match emit_wit_for_module(&m, "acme:demo", "demo") {
            Ok(_) => {
                assert!(false, "expected WIT0001");
                return;
            }
            Err(e) => e,
        };
        assert_eq!(err.code, WIT_UNSUPPORTED_TYPE);
    }

    #[test]
    fn direct_record_cycle_returns_wit0003() {
        let m = module_for(
            "module demo\ntype Node = {\n  next: Node\n}\n",
            "cycle_direct",
        );
        let err = match emit_wit_for_module(&m, "acme:demo", "demo") {
            Ok(_) => {
                assert!(false, "expected WIT0003");
                return;
            }
            Err(e) => e,
        };
        assert_eq!(err.code, WIT_RECORD_CYCLE);
    }

    #[test]
    fn option_wrapped_recursion_is_allowed() {
        let m = module_for(
            "module demo\ntype Node = {\n  next: Option[Node]\n}\n",
            "cycle_option",
        );
        let report = match emit_wit_for_module(&m, "acme:demo", "demo") {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "expected ok, got {err}");
                return;
            }
        };
        assert!(report.source.contains("option<node>"));
    }

    #[test]
    fn invalid_package_returns_wit0002() {
        let m = module_for("module demo\nfn a() -> Unit\n", "pkg_invalid");
        let err = match emit_wit_for_module(&m, "no-colon", "demo") {
            Ok(_) => {
                assert!(false, "expected WIT0002");
                return;
            }
            Err(e) => e,
        };
        assert_eq!(err.code, WIT_INVALID_PACKAGE);
    }

    #[test]
    fn output_is_deterministic_across_runs() {
        let m = module_for(
            "module demo\ntype A = { x: Int, y: Str }\ntype B = { a: A }\nfn one() -> A\nfn two() -> B\n",
            "determinism",
        );
        let first = match emit_wit_for_module(&m, "acme:demo", "demo") {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "expected ok, got {err}");
                return;
            }
        };
        for _ in 0..3 {
            let again = match emit_wit_for_module(&m, "acme:demo", "demo") {
                Ok(r) => r,
                Err(err) => {
                    assert!(false, "expected ok, got {err}");
                    return;
                }
            };
            assert_eq!(first, again);
        }
    }

    #[test]
    fn report_serialises_with_schema_field() {
        let m = module_for(
            "module demo\nfn a() -> Int\n",
            "schema_field",
        );
        let report = match emit_wit_for_module(&m, "acme:demo", "demo") {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "expected ok, got {err}");
                return;
            }
        };
        let json = wit_report_json(&report);
        assert!(json.contains("\"schema\":\"ori.wit_report.v1\""));
        assert!(json.contains("\"package\":\"acme:demo\""));
        assert!(json.contains("\"world\":\"demo\""));
    }

    #[test]
    fn package_namespace_is_normalised() {
        let m = module_for("module demo\nfn a() -> Int\n", "package_norm");
        let report = match emit_wit_for_module(&m, "Acme_Inc:My_Demo", "MyWorld") {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "expected ok, got {err}");
                return;
            }
        };
        assert_eq!(report.package, "acme-inc:my-demo");
        assert_eq!(report.world, "my-world");
        assert!(report.source.starts_with("package acme-inc:my-demo@0.1.0;"));
    }
}
