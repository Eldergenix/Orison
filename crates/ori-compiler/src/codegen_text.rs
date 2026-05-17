//! Textual pseudo-IR emitter (bootstrap stand-in for native AOT).
//!
//! This module produces a deterministic, human-readable text artefact that
//! *looks like* LLVM IR but is intentionally not consumable by any LLVM
//! toolchain. The goal is to prove the codegen pipeline shape (`MirModule
//! -> textual artefact`) without taking on a native-codegen dependency in
//! the bootstrap. A later M10 pass can swap this implementation for a real
//! LLVM/MIR backend without changing the call sites.
//!
//! Output schema:
//!   - `; ModuleID = '<module>'` (header line)
//!   - `; ori.codegen_text.v1` (schema tag for downstream tools)
//!   - one blank separator line
//!   - per function, in source order: `define i32 @<name>() {`, `entry:`,
//!     `  ret i32 0`, `}`, followed by one blank line.
//!
//! The trailing newline is always present so file writers do not need to
//! special-case it.

use crate::expr::{Expr, InterpPart, Literal, MatchArm, Pattern, Stmt};
use crate::expr_ops::{BinOp, UnOp};
use crate::mir::MirModule;

/// Emit deterministic textual IR for the given MIR module.
pub fn emit_textual_ir(mir: &MirModule) -> String {
    let mut out = String::new();
    out.push_str("; ModuleID = '");
    out.push_str(&mir.module);
    out.push_str("'\n");
    out.push_str("; ori.codegen_text.v1\n");
    out.push('\n');

    for func in &mir.functions {
        out.push_str("define i32 @");
        out.push_str(&func.name);
        out.push_str("() {\n");
        out.push_str("entry:\n");
        out.push_str("  ret i32 0\n");
        out.push_str("}\n");
        out.push('\n');
    }

    out
}

// ---------------------------------------------------------------------------
// M24: textual expression emitter.
//
// `emit_expr` round-trips an `Expr` AST back to surface-syntax source that
// the body parser (`crate::expr::parse_body_expr`) can re-parse to an
// equivalent AST modulo whitespace and span data. The emitter is purely
// deterministic — every container is rendered in source order, record
// fields are sorted by key, and operator lexemes come from the
// `BinOp::lexeme` / `UnOp::lexeme` helpers so there is exactly one source
// of truth.
//
// Conventions:
//   * Binary / unary forms are always parenthesised so we never lose
//     precedence information when round-tripping nested expressions.
//   * String literals are escaped using `escape_string_literal`; the only
//     characters that need handling are `\`, `"`, newline, carriage
//     return, and tab.
//   * Interpolated strings re-escape `{`/`}` inside literal parts so the
//     re-lexer recognises them as literal braces, and surround embedded
//     expressions with `{ … }`.
//   * Raw strings emit the recorded `hashes` count verbatim on both
//     sides so `r#"…"#` shapes survive a round trip.
// ---------------------------------------------------------------------------

/// Render an [`Expr`] back to surface-syntax source.
///
/// The output is intended to be re-parseable by
/// [`crate::expr::parse_body_expr`] and produce an equivalent AST. Two
/// caveats apply:
///
/// * Span information is not preserved (the parser would assign fresh
///   spans on re-parse anyway).
/// * `Expr::Error` recovery nodes round-trip to a placeholder identifier
///   so the re-parser sees *something* parseable; callers that care about
///   error-preservation should inspect the AST before emitting.
pub fn emit_expr(expr: &Expr) -> String {
    let mut out = String::new();
    write_expr(&mut out, expr);
    out
}

fn write_expr(out: &mut String, expr: &Expr) {
    match expr {
        Expr::Lit(lit) => write_literal(out, lit),
        Expr::Var(name) => out.push_str(name),
        Expr::Call { callee, args } => {
            write_expr(out, callee);
            out.push('(');
            write_comma_list(out, args);
            out.push(')');
        }
        Expr::Field { base, name } => {
            write_expr(out, base);
            out.push('.');
            out.push_str(name);
        }
        Expr::Block { stmts, tail } => {
            out.push_str("{ ");
            let mut first = true;
            for stmt in stmts {
                if !first {
                    out.push_str("; ");
                }
                first = false;
                write_stmt(out, stmt);
            }
            if let Some(tail_expr) = tail {
                if !first {
                    out.push_str("; ");
                }
                write_expr(out, tail_expr);
            }
            out.push_str(" }");
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            out.push_str("if ");
            write_expr(out, cond);
            out.push_str(": ");
            write_expr(out, then_branch);
            if let Some(else_e) = else_branch {
                out.push_str(" else: ");
                write_expr(out, else_e);
            }
        }
        Expr::Match { scrutinee, arms } => {
            out.push_str("match ");
            write_expr(out, scrutinee);
            out.push(':');
            for arm in arms {
                out.push_str(" | ");
                write_arm(out, arm);
            }
        }
        Expr::Return(value) => {
            out.push_str("return");
            if let Some(v) = value {
                out.push(' ');
                write_expr(out, v);
            }
        }
        Expr::Construct { variant, args } => {
            out.push_str(variant);
            if !args.is_empty() {
                out.push('(');
                write_comma_list(out, args);
                out.push(')');
            }
        }
        Expr::Try(inner) => {
            write_expr(out, inner);
            out.push('?');
        }
        Expr::Tuple(items) => {
            out.push('(');
            write_comma_list(out, items);
            // Single-element tuples need a trailing comma to disambiguate
            // from a parenthesised expression. Multi-element tuples
            // round-trip without it.
            if items.len() == 1 {
                out.push(',');
            }
            out.push(')');
        }
        Expr::Record { fields } => {
            // Deterministic order: sort by field name.
            let mut sorted: Vec<&(String, Expr)> = fields.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            out.push_str("{ ");
            for (i, (name, value)) in sorted.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(name);
                out.push_str(": ");
                write_expr(out, value);
            }
            out.push_str(" }");
        }
        Expr::Lambda { params, body } => {
            out.push_str("fn (");
            for (i, (name, ty)) in params.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(name);
                if let Some(ty) = ty {
                    out.push_str(": ");
                    out.push_str(&ty.display());
                }
            }
            out.push_str(") => ");
            write_expr(out, body);
        }
        Expr::Binary { op, lhs, rhs } => {
            out.push('(');
            write_expr(out, lhs);
            out.push(' ');
            out.push_str(binop_to_str(*op));
            out.push(' ');
            write_expr(out, rhs);
            out.push(')');
        }
        Expr::Unary { op, operand } => {
            out.push('(');
            out.push_str(unop_to_str(*op));
            write_expr(out, operand);
            out.push(')');
        }
        Expr::InterpString { parts } => {
            out.push('"');
            for part in parts {
                match part {
                    InterpPart::Lit(text) => {
                        for ch in text.chars() {
                            match ch {
                                '\\' => out.push_str("\\\\"),
                                '"' => out.push_str("\\\""),
                                '\n' => out.push_str("\\n"),
                                '\r' => out.push_str("\\r"),
                                '\t' => out.push_str("\\t"),
                                '{' => out.push_str("\\{"),
                                '}' => out.push_str("\\}"),
                                _ => out.push(ch),
                            }
                        }
                    }
                    InterpPart::Expr(inner) => {
                        out.push_str("{ ");
                        write_expr(out, inner);
                        out.push_str(" }");
                    }
                }
            }
            out.push('"');
        }
        Expr::RawStr { text, hashes } => {
            out.push('r');
            for _ in 0..*hashes {
                out.push('#');
            }
            out.push('"');
            out.push_str(text);
            out.push('"');
            for _ in 0..*hashes {
                out.push('#');
            }
        }
        Expr::Error => {
            // Recovery sentinel — emit a parseable placeholder so the
            // re-parser sees a well-formed identifier.
            out.push_str("__error__");
        }
    }
}

fn write_stmt(out: &mut String, stmt: &Stmt) {
    match stmt {
        Stmt::Let { name, ty, init } => {
            out.push_str("let ");
            out.push_str(name);
            if let Some(ty) = ty {
                out.push_str(": ");
                out.push_str(&ty.display());
            }
            out.push_str(" = ");
            write_expr(out, init);
        }
        Stmt::Expr(e) => write_expr(out, e),
        Stmt::Return(value) => {
            out.push_str("return");
            if let Some(v) = value {
                out.push(' ');
                write_expr(out, v);
            }
        }
    }
}

fn write_arm(out: &mut String, arm: &MatchArm) {
    write_pattern(out, &arm.pattern);
    out.push_str(" => ");
    write_expr(out, &arm.body);
}

fn write_pattern(out: &mut String, pat: &Pattern) {
    match pat {
        Pattern::Wildcard => out.push('_'),
        Pattern::Binding(name) => out.push_str(name),
        Pattern::Literal(lit) => write_literal(out, lit),
        Pattern::Constructor { name, args } => {
            out.push_str(name);
            if !args.is_empty() {
                out.push('(');
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    write_pattern(out, a);
                }
                out.push(')');
            }
        }
    }
}

fn write_literal(out: &mut String, lit: &Literal) {
    match lit {
        Literal::Int(n) => out.push_str(&n.to_string()),
        Literal::Float(f) => {
            let s = format!("{f}");
            // Ensure the textual form is recognised as a float by the
            // lexer (must contain a `.`); otherwise it would round-trip
            // as an `Int`.
            if s.contains('.') || s.contains('e') || s.contains('E') {
                out.push_str(&s);
            } else {
                out.push_str(&s);
                out.push_str(".0");
            }
        }
        Literal::Str(s) => {
            out.push('"');
            out.push_str(&escape_string_literal(s));
            out.push('"');
        }
        Literal::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Literal::Unit => out.push_str("()"),
    }
}

fn write_comma_list(out: &mut String, exprs: &[Expr]) {
    for (i, e) in exprs.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        write_expr(out, e);
    }
}

fn binop_to_str(op: BinOp) -> &'static str {
    op.lexeme()
}

fn unop_to_str(op: UnOp) -> &'static str {
    op.lexeme()
}

fn escape_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::HirParam;
    use crate::mir::{MirBlock, MirFunction, MirInstruction};

    fn mir(name: &str, funcs: Vec<MirFunction>) -> MirModule {
        MirModule {
            module: name.to_string(),
            functions: funcs,
        }
    }

    fn func(name: &str) -> MirFunction {
        MirFunction {
            name: name.to_string(),
            params: Vec::<HirParam>::new(),
            return_type: "Int".to_string(),
            blocks: vec![MirBlock {
                id: 0,
                instructions: vec![MirInstruction {
                    op: "const_default".to_string(),
                    args: vec!["Int".to_string()],
                    result: Some(format!("%ret:{name}")),
                }],
            }],
        }
    }

    #[test]
    fn empty_module_emits_header_only() {
        let text = emit_textual_ir(&mir("empty", Vec::new()));
        let expected = "; ModuleID = 'empty'\n; ori.codegen_text.v1\n\n";
        assert_eq!(text, expected);
    }

    #[test]
    fn single_function_emits_define_block() {
        let text = emit_textual_ir(&mir("demo", vec![func("main")]));
        let expected_lines: Vec<&str> = vec![
            "; ModuleID = 'demo'",
            "; ori.codegen_text.v1",
            "",
            "define i32 @main() {",
            "entry:",
            "  ret i32 0",
            "}",
            "",
        ];
        let expected = format!("{}\n", expected_lines.join("\n"));
        assert_eq!(text, expected);
    }

    #[test]
    fn multiple_functions_preserve_source_order() {
        let text = emit_textual_ir(&mir("multi", vec![func("alpha"), func("beta")]));
        let alpha_idx = text.find("@alpha");
        let beta_idx = text.find("@beta");
        assert!(alpha_idx.is_some());
        assert!(beta_idx.is_some());
        assert!(alpha_idx < beta_idx, "alpha must appear before beta");
    }

    #[test]
    fn output_is_deterministic() {
        let m = mir("repeat", vec![func("a"), func("b")]);
        assert_eq!(emit_textual_ir(&m), emit_textual_ir(&m));
    }

    #[test]
    fn output_includes_schema_tag() {
        let text = emit_textual_ir(&mir("any", Vec::new()));
        assert!(text.contains("ori.codegen_text.v1"));
    }

    #[test]
    fn output_always_ends_with_newline() {
        let with_funcs = emit_textual_ir(&mir("a", vec![func("f")]));
        let without_funcs = emit_textual_ir(&mir("a", Vec::new()));
        assert!(with_funcs.ends_with('\n'));
        assert!(without_funcs.ends_with('\n'));
    }

    // -----------------------------------------------------------------
    // M24 textual codegen round-trip tests.
    //
    // Each test builds an `Expr` AST, runs it through `emit_expr`, then
    // re-parses the emitted source through the body parser and compares
    // the result against the original. Comparison is by `Debug` output —
    // span information is rebuilt by the parser so we can't use raw `==`
    // without comparing field-by-field, and Debug strings are stable for
    // the structural fields we care about.
    // -----------------------------------------------------------------

    use crate::expr::{parse_body_expr, Expr, InterpPart, Literal, MatchArm, Pattern};
    use crate::expr_ops::{BinOp, UnOp};
    use crate::lexer::lex;
    use crate::source::SourceFile;
    use crate::types::TypeRef;

    fn round_trip(expr: &Expr) -> Expr {
        let source = emit_expr(expr);
        let src = SourceFile::new("<codegen_text>", source.clone());
        let tokens = lex(&src);
        let (parsed, diags) = parse_body_expr(&tokens);
        #[allow(clippy::assertions_on_constants)]
        if !diags.is_empty() {
            assert!(false, "round-trip diagnostics for `{source}`: {diags:#?}");
        }
        parsed
    }

    /// Walk the AST and strip trailing empty `InterpPart::Lit("")` entries
    /// from every `Expr::InterpString`. The parser produces those as
    /// boundary markers but they are semantically empty; the emitter has
    /// no reason to materialise them and we don't want them to defeat
    /// structural comparison.
    fn canonicalise(expr: &Expr) -> Expr {
        match expr {
            Expr::InterpString { parts } => {
                let mut new_parts: Vec<InterpPart> = parts
                    .iter()
                    .map(|p| match p {
                        InterpPart::Lit(s) => InterpPart::Lit(s.clone()),
                        InterpPart::Expr(inner) => InterpPart::Expr(Box::new(canonicalise(inner))),
                    })
                    .collect();
                while matches!(new_parts.last(), Some(InterpPart::Lit(s)) if s.is_empty()) {
                    new_parts.pop();
                }
                Expr::InterpString { parts: new_parts }
            }
            Expr::Binary { op, lhs, rhs } => Expr::Binary {
                op: *op,
                lhs: Box::new(canonicalise(lhs)),
                rhs: Box::new(canonicalise(rhs)),
            },
            Expr::Unary { op, operand } => Expr::Unary {
                op: *op,
                operand: Box::new(canonicalise(operand)),
            },
            Expr::Call { callee, args } => Expr::Call {
                callee: Box::new(canonicalise(callee)),
                args: args.iter().map(canonicalise).collect(),
            },
            Expr::Field { base, name } => Expr::Field {
                base: Box::new(canonicalise(base)),
                name: name.clone(),
            },
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => Expr::If {
                cond: Box::new(canonicalise(cond)),
                then_branch: Box::new(canonicalise(then_branch)),
                else_branch: else_branch.as_ref().map(|e| Box::new(canonicalise(e))),
            },
            Expr::Match { scrutinee, arms } => Expr::Match {
                scrutinee: Box::new(canonicalise(scrutinee)),
                arms: arms
                    .iter()
                    .map(|a| MatchArm {
                        pattern: a.pattern.clone(),
                        body: canonicalise(&a.body),
                    })
                    .collect(),
            },
            Expr::Return(v) => Expr::Return(v.as_ref().map(|e| Box::new(canonicalise(e)))),
            Expr::Construct { variant, args } => Expr::Construct {
                variant: variant.clone(),
                args: args.iter().map(canonicalise).collect(),
            },
            Expr::Try(inner) => Expr::Try(Box::new(canonicalise(inner))),
            Expr::Tuple(items) => Expr::Tuple(items.iter().map(canonicalise).collect()),
            Expr::Record { fields } => Expr::Record {
                fields: fields
                    .iter()
                    .map(|(k, v)| (k.clone(), canonicalise(v)))
                    .collect(),
            },
            Expr::Lambda { params, body } => Expr::Lambda {
                params: params.clone(),
                body: Box::new(canonicalise(body)),
            },
            Expr::Block { stmts, tail } => Expr::Block {
                stmts: stmts.clone(),
                tail: tail.as_ref().map(|e| Box::new(canonicalise(e))),
            },
            Expr::Lit(_) | Expr::Var(_) | Expr::RawStr { .. } | Expr::Error => expr.clone(),
        }
    }

    fn assert_round_trip(expr: Expr) {
        let parsed = round_trip(&expr);
        let want = format!("{:?}", canonicalise(&expr));
        let got = format!("{:?}", canonicalise(&parsed));
        assert_eq!(
            got,
            want,
            "round-trip mismatch\n  emitted: {}",
            emit_expr(&expr)
        );
    }

    fn int(n: i64) -> Expr {
        Expr::Lit(Literal::Int(n))
    }

    #[test]
    fn round_trip_binary_add() {
        // a + b
        let expr = Expr::Binary {
            op: BinOp::Add,
            lhs: Box::new(Expr::Var("a".to_string())),
            rhs: Box::new(Expr::Var("b".to_string())),
        };
        assert_round_trip(expr);
    }

    #[test]
    fn round_trip_unary_neg() {
        // -x
        let expr = Expr::Unary {
            op: UnOp::Neg,
            operand: Box::new(Expr::Var("x".to_string())),
        };
        assert_round_trip(expr);
    }

    #[test]
    fn round_trip_unary_not() {
        // !flag
        let expr = Expr::Unary {
            op: UnOp::Not,
            operand: Box::new(Expr::Var("flag".to_string())),
        };
        assert_round_trip(expr);
    }

    #[test]
    fn round_trip_interp_string() {
        // "hello {name}"
        let expr = Expr::InterpString {
            parts: vec![
                InterpPart::Lit("hello ".to_string()),
                InterpPart::Expr(Box::new(Expr::Var("name".to_string()))),
            ],
        };
        assert_round_trip(expr);
    }

    #[test]
    fn round_trip_raw_string_zero_hashes() {
        let expr = Expr::RawStr {
            text: "no escapes here".to_string(),
            hashes: 0,
        };
        assert_round_trip(expr);
    }

    #[test]
    fn round_trip_raw_string_one_hash() {
        // The bootstrap body parser only re-parses hash-free raw strings
        // (`try_parse_raw_string` notes that hashed forms need the
        // dedicated `lex_string_extended` path that is not wired through
        // `lex`). For 1+ hashes we therefore validate the emitted text
        // shape instead of a full AST round-trip — emission is the M24
        // scope, and the shape itself is what the formatter will hand to
        // any later lexer that supports `r#"…"#`.
        let expr = Expr::RawStr {
            text: "say hi".to_string(),
            hashes: 1,
        };
        assert_eq!(emit_expr(&expr), "r#\"say hi\"#");
    }

    #[test]
    fn round_trip_raw_string_two_hashes() {
        // See `round_trip_raw_string_one_hash` for why this is a textual
        // assertion rather than a full AST round-trip.
        let expr = Expr::RawStr {
            text: "two hashes".to_string(),
            hashes: 2,
        };
        assert_eq!(emit_expr(&expr), "r##\"two hashes\"##");
    }

    #[test]
    fn round_trip_tuple_multi() {
        // (1, 2, 3)
        let expr = Expr::Tuple(vec![int(1), int(2), int(3)]);
        assert_round_trip(expr);
    }

    #[test]
    fn round_trip_record_sorted() {
        // { a: 1, b: 2 } — fields supplied in reverse order so we exercise
        // the deterministic sort in the emitter.
        let expr = Expr::Record {
            fields: vec![("b".to_string(), int(2)), ("a".to_string(), int(1))],
        };
        // Build the expected post-sort AST so the round-trip compares
        // structurally identical inputs.
        let sorted = Expr::Record {
            fields: vec![("a".to_string(), int(1)), ("b".to_string(), int(2))],
        };
        let parsed = round_trip(&expr);
        assert_eq!(format!("{parsed:?}"), format!("{sorted:?}"));
    }

    #[test]
    fn round_trip_lambda_with_annotations() {
        // fn (p1: Int, p2) => (p1 + p2)
        let expr = Expr::Lambda {
            params: vec![
                (
                    "p1".to_string(),
                    Some(TypeRef::Primitive("Int".to_string())),
                ),
                ("p2".to_string(), None),
            ],
            body: Box::new(Expr::Binary {
                op: BinOp::Add,
                lhs: Box::new(Expr::Var("p1".to_string())),
                rhs: Box::new(Expr::Var("p2".to_string())),
            }),
        };
        assert_round_trip(expr);
    }

    #[test]
    fn round_trip_match_multiple_arms() {
        // match x: | Some(v) => v | None => 0
        let expr = Expr::Match {
            scrutinee: Box::new(Expr::Var("x".to_string())),
            arms: vec![
                MatchArm {
                    pattern: Pattern::Constructor {
                        name: "Some".to_string(),
                        args: vec![Pattern::Binding("v".to_string())],
                    },
                    body: Expr::Var("v".to_string()),
                },
                MatchArm {
                    pattern: Pattern::Constructor {
                        name: "None".to_string(),
                        args: Vec::new(),
                    },
                    body: int(0),
                },
            ],
        };
        assert_round_trip(expr);
    }

    #[test]
    fn round_trip_nested_arithmetic_precedence() {
        // (a + (b * c)) — parens are explicit so re-parse preserves shape.
        let expr = Expr::Binary {
            op: BinOp::Add,
            lhs: Box::new(Expr::Var("a".to_string())),
            rhs: Box::new(Expr::Binary {
                op: BinOp::Mul,
                lhs: Box::new(Expr::Var("b".to_string())),
                rhs: Box::new(Expr::Var("c".to_string())),
            }),
        };
        assert_round_trip(expr);
    }

    #[test]
    fn round_trip_if_else() {
        // if cond: 1 else: 2
        let expr = Expr::If {
            cond: Box::new(Expr::Var("cond".to_string())),
            then_branch: Box::new(int(1)),
            else_branch: Some(Box::new(int(2))),
        };
        assert_round_trip(expr);
    }

    #[test]
    fn round_trip_interp_string_escapes_braces() {
        // Literal `{` and `}` inside a literal part must be escaped on
        // emit so the re-lexer treats them as plain text, not as the
        // start/end of an interpolation hole.
        let expr = Expr::InterpString {
            parts: vec![
                InterpPart::Lit("a{b}c".to_string()),
                InterpPart::Expr(Box::new(Expr::Var("x".to_string()))),
            ],
        };
        assert_round_trip(expr);
    }
}
