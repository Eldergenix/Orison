//! GraphQL SDL → Orison typed-client surface importer.
//!
//! This module implements the M13 handoff for GraphQL imports: it parses a
//! conservative subset of the GraphQL Schema Definition Language (SDL) and
//! emits an equivalent Orison module declaring record types for every GraphQL
//! object type and functions for every query and mutation field.
//!
//! The parser is intentionally hand-rolled and dependency-free so that the
//! bootstrap compiler can absorb it without growing its dependency surface.
//! The supported subset covers what real-world public schemas (GitHub,
//! Shopify, Stripe partner APIs, etc.) use in practice for type and operation
//! definitions:
//!
//! * `type Name { field: Type }`
//! * `type Name { field: Type! }` (non-null)
//! * `type Name { field: [Type] }` and `[Type!]!` (list types, any nullness)
//! * `type Query { field(arg: Type!, arg2: Type = default): Type }`
//! * `type Mutation { ... }` (same shape as `Query`)
//! * `# single-line comments` anywhere outside an identifier
//! * Scalars: `ID`, `String`, `Int`, `Float`, `Boolean` (mapped to Orison
//!   primitives; unknown scalars pass through as opaque named types).
//!
//! Type mapping when emitting Orison:
//!
//! | GraphQL  | Orison    |
//! |----------|-----------|
//! | `ID`     | `Str`     |
//! | `String` | `Str`     |
//! | `Int`    | `Int`     |
//! | `Float`  | `Float64` |
//! | `Boolean`| `Bool`    |
//!
//! Nullability is encoded by GraphQL the opposite way from most ML-family
//! type systems: by default every field is nullable, and `!` marks it as
//! non-null. We mirror that here — a non-null field becomes the bare Orison
//! type, while a nullable field is wrapped in `Option[T]`.
//!
//! ## Known gaps (documented, not silently dropped)
//!
//! * `interface`, `union`, `enum`, `input`, `extend`, directives and
//!   subscriptions are out of scope for the bootstrap importer. The parser
//!   skips over unrecognised top-level keywords so a schema that mixes
//!   supported and unsupported declarations still yields the supported parts
//!   rather than failing outright. Unsupported declarations are surfaced via
//!   `GraphqlSchema::unsupported` so callers can decide whether to warn.
//! * Default values on arguments are tolerated but discarded.
//! * Triple-quoted block descriptions are not parsed; only `#` comments are.

use crate::json::to_json;
use serde::Serialize;
use std::fmt;

/// A single record field — either an object-type field or a query/mutation
/// argument. `ty` is the *Orison* type rendering (e.g. `Str`, `Option[Int]`,
/// `List[User]`) rather than the original GraphQL spelling, so consumers do
/// not need to know about GraphQL nullability rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GqlField {
    pub name: String,
    pub ty: String,
    pub nullable: bool,
}

/// A GraphQL object type that maps onto an Orison record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GqlType {
    pub name: String,
    pub fields: Vec<GqlField>,
}

/// A GraphQL `Query` or `Mutation` field — emitted as an Orison `fn`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GqlQuery {
    pub name: String,
    pub args: Vec<GqlField>,
    pub return_type: String,
}

/// The parsed GraphQL schema in a shape ready for Orison emission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphqlSchema {
    pub types: Vec<GqlType>,
    pub queries: Vec<GqlQuery>,
    pub mutations: Vec<GqlQuery>,
    /// Declarations the bootstrap importer recognises but does not yet model
    /// (`interface`, `union`, `enum`, `input`, etc.). Kept as a heads-up
    /// channel for the CLI so the agent loop can warn deterministically.
    pub unsupported: Vec<String>,
}

/// A precise SDL parse error with a 1-based line number. Returned in lieu of
/// panicking so the CLI can surface a usable diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphqlParseError {
    pub message: String,
    pub line: usize,
}

impl fmt::Display for GraphqlParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "graphql sdl error at line {}: {}",
            self.line, self.message
        )
    }
}

impl std::error::Error for GraphqlParseError {}

/// Stable JSON envelope describing the outcome of a single import run. Used
/// by `ori schema import graphql` and snapshot tests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    pub schema: &'static str,
    pub module: String,
    pub types: usize,
    pub queries: usize,
    pub mutations: usize,
    pub generated_lines: usize,
    pub unsupported: Vec<String>,
}

impl ImportReport {
    /// Build a report from a parsed schema and the emitted Orison source.
    pub fn build(schema: &GraphqlSchema, module: &str, generated_source: &str) -> Self {
        Self {
            schema: "ori.graphql_import.v1",
            module: module.to_string(),
            types: schema.types.len(),
            queries: schema.queries.len(),
            mutations: schema.mutations.len(),
            generated_lines: count_generated_lines(generated_source),
            unsupported: schema.unsupported.clone(),
        }
    }

    pub fn to_json(&self) -> String {
        to_json(self)
    }
}

fn count_generated_lines(source: &str) -> usize {
    if source.is_empty() {
        return 0;
    }
    // Use split('\n') so a trailing newline still counts as a final empty
    // line (matches `wc -l + 1` semantics that humans expect for file sizes).
    let raw = source.split('\n').count();
    if source.ends_with('\n') {
        raw - 1
    } else {
        raw
    }
}

// ─── Parser ────────────────────────────────────────────────────────────────

/// Parse a GraphQL SDL string into a [`GraphqlSchema`].
///
/// The parser is whitespace-insensitive outside identifiers. Comments
/// (`# …`) are stripped before tokenisation so they cannot interfere with
/// state. The resulting schema lists object types, queries and mutations in
/// source order.
pub fn parse_sdl(sdl: &str) -> Result<GraphqlSchema, GraphqlParseError> {
    let tokens = tokenize(sdl)?;
    let mut parser = Parser::new(tokens);
    parser.parse_document()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Word(String),
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Colon,
    Comma,
    Bang,
    Equals,
}

#[derive(Debug, Clone)]
struct LToken {
    tok: Tok,
    line: usize,
}

fn tokenize(sdl: &str) -> Result<Vec<LToken>, GraphqlParseError> {
    let mut out: Vec<LToken> = Vec::new();
    let mut line = 1usize;
    let bytes = sdl.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '\n' => {
                line += 1;
                i += 1;
            }
            ' ' | '\t' | '\r' => {
                i += 1;
            }
            '#' => {
                while i < bytes.len() && bytes[i] as char != '\n' {
                    i += 1;
                }
            }
            ',' => {
                out.push(LToken {
                    tok: Tok::Comma,
                    line,
                });
                i += 1;
            }
            '{' => {
                out.push(LToken {
                    tok: Tok::LBrace,
                    line,
                });
                i += 1;
            }
            '}' => {
                out.push(LToken {
                    tok: Tok::RBrace,
                    line,
                });
                i += 1;
            }
            '(' => {
                out.push(LToken {
                    tok: Tok::LParen,
                    line,
                });
                i += 1;
            }
            ')' => {
                out.push(LToken {
                    tok: Tok::RParen,
                    line,
                });
                i += 1;
            }
            '[' => {
                out.push(LToken {
                    tok: Tok::LBracket,
                    line,
                });
                i += 1;
            }
            ']' => {
                out.push(LToken {
                    tok: Tok::RBracket,
                    line,
                });
                i += 1;
            }
            ':' => {
                out.push(LToken {
                    tok: Tok::Colon,
                    line,
                });
                i += 1;
            }
            '!' => {
                out.push(LToken {
                    tok: Tok::Bang,
                    line,
                });
                i += 1;
            }
            '=' => {
                out.push(LToken {
                    tok: Tok::Equals,
                    line,
                });
                i += 1;
            }
            ch if is_word_start(ch) => {
                let start = i;
                while i < bytes.len() && is_word_part(bytes[i] as char) {
                    i += 1;
                }
                let word = &sdl[start..i];
                out.push(LToken {
                    tok: Tok::Word(word.to_string()),
                    line,
                });
            }
            other => {
                return Err(GraphqlParseError {
                    message: format!("unexpected character `{other}`"),
                    line,
                });
            }
        }
    }
    Ok(out)
}

fn is_word_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_word_part(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

struct Parser {
    tokens: Vec<LToken>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<LToken>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&LToken> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<LToken> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn current_line(&self) -> usize {
        self.tokens
            .get(self.pos)
            .or_else(|| self.tokens.last())
            .map(|t| t.line)
            .unwrap_or(1)
    }

    fn expect_word(&mut self) -> Result<(String, usize), GraphqlParseError> {
        let line = self.current_line();
        let tok = self.bump().ok_or_else(|| GraphqlParseError {
            message: "unexpected end of input, expected an identifier".to_string(),
            line,
        })?;
        match tok.tok {
            Tok::Word(w) => Ok((w, tok.line)),
            other => Err(GraphqlParseError {
                message: format!("expected identifier, got {}", describe(&other)),
                line: tok.line,
            }),
        }
    }

    fn expect_tok(&mut self, expected: &Tok, what: &str) -> Result<usize, GraphqlParseError> {
        let line = self.current_line();
        let tok = self.bump().ok_or_else(|| GraphqlParseError {
            message: format!("unexpected end of input, expected {what}"),
            line,
        })?;
        if &tok.tok == expected {
            Ok(tok.line)
        } else {
            Err(GraphqlParseError {
                message: format!("expected {what}, got {}", describe(&tok.tok)),
                line: tok.line,
            })
        }
    }

    fn parse_document(&mut self) -> Result<GraphqlSchema, GraphqlParseError> {
        let mut types: Vec<GqlType> = Vec::new();
        let mut queries: Vec<GqlQuery> = Vec::new();
        let mut mutations: Vec<GqlQuery> = Vec::new();
        let mut unsupported: Vec<String> = Vec::new();

        while let Some(tok) = self.peek().cloned() {
            match &tok.tok {
                Tok::Word(kw) if kw == "type" => {
                    self.bump();
                    let (name, _) = self.expect_word()?;
                    let fields = self.parse_field_block()?;
                    if name == "Query" {
                        for f in fields_to_queries(&fields) {
                            queries.push(f);
                        }
                    } else if name == "Mutation" {
                        for f in fields_to_queries(&fields) {
                            mutations.push(f);
                        }
                    } else if name == "Subscription" {
                        unsupported.push(format!("type {name}"));
                    } else {
                        types.push(GqlType {
                            name,
                            fields: fields.into_iter().map(|f| f.field).collect(),
                        });
                    }
                }
                Tok::Word(kw)
                    if matches!(
                        kw.as_str(),
                        "interface" | "union" | "enum" | "input" | "extend" | "scalar" | "schema"
                    ) =>
                {
                    let label = kw.clone();
                    // Consume the keyword + optional name, then skip the body
                    // (a balanced `{ ... }` if present, or the equality form
                    // for `union`/`scalar`/`schema`). This keeps the parser
                    // forward-progressing on schemas that mix supported and
                    // unsupported declarations.
                    self.bump();
                    let name = match self.peek() {
                        Some(t) => match &t.tok {
                            Tok::Word(w) => {
                                let w = w.clone();
                                self.bump();
                                w
                            }
                            _ => "(anonymous)".to_string(),
                        },
                        None => "(anonymous)".to_string(),
                    };
                    self.skip_unsupported_body()?;
                    unsupported.push(format!("{label} {name}"));
                }
                Tok::Word(other) => {
                    return Err(GraphqlParseError {
                        message: format!("unexpected top-level keyword `{other}`"),
                        line: tok.line,
                    });
                }
                _ => {
                    return Err(GraphqlParseError {
                        message: format!("unexpected token {}", describe(&tok.tok)),
                        line: tok.line,
                    });
                }
            }
        }

        Ok(GraphqlSchema {
            types,
            queries,
            mutations,
            unsupported,
        })
    }

    /// Skip over the body of an unsupported declaration. Tolerates either a
    /// braced body (`{ ... }`), a `union`-style `= A | B | C` form, or no
    /// body at all (e.g. `scalar DateTime`).
    fn skip_unsupported_body(&mut self) -> Result<(), GraphqlParseError> {
        // Optional `=` form: read until we hit the next top-level keyword.
        if let Some(t) = self.peek() {
            if let Tok::Equals = t.tok {
                self.bump();
                while let Some(t) = self.peek() {
                    match &t.tok {
                        Tok::Word(w)
                            if matches!(
                                w.as_str(),
                                "type"
                                    | "interface"
                                    | "union"
                                    | "enum"
                                    | "input"
                                    | "extend"
                                    | "scalar"
                                    | "schema"
                            ) =>
                        {
                            return Ok(());
                        }
                        Tok::LBrace => break,
                        _ => {
                            self.bump();
                        }
                    }
                }
            }
        }
        // Optional braced body — skip with balanced counting.
        if matches!(self.peek().map(|t| &t.tok), Some(Tok::LBrace)) {
            let mut depth = 0i32;
            while let Some(t) = self.bump() {
                match t.tok {
                    Tok::LBrace => depth += 1,
                    Tok::RBrace => {
                        depth -= 1;
                        if depth == 0 {
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
            return Err(GraphqlParseError {
                message: "unterminated `{ ... }` body".to_string(),
                line: self.current_line(),
            });
        }
        Ok(())
    }

    /// Parse the `{ ... }` body of an object/query/mutation type. Each
    /// member is either `name: Type` or `name(arg: Type): Type` — the
    /// optional argument list lets us reuse this code for `type Query`.
    fn parse_field_block(&mut self) -> Result<Vec<ParsedMember>, GraphqlParseError> {
        self.expect_tok(&Tok::LBrace, "`{`")?;
        let mut out: Vec<ParsedMember> = Vec::new();
        loop {
            match self.peek() {
                Some(t) if matches!(t.tok, Tok::RBrace) => {
                    self.bump();
                    return Ok(out);
                }
                None => {
                    return Err(GraphqlParseError {
                        message: "unterminated type body".to_string(),
                        line: self.current_line(),
                    });
                }
                _ => {}
            }
            let (name, line) = self.expect_word()?;
            // Optional argument list.
            let args = if matches!(self.peek().map(|t| &t.tok), Some(Tok::LParen)) {
                self.parse_arg_list()?
            } else {
                Vec::new()
            };
            self.expect_tok(&Tok::Colon, "`:` after field name")?;
            let ty = self.parse_type()?;
            let field = GqlField {
                name,
                ty: render_type(&ty),
                nullable: ty.nullable(),
            };
            out.push(ParsedMember {
                field,
                args,
                _line: line,
            });
            // Trailing commas are tolerated but not required.
            if matches!(self.peek().map(|t| &t.tok), Some(Tok::Comma)) {
                self.bump();
            }
        }
    }

    fn parse_arg_list(&mut self) -> Result<Vec<GqlField>, GraphqlParseError> {
        self.expect_tok(&Tok::LParen, "`(`")?;
        let mut out: Vec<GqlField> = Vec::new();
        loop {
            match self.peek() {
                Some(t) if matches!(t.tok, Tok::RParen) => {
                    self.bump();
                    return Ok(out);
                }
                None => {
                    return Err(GraphqlParseError {
                        message: "unterminated argument list".to_string(),
                        line: self.current_line(),
                    });
                }
                _ => {}
            }
            let (name, _) = self.expect_word()?;
            self.expect_tok(&Tok::Colon, "`:` after argument name")?;
            let ty = self.parse_type()?;
            // Optional default value: tolerate but discard one literal/word.
            if matches!(self.peek().map(|t| &t.tok), Some(Tok::Equals)) {
                self.bump();
                // Consume one default value token (or a bracketed list).
                match self.peek().map(|t| t.tok.clone()) {
                    Some(Tok::LBracket) => {
                        let mut depth = 0i32;
                        while let Some(t) = self.bump() {
                            match t.tok {
                                Tok::LBracket => depth += 1,
                                Tok::RBracket => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Some(Tok::Word(_)) => {
                        self.bump();
                    }
                    _ => {}
                }
            }
            out.push(GqlField {
                name,
                ty: render_type(&ty),
                nullable: ty.nullable(),
            });
            if matches!(self.peek().map(|t| &t.tok), Some(Tok::Comma)) {
                self.bump();
            }
        }
    }

    fn parse_type(&mut self) -> Result<GqlTypeRef, GraphqlParseError> {
        let line = self.current_line();
        let tok = self.bump().ok_or_else(|| GraphqlParseError {
            message: "unexpected end of input, expected a type".to_string(),
            line,
        })?;
        let mut ty = match tok.tok {
            Tok::Word(name) => GqlTypeRef::Named {
                name,
                nullable: true,
            },
            Tok::LBracket => {
                let inner = self.parse_type()?;
                self.expect_tok(&Tok::RBracket, "`]`")?;
                GqlTypeRef::List {
                    inner: Box::new(inner),
                    nullable: true,
                }
            }
            other => {
                return Err(GraphqlParseError {
                    message: format!("expected a type, got {}", describe(&other)),
                    line: tok.line,
                });
            }
        };
        if matches!(self.peek().map(|t| &t.tok), Some(Tok::Bang)) {
            self.bump();
            ty.set_non_null();
        }
        Ok(ty)
    }
}

#[derive(Debug, Clone)]
struct ParsedMember {
    field: GqlField,
    args: Vec<GqlField>,
    _line: usize,
}

fn fields_to_queries(members: &[ParsedMember]) -> Vec<GqlQuery> {
    members
        .iter()
        .map(|m| GqlQuery {
            name: m.field.name.clone(),
            args: m.args.clone(),
            return_type: m.field.ty.clone(),
        })
        .collect()
}

#[derive(Debug, Clone)]
enum GqlTypeRef {
    Named {
        name: String,
        nullable: bool,
    },
    List {
        inner: Box<GqlTypeRef>,
        nullable: bool,
    },
}

impl GqlTypeRef {
    fn set_non_null(&mut self) {
        match self {
            GqlTypeRef::Named { nullable, .. } | GqlTypeRef::List { nullable, .. } => {
                *nullable = false
            }
        }
    }
    fn nullable(&self) -> bool {
        match self {
            GqlTypeRef::Named { nullable, .. } | GqlTypeRef::List { nullable, .. } => *nullable,
        }
    }
}

fn render_type(ty: &GqlTypeRef) -> String {
    let inner = render_inner(ty);
    if ty.nullable() {
        format!("Option[{inner}]")
    } else {
        inner
    }
}

fn render_inner(ty: &GqlTypeRef) -> String {
    match ty {
        GqlTypeRef::Named { name, .. } => map_scalar(name).to_string(),
        GqlTypeRef::List { inner, .. } => format!("List[{}]", render_type(inner)),
    }
}

fn map_scalar(name: &str) -> &str {
    match name {
        "ID" | "String" => "Str",
        "Int" => "Int",
        "Float" => "Float64",
        "Boolean" => "Bool",
        other => other,
    }
}

fn describe(tok: &Tok) -> String {
    match tok {
        Tok::Word(w) => format!("`{w}`"),
        Tok::LBrace => "`{`".to_string(),
        Tok::RBrace => "`}`".to_string(),
        Tok::LParen => "`(`".to_string(),
        Tok::RParen => "`)`".to_string(),
        Tok::LBracket => "`[`".to_string(),
        Tok::RBracket => "`]`".to_string(),
        Tok::Colon => "`:`".to_string(),
        Tok::Comma => "`,`".to_string(),
        Tok::Bang => "`!`".to_string(),
        Tok::Equals => "`=`".to_string(),
    }
}

// ─── Emission ──────────────────────────────────────────────────────────────

/// Render the parsed schema into an Orison source string. The output begins
/// with a `module <module_name>` header so it can be saved as-is into the
/// importing project's `src/` tree.
pub fn to_orison_module(schema: &GraphqlSchema, module_name: &str) -> String {
    let mut out = String::new();
    out.push_str("// Generated by `ori schema import graphql` — do not edit by hand.\n");
    out.push_str("module ");
    out.push_str(module_name);
    out.push_str("\n\n");

    for ty in &schema.types {
        out.push_str("type ");
        out.push_str(&ty.name);
        out.push_str(" = {\n");
        for (idx, field) in ty.fields.iter().enumerate() {
            out.push_str("  ");
            out.push_str(&field.name);
            out.push_str(": ");
            out.push_str(&field.ty);
            if idx + 1 < ty.fields.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("}\n\n");
    }

    for q in &schema.queries {
        emit_fn(&mut out, q, "http");
    }
    for m in &schema.mutations {
        emit_fn(&mut out, m, "http");
    }

    out
}

fn emit_fn(out: &mut String, q: &GqlQuery, effect: &str) {
    out.push_str("fn ");
    out.push_str(&q.name);
    out.push('(');
    for (idx, arg) in q.args.iter().enumerate() {
        if idx > 0 {
            out.push_str(", ");
        }
        out.push_str(&arg.name);
        out.push_str(": ");
        out.push_str(&arg.ty);
    }
    out.push_str(") -> ");
    out.push_str(&q.return_type);
    out.push_str(" uses ");
    out.push_str(effect);
    out.push('\n');
}

#[cfg(test)]
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;
    use crate::compiler::Compiler;
    use crate::source::SourceFile;

    fn parse_ok(sdl: &str) -> GraphqlSchema {
        match parse_sdl(sdl) {
            Ok(schema) => schema,
            Err(err) => {
                assert!(false, "expected ok schema, got error: {err}");
                unreachable!()
            }
        }
    }

    fn check_orison(source: &str) -> usize {
        let file = SourceFile::new("/generated.ori", source);
        let result = Compiler::check_source(file);
        result.diagnostics.iter().filter(|d| d.is_error()).count()
    }

    #[test]
    fn parses_single_type() {
        let schema = parse_ok("type User { id: ID, name: String }");
        assert_eq!(schema.types.len(), 1);
        assert_eq!(schema.types[0].name, "User");
        assert_eq!(schema.types[0].fields.len(), 2);
    }

    #[test]
    fn parses_nullable_and_non_null_fields() {
        let schema = parse_ok("type User { id: ID!, nickname: String }");
        let ty = &schema.types[0];
        assert!(!ty.fields[0].nullable, "id should be non-null");
        assert_eq!(ty.fields[0].ty, "Str");
        assert!(ty.fields[1].nullable, "nickname should be nullable");
        assert_eq!(ty.fields[1].ty, "Option[Str]");
    }

    #[test]
    fn parses_query_with_args() {
        let schema = parse_ok("type Query { user(id: ID!): User }");
        assert_eq!(schema.queries.len(), 1);
        let q = &schema.queries[0];
        assert_eq!(q.name, "user");
        assert_eq!(q.args.len(), 1);
        assert_eq!(q.args[0].name, "id");
        assert_eq!(q.args[0].ty, "Str");
        assert_eq!(q.return_type, "Option[User]");
    }

    #[test]
    fn parses_mutation() {
        let schema = parse_ok("type Mutation { ping: Boolean! }");
        assert_eq!(schema.mutations.len(), 1);
        assert_eq!(schema.mutations[0].return_type, "Bool");
    }

    #[test]
    fn strips_comments() {
        let sdl = "# leading\n# another\ntype User { # trailing\n  id: ID! # eol\n}\n";
        let schema = parse_ok(sdl);
        assert_eq!(schema.types.len(), 1);
        assert_eq!(schema.types[0].fields.len(), 1);
    }

    #[test]
    fn malformed_sdl_returns_error_with_line() {
        let sdl = "type User {\n  id ID!\n}";
        match parse_sdl(sdl) {
            Ok(_) => assert!(false, "missing colon should not parse"),
            Err(err) => {
                assert!(
                    err.line >= 2,
                    "error line should point at field, got {}",
                    err.line
                );
                assert!(!err.message.is_empty());
            }
        }
    }

    #[test]
    fn handles_list_types() {
        let schema = parse_ok("type Q { tags: [String!]!, ids: [ID] }");
        let ty = &schema.types[0];
        assert_eq!(ty.fields[0].ty, "List[Str]");
        assert!(!ty.fields[0].nullable);
        assert_eq!(ty.fields[1].ty, "Option[List[Option[Str]]]");
        assert!(ty.fields[1].nullable);
    }

    #[test]
    fn generated_orison_parses_clean() {
        let sdl = "type User { id: ID!, nickname: String, scores: [Int!] }\n\
                   type Query { user(id: ID!): User }\n\
                   type Mutation { rename(id: ID!, name: String!): User! }";
        let schema = parse_ok(sdl);
        let source = to_orison_module(&schema, "imported.graphql");
        assert!(source.starts_with("// Generated"));
        assert!(source.contains("module imported.graphql"));
        let errors = check_orison(&source);
        assert!(
            errors == 0,
            "expected zero errors, got: {errors}\n--- source ---\n{source}"
        );
    }

    #[test]
    fn import_report_counts_match() {
        let sdl = "type A { x: Int! } type B { y: Int }\n\
                   type Query { a: A, b: B } type Mutation { m: Boolean! }";
        let schema = parse_ok(sdl);
        let source = to_orison_module(&schema, "demo");
        let report = ImportReport::build(&schema, "demo", &source);
        assert_eq!(report.schema, "ori.graphql_import.v1");
        assert_eq!(report.types, 2);
        assert_eq!(report.queries, 2);
        assert_eq!(report.mutations, 1);
        assert!(report.generated_lines > 0);
        assert_eq!(report.module, "demo");
    }

    #[test]
    fn parser_is_deterministic() {
        let sdl = "type User { id: ID!, name: String }\n\
                   type Query { me: User }";
        let a = parse_ok(sdl);
        let b = parse_ok(sdl);
        assert_eq!(a, b);
        let sa = to_orison_module(&a, "demo");
        let sb = to_orison_module(&b, "demo");
        assert_eq!(sa, sb);
    }

    #[test]
    fn unsupported_declarations_are_recorded_not_dropped_silently() {
        let sdl = "scalar DateTime\n\
                   enum Role { ADMIN USER }\n\
                   type User { id: ID! }";
        let schema = parse_ok(sdl);
        assert_eq!(schema.types.len(), 1);
        assert!(schema.unsupported.iter().any(|u| u.starts_with("scalar")));
        assert!(schema.unsupported.iter().any(|u| u.starts_with("enum")));
    }

    #[test]
    fn scalar_mapping_is_exhaustive_for_known_scalars() {
        let sdl = "type S { a: ID!, b: String!, c: Int!, d: Float!, e: Boolean! }";
        let schema = parse_ok(sdl);
        let fields = &schema.types[0].fields;
        assert_eq!(fields[0].ty, "Str");
        assert_eq!(fields[1].ty, "Str");
        assert_eq!(fields[2].ty, "Int");
        assert_eq!(fields[3].ty, "Float64");
        assert_eq!(fields[4].ty, "Bool");
    }

    #[test]
    fn unknown_scalar_passes_through_as_named_type() {
        let schema = parse_ok("type T { ts: DateTime! }");
        assert_eq!(schema.types[0].fields[0].ty, "DateTime");
    }
}
