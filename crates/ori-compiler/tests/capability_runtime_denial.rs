//! Capability runtime denial.
//!
//! The compiler's effect checker is the runtime "deny path" for capability
//! mismatches: a function that declares `uses unsafe` (or any other effect)
//! must surface a diagnostic when the package policy does not list that
//! capability. The bootstrap channel for this is `E0410` from
//! `effect_check::effect_diagnostics` plus `W0401` from the parser when the
//! effect name is unknown entirely.
//!
//! This test:
//!
//! 1. Drives `Compiler::check_source` on a tiny `.ori` source so we exercise
//!    the full parser pipeline (lexer → parser → style diagnostics).
//! 2. Re-runs `effect_check::effect_diagnostics` on the same parsed module
//!    with a deliberately restrictive policy and asserts an `E0410` deny
//!    finding is produced for `unsafe`, complete with the symbol id, agent
//!    summary, and documentation pointer downstream tools rely on.
//! 3. Asserts that a completely unknown capability name yields `W0401` from
//!    the parser path so the warning surface stays wired in.

use ori_compiler::effect_check::effect_diagnostics;
use ori_compiler::{Compiler, SourceFile};

const UNSAFE_SOURCE: &str = "module demo\nfn raw_pointer_dance() -> Unit uses unsafe\n";
const UNKNOWN_SOURCE: &str = "module demo\nfn warp_drive() -> Unit uses warp.engine\n";

#[test]
#[allow(clippy::assertions_on_constants)]
fn unsafe_effect_denied_when_not_in_policy() {
    // Drive the full compile pipeline first so we exercise the same path
    // that `ori check` runs in production.
    let source = SourceFile::new("denial.ori", UNSAFE_SOURCE);
    let result = Compiler::check_source(source);

    // The parser must surface the symbol with its declared effect; if not,
    // the rest of the test means nothing.
    let symbol = result
        .module
        .symbols
        .iter()
        .find(|s| s.name == "raw_pointer_dance");
    let symbol = match symbol {
        Some(s) => s,
        None => {
            assert!(
                false,
                "parser failed to surface `raw_pointer_dance`; got {:?}",
                result.module.symbols
            );
            return;
        }
    };
    assert!(
        symbol.effects.iter().any(|e| e == "unsafe"),
        "parsed symbol must carry `unsafe` in its effect list, got {:?}",
        symbol.effects
    );

    // `unsafe` is a known effect, so the parser does NOT emit W0401 here.
    // Confirm that, otherwise the negative coverage below is misleading.
    let w0401_for_unsafe = result.diagnostics.iter().any(|d| {
        d.id == "W0401" && d.symbol.as_ref().map(|s| s.id.as_str()) == Some(symbol.id.as_str())
    });
    assert!(
        !w0401_for_unsafe,
        "W0401 should not fire for the known `unsafe` effect; got {:?}",
        result.diagnostics
    );

    // Now apply a deliberately restrictive package policy that intentionally
    // omits `unsafe`. The effect checker must emit `E0410` and point at the
    // offending symbol so the runtime denial is unambiguous.
    let policy: Vec<String> = vec!["fs.read".to_string()];
    let diags = effect_diagnostics(&result.module, &policy);
    let denial = diags.iter().find(|d| {
        d.id == "E0410"
            && d.symbol.as_ref().map(|s| s.id.as_str()) == Some(symbol.id.as_str())
            && d.found.iter().any(|f| f == "unsafe")
    });
    let denial = match denial {
        Some(d) => d,
        None => {
            assert!(
                false,
                "expected E0410 deny path for `unsafe`, got {diags:?}"
            );
            return;
        }
    };

    assert!(
        denial.is_error(),
        "E0410 must be error-level, got {:?}",
        denial.level
    );
    assert!(
        denial
            .expected
            .iter()
            .any(|e| e.contains("declare `unsafe`")),
        "expected[] must guide the developer to declare the capability, got {:?}",
        denial.expected
    );
    assert!(
        !denial.agent.summary.is_empty(),
        "E0410 must carry an agent summary so `ori agent` can route the fix"
    );
    assert!(
        denial
            .agent
            .docs
            .iter()
            .any(|doc| doc == "doc:effects.policy"),
        "E0410 must link to the policy doc, got {:?}",
        denial.agent.docs
    );
    assert!(
        denial.message.contains("unsafe"),
        "E0410 message must name the offending effect, got {:?}",
        denial.message
    );
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn unknown_capability_emits_w0401_from_parser() {
    // Complementary deny path: an unrecognised effect name flows through
    // `W0401` regardless of policy. This keeps the parser-side coverage
    // wired so we notice if either rule disappears.
    let source = SourceFile::new("unknown.ori", UNKNOWN_SOURCE);
    let result = Compiler::check_source(source);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.id == "W0401" && d.found.iter().any(|f| f == "warp.engine"));
    let warning = match warning {
        Some(w) => w,
        None => {
            assert!(
                false,
                "expected W0401 for unknown effect `warp.engine`, got {:?}",
                result.diagnostics
            );
            return;
        }
    };
    assert!(
        warning
            .agent
            .docs
            .iter()
            .any(|doc| doc == "doc:effects.known-effects"),
        "W0401 must link to known-effects doc, got {:?}",
        warning.agent.docs
    );
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn empty_policy_does_not_silently_deny() {
    // Guardrail: when the package policy list is empty, `effect_diagnostics`
    // must NOT emit E0410 (that would create false positives for packages
    // that have not yet adopted a capability policy). This locks in the
    // current opt-in semantics so a future refactor cannot regress it
    // without updating this test.
    let source = SourceFile::new("opt_in.ori", UNSAFE_SOURCE);
    let result = Compiler::check_source(source);
    let diags = effect_diagnostics(&result.module, &[]);
    assert!(
        !diags.iter().any(|d| d.id == "E0410"),
        "E0410 must stay quiet when the package policy is empty, got {diags:?}"
    );
}
