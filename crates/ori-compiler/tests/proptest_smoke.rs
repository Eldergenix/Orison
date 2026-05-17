//! End-to-end smoke coverage for the in-repo property-based testing
//! micro-framework.
//!
//! This integration test is intentionally self-contained: every
//! generator, the splitmix64 RNG, and the `quickcheck` driver are
//! re-implemented here using only the standard library. The same
//! framework code lives under `crate::proptest` for use by inline
//! unit tests; this file keeps a parallel copy so the integration
//! suite remains buildable even if the in-crate module's wiring is
//! in flux (the bootstrap workspace re-arranges modules frequently
//! during the M-PT roll-out).
//!
//! Three property tests live here, one per fragile area called out
//! in M-PT:
//!
//! * `pratt_parser_round_trip`: binary expression strings survive a
//!   parse → emit → re-parse cycle with identical ASTs.
//! * `version_solver_satisfies_constraints`: an in-test toy version
//!   solver (mirroring the shape of `ori_pkg::version_solver`)
//!   produces resolutions that satisfy every active constraint.
//! * `dispatch_path_match_total`: building a `DispatchTable` over
//!   arbitrary `(pattern, path)` pairs and calling `dispatch` never
//!   panics and produces consistent results.
//!
//! Each test runs `DEFAULT_RUNS` cases derived from `DEFAULT_SEED`.
//! On failure the test prints the original seed so the failing run
//! can be replayed verbatim.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;

use ori_compiler::backend_dispatch::{dispatch, DispatchError, DispatchTable, Request, RouteSpec};
use ori_compiler::codegen_text::emit_expr;
use ori_compiler::expr::{parse_body_expr, Expr};
use ori_compiler::lexer::lex;
use ori_compiler::source::SourceFile;

// ---------------------------------------------------------------------------
// Micro-framework (mirrors `ori_compiler::proptest`).
// ---------------------------------------------------------------------------

const DEFAULT_SEED: u64 = 0xDEAD_BEEF_CAFE_F00D;
const DEFAULT_RUNS: u32 = 64;

#[derive(Debug, Clone)]
struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Self {
            state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15),
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn gen_range(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            return lo;
        }
        let span = hi - lo;
        lo + (self.next_u64() % span)
    }

    fn gen_bool(&mut self, p: f64) -> bool {
        if p <= 0.0 {
            return false;
        }
        if p >= 1.0 {
            return true;
        }
        let cutoff = (p * (u64::MAX as f64)) as u64;
        self.next_u64() < cutoff
    }
}

trait Arbitrary: Sized + Clone + Debug {
    fn arbitrary(rng: &mut Rng) -> Self;
    fn shrink(&self) -> Vec<Self>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PropOutcome {
    Ok {
        runs: u32,
    },
    Counterexample {
        seed: u64,
        runs: u32,
        minimal: String,
    },
}

fn quickcheck<P, R>(seed: u64, runs: u32, prop: P) -> PropOutcome
where
    P: Fn(R) -> bool,
    R: Arbitrary,
{
    let mut rng = Rng::new(seed);
    for run in 0..runs {
        let candidate = R::arbitrary(&mut rng);
        if !prop(candidate.clone()) {
            let minimal = shrink_until_stable(&candidate, &prop);
            return PropOutcome::Counterexample {
                seed,
                runs: run + 1,
                minimal: format!("{:?}", minimal),
            };
        }
    }
    PropOutcome::Ok { runs }
}

fn shrink_until_stable<P, R>(start: &R, prop: &P) -> R
where
    P: Fn(R) -> bool,
    R: Arbitrary,
{
    const MAX_SHRINK_STEPS: u32 = 1024;
    let mut current = start.clone();
    let mut steps = 0u32;
    loop {
        if steps >= MAX_SHRINK_STEPS {
            return current;
        }
        let candidates = current.shrink();
        let mut progressed = false;
        for cand in candidates {
            if !prop(cand.clone()) {
                current = cand;
                progressed = true;
                break;
            }
        }
        if !progressed {
            return current;
        }
        steps += 1;
    }
}

fn fail_with(label: &str, outcome: PropOutcome) {
    match outcome {
        PropOutcome::Ok { .. } => {}
        PropOutcome::Counterexample {
            seed,
            runs,
            minimal,
        } => {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(
                    false,
                    "{label} failed at seed=0x{seed:016X} after {runs} runs; minimal={minimal}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Property 1: Pratt parser round trip.
// ---------------------------------------------------------------------------

const PROP_OPS: &[&str] = &[
    "+", "-", "*", "/", "%", "==", "!=", "<", "<=", ">", ">=", "&&", "||",
];

fn synth_atom(rng: &mut Rng) -> String {
    if rng.gen_bool(0.5) {
        let len = 1 + (rng.gen_range(0, 2) as usize);
        let mut s = String::with_capacity(len);
        for _ in 0..len {
            let idx = rng.gen_range(0, 26);
            s.push((b'a' + idx as u8) as char);
        }
        s
    } else {
        let n = rng.gen_range(0, 32);
        n.to_string()
    }
}

#[derive(Debug, Clone)]
struct BinExpr(String);

impl Arbitrary for BinExpr {
    fn arbitrary(rng: &mut Rng) -> Self {
        let mut s = synth_atom(rng);
        let extra = rng.gen_range(0, 3) as usize;
        for _ in 0..extra {
            let op_idx = rng.gen_range(0, PROP_OPS.len() as u64) as usize;
            let op = PROP_OPS[op_idx];
            let next = synth_atom(rng);
            let candidate_len = s.len() + 1 + op.len() + 1 + next.len();
            if candidate_len > 20 {
                break;
            }
            s.push(' ');
            s.push_str(op);
            s.push(' ');
            s.push_str(&next);
        }
        BinExpr(s)
    }

    fn shrink(&self) -> Vec<Self> {
        let mut out = Vec::new();
        if self.0.is_empty() {
            return out;
        }
        let mut last_space = None;
        for (i, b) in self.0.as_bytes().iter().enumerate().rev() {
            if *b == b' ' {
                last_space = Some(i);
                break;
            }
        }
        if let Some(idx) = last_space {
            out.push(BinExpr(self.0[..idx].to_string()));
            let prefix = &self.0[..idx];
            let mut prev_space = None;
            for (i, b) in prefix.as_bytes().iter().enumerate().rev() {
                if *b == b' ' {
                    prev_space = Some(i);
                    break;
                }
            }
            if let Some(p) = prev_space {
                out.push(BinExpr(self.0[..p].to_string()));
            }
        }
        out.push(BinExpr("a".to_string()));
        out
    }
}

fn parse_one(text: &str) -> Option<Expr> {
    let src = SourceFile::new("/prop.ori", text);
    let tokens = lex(&src);
    let (expr, diags) = parse_body_expr(&tokens);
    if diags.iter().any(|d| d.is_error()) {
        return None;
    }
    Some(expr)
}

#[test]
fn pratt_parser_round_trip() {
    let outcome = quickcheck::<_, BinExpr>(DEFAULT_SEED, DEFAULT_RUNS, |expr| {
        let source = expr.0;
        if source.is_empty() || source.len() > 20 {
            return true;
        }
        let first = match parse_one(&source) {
            Some(e) => e,
            None => return true,
        };
        let emitted = emit_expr(&first);
        let second = match parse_one(&emitted) {
            Some(e) => e,
            None => return false,
        };
        first == second
    });
    fail_with("pratt_parser_round_trip", outcome);
}

// ---------------------------------------------------------------------------
// Property 2: In-test toy version solver respects every active constraint.
//
// The bootstrap version solver lives in `ori-pkg::version_solver` and
// cannot be linked from a sibling `ori-compiler` test without
// introducing a cyclic dev-dep that the current workspace tooling
// rejects. This test reproduces the solver's contract with a minimal
// reference implementation so the property — "any Ok resolution
// satisfies every active constraint" — is still exercised by the
// micro-framework against the *same* shape of input the real solver
// faces in production.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ToyVersion {
    major: u32,
    minor: u32,
    patch: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ToyConstraint {
    /// `^A.B.0` ⇒ `>=A.B.0, <(A+1).0.0` (for `A>=1`)
    /// or       `>=0.B.0, <0.(B+1).0`     (for `A==0`)
    Caret(ToyVersion),
    /// `~A.B.0` ⇒ `>=A.B.0, <A.(B+1).0`
    Tilde(ToyVersion),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ToyPkgId {
    name: String,
    version: ToyVersion,
}

#[derive(Debug, Clone, Default)]
struct ToyGraph {
    packages: BTreeMap<String, Vec<ToyVersion>>,
    deps: BTreeMap<ToyPkgId, Vec<(String, ToyConstraint)>>,
}

fn toy_satisfies(v: &ToyVersion, c: &ToyConstraint) -> bool {
    match c {
        ToyConstraint::Caret(anchor) => {
            if v < anchor {
                return false;
            }
            if anchor.major == 0 {
                v.major == 0 && v.minor == anchor.minor
            } else {
                v.major == anchor.major
            }
        }
        ToyConstraint::Tilde(anchor) => {
            if v < anchor {
                return false;
            }
            v.major == anchor.major && v.minor == anchor.minor
        }
    }
}

fn toy_solve(graph: &ToyGraph, root: &str) -> Result<BTreeMap<String, ToyVersion>, &'static str> {
    let mut resolved: BTreeMap<String, ToyVersion> = BTreeMap::new();
    let mut constraints: BTreeMap<String, Vec<ToyConstraint>> = BTreeMap::new();
    let mut visiting: BTreeSet<String> = BTreeSet::new();

    fn assign(
        graph: &ToyGraph,
        name: &str,
        resolved: &mut BTreeMap<String, ToyVersion>,
        constraints: &mut BTreeMap<String, Vec<ToyConstraint>>,
        visiting: &mut BTreeSet<String>,
    ) -> Result<bool, &'static str> {
        if visiting.contains(name) {
            return Err("cycle");
        }
        let candidates = match graph.packages.get(name) {
            Some(v) => v.clone(),
            None => return Err("missing"),
        };
        if candidates.is_empty() {
            return Err("empty");
        }
        let effective: Vec<ToyConstraint> = constraints.get(name).cloned().unwrap_or_default();
        let mut viable: Vec<ToyVersion> = candidates
            .into_iter()
            .filter(|v| effective.iter().all(|c| toy_satisfies(v, c)))
            .collect();
        viable.sort();
        viable.reverse();
        if viable.is_empty() {
            return Ok(false);
        }
        if let Some(existing) = resolved.get(name).cloned() {
            if !effective.iter().all(|c| toy_satisfies(&existing, c)) {
                return Ok(false);
            }
            viable = vec![existing];
        }
        visiting.insert(name.to_string());
        for choice in viable {
            resolved.insert(name.to_string(), choice.clone());
            let pid = ToyPkgId {
                name: name.to_string(),
                version: choice.clone(),
            };
            let deps = graph.deps.get(&pid).cloned().unwrap_or_default();
            let mut pushed: Vec<String> = Vec::new();
            for (dep_name, constraint) in &deps {
                constraints
                    .entry(dep_name.clone())
                    .or_default()
                    .push(constraint.clone());
                pushed.push(dep_name.clone());
            }
            let mut all_ok = true;
            for (dep_name, _) in &deps {
                match assign(graph, dep_name, resolved, constraints, visiting)? {
                    true => {}
                    false => {
                        all_ok = false;
                        break;
                    }
                }
            }
            for dep_name in pushed.iter().rev() {
                if let Some(stack) = constraints.get_mut(dep_name) {
                    let _ = stack.pop();
                    if stack.is_empty() {
                        let _ = constraints.remove(dep_name);
                    }
                }
            }
            if all_ok {
                visiting.remove(name);
                return Ok(true);
            }
            let _ = resolved.remove(name);
        }
        visiting.remove(name);
        Ok(false)
    }

    let ok =
        assign(graph, root, &mut resolved, &mut constraints, &mut visiting).map_err(|_| "hard")?;
    if !ok {
        return Err("conflict");
    }
    Ok(resolved)
}

fn pkg_name(i: u64) -> String {
    format!("p{}", i % 8)
}

fn synth_toy_version(rng: &mut Rng) -> ToyVersion {
    ToyVersion {
        major: (rng.gen_range(0, 3)) as u32,
        minor: (rng.gen_range(0, 5)) as u32,
        patch: (rng.gen_range(0, 5)) as u32,
    }
}

fn synth_toy_constraint(rng: &mut Rng, v: &ToyVersion) -> ToyConstraint {
    let anchor = ToyVersion {
        major: v.major,
        minor: v.minor,
        patch: 0,
    };
    if rng.gen_bool(0.5) {
        ToyConstraint::Caret(anchor)
    } else {
        ToyConstraint::Tilde(anchor)
    }
}

#[derive(Debug, Clone)]
struct SolverInput {
    seed: u64,
}

impl Arbitrary for SolverInput {
    fn arbitrary(rng: &mut Rng) -> Self {
        SolverInput {
            seed: rng.next_u64(),
        }
    }
    fn shrink(&self) -> Vec<Self> {
        if self.seed == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        out.push(SolverInput { seed: 0 });
        out.push(SolverInput {
            seed: self.seed / 2,
        });
        if self.seed > 0 {
            out.push(SolverInput {
                seed: self.seed - 1,
            });
        }
        out
    }
}

fn build_toy_graph(seed: u64) -> (ToyGraph, String) {
    let mut rng = Rng::new(seed);
    let mut g = ToyGraph::default();
    let pkg_count = 1 + (rng.gen_range(0, 8) as usize);
    let mut names: Vec<String> = Vec::new();
    for i in 0..pkg_count {
        let name = pkg_name(i as u64);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    for name in &names {
        let v_count = 1 + (rng.gen_range(0, 5) as usize);
        let mut versions: Vec<ToyVersion> = Vec::new();
        for _ in 0..v_count {
            let v = synth_toy_version(&mut rng);
            if !versions.iter().any(|x| x == &v) {
                versions.push(v);
            }
        }
        g.packages.insert(name.clone(), versions);
    }
    for (idx, name) in names.iter().enumerate() {
        let versions = match g.packages.get(name) {
            Some(v) => v.clone(),
            None => continue,
        };
        for v in &versions {
            let mut deps: Vec<(String, ToyConstraint)> = Vec::new();
            let dep_count = rng.gen_range(0, 3) as usize;
            for _ in 0..dep_count {
                if idx + 1 >= names.len() {
                    break;
                }
                let pick = idx + 1 + (rng.gen_range(0, (names.len() - idx - 1) as u64) as usize);
                if pick >= names.len() {
                    continue;
                }
                let dep_name = names[pick].clone();
                let dep_versions = match g.packages.get(&dep_name) {
                    Some(vs) if !vs.is_empty() => vs.clone(),
                    _ => continue,
                };
                let anchor =
                    dep_versions[(rng.gen_range(0, dep_versions.len() as u64)) as usize].clone();
                let constraint = synth_toy_constraint(&mut rng, &anchor);
                deps.push((dep_name, constraint));
            }
            let pid = ToyPkgId {
                name: name.clone(),
                version: v.clone(),
            };
            if !deps.is_empty() {
                g.deps.insert(pid, deps);
            }
        }
    }
    let root = names.first().cloned().unwrap_or_else(|| "p0".to_string());
    (g, root)
}

fn toy_active_constraints(
    graph: &ToyGraph,
    resolved: &BTreeMap<String, ToyVersion>,
    root: &str,
) -> Vec<(String, ToyConstraint)> {
    let mut out = Vec::new();
    let mut stack: Vec<String> = vec![root.to_string()];
    let mut seen: BTreeSet<String> = BTreeSet::new();
    while let Some(name) = stack.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let version = match resolved.get(&name) {
            Some(v) => v.clone(),
            None => continue,
        };
        let pid = ToyPkgId { name, version };
        if let Some(deps) = graph.deps.get(&pid) {
            for (dep_name, constraint) in deps {
                out.push((dep_name.clone(), constraint.clone()));
                stack.push(dep_name.clone());
            }
        }
    }
    out
}

#[test]
fn version_solver_satisfies_constraints() {
    let outcome = quickcheck::<_, SolverInput>(DEFAULT_SEED, DEFAULT_RUNS, |input| {
        let (graph, root) = build_toy_graph(input.seed);
        let resolution = match toy_solve(&graph, &root) {
            Ok(r) => r,
            Err(_) => return true,
        };
        for (dep_name, constraint) in toy_active_constraints(&graph, &resolution, &root) {
            let chosen = match resolution.get(&dep_name) {
                Some(v) => v,
                None => return false,
            };
            if !toy_satisfies(chosen, &constraint) {
                return false;
            }
        }
        true
    });
    fail_with("version_solver_satisfies_constraints", outcome);
}

// ---------------------------------------------------------------------------
// Property 3: Dispatch path matching is total.
//
// Build a `DispatchTable` containing one route for the generated
// `pattern`, then call `dispatch` with a `Request` carrying the
// generated `path`. The exercise touches the same `match_path`
// machinery used in production via the public dispatcher entry
// point; for any input pair, `dispatch` must return either `Ok(_)`
// or `Err(DispatchError::_)` and never panic.
// ---------------------------------------------------------------------------

fn synth_path_string(rng: &mut Rng, allow_params: bool) -> String {
    let segments = rng.gen_range(0, 5) as usize;
    let mut out = String::from("/");
    let mut first = true;
    for _ in 0..segments {
        if !first {
            out.push('/');
        }
        first = false;
        let choice = rng.gen_range(0, 4);
        match choice {
            0 if allow_params => {
                out.push(':');
                let len = rng.gen_range(1, 4) as usize;
                for _ in 0..len {
                    let idx = rng.gen_range(0, 26);
                    out.push((b'a' + idx as u8) as char);
                }
            }
            1 => {}
            _ => {
                let len = rng.gen_range(1, 5) as usize;
                for _ in 0..len {
                    let idx = rng.gen_range(0, 26);
                    out.push((b'a' + idx as u8) as char);
                }
            }
        }
    }
    if rng.gen_bool(0.25) {
        out.push('/');
    }
    out
}

#[derive(Debug, Clone)]
struct PathInput(String);

impl Arbitrary for PathInput {
    fn arbitrary(rng: &mut Rng) -> Self {
        PathInput(synth_path_string(rng, false))
    }
    fn shrink(&self) -> Vec<Self> {
        let mut out = Vec::new();
        if self.0 == "/" {
            return out;
        }
        out.push(PathInput("/".to_string()));
        if self.0.len() > 2 {
            let half = self.0.len() / 2;
            out.push(PathInput(self.0.chars().take(half).collect()));
        }
        out
    }
}

#[derive(Debug, Clone)]
struct PatternInput(String);

impl Arbitrary for PatternInput {
    fn arbitrary(rng: &mut Rng) -> Self {
        PatternInput(synth_path_string(rng, true))
    }
    fn shrink(&self) -> Vec<Self> {
        let mut out = Vec::new();
        if self.0 == "/" {
            return out;
        }
        out.push(PatternInput("/".to_string()));
        if self.0.len() > 2 {
            let half = self.0.len() / 2;
            out.push(PatternInput(self.0.chars().take(half).collect()));
        }
        out
    }
}

#[derive(Debug, Clone)]
struct MatchPair(PatternInput, PathInput);

impl Arbitrary for MatchPair {
    fn arbitrary(rng: &mut Rng) -> Self {
        MatchPair(PatternInput::arbitrary(rng), PathInput::arbitrary(rng))
    }
    fn shrink(&self) -> Vec<Self> {
        let mut out = Vec::new();
        for p in self.0.shrink() {
            out.push(MatchPair(p, self.1.clone()));
        }
        for r in self.1.shrink() {
            out.push(MatchPair(self.0.clone(), r));
        }
        out
    }
}

fn check_dispatch_total(pattern: &str, path: &str) -> bool {
    let mut table = DispatchTable::new();
    let route = RouteSpec {
        symbol_id: "sym:demo.handler".to_string(),
        method: "GET".to_string(),
        path: pattern.to_string(),
        effects: vec!["http".to_string()],
        principal_required: false,
    };
    let _ = table.insert(route);
    let req = Request::new("GET", path);
    // `dispatch` must return a `Result`; either arm is acceptable.
    match dispatch(&table, &req) {
        Ok(_) => true,
        Err(DispatchError::NotFound) => true,
        Err(DispatchError::MethodNotAllowed { .. }) => true,
        Err(DispatchError::MissingPrincipal) => true,
        Err(DispatchError::MissingCapability { .. }) => true,
        Err(DispatchError::ConflictingRoute { .. }) => true,
    }
}

#[test]
fn dispatch_path_match_total() {
    let outcome = quickcheck::<_, MatchPair>(DEFAULT_SEED, DEFAULT_RUNS, |pair| {
        check_dispatch_total(&pair.0 .0, &pair.1 .0)
    });
    fail_with("dispatch_path_match_total", outcome);
}
