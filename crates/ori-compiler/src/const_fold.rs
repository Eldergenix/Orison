//! Conservative constant folder for the bootstrap compiler.
//!
//! This pass walks an [`Expr`] tree and collapses sub-trees that can be
//! reduced without observing side effects. It is intentionally narrow —
//! the bootstrap doesn't model arithmetic operators yet, so we only fold
//! the rules the body parser already produces:
//!
//! * `if true then A else _` → `A`
//! * `if false then _ else B` → `B`
//! * `Block { stmts: [], tail: Some(Lit v) }` → `Lit(v)`
//! * `Try(Construct("Ok", [v]))` → `v`
//! * `Try(Construct("Err", [e]))` → `Construct("Err", [e])` (early return form)
//! * `Match(Lit(_), arms)` picks the first matching literal arm.
//!
//! The folder is pure: it never panics, never mutates global state, and
//! is idempotent — `fold(fold(x)) == fold(x)` for every `x`.

use crate::expr::{Expr, Literal, MatchArm, Pattern, Stmt};

/// Fold literal-only sub-trees of `expr`. See module docs for the exact
/// set of rules applied.
pub fn fold_expr(expr: &Expr) -> Expr {
    let walked = walk(expr);
    rewrite(walked)
}

// ---------------------------------------------------------------------------
// Recursive descent — fold every sub-expression bottom-up first.
// ---------------------------------------------------------------------------

fn walk(expr: &Expr) -> Expr {
    match expr {
        Expr::Lit(_) | Expr::Var(_) | Expr::Error => expr.clone(),

        Expr::Call { callee, args } => Expr::Call {
            callee: Box::new(fold_expr(callee)),
            args: args.iter().map(fold_expr).collect(),
        },

        Expr::Field { base, name } => Expr::Field {
            base: Box::new(fold_expr(base)),
            name: name.clone(),
        },

        Expr::Block { stmts, tail } => Expr::Block {
            stmts: stmts.iter().map(fold_stmt).collect(),
            tail: tail.as_ref().map(|t| Box::new(fold_expr(t))),
        },

        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => Expr::If {
            cond: Box::new(fold_expr(cond)),
            then_branch: Box::new(fold_expr(then_branch)),
            else_branch: else_branch.as_ref().map(|e| Box::new(fold_expr(e))),
        },

        Expr::Match { scrutinee, arms } => Expr::Match {
            scrutinee: Box::new(fold_expr(scrutinee)),
            arms: arms
                .iter()
                .map(|arm| MatchArm {
                    pattern: arm.pattern.clone(),
                    body: fold_expr(&arm.body),
                })
                .collect(),
        },

        Expr::Return(value) => Expr::Return(value.as_ref().map(|v| Box::new(fold_expr(v)))),

        Expr::Construct { variant, args } => Expr::Construct {
            variant: variant.clone(),
            args: args.iter().map(fold_expr).collect(),
        },

        Expr::Try(inner) => Expr::Try(Box::new(fold_expr(inner))),

        Expr::Tuple(items) => Expr::Tuple(items.iter().map(fold_expr).collect()),

        Expr::Record { fields } => Expr::Record {
            fields: fields
                .iter()
                .map(|(name, v)| (name.clone(), fold_expr(v)))
                .collect(),
        },

        Expr::Lambda { params, body } => Expr::Lambda {
            params: params.clone(),
            body: Box::new(fold_expr(body)),
        },

        // Interpolated and raw string literals are leaves from the folder's
        // point of view: no constant folding rule rewrites their contents
        // (that lives in the string-literals lowering pass).
        Expr::InterpString { .. } | Expr::RawStr { .. } => expr.clone(),

        Expr::Binary { op, lhs, rhs } => Expr::Binary {
            op: *op,
            lhs: Box::new(fold_expr(lhs)),
            rhs: Box::new(fold_expr(rhs)),
        },

        Expr::Unary { op, operand } => Expr::Unary {
            op: *op,
            operand: Box::new(fold_expr(operand)),
        },
    }
}

fn fold_stmt(stmt: &Stmt) -> Stmt {
    match stmt {
        Stmt::Let { name, ty, init } => Stmt::Let {
            name: name.clone(),
            ty: ty.clone(),
            init: fold_expr(init),
        },
        Stmt::Expr(e) => Stmt::Expr(fold_expr(e)),
        Stmt::Return(value) => Stmt::Return(value.as_ref().map(fold_expr)),
    }
}

// ---------------------------------------------------------------------------
// Top-level rewrite rules — applied after children are folded.
// ---------------------------------------------------------------------------

fn rewrite(expr: Expr) -> Expr {
    match expr {
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => rewrite_if(*cond, *then_branch, else_branch.map(|b| *b)),

        Expr::Block { stmts, tail } => rewrite_block(stmts, tail.map(|t| *t)),

        Expr::Try(inner) => rewrite_try(*inner),

        Expr::Match { scrutinee, arms } => rewrite_match(*scrutinee, arms),

        other => other,
    }
}

fn rewrite_if(cond: Expr, then_branch: Expr, else_branch: Option<Expr>) -> Expr {
    if let Expr::Lit(Literal::Bool(true)) = cond {
        return then_branch;
    }
    if let Expr::Lit(Literal::Bool(false)) = cond {
        return match else_branch {
            Some(e) => e,
            None => Expr::Lit(Literal::Unit),
        };
    }
    Expr::If {
        cond: Box::new(cond),
        then_branch: Box::new(then_branch),
        else_branch: else_branch.map(Box::new),
    }
}

fn rewrite_block(stmts: Vec<Stmt>, tail: Option<Expr>) -> Expr {
    // Empty stmts + literal tail → unwrap to that literal.
    if stmts.is_empty() {
        if let Some(Expr::Lit(lit)) = tail {
            return Expr::Lit(lit);
        }
        if let Some(other) = tail {
            return Expr::Block {
                stmts,
                tail: Some(Box::new(other)),
            };
        }
        return Expr::Lit(Literal::Unit);
    }
    Expr::Block {
        stmts,
        tail: tail.map(Box::new),
    }
}

fn rewrite_try(inner: Expr) -> Expr {
    if let Expr::Construct { variant, mut args } = inner {
        if variant == "Ok" && args.len() == 1 {
            // Try(Ok(v)) → v
            let value = args.remove(0);
            return value;
        }
        if variant == "Err" && args.len() == 1 {
            // Try(Err(e)) → Err(e) (the "early return" canonical form).
            let err = args.remove(0);
            return Expr::Construct {
                variant: "Err".to_string(),
                args: vec![err],
            };
        }
        return Expr::Try(Box::new(Expr::Construct { variant, args }));
    }
    Expr::Try(Box::new(inner))
}

fn rewrite_match(scrutinee: Expr, arms: Vec<MatchArm>) -> Expr {
    let lit = match &scrutinee {
        Expr::Lit(lit) => lit.clone(),
        _ => {
            return Expr::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            };
        }
    };

    for arm in &arms {
        if pattern_matches_literal(&arm.pattern, &lit) {
            return arm.body.clone();
        }
        if matches!(arm.pattern, Pattern::Wildcard | Pattern::Binding(_)) {
            return arm.body.clone();
        }
    }

    // No matching arm — leave the match unchanged.
    Expr::Match {
        scrutinee: Box::new(scrutinee),
        arms,
    }
}

fn pattern_matches_literal(pattern: &Pattern, lit: &Literal) -> bool {
    match pattern {
        Pattern::Literal(other) => literals_equal(other, lit),
        _ => false,
    }
}

fn literals_equal(a: &Literal, b: &Literal) -> bool {
    match (a, b) {
        (Literal::Int(x), Literal::Int(y)) => x == y,
        (Literal::Float(x), Literal::Float(y)) => x.to_bits() == y.to_bits(),
        (Literal::Str(x), Literal::Str(y)) => x == y,
        (Literal::Bool(x), Literal::Bool(y)) => x == y,
        (Literal::Unit, Literal::Unit) => true,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn int(v: i64) -> Expr {
        Expr::Lit(Literal::Int(v))
    }

    fn boolean(v: bool) -> Expr {
        Expr::Lit(Literal::Bool(v))
    }

    // --- 1. if true → then ---
    #[test]
    fn fold_if_true() {
        let e = Expr::If {
            cond: Box::new(boolean(true)),
            then_branch: Box::new(int(1)),
            else_branch: Some(Box::new(int(2))),
        };
        assert_eq!(fold_expr(&e), int(1));
    }

    // --- 2. if false with else → else ---
    #[test]
    fn fold_if_false_with_else() {
        let e = Expr::If {
            cond: Box::new(boolean(false)),
            then_branch: Box::new(int(1)),
            else_branch: Some(Box::new(int(2))),
        };
        assert_eq!(fold_expr(&e), int(2));
    }

    // --- 3. if false without else → Unit ---
    #[test]
    fn fold_if_false_without_else_is_unit() {
        let e = Expr::If {
            cond: Box::new(boolean(false)),
            then_branch: Box::new(int(1)),
            else_branch: None,
        };
        assert_eq!(fold_expr(&e), Expr::Lit(Literal::Unit));
    }

    // --- 4. if non-literal cond is preserved ---
    #[test]
    fn fold_if_non_literal_is_preserved() {
        let e = Expr::If {
            cond: Box::new(Expr::Var("flag".into())),
            then_branch: Box::new(int(1)),
            else_branch: Some(Box::new(int(2))),
        };
        let folded = fold_expr(&e);
        assert!(matches!(folded, Expr::If { .. }));
    }

    // --- 5. Block with no stmts and literal tail → that literal ---
    #[test]
    fn fold_block_with_literal_tail() {
        let e = Expr::Block {
            stmts: vec![],
            tail: Some(Box::new(int(42))),
        };
        assert_eq!(fold_expr(&e), int(42));
    }

    // --- 6. Block with stmts is not collapsed ---
    #[test]
    fn fold_block_with_stmts_is_not_collapsed() {
        let e = Expr::Block {
            stmts: vec![Stmt::Expr(Expr::Var("io".into()))],
            tail: Some(Box::new(int(1))),
        };
        let folded = fold_expr(&e);
        assert!(matches!(folded, Expr::Block { .. }));
    }

    // --- 7. Try(Ok(v)) → v ---
    #[test]
    fn fold_try_ok() {
        let e = Expr::Try(Box::new(Expr::Construct {
            variant: "Ok".into(),
            args: vec![int(7)],
        }));
        assert_eq!(fold_expr(&e), int(7));
    }

    // --- 8. Try(Err(e)) → Err(e) ---
    #[test]
    fn fold_try_err() {
        let e = Expr::Try(Box::new(Expr::Construct {
            variant: "Err".into(),
            args: vec![Expr::Var("e".into())],
        }));
        assert_eq!(
            fold_expr(&e),
            Expr::Construct {
                variant: "Err".into(),
                args: vec![Expr::Var("e".into())]
            }
        );
    }

    // --- 9. Try over arbitrary value preserved ---
    #[test]
    fn fold_try_unknown_is_preserved() {
        let e = Expr::Try(Box::new(Expr::Var("maybe".into())));
        let folded = fold_expr(&e);
        assert!(matches!(folded, Expr::Try(_)));
    }

    // --- 10. Match on literal picks the matching arm ---
    #[test]
    fn fold_match_picks_literal_arm() {
        let e = Expr::Match {
            scrutinee: Box::new(int(2)),
            arms: vec![
                MatchArm {
                    pattern: Pattern::Literal(Literal::Int(1)),
                    body: Expr::Var("one".into()),
                },
                MatchArm {
                    pattern: Pattern::Literal(Literal::Int(2)),
                    body: Expr::Var("two".into()),
                },
            ],
        };
        assert_eq!(fold_expr(&e), Expr::Var("two".into()));
    }

    // --- 11. Match on literal with no matching arm is preserved ---
    #[test]
    fn fold_match_no_arm_is_preserved() {
        let e = Expr::Match {
            scrutinee: Box::new(int(5)),
            arms: vec![MatchArm {
                pattern: Pattern::Literal(Literal::Int(1)),
                body: Expr::Var("one".into()),
            }],
        };
        let folded = fold_expr(&e);
        assert!(matches!(folded, Expr::Match { .. }));
    }

    // --- 12. Match falls through to wildcard arm ---
    #[test]
    fn fold_match_falls_through_to_wildcard() {
        let e = Expr::Match {
            scrutinee: Box::new(int(99)),
            arms: vec![
                MatchArm {
                    pattern: Pattern::Literal(Literal::Int(1)),
                    body: Expr::Var("one".into()),
                },
                MatchArm {
                    pattern: Pattern::Wildcard,
                    body: Expr::Var("other".into()),
                },
            ],
        };
        assert_eq!(fold_expr(&e), Expr::Var("other".into()));
    }

    // --- 13. Idempotence — fold(fold(x)) == fold(x) ---
    #[test]
    fn fold_is_idempotent() {
        let exprs = vec![
            Expr::If {
                cond: Box::new(boolean(true)),
                then_branch: Box::new(Expr::Block {
                    stmts: vec![],
                    tail: Some(Box::new(int(5))),
                }),
                else_branch: Some(Box::new(int(0))),
            },
            Expr::Try(Box::new(Expr::Construct {
                variant: "Ok".into(),
                args: vec![int(1)],
            })),
            Expr::Match {
                scrutinee: Box::new(int(1)),
                arms: vec![MatchArm {
                    pattern: Pattern::Literal(Literal::Int(1)),
                    body: Expr::If {
                        cond: Box::new(boolean(false)),
                        then_branch: Box::new(int(10)),
                        else_branch: Some(Box::new(int(20))),
                    },
                }],
            },
            Expr::Var("x".into()),
            int(42),
            Expr::Lambda {
                params: vec![],
                body: Box::new(Expr::If {
                    cond: Box::new(boolean(true)),
                    then_branch: Box::new(int(1)),
                    else_branch: None,
                }),
            },
        ];
        for e in &exprs {
            let once = fold_expr(e);
            let twice = fold_expr(&once);
            if once != twice {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "fold not idempotent for {e:?}");
                }
            }
        }
    }

    // --- 14. Nested rule firing — if true + block tail → literal ---
    #[test]
    fn fold_nested_if_true_with_block_tail() {
        let e = Expr::If {
            cond: Box::new(boolean(true)),
            then_branch: Box::new(Expr::Block {
                stmts: vec![],
                tail: Some(Box::new(int(7))),
            }),
            else_branch: Some(Box::new(int(0))),
        };
        assert_eq!(fold_expr(&e), int(7));
    }

    // --- 15. Nested rule firing — Try(Ok(if true ...)) ---
    #[test]
    fn fold_nested_try_ok_with_if_inside() {
        let e = Expr::Try(Box::new(Expr::Construct {
            variant: "Ok".into(),
            args: vec![Expr::If {
                cond: Box::new(boolean(true)),
                then_branch: Box::new(int(9)),
                else_branch: Some(Box::new(int(0))),
            }],
        }));
        assert_eq!(fold_expr(&e), int(9));
    }
}
