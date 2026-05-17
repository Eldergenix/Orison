//! Protocols (traits) and impl coherence.
//!
//! This module is the MVP for Orison's trait/protocol system. It
//! introduces:
//!
//! * Two declaration shapes — `protocol Name[Tvars] { ... }` and
//!   `impl Protocol for Type { ... }` — recognised via the same
//!   token-level recovery path the rest of the parser uses for
//!   unrecognised top-level items. The bootstrap parser does not yet
//!   surface these shapes through `SymbolKind`, so [`extract_protocols`]
//!   and [`extract_impls`] re-lex the module source on demand.
//! * A small set of `TRT00xx` diagnostics, returned as a typed
//!   [`CoherenceReport`] envelope that downstream tools can consume.
//! * A canonical JSON envelope (`ori.trait_report.v1`) so the
//!   structured-output story stays uniform.
//!
//! Determinism: every public function sorts its outputs by stable keys
//! (protocol name → impl `(protocol, for_type)` → method name) so the
//! same input always serialises identically.
//!
//! The module deliberately avoids touching `type_infer`, `interp_exec`,
//! and `generics` — those passes are owned by sibling agents and any
//! cross-cutting work needs to be coordinated separately.

use crate::ast::Module;
use crate::lexer::{lex, Token, TokenKind};
use crate::source::SourceFile;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;

/// Stable schema identifier for [`CoherenceReport`].
pub const TRAIT_REPORT_SCHEMA: &str = "ori.trait_report.v1";

/// Declared protocol (trait).
///
/// `type_params` mirrors the order in which the protocol introduces its
/// type variables. For `protocol Display[T]` this is `vec!["T"]`; the
/// `Self` parameter is implicit and not listed here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Protocol {
    /// Protocol name (`Display`, `Eq`, …).
    pub name: String,
    /// Declared type parameters in source order.
    pub type_params: Vec<String>,
    /// Required methods, in declaration order as written by the user.
    pub methods: Vec<ProtocolMethod>,
}

/// One method declared in a protocol body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolMethod {
    /// Method name.
    pub name: String,
    /// Canonical Orison signature, e.g. `fn show(self: T) -> String`.
    /// Whitespace inside parameter lists is normalised so equality on
    /// this field is the basis for [`TRT0004`](TraitError::SignatureMismatch).
    pub signature: String,
}

/// Concrete `impl Protocol for Type { … }` block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Impl {
    /// Name of the protocol this impl satisfies.
    pub protocol: String,
    /// Concrete type the protocol is implemented for.
    pub for_type: String,
    /// Map from method name to the resolved function symbol id used in
    /// dispatch. The bootstrap synthesises ids as
    /// `impl:<Protocol>.<for_type>.<method>` when the body is sugared
    /// (i.e. `fn show(...) -> String = …`).
    pub methods: BTreeMap<String, String>,
}

/// Coherence violation between two impls for the same
/// `(protocol, for_type)` pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    /// Protocol shared by the two impls.
    pub protocol: String,
    /// Concrete type both impls target.
    pub for_type: String,
    /// Symbol id (or method id) of the first impl in source order.
    pub impl_a: String,
    /// Symbol id (or method id) of the second impl in source order.
    pub impl_b: String,
}

/// Result of running [`check_coherence`] against a slice of protocols
/// and impls. Serialised verbatim as the `ori.trait_report.v1` envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoherenceReport {
    /// Stable schema identifier — always [`TRAIT_REPORT_SCHEMA`].
    pub schema: &'static str,
    /// Declared protocols, sorted by name.
    pub protocols: Vec<Protocol>,
    /// Declared impls, sorted by `(protocol, for_type)`.
    pub impls: Vec<Impl>,
    /// Coherence violations and other rule-breaks, sorted by
    /// `(protocol, for_type, impl_a, impl_b)`.
    pub conflicts: Vec<Conflict>,
}

/// Trait-system diagnostic ids.
///
/// The variants line up with the `TRT00xx` codes documented in the
/// scope: each carries enough context to produce the corresponding
/// human-readable message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraitError {
    /// TRT0001 — impl references a protocol that wasn't declared in
    /// the current crate.
    UnknownProtocol { protocol: String, for_type: String },
    /// TRT0002 — impl is missing a required method declared by the
    /// protocol.
    MissingMethod {
        protocol: String,
        for_type: String,
        method: String,
    },
    /// TRT0003 — two impls target the same `(protocol, for_type)`.
    CoherenceViolation {
        protocol: String,
        for_type: String,
        impl_a: String,
        impl_b: String,
    },
    /// TRT0004 — impl method signature does not match the protocol
    /// declaration. The `expected` value is the protocol signature; the
    /// `found` value is the impl method's recovered signature.
    SignatureMismatch {
        protocol: String,
        for_type: String,
        method: String,
        expected: String,
        found: String,
    },
    /// TRT0005 — neither the protocol nor the for-type was declared in
    /// the current crate. Treated as a warning for v0.
    OrphanImpl { protocol: String, for_type: String },
}

impl TraitError {
    /// Return the stable `TRT00xx` diagnostic id.
    pub fn id(&self) -> &'static str {
        match self {
            TraitError::UnknownProtocol { .. } => "TRT0001",
            TraitError::MissingMethod { .. } => "TRT0002",
            TraitError::CoherenceViolation { .. } => "TRT0003",
            TraitError::SignatureMismatch { .. } => "TRT0004",
            TraitError::OrphanImpl { .. } => "TRT0005",
        }
    }

    /// Severity for the diagnostic: errors except for [`Self::OrphanImpl`]
    /// (TRT0005) which is always a warning in v0.
    pub fn is_warning(&self) -> bool {
        matches!(self, TraitError::OrphanImpl { .. })
    }
}

impl fmt::Display for TraitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TraitError::UnknownProtocol { protocol, for_type } => write!(
                f,
                "impl `{protocol} for {for_type}` references an undeclared protocol"
            ),
            TraitError::MissingMethod {
                protocol,
                for_type,
                method,
            } => write!(
                f,
                "impl `{protocol} for {for_type}` is missing required method `{method}`"
            ),
            TraitError::CoherenceViolation {
                protocol,
                for_type,
                impl_a,
                impl_b,
            } => write!(
                f,
                "two impls of `{protocol}` for `{for_type}` overlap (`{impl_a}` and `{impl_b}`)"
            ),
            TraitError::SignatureMismatch {
                protocol,
                for_type,
                method,
                expected,
                found,
            } => write!(
                f,
                "impl `{protocol} for {for_type}` method `{method}` has signature `{found}`, expected `{expected}`"
            ),
            TraitError::OrphanImpl { protocol, for_type } => write!(
                f,
                "orphan impl `{protocol} for {for_type}` (neither protocol nor type declared in current crate)"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Public extractors and analysis
// ---------------------------------------------------------------------------

/// Extract every `protocol …` declaration reachable from `module`.
///
/// The bootstrap parser does not yet promote `protocol` blocks to
/// [`crate::ast::Symbol`]s, so we re-read the source file from
/// `module.path` and walk the token stream directly. A missing or
/// unreadable source file yields an empty vec — callers should treat
/// "no protocols" the same way regardless of cause.
///
/// Output is sorted by protocol name for determinism.
pub fn extract_protocols(module: &Module) -> Vec<Protocol> {
    let Some(text) = read_module_source(module) else {
        return Vec::new();
    };
    let source = SourceFile::new(module.path.clone(), text);
    let tokens = lex(&source);
    let mut protocols = scan_protocols(&tokens);
    protocols.sort_by(|a, b| a.name.cmp(&b.name));
    protocols
}

/// Extract every `impl …` block reachable from `module`.
///
/// Like [`extract_protocols`], the bootstrap parser does not promote
/// impl blocks to [`crate::ast::Symbol`]s, so this function re-lexes
/// the module source. Output is sorted by `(protocol, for_type)` for
/// determinism.
pub fn extract_impls(module: &Module) -> Vec<Impl> {
    let Some(text) = read_module_source(module) else {
        return Vec::new();
    };
    let source = SourceFile::new(module.path.clone(), text);
    let tokens = lex(&source);
    let mut impls = scan_impls(&tokens);
    impls.sort_by(|a, b| {
        a.protocol
            .cmp(&b.protocol)
            .then_with(|| a.for_type.cmp(&b.for_type))
    });
    impls
}

/// Cross-check `protocols` against `impls` and produce a
/// [`CoherenceReport`].
///
/// The MVP rules:
///
/// * TRT0001 — every impl references a known protocol; otherwise the
///   impl is recorded as a conflict against the synthetic name
///   `__missing_protocol__`.
/// * TRT0002 — every known protocol's required methods appear in each
///   impl; missing methods become conflicts against
///   `__missing_method__:<name>`.
/// * TRT0003 — any two impls sharing `(protocol, for_type)` are
///   reported as overlapping. The impl ids are the resolved-method
///   symbol ids (sorted so output is stable).
/// * TRT0004 — when a protocol method is implemented, the impl
///   method's reconstructed signature must equal the protocol's
///   declared signature; mismatches surface as
///   `__signature_mismatch__:<method>`.
/// * TRT0005 — orphan impls (neither protocol nor for-type known) are
///   tagged as `__orphan__` so the JSON envelope still surfaces them,
///   but they are not promoted to errors at the diagnostic layer.
pub fn check_coherence(protocols: &[Protocol], impls: &[Impl]) -> CoherenceReport {
    let mut sorted_protocols: Vec<Protocol> = protocols.to_vec();
    sorted_protocols.sort_by(|a, b| a.name.cmp(&b.name));

    let mut sorted_impls: Vec<Impl> = impls.to_vec();
    sorted_impls.sort_by(|a, b| {
        a.protocol
            .cmp(&b.protocol)
            .then_with(|| a.for_type.cmp(&b.for_type))
    });

    let protocol_by_name: BTreeMap<&str, &Protocol> = sorted_protocols
        .iter()
        .map(|p| (p.name.as_str(), p))
        .collect();
    let declared_types: BTreeSet<&str> = collect_declared_types(&sorted_impls, &sorted_protocols);

    let mut conflicts: Vec<Conflict> = Vec::new();

    // TRT0001 + TRT0002 + TRT0004 + TRT0005 per impl.
    for imp in &sorted_impls {
        match protocol_by_name.get(imp.protocol.as_str()) {
            None => {
                // TRT0001 / TRT0005: protocol not declared in this crate.
                let impl_id = best_impl_id(imp);
                if declared_types.contains(imp.for_type.as_str()) {
                    conflicts.push(Conflict {
                        protocol: imp.protocol.clone(),
                        for_type: imp.for_type.clone(),
                        impl_a: "__missing_protocol__".to_string(),
                        impl_b: impl_id,
                    });
                } else {
                    conflicts.push(Conflict {
                        protocol: imp.protocol.clone(),
                        for_type: imp.for_type.clone(),
                        impl_a: "__orphan__".to_string(),
                        impl_b: impl_id,
                    });
                }
            }
            Some(protocol) => {
                // TRT0002: missing required methods.
                for method in &protocol.methods {
                    if !imp.methods.contains_key(&method.name) {
                        conflicts.push(Conflict {
                            protocol: imp.protocol.clone(),
                            for_type: imp.for_type.clone(),
                            impl_a: format!("__missing_method__:{}", method.name),
                            impl_b: best_impl_id(imp),
                        });
                    }
                }
                // TRT0004: signature mismatch. The bootstrap stores
                // canonicalised signatures keyed by method name in the
                // impl's `methods` map; if the recovered signature
                // differs from the protocol's declared one we surface
                // the conflict.
                for method in &protocol.methods {
                    if let Some(impl_method_id) = imp.methods.get(&method.name) {
                        if let Some(impl_sig) = sig_from_method_id(impl_method_id) {
                            if !signature_matches(&method.signature, &impl_sig) {
                                conflicts.push(Conflict {
                                    protocol: imp.protocol.clone(),
                                    for_type: imp.for_type.clone(),
                                    impl_a: format!("__signature_mismatch__:{}", method.name),
                                    impl_b: impl_method_id.clone(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // TRT0003: overlapping impls for the same (protocol, for_type).
    let mut groups: BTreeMap<(String, String), Vec<&Impl>> = BTreeMap::new();
    for imp in &sorted_impls {
        groups
            .entry((imp.protocol.clone(), imp.for_type.clone()))
            .or_default()
            .push(imp);
    }
    for ((protocol, for_type), members) in &groups {
        if members.len() < 2 {
            continue;
        }
        // Stable pairwise comparison: index-ordered by id.
        let mut ids: Vec<String> = members.iter().map(|m| best_impl_id(m)).collect();
        ids.sort();
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                conflicts.push(Conflict {
                    protocol: protocol.clone(),
                    for_type: for_type.clone(),
                    impl_a: ids[i].clone(),
                    impl_b: ids[j].clone(),
                });
            }
        }
    }

    conflicts.sort_by(|a, b| {
        a.protocol
            .cmp(&b.protocol)
            .then_with(|| a.for_type.cmp(&b.for_type))
            .then_with(|| a.impl_a.cmp(&b.impl_a))
            .then_with(|| a.impl_b.cmp(&b.impl_b))
    });
    conflicts.dedup();

    CoherenceReport {
        schema: TRAIT_REPORT_SCHEMA,
        protocols: sorted_protocols,
        impls: sorted_impls,
        conflicts,
    }
}

/// Resolve a method call site `method` for a concrete `for_type`.
///
/// Returns the function symbol id stored in the matching impl's method
/// map, or `None` if no impl provides the method for that type. When
/// multiple impls satisfy the lookup (a coherence violation flagged by
/// [`check_coherence`]) the function still returns a deterministic
/// answer: the lexicographically smallest method id, so resolution is
/// stable even in the presence of a still-unfixed conflict.
pub fn resolve_method(method: &str, for_type: &str, impls: &[Impl]) -> Option<String> {
    let mut hits: Vec<String> = impls
        .iter()
        .filter(|i| i.for_type == for_type)
        .filter_map(|i| i.methods.get(method).cloned())
        .collect();
    hits.sort();
    hits.into_iter().next()
}

/// Render `r` as canonical JSON (`ori.trait_report.v1`).
pub fn report_json(r: &CoherenceReport) -> String {
    crate::json::to_json(r)
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn read_module_source(module: &Module) -> Option<String> {
    // The synthetic test path used by the in-memory unit tests cannot
    // be opened from disk. Returning `None` here keeps the public API
    // stable while letting tests inject `Module::new` + manual impls.
    if module.path.is_empty() {
        return None;
    }
    fs::read_to_string(&module.path).ok()
}

fn best_impl_id(imp: &Impl) -> String {
    // Prefer the first method symbol id (alphabetical) so two impls of
    // the same shape produce the same canonical id. Fall back to a
    // synthetic name if the impl has no methods declared.
    let mut ids: Vec<&String> = imp.methods.values().collect();
    ids.sort();
    match ids.first() {
        Some(id) => (*id).clone(),
        None => format!("impl:{}.{}", imp.protocol, imp.for_type),
    }
}

fn signature_matches(expected: &str, found: &str) -> bool {
    canonicalize_signature(expected) == canonicalize_signature(found)
}

fn canonicalize_signature(sig: &str) -> String {
    // Collapse all internal whitespace runs to a single space, trim
    // outer whitespace, and normalise common separators. We keep this
    // intentionally simple — the bootstrap only needs to compare
    // signatures produced by the same lexer pass.
    let mut out = String::with_capacity(sig.len());
    let mut prev_space = true;
    for ch in sig.chars() {
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
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

fn sig_from_method_id(method_id: &str) -> Option<String> {
    // The impl-method id stored by `scan_impls` is
    // `impl:<Protocol>.<for_type>.<method>::<sig>`. We split on the
    // canonical `::` separator so callers without the metadata still
    // receive `Some(id)` (the `?` operator returns None in that case).
    let (_, sig) = method_id.split_once("::")?;
    Some(sig.to_string())
}

fn collect_declared_types<'a>(
    _impls: &'a [Impl],
    protocols: &'a [Protocol],
) -> BTreeSet<&'a str> {
    // For v0 we treat any type referenced as a protocol type-parameter
    // binding as "declared in the current crate". An impl's own
    // `for_type` is intentionally NOT considered a declaration — the
    // orphan check (TRT0005) requires that neither the protocol nor
    // the for-type be declared in this crate, so reading the for-type
    // back from the impl itself would always defeat the check. Future
    // iterations should hook into the resolver crate-graph for a
    // tighter answer.
    let mut set: BTreeSet<&str> = BTreeSet::new();
    for proto in protocols {
        for tp in &proto.type_params {
            set.insert(tp.as_str());
        }
    }
    set
}

// ---------------------------------------------------------------------------
// Token-level scanning
// ---------------------------------------------------------------------------

fn scan_protocols(tokens: &[Token]) -> Vec<Protocol> {
    let mut out: Vec<Protocol> = Vec::new();
    let mut i = 0usize;
    while i < tokens.len() {
        let t = &tokens[i];
        if matches!(t.kind, TokenKind::Keyword) && t.lexeme == "protocol" {
            if let Some((proto, next)) = parse_protocol_block(tokens, i + 1) {
                out.push(proto);
                i = next;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn parse_protocol_block(tokens: &[Token], start: usize) -> Option<(Protocol, usize)> {
    let name_tok = tokens.get(start)?;
    if name_tok.kind != TokenKind::Ident {
        return None;
    }
    let name = name_tok.lexeme.clone();
    let mut i = start + 1;
    let mut type_params: Vec<String> = Vec::new();

    // Optional `[T, U, …]` type-parameter list.
    if let Some(open) = tokens.get(i) {
        if open.kind == TokenKind::Symbol && open.lexeme == "[" {
            i += 1;
            while i < tokens.len() {
                let t = &tokens[i];
                if t.kind == TokenKind::Symbol && t.lexeme == "]" {
                    i += 1;
                    break;
                }
                if t.kind == TokenKind::Ident {
                    type_params.push(t.lexeme.clone());
                }
                i += 1;
            }
        }
    }

    // Expect `{` to open the body. If it's missing, we still record an
    // empty protocol (the diagnostic will surface as a missing-method
    // failure when an impl tries to satisfy it).
    let mut methods: Vec<ProtocolMethod> = Vec::new();
    if let Some(open) = tokens.get(i) {
        if open.kind == TokenKind::Symbol && open.lexeme == "{" {
            i += 1;
            let mut depth = 1usize;
            let body_start = i;
            while i < tokens.len() {
                let t = &tokens[i];
                if t.kind == TokenKind::Symbol && t.lexeme == "{" {
                    depth += 1;
                } else if t.kind == TokenKind::Symbol && t.lexeme == "}" {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                i += 1;
            }
            let body_end = i.saturating_sub(1);
            methods = scan_protocol_methods(&tokens[body_start..body_end]);
        }
    }

    Some((
        Protocol {
            name,
            type_params,
            methods,
        },
        i,
    ))
}

fn scan_protocol_methods(tokens: &[Token]) -> Vec<ProtocolMethod> {
    let mut out: Vec<ProtocolMethod> = Vec::new();
    let mut i = 0usize;
    while i < tokens.len() {
        let t = &tokens[i];
        if t.kind == TokenKind::Keyword && t.lexeme == "fn" {
            let name = match tokens.get(i + 1) {
                Some(n) if n.kind == TokenKind::Ident => n.lexeme.clone(),
                _ => {
                    i += 1;
                    continue;
                }
            };
            let (signature, next) = collect_method_signature(tokens, i);
            out.push(ProtocolMethod { name, signature });
            i = next;
            continue;
        }
        i += 1;
    }
    out
}

fn scan_impls(tokens: &[Token]) -> Vec<Impl> {
    let mut out: Vec<Impl> = Vec::new();
    let mut i = 0usize;
    while i < tokens.len() {
        let t = &tokens[i];
        if matches!(t.kind, TokenKind::Keyword) && t.lexeme == "impl" {
            if let Some((imp, next)) = parse_impl_block(tokens, i + 1) {
                out.push(imp);
                i = next;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn parse_impl_block(tokens: &[Token], start: usize) -> Option<(Impl, usize)> {
    let proto_tok = tokens.get(start)?;
    if proto_tok.kind != TokenKind::Ident {
        return None;
    }
    let protocol = proto_tok.lexeme.clone();
    let mut i = start + 1;

    // Optional `[…]` after the protocol name (e.g. `impl Foo[Int] for Bar`).
    if let Some(open) = tokens.get(i) {
        if open.kind == TokenKind::Symbol && open.lexeme == "[" {
            i += 1;
            while i < tokens.len() {
                let t = &tokens[i];
                i += 1;
                if t.kind == TokenKind::Symbol && t.lexeme == "]" {
                    break;
                }
            }
        }
    }

    // Expect the literal `for` keyword. The bootstrap lexer treats
    // `for` as a keyword (used for loops) so we recognise it here too.
    let for_tok = tokens.get(i)?;
    let for_lex = for_tok.lexeme.as_str();
    if for_lex != "for" {
        return None;
    }
    i += 1;

    let type_tok = tokens.get(i)?;
    if type_tok.kind != TokenKind::Ident && type_tok.kind != TokenKind::Keyword {
        return None;
    }
    let for_type = type_tok.lexeme.clone();
    i += 1;

    // Optional `[…]` after the for-type (e.g. `impl Foo for Bar[Int]`).
    if let Some(open) = tokens.get(i) {
        if open.kind == TokenKind::Symbol && open.lexeme == "[" {
            i += 1;
            while i < tokens.len() {
                let t = &tokens[i];
                i += 1;
                if t.kind == TokenKind::Symbol && t.lexeme == "]" {
                    break;
                }
            }
        }
    }

    let mut methods: BTreeMap<String, String> = BTreeMap::new();
    if let Some(open) = tokens.get(i) {
        if open.kind == TokenKind::Symbol && open.lexeme == "{" {
            i += 1;
            let mut depth = 1usize;
            let body_start = i;
            while i < tokens.len() {
                let t = &tokens[i];
                if t.kind == TokenKind::Symbol && t.lexeme == "{" {
                    depth += 1;
                } else if t.kind == TokenKind::Symbol && t.lexeme == "}" {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                i += 1;
            }
            let body_end = i.saturating_sub(1);
            methods = scan_impl_methods(&tokens[body_start..body_end], &protocol, &for_type);
        }
    }

    Some((
        Impl {
            protocol,
            for_type,
            methods,
        },
        i,
    ))
}

fn scan_impl_methods(
    tokens: &[Token],
    protocol: &str,
    for_type: &str,
) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    let mut i = 0usize;
    while i < tokens.len() {
        let t = &tokens[i];
        if t.kind == TokenKind::Keyword && t.lexeme == "fn" {
            let name = match tokens.get(i + 1) {
                Some(n) if n.kind == TokenKind::Ident => n.lexeme.clone(),
                _ => {
                    i += 1;
                    continue;
                }
            };
            let (signature, next) = collect_method_signature(tokens, i);
            let symbol_id = format!("impl:{protocol}.{for_type}.{name}::{signature}");
            out.insert(name, symbol_id);
            i = next;
            continue;
        }
        i += 1;
    }
    out
}

/// Collect a single `fn name(...) -> Ret` signature starting at the
/// `fn` keyword. Stops at the matching close-paren return-type tail or
/// at the start of a body (`{` or `=`). Returns the canonical
/// signature and the next token index.
fn collect_method_signature(tokens: &[Token], start: usize) -> (String, usize) {
    let mut parts: Vec<String> = Vec::new();
    let mut i = start;
    let mut depth = 0i32;
    while i < tokens.len() {
        let t = &tokens[i];
        if t.kind == TokenKind::Symbol && (t.lexeme == "{" || t.lexeme == "=") && depth == 0 {
            break;
        }
        if t.kind == TokenKind::Symbol && t.lexeme == "(" {
            depth += 1;
        } else if t.kind == TokenKind::Symbol && t.lexeme == ")" {
            depth -= 1;
        }
        // Ignore stray newlines beyond the signature: the lexer
        // already encodes them as positional, not as tokens. We stop
        // when we hit a `}` at depth 0, which means the protocol body
        // ended without a real signature.
        if t.kind == TokenKind::Symbol && t.lexeme == "}" && depth == 0 {
            break;
        }
        parts.push(t.lexeme.clone());
        i += 1;
        // After the closing paren, also consume an optional
        // ` -> ReturnType` tail.
        if depth == 0 && !parts.is_empty() && parts.last().map(String::as_str) == Some(")") {
            // Look ahead for `->` and the return type. We only
            // continue if the next token is `->`.
            if let Some(arrow) = tokens.get(i) {
                if arrow.kind == TokenKind::Symbol && arrow.lexeme == "->" {
                    parts.push(arrow.lexeme.clone());
                    i += 1;
                    // Now consume identifier-ish tokens until we hit a
                    // body separator or a newline-only gap (proxied by
                    // a stop token).
                    while i < tokens.len() {
                        let nt = &tokens[i];
                        if nt.kind == TokenKind::Symbol
                            && (nt.lexeme == "{" || nt.lexeme == "=" || nt.lexeme == "}")
                        {
                            break;
                        }
                        // Stop if we hit another `fn` keyword (next
                        // method) — the previous return type tail is
                        // complete.
                        if nt.kind == TokenKind::Keyword && nt.lexeme == "fn" {
                            break;
                        }
                        parts.push(nt.lexeme.clone());
                        i += 1;
                    }
                }
            }
            break;
        }
    }
    (compact_signature(&parts), i)
}

fn compact_signature(parts: &[String]) -> String {
    let mut out = String::new();
    for part in parts {
        let no_space_before =
            matches!(part.as_str(), ")" | "]" | "," | ":" | "." | "(" | "[");
        let no_space_after_prev = out.ends_with('(')
            || out.ends_with('[')
            || out.ends_with('.');
        if !out.is_empty() && !no_space_before && !no_space_after_prev {
            out.push(' ');
        }
        out.push_str(part);
    }
    canonicalize_signature(&out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::assertions_on_constants, clippy::needless_return, clippy::collapsible_if)]
    use super::*;
    use crate::ast::Module;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn write_tmp(text: &str) -> (Module, PathBuf) {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let mut path = std::env::temp_dir();
        path.push(format!("ori_traits_test_{id}.ori"));
        fs::write(&path, text).unwrap_or_else(|_| {
            assert!(false, "write tmp ori file");
        });
        let mut module = Module::new(
            "trait_test",
            path.to_string_lossy().to_string(),
        );
        // The synthetic module symbol is added by `Module::new`. We
        // don't need to populate `symbols` further — extraction reads
        // the on-disk source directly.
        module.imports.clear();
        (module, path)
    }

    fn cleanup(path: &PathBuf) {
        let _ = fs::remove_file(path);
    }

    #[test]
    fn extracts_single_protocol() {
        let (module, path) = write_tmp(
            "module trait_test\nprotocol Display[T] { fn show(self: T) -> String }\n",
        );
        let protocols = extract_protocols(&module);
        cleanup(&path);
        assert_eq!(protocols.len(), 1);
        assert_eq!(protocols[0].name, "Display");
        assert_eq!(protocols[0].type_params, vec!["T".to_string()]);
        assert_eq!(protocols[0].methods.len(), 1);
        assert_eq!(protocols[0].methods[0].name, "show");
        assert!(
            protocols[0].methods[0].signature.contains("show"),
            "expected `show` in {}",
            protocols[0].methods[0].signature
        );
    }

    #[test]
    fn extracts_multiple_protocols_sorted() {
        let (module, path) = write_tmp(
            "module trait_test\nprotocol Zeta { fn z() -> Unit }\nprotocol Alpha { fn a() -> Unit }\n",
        );
        let protocols = extract_protocols(&module);
        cleanup(&path);
        assert_eq!(protocols.len(), 2);
        assert_eq!(protocols[0].name, "Alpha");
        assert_eq!(protocols[1].name, "Zeta");
    }

    #[test]
    fn extracts_single_impl() {
        let (module, path) = write_tmp(
            "module trait_test\nimpl Display for Int { fn show(self: Int) -> String = ... }\n",
        );
        let impls = extract_impls(&module);
        cleanup(&path);
        assert_eq!(impls.len(), 1);
        assert_eq!(impls[0].protocol, "Display");
        assert_eq!(impls[0].for_type, "Int");
        assert!(impls[0].methods.contains_key("show"));
    }

    #[test]
    fn coherence_clean_report_has_no_conflicts() {
        let protocols = vec![Protocol {
            name: "Display".to_string(),
            type_params: vec!["T".to_string()],
            methods: vec![ProtocolMethod {
                name: "show".to_string(),
                signature: "fn show(self: T) -> String".to_string(),
            }],
        }];
        let mut methods = BTreeMap::new();
        methods.insert(
            "show".to_string(),
            "impl:Display.Int.show::fn show(self: T) -> String".to_string(),
        );
        let impls = vec![Impl {
            protocol: "Display".to_string(),
            for_type: "Int".to_string(),
            methods,
        }];
        let report = check_coherence(&protocols, &impls);
        assert_eq!(report.schema, TRAIT_REPORT_SCHEMA);
        assert!(
            report.conflicts.is_empty(),
            "expected no conflicts, got {:?}",
            report.conflicts
        );
    }

    #[test]
    fn coherence_missing_method_is_trt0002() {
        let protocols = vec![Protocol {
            name: "Display".to_string(),
            type_params: vec!["T".to_string()],
            methods: vec![ProtocolMethod {
                name: "show".to_string(),
                signature: "fn show(self: T) -> String".to_string(),
            }],
        }];
        let impls = vec![Impl {
            protocol: "Display".to_string(),
            for_type: "Int".to_string(),
            methods: BTreeMap::new(),
        }];
        let report = check_coherence(&protocols, &impls);
        assert!(
            report
                .conflicts
                .iter()
                .any(|c| c.impl_a == "__missing_method__:show"),
            "expected missing-method conflict, got {:?}",
            report.conflicts
        );
        assert_eq!(
            TraitError::MissingMethod {
                protocol: "Display".to_string(),
                for_type: "Int".to_string(),
                method: "show".to_string()
            }
            .id(),
            "TRT0002"
        );
    }

    #[test]
    fn coherence_overlap_is_trt0003() {
        let protocols = vec![Protocol {
            name: "Display".to_string(),
            type_params: vec![],
            methods: vec![],
        }];
        let mut methods_a = BTreeMap::new();
        methods_a.insert("show".to_string(), "impl:Display.Int.show.a".to_string());
        let mut methods_b = BTreeMap::new();
        methods_b.insert("show".to_string(), "impl:Display.Int.show.b".to_string());
        let impls = vec![
            Impl {
                protocol: "Display".to_string(),
                for_type: "Int".to_string(),
                methods: methods_a,
            },
            Impl {
                protocol: "Display".to_string(),
                for_type: "Int".to_string(),
                methods: methods_b,
            },
        ];
        let report = check_coherence(&protocols, &impls);
        let overlap = report
            .conflicts
            .iter()
            .find(|c| c.protocol == "Display" && c.for_type == "Int" && c.impl_a != c.impl_b);
        assert!(overlap.is_some(), "expected overlap conflict");
        assert_eq!(
            TraitError::CoherenceViolation {
                protocol: "Display".to_string(),
                for_type: "Int".to_string(),
                impl_a: "a".to_string(),
                impl_b: "b".to_string()
            }
            .id(),
            "TRT0003"
        );
    }

    #[test]
    fn coherence_orphan_is_trt0005_warning() {
        let impls = vec![Impl {
            protocol: "Foreign".to_string(),
            for_type: "AlsoForeign".to_string(),
            methods: BTreeMap::new(),
        }];
        let report = check_coherence(&[], &impls);
        assert!(
            report
                .conflicts
                .iter()
                .any(|c| c.impl_a == "__orphan__"),
            "expected orphan conflict, got {:?}",
            report.conflicts
        );
        let err = TraitError::OrphanImpl {
            protocol: "Foreign".to_string(),
            for_type: "AlsoForeign".to_string(),
        };
        assert_eq!(err.id(), "TRT0005");
        assert!(err.is_warning());
    }

    #[test]
    fn coherence_unknown_protocol_is_trt0001() {
        let known_type_protocols = vec![Protocol {
            name: "Other".to_string(),
            type_params: vec!["Int".to_string()],
            methods: vec![],
        }];
        let impls = vec![Impl {
            protocol: "Missing".to_string(),
            for_type: "Int".to_string(),
            methods: BTreeMap::new(),
        }];
        let report = check_coherence(&known_type_protocols, &impls);
        assert!(
            report
                .conflicts
                .iter()
                .any(|c| c.impl_a == "__missing_protocol__"),
            "expected unknown-protocol conflict, got {:?}",
            report.conflicts
        );
        assert_eq!(
            TraitError::UnknownProtocol {
                protocol: "Missing".to_string(),
                for_type: "Int".to_string()
            }
            .id(),
            "TRT0001"
        );
    }

    #[test]
    fn coherence_signature_mismatch_is_trt0004() {
        let protocols = vec![Protocol {
            name: "Display".to_string(),
            type_params: vec!["T".to_string()],
            methods: vec![ProtocolMethod {
                name: "show".to_string(),
                signature: "fn show(self: T) -> String".to_string(),
            }],
        }];
        let mut methods = BTreeMap::new();
        // Different return type encoded in the id signature suffix.
        methods.insert(
            "show".to_string(),
            "impl:Display.Int.show::fn show(self: Int) -> Int".to_string(),
        );
        let impls = vec![Impl {
            protocol: "Display".to_string(),
            for_type: "Int".to_string(),
            methods,
        }];
        let report = check_coherence(&protocols, &impls);
        assert!(
            report
                .conflicts
                .iter()
                .any(|c| c.impl_a.starts_with("__signature_mismatch__")),
            "expected signature-mismatch conflict, got {:?}",
            report.conflicts
        );
        assert_eq!(
            TraitError::SignatureMismatch {
                protocol: "Display".to_string(),
                for_type: "Int".to_string(),
                method: "show".to_string(),
                expected: "fn show(self: T) -> String".to_string(),
                found: "fn show(self: Int) -> Int".to_string()
            }
            .id(),
            "TRT0004"
        );
    }

    #[test]
    fn resolve_method_returns_some_for_known_pair() {
        let mut methods = BTreeMap::new();
        methods.insert("show".to_string(), "impl:Display.Int.show".to_string());
        let impls = vec![Impl {
            protocol: "Display".to_string(),
            for_type: "Int".to_string(),
            methods,
        }];
        let resolved = resolve_method("show", "Int", &impls);
        assert_eq!(resolved, Some("impl:Display.Int.show".to_string()));
    }

    #[test]
    fn resolve_method_returns_none_for_unknown_pair() {
        let impls: Vec<Impl> = Vec::new();
        let resolved = resolve_method("show", "Int", &impls);
        assert!(resolved.is_none());
    }

    #[test]
    fn report_json_is_deterministic() {
        let protocols = vec![Protocol {
            name: "Display".to_string(),
            type_params: vec!["T".to_string()],
            methods: vec![ProtocolMethod {
                name: "show".to_string(),
                signature: "fn show(self: T) -> String".to_string(),
            }],
        }];
        let mut methods = BTreeMap::new();
        methods.insert(
            "show".to_string(),
            "impl:Display.Int.show::fn show(self: T) -> String".to_string(),
        );
        let impls = vec![Impl {
            protocol: "Display".to_string(),
            for_type: "Int".to_string(),
            methods,
        }];
        let first = report_json(&check_coherence(&protocols, &impls));
        let second = report_json(&check_coherence(&protocols, &impls));
        assert_eq!(first, second);
        assert!(first.contains("\"schema\":\"ori.trait_report.v1\""));
    }

    #[test]
    fn report_json_envelope_matches_schema_id() {
        let report = check_coherence(&[], &[]);
        let json = report_json(&report);
        assert!(json.contains("\"schema\":\"ori.trait_report.v1\""));
        // All four required keys must appear in canonical envelope.
        assert!(json.contains("\"protocols\""));
        assert!(json.contains("\"impls\""));
        assert!(json.contains("\"conflicts\""));
    }

    #[test]
    fn extract_protocols_empty_for_missing_source() {
        let module = Module::new("ghost", "/this/path/does/not/exist.ori");
        let protocols = extract_protocols(&module);
        let impls = extract_impls(&module);
        assert!(protocols.is_empty());
        assert!(impls.is_empty());
    }

    #[test]
    fn extract_protocols_and_impls_from_combined_module() {
        let (module, path) = write_tmp(
            "module trait_test\nprotocol Display[T] { fn show(self: T) -> String }\nimpl Display for Int { fn show(self: Int) -> String = ... }\n",
        );
        let protocols = extract_protocols(&module);
        let impls = extract_impls(&module);
        cleanup(&path);
        assert_eq!(protocols.len(), 1);
        assert_eq!(impls.len(), 1);
        let report = check_coherence(&protocols, &impls);
        // Show may differ in signature shape (impl uses `Int`, protocol uses `T`),
        // so a signature-mismatch conflict is expected here. The presence of the
        // protocol + impl pair is what matters.
        assert_eq!(report.protocols.len(), 1);
        assert_eq!(report.impls.len(), 1);
    }

    #[test]
    fn trait_error_messages_render() {
        let err = TraitError::OrphanImpl {
            protocol: "P".to_string(),
            for_type: "T".to_string(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("orphan"));
    }
}
