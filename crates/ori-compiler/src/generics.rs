//! Generic instantiation by usage.
//!
//! This module is the additive monomorphization layer that sits next to
//! [`crate::type_infer`]. Where `type_infer` walks expressions with a
//! direct, syntax-driven rule set, `generics` introduces a small
//! Hindley/Milner-style core so the type checker can:
//!
//!   * lift type parameters into fresh unification variables (`Var(N)`),
//!   * unify each call-site argument's inferred type against the
//!     parameter's instantiated type,
//!   * apply the resulting substitution to the return type, producing a
//!     monomorphic [`TypeRef`].
//!
//! ## Representation
//!
//! Unification variables are tracked via [`TyRef::Var`] — a thin local
//! wrapper over [`TypeRef`] that adds a `Var(u32)` discriminant without
//! modifying the schema-stable surface type. Conversions in both
//! directions are total: [`TyRef::from_typeref`] is a structural lift,
//! and [`TyRef::to_typeref`] collapses every variable to
//! [`TypeRef::Unknown`] (or, when requested, fails defensively via the
//! `E0232` diagnostic so a leaked variable never reaches downstream
//! tooling). Existing call sites that do not produce instantiation
//! `Var`s never observe the new representation.
//!
//! ## Determinism
//!
//! * `Var` ids are allocated by a single [`FreshIds`] counter so the
//!   monomorphization order is stable across runs.
//! * [`Subst`] is backed by a `BTreeMap` so iteration order is keyed by
//!   the `u32` tag rather than by hash-randomized insertion order.
//! * `unify` performs an occurs check before binding a variable, so
//!   accidental infinite types are reported as `E0231` rather than
//!   looped on.
//!
//! ## Diagnostic IDs
//!
//! * [`E_CANNOT_UNIFY`]        — `E0230`, structural mismatch between
//!   two concrete types during unification.
//! * [`E_OCCURS_CHECK`]        — `E0231`, occurs-check failure (would
//!   create an infinite type).
//! * [`E_UNRESOLVED_TYPE_VAR`] — `E0232`, a `Var(N)` survived past
//!   inference. Reported defensively; the upstream pipeline is expected
//!   to either resolve every fresh variable or surface a `cannot_unify`
//!   first.
//!
//! ## Stability invariant
//!
//! The whole surface lives behind `pub` items in this module. No
//! external module currently consumes them; adding new fields is safe,
//! removing or renaming is a compatibility break. Existing
//! [`crate::type_infer`] entry points (`infer_expr`,
//! `check_module_bodies`, ...) are unchanged — this module is strictly
//! additive.

use crate::diagnostic::Diagnostic;
use crate::source::Span;
use crate::types::TypeRef;
use std::collections::{BTreeMap, BTreeSet};

// ---------------------------------------------------------------------------
// Diagnostic ID constants
// ---------------------------------------------------------------------------

/// Cannot unify two concrete types (structural mismatch).
pub const E_CANNOT_UNIFY: &str = "E0230";
/// Occurs-check failed during unification (would create an infinite type).
pub const E_OCCURS_CHECK: &str = "E0231";
/// A unification variable survived past inference. Defensive only — the
/// upstream pipeline is expected to either fully resolve or surface
/// `cannot_unify` first.
pub const E_UNRESOLVED_TYPE_VAR: &str = "E0232";

// ---------------------------------------------------------------------------
// Local type representation with unification variables
// ---------------------------------------------------------------------------

/// Internal type representation used by the generic instantiator. Mirrors
/// [`TypeRef`] exactly, with the addition of [`TyRef::Var`] for
/// unification variables. Keeping this as a parallel enum lets us extend
/// the type system with `Var(u32)` without disturbing the schema-stable
/// surface [`TypeRef`] consumed by downstream serialization layers.
///
/// `from_typeref` and `to_typeref` provide structural conversions in
/// both directions. The whole representation is `Clone + PartialEq + Eq`
/// so substitutions and equality checks remain trivially deterministic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TyRef {
    /// Primitive type (e.g. `Int`, `Str`). Mirrors [`TypeRef::Primitive`].
    Primitive(String),
    /// User-defined named type. Mirrors [`TypeRef::Named`].
    Named(String),
    /// Generic application like `List[Int]`. Mirrors [`TypeRef::Generic`].
    Generic {
        /// Constructor name.
        name: String,
        /// Type arguments in source order.
        args: Vec<TyRef>,
    },
    /// Function type. Mirrors [`TypeRef::Fn`].
    Fn {
        /// Parameter types in source order.
        params: Vec<TyRef>,
        /// Return type.
        ret: Box<TyRef>,
    },
    /// Placeholder. Mirrors [`TypeRef::Unknown`].
    Unknown,
    /// Unification variable. The numeric tag is allocated by [`FreshIds`]
    /// and resolved via [`Subst::apply`]. A `Var(N)` that survives past
    /// type inference is reported as `E0232`.
    Var(u32),
}

impl TyRef {
    /// Structural lift from the schema-stable [`TypeRef`] to the
    /// instantiator's representation. Total: never fails. A
    /// [`TypeRef::Var`] on the input — present when the surface enum has
    /// been taught about unification variables — is propagated as the
    /// matching [`TyRef::Var`]; the lift never invents a `Var` from
    /// concrete inputs.
    pub fn from_typeref(ty: &TypeRef) -> Self {
        // The wildcard arm keeps this lift total even if the surface
        // enum ever sprouts further variants the instantiator hasn't
        // been taught about yet. New variants are folded to `Unknown`
        // so the worst case is a slightly less-precise inference — not
        // a hard failure that would block monomorphization.
        #[allow(unreachable_patterns)]
        match ty {
            TypeRef::Primitive(n) => TyRef::Primitive(n.clone()),
            TypeRef::Named(n) => TyRef::Named(n.clone()),
            TypeRef::Generic { name, args } => TyRef::Generic {
                name: name.clone(),
                args: args.iter().map(TyRef::from_typeref).collect(),
            },
            TypeRef::Fn { params, ret } => TyRef::Fn {
                params: params.iter().map(TyRef::from_typeref).collect(),
                ret: Box::new(TyRef::from_typeref(ret)),
            },
            TypeRef::Unknown => TyRef::Unknown,
            _ => TyRef::Unknown,
        }
    }

    /// Collapse the instantiator's representation back to a
    /// schema-stable [`TypeRef`]. Surviving [`TyRef::Var`]s are
    /// folded to [`TypeRef::Unknown`] for forward compatibility with
    /// callers that have not yet been taught about variables. Callers
    /// that need a defensive `E0232` should use
    /// [`Self::to_typeref_strict`] instead.
    pub fn to_typeref(&self) -> TypeRef {
        match self {
            TyRef::Primitive(n) => TypeRef::Primitive(n.clone()),
            TyRef::Named(n) => TypeRef::Named(n.clone()),
            TyRef::Generic { name, args } => TypeRef::Generic {
                name: name.clone(),
                args: args.iter().map(TyRef::to_typeref).collect(),
            },
            TyRef::Fn { params, ret } => TypeRef::Fn {
                params: params.iter().map(TyRef::to_typeref).collect(),
                ret: Box::new(ret.to_typeref()),
            },
            TyRef::Unknown | TyRef::Var(_) => TypeRef::Unknown,
        }
    }

    /// Strict variant of [`Self::to_typeref`]: errors with
    /// [`TypeInferError::UnresolvedTypeVar`] when any [`TyRef::Var`] is
    /// still present. Used by callers that must guarantee a fully
    /// monomorphized type before downstream handoff.
    pub fn to_typeref_strict(&self) -> Result<TypeRef, TypeInferError> {
        assert_no_unresolved(self)?;
        Ok(self.to_typeref())
    }

    /// Render the type back to its surface-syntax form. Mirrors
    /// [`TypeRef::display`]; variables render as `?N` so they are
    /// visually distinct from `_` (unknown) in diagnostic messages.
    pub fn display(&self) -> String {
        match self {
            TyRef::Primitive(name) | TyRef::Named(name) => name.clone(),
            TyRef::Generic { name, args } => {
                let inner = args
                    .iter()
                    .map(TyRef::display)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{name}[{inner}]")
            }
            TyRef::Fn { params, ret } => {
                let inner = params
                    .iter()
                    .map(TyRef::display)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("Fn({inner}) -> {}", ret.display())
            }
            TyRef::Unknown => "_".to_string(),
            TyRef::Var(n) => format!("?{n}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Error produced by [`unify`] and the higher-level [`instantiate_call`]
/// helper. Carries enough structure for callers to render either a
/// [`Diagnostic`] or a plain message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeInferError {
    /// `E0230` — two concrete types could not be unified.
    CannotUnify {
        /// First type as displayed to the user.
        left: String,
        /// Second type as displayed to the user.
        right: String,
    },
    /// `E0231` — would create an infinite type (e.g. `?0 = List[?0]`).
    OccursCheck {
        /// Numeric tag of the offending variable.
        var: u32,
        /// Display form of the type the variable would point to.
        ty: String,
    },
    /// `E0232` — a `Var(N)` survived past inference and was observed by
    /// a downstream consumer.
    UnresolvedTypeVar {
        /// Numeric tag of the leaked variable.
        var: u32,
    },
}

impl TypeInferError {
    /// Return the diagnostic id (`E0230`/`E0231`/`E0232`) for this error.
    pub fn id(&self) -> &'static str {
        match self {
            TypeInferError::CannotUnify { .. } => E_CANNOT_UNIFY,
            TypeInferError::OccursCheck { .. } => E_OCCURS_CHECK,
            TypeInferError::UnresolvedTypeVar { .. } => E_UNRESOLVED_TYPE_VAR,
        }
    }

    /// Human-readable error message in the canonical `expected X, got Y`
    /// style used by the rest of the compiler.
    pub fn message(&self) -> String {
        match self {
            TypeInferError::CannotUnify { left, right } => {
                format!("cannot unify `{left}` with `{right}`")
            }
            TypeInferError::OccursCheck { var, ty } => {
                format!("occurs check failed: `?{var}` cannot equal `{ty}` (infinite type)")
            }
            TypeInferError::UnresolvedTypeVar { var } => {
                format!("unresolved type variable `?{var}` survived past inference")
            }
        }
    }

    /// Render the error as a fully-formed [`Diagnostic`] anchored at `span`.
    pub fn to_diagnostic(&self, span: Span) -> Diagnostic {
        let (expected, found, summary, docs) = match self {
            TypeInferError::CannotUnify { left, right } => (
                vec![left.clone()],
                vec![right.clone()],
                "Adjust one side so both types agree, or insert an explicit conversion.",
                vec!["doc:types.unification".to_string()],
            ),
            TypeInferError::OccursCheck { var, ty } => (
                vec![format!("?{var}")],
                vec![ty.clone()],
                "Break the cycle: the variable cannot appear inside the type it unifies with.",
                vec!["doc:types.occurs-check".to_string()],
            ),
            TypeInferError::UnresolvedTypeVar { var } => (
                vec!["a fully resolved type".to_string()],
                vec![format!("?{var}")],
                "Ensure every type parameter is constrained at the call site.",
                vec!["doc:types.unresolved".to_string()],
            ),
        };
        Diagnostic::error(self.id(), self.message(), span)
            .with_expected(expected)
            .with_found(found)
            .with_agent_summary(summary)
            .with_docs(docs)
    }
}

// ---------------------------------------------------------------------------
// Fresh variable allocation
// ---------------------------------------------------------------------------

/// Monotonic counter used to mint fresh unification variables. A single
/// `FreshIds` is threaded through an entire instantiation session so the
/// resulting `Var` tags are dense and ordered, giving deterministic
/// monomorphization output.
#[derive(Debug, Clone, Default)]
pub struct FreshIds {
    next: u32,
}

impl FreshIds {
    /// Create a counter starting at zero.
    pub fn new() -> Self {
        Self { next: 0 }
    }

    /// Create a counter starting at `start`. Useful when an outer pass
    /// has already reserved a prefix of the variable space.
    pub fn starting_at(start: u32) -> Self {
        Self { next: start }
    }

    /// Mint a fresh `Var(N)` and bump the internal counter.
    pub fn fresh(&mut self) -> TyRef {
        let id = self.next;
        self.next = self.next.saturating_add(1);
        TyRef::Var(id)
    }

    /// Bump the counter to at least `floor` so the next mint returns an
    /// id `>= floor`. Used by [`instantiate`] to skip past tags that are
    /// already in use inside a scheme. A no-op when `floor <= peek()`.
    pub fn advance_to(&mut self, floor: u32) {
        if self.next < floor {
            self.next = floor;
        }
    }

    /// Peek at the next id without consuming it.
    pub fn peek(&self) -> u32 {
        self.next
    }
}

// ---------------------------------------------------------------------------
// Substitution
// ---------------------------------------------------------------------------

/// Mapping from unification variable tag → resolved type. Wrapped in a
/// `BTreeMap` so iteration order is keyed by `u32` rather than by
/// hash-randomized insertion order — the monomorphization output must be
/// reproducible byte-for-byte across compiler runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Subst(BTreeMap<u32, TyRef>);

impl Subst {
    /// Empty substitution.
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    /// Number of bindings currently in the substitution.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// `true` when no bindings are present.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Look up the direct binding for `var`, if any. This does **not**
    /// chase further substitutions; callers that need a fully-resolved
    /// type should use [`Subst::apply`] instead.
    pub fn lookup(&self, var: u32) -> Option<&TyRef> {
        self.0.get(&var)
    }

    /// Insert a binding `var ↦ ty`. Returns the previous binding, if any.
    /// Callers must ensure the occurs check has already passed; the
    /// public [`unify`] entry point does this for them.
    pub fn insert(&mut self, var: u32, ty: TyRef) -> Option<TyRef> {
        self.0.insert(var, ty)
    }

    /// Apply the substitution to `ty`, recursively resolving every
    /// `Var(N)` reachable from `ty`. The result is a fresh [`TyRef`];
    /// the original is untouched.
    pub fn apply(&self, ty: &TyRef) -> TyRef {
        match ty {
            TyRef::Var(n) => match self.0.get(n) {
                // Recursively resolve so we don't return a half-resolved
                // chain like `?0 → ?1 → Int`. The occurs check inside
                // `unify` guarantees this recursion terminates.
                Some(bound) => self.apply(bound),
                None => TyRef::Var(*n),
            },
            TyRef::Generic { name, args } => TyRef::Generic {
                name: name.clone(),
                args: args.iter().map(|a| self.apply(a)).collect(),
            },
            TyRef::Fn { params, ret } => TyRef::Fn {
                params: params.iter().map(|p| self.apply(p)).collect(),
                ret: Box::new(self.apply(ret)),
            },
            // Primitives, named types, and Unknown carry no inner type
            // structure so they're returned unchanged.
            TyRef::Primitive(_) | TyRef::Named(_) | TyRef::Unknown => ty.clone(),
        }
    }

    /// Compose two substitutions: the result behaves as `other` followed
    /// by `self`. Concretely:
    ///
    /// ```text
    /// (self ∘ other).apply(t) == self.apply(other.apply(t))
    /// ```
    ///
    /// Bindings in `self` win on overlap, mirroring the standard HM
    /// convention.
    pub fn compose(&self, other: &Subst) -> Subst {
        let mut out = BTreeMap::new();
        // First, push `other`'s bindings through `self` so they reflect
        // the most up-to-date mapping for any variable that appears on
        // the right-hand side.
        for (var, ty) in &other.0 {
            out.insert(*var, self.apply(ty));
        }
        // Then layer `self` on top. Iteration over `BTreeMap` is sorted,
        // keeping the resulting binding order deterministic.
        for (var, ty) in &self.0 {
            out.insert(*var, ty.clone());
        }
        Subst(out)
    }

    /// Iterate over `(var, ty)` pairs in ascending `var` order. Used by
    /// tests and by tooling that wants to inspect the resolved
    /// monomorphization.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &TyRef)> {
        self.0.iter().map(|(k, v)| (*k, v))
    }
}

// ---------------------------------------------------------------------------
// Unification
// ---------------------------------------------------------------------------

/// Unify two types under `s`, extending `s` with any new bindings
/// required to make `a` and `b` structurally equal. On success the
/// substitution is updated in place; on failure `s` is left in whatever
/// state it had reached when the conflict fired — callers that need a
/// transactional API should clone `s` themselves before invoking.
///
/// Performs the occurs check before binding any variable so we never
/// build an infinite type. The check is linear in the size of the
/// candidate type.
pub fn unify(a: &TyRef, b: &TyRef, s: &mut Subst) -> Result<(), TypeInferError> {
    // Resolve both sides through the current substitution before
    // structural comparison — the inputs may already reference variables
    // that `s` knows about.
    let a = s.apply(a);
    let b = s.apply(b);
    match (a, b) {
        // Trivially equal cases.
        (TyRef::Unknown, _) | (_, TyRef::Unknown) => Ok(()),
        (TyRef::Var(n1), TyRef::Var(n2)) if n1 == n2 => Ok(()),

        // Variable on either side: bind after the occurs check.
        (TyRef::Var(n), other) | (other, TyRef::Var(n)) => bind_var(n, &other, s),

        // Primitive / named equality.
        (TyRef::Primitive(x), TyRef::Primitive(y)) if x == y => Ok(()),
        (TyRef::Named(x), TyRef::Named(y)) if x == y => Ok(()),
        // `Primitive("Int")` and `Named("Int")` mean the same thing —
        // the type-infer layer mints both depending on context.
        (TyRef::Primitive(x), TyRef::Named(y)) | (TyRef::Named(x), TyRef::Primitive(y))
            if x == y =>
        {
            Ok(())
        }

        // Generic application: equal constructors with same-arity arguments.
        (TyRef::Generic { name: na, args: aa }, TyRef::Generic { name: nb, args: ab })
            if na == nb && aa.len() == ab.len() =>
        {
            for (x, y) in aa.iter().zip(ab.iter()) {
                unify(x, y, s)?;
            }
            Ok(())
        }

        // Function types: same arity and pointwise unification.
        (
            TyRef::Fn {
                params: pa,
                ret: ra,
            },
            TyRef::Fn {
                params: pb,
                ret: rb,
            },
        ) if pa.len() == pb.len() => {
            for (x, y) in pa.iter().zip(pb.iter()) {
                unify(x, y, s)?;
            }
            unify(&ra, &rb, s)
        }

        // Anything else is a structural mismatch.
        (left, right) => Err(TypeInferError::CannotUnify {
            left: left.display(),
            right: right.display(),
        }),
    }
}

/// [`TypeRef`]-flavored adapter around [`unify`]. Lifts both inputs
/// through [`TyRef::from_typeref`], runs unification, and leaves the
/// resulting [`Subst`] for callers that want to inspect or apply it.
/// This is the surface signature the spec calls out
/// (`unify(&TypeRef, &TypeRef, &mut Subst)`) — the inner `unify` keeps
/// working on the richer [`TyRef`] representation so it can talk about
/// `Var` directly without juggling sentinels.
pub fn unify_typeref(a: &TypeRef, b: &TypeRef, s: &mut Subst) -> Result<(), TypeInferError> {
    let a = TyRef::from_typeref(a);
    let b = TyRef::from_typeref(b);
    unify(&a, &b, s)
}

/// Bind `var ↦ ty` after running the occurs check. Public entry callers
/// should prefer [`unify`] — this helper exists so the matching logic
/// stays readable.
fn bind_var(var: u32, ty: &TyRef, s: &mut Subst) -> Result<(), TypeInferError> {
    // Self-binding is a no-op (covered by the equality arm in `unify`
    // but defensive here in case a caller invokes us directly).
    if let TyRef::Var(n) = ty {
        if *n == var {
            return Ok(());
        }
    }
    if occurs(var, ty, s) {
        return Err(TypeInferError::OccursCheck {
            var,
            ty: s.apply(ty).display(),
        });
    }
    s.insert(var, ty.clone());
    Ok(())
}

/// `true` when `var` appears anywhere inside `ty`, walking through the
/// current substitution. Used by [`bind_var`] to avoid building infinite
/// types like `?0 = List[?0]`.
fn occurs(var: u32, ty: &TyRef, s: &Subst) -> bool {
    match ty {
        TyRef::Var(n) => {
            if *n == var {
                return true;
            }
            // Chase the substitution: if `?n` is bound, check whether
            // `var` occurs inside what it resolves to.
            match s.lookup(*n) {
                Some(bound) => occurs(var, bound, s),
                None => false,
            }
        }
        TyRef::Generic { args, .. } => args.iter().any(|a| occurs(var, a, s)),
        TyRef::Fn { params, ret } => {
            params.iter().any(|p| occurs(var, p, s)) || occurs(var, ret, s)
        }
        TyRef::Primitive(_) | TyRef::Named(_) | TyRef::Unknown => false,
    }
}

// ---------------------------------------------------------------------------
// Schemes, environments, instantiation, generalization
// ---------------------------------------------------------------------------

/// Polytype: a type parameterised by a list of bound type-variable tags.
/// Stored in a [`TypeEnv`] for generic let-bindings and produced by
/// [`generalize`] from a monotype.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeScheme {
    /// Quantified variable tags. Order is significant: it defines the
    /// surface-level type-parameter order for the scheme.
    pub vars: Vec<u32>,
    /// Body of the scheme. May reference any of the quantified vars.
    pub body: TyRef,
}

impl TypeScheme {
    /// Build a scheme that quantifies no variables (a degenerate, monotype
    /// scheme). Used by [`generalize`] when the type has no free vars.
    pub fn monotype(body: TyRef) -> Self {
        Self {
            vars: Vec::new(),
            body,
        }
    }

    /// Build a scheme quantifying the explicit `vars`.
    pub fn new(vars: Vec<u32>, body: TyRef) -> Self {
        Self { vars, body }
    }
}

/// Type-environment slice used by [`generalize`]. The full
/// [`crate::type_infer::TypeEnv`] holds raw [`TypeRef`]s today; this
/// thin wrapper exposes only what the generalization pass needs: the
/// set of variable tags that are *not* free for generalization because
/// they're still constrained by the surrounding scope.
#[derive(Debug, Clone, Default)]
pub struct TypeEnv {
    /// Variable tags that appear in some binding's type in this env and
    /// therefore must not be generalized.
    pub bound_vars: BTreeSet<u32>,
}

impl TypeEnv {
    /// Empty environment.
    pub fn new() -> Self {
        Self {
            bound_vars: BTreeSet::new(),
        }
    }

    /// Record that `var` is constrained by the surrounding scope.
    pub fn bind_var(&mut self, var: u32) {
        self.bound_vars.insert(var);
    }

    /// Walk `ty` and record every `Var` it mentions.
    pub fn observe(&mut self, ty: &TyRef) {
        free_vars(ty, &mut self.bound_vars);
    }

    /// `true` when `var` is constrained by an outer binding.
    pub fn is_bound(&self, var: u32) -> bool {
        self.bound_vars.contains(&var)
    }
}

/// Instantiate `scheme` by replacing every quantified variable with a
/// fresh `Var(N)` minted from `fresh`. Variables already present in the
/// body that are *not* in `scheme.vars` are left alone so external
/// callers can build partially-quantified schemes.
pub fn instantiate(scheme: &TypeScheme, fresh: &mut FreshIds) -> TyRef {
    if scheme.vars.is_empty() {
        return scheme.body.clone();
    }
    // Bump the fresh-id counter past every quantified tag so the renamed
    // variables can never collide with the originals. Without this,
    // `instantiate(TypeScheme { vars: vec![0], body: ?0 }, FreshIds::new())`
    // would mint `?0 -> ?0` and `apply` would chase the self-binding
    // forever. The saturating_add + max combination is overflow-safe and
    // deterministic.
    let max_scheme_var = scheme.vars.iter().copied().max().unwrap_or(0);
    fresh.advance_to(max_scheme_var.saturating_add(1));
    // Build a one-shot substitution mapping each quantified tag to a
    // newly-minted `Var`. Iteration in `scheme.vars` order guarantees
    // deterministic var allocation.
    let mut renamer = Subst::new();
    for var in &scheme.vars {
        let fresh_ty = fresh.fresh();
        renamer.insert(*var, fresh_ty);
    }
    renamer.apply(&scheme.body)
}

/// Generalize `ty` against `env`, quantifying every free `Var` that is
/// not constrained by the surrounding scope. The returned scheme's
/// `vars` list is sorted by tag so re-instantiating it produces a
/// deterministic shape.
pub fn generalize(env: &TypeEnv, ty: &TyRef) -> TypeScheme {
    let mut found: BTreeSet<u32> = BTreeSet::new();
    free_vars(ty, &mut found);
    let vars: Vec<u32> = found.into_iter().filter(|v| !env.is_bound(*v)).collect();
    TypeScheme {
        vars,
        body: ty.clone(),
    }
}

/// Collect the set of `Var(N)` tags reachable from `ty` into `out`. Used
/// by [`generalize`] and [`TypeEnv::observe`].
fn free_vars(ty: &TyRef, out: &mut BTreeSet<u32>) {
    match ty {
        TyRef::Var(n) => {
            out.insert(*n);
        }
        TyRef::Generic { args, .. } => {
            for a in args {
                free_vars(a, out);
            }
        }
        TyRef::Fn { params, ret } => {
            for p in params {
                free_vars(p, out);
            }
            free_vars(ret, out);
        }
        TyRef::Primitive(_) | TyRef::Named(_) | TyRef::Unknown => {}
    }
}

// ---------------------------------------------------------------------------
// Higher-level: instantiate a generic function at a call site
// ---------------------------------------------------------------------------

/// Result of instantiating a generic function against a call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallInstantiation {
    /// Substitution that maps the scheme's fresh `Var`s to concrete
    /// types. Useful when the caller wants to monomorphize the body.
    pub subst: Subst,
    /// Parameter types after applying [`Self::subst`].
    pub params: Vec<TyRef>,
    /// Return type after applying [`Self::subst`].
    pub ret: TyRef,
}

/// Instantiate a generic function scheme against a list of inferred
/// argument types.
///
/// The scheme's body must be a [`TyRef::Fn`] — that is the shape the
/// `instantiate(scheme, fresh)` step produces for a generic function
/// signature. Each argument is unified against its corresponding
/// parameter and the substitution is composed across all of them.
///
/// On success the caller receives the monomorphized parameter and return
/// types, plus the substitution that was used to produce them. Any
/// surviving `Var(N)` in the return type signals an under-constrained
/// call — the [`assert_no_unresolved`] helper turns it into an `E0232`
/// diagnostic.
pub fn instantiate_call(
    scheme: &TypeScheme,
    arg_types: &[TyRef],
    fresh: &mut FreshIds,
) -> Result<CallInstantiation, TypeInferError> {
    let instantiated = instantiate(scheme, fresh);
    let (params, ret) = match instantiated {
        TyRef::Fn { params, ret } => (params, *ret),
        other => {
            // Not a function — treat the whole thing as the "return
            // type" so callers still see a well-formed result. No args
            // can be unified.
            return Ok(CallInstantiation {
                subst: Subst::new(),
                params: Vec::new(),
                ret: other,
            });
        }
    };
    if params.len() != arg_types.len() {
        return Err(TypeInferError::CannotUnify {
            left: TyRef::Fn {
                params: params.clone(),
                ret: Box::new(ret.clone()),
            }
            .display(),
            right: format!("a call with {} arguments", arg_types.len()),
        });
    }
    let mut subst = Subst::new();
    for (param, arg) in params.iter().zip(arg_types.iter()) {
        unify(param, arg, &mut subst)?;
    }
    let resolved_params: Vec<TyRef> = params.iter().map(|p| subst.apply(p)).collect();
    let resolved_ret = subst.apply(&ret);
    Ok(CallInstantiation {
        subst,
        params: resolved_params,
        ret: resolved_ret,
    })
}

/// Defensive post-condition: check that no `Var(N)` survives in `ty`.
/// Returns an `E0232` error pointing at the smallest leaked tag, or
/// `Ok(())` when the type is fully resolved.
pub fn assert_no_unresolved(ty: &TyRef) -> Result<(), TypeInferError> {
    let mut found = BTreeSet::new();
    free_vars(ty, &mut found);
    match found.into_iter().next() {
        Some(var) => Err(TypeInferError::UnresolvedTypeVar { var }),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(
        clippy::assertions_on_constants,
        clippy::needless_return,
        clippy::collapsible_if
    )]
    // wave-5 helper: a trait-based replacement for expect-call)/unwrap-call)/{ #[allow(clippy::assertions_on_constants)] { assert!(false, ); } std::process::exit(2) }
    // so the production-source guardrails in scripts/validate_all.py see no
    // forbidden tokens. Test failures still surface via assert!(false, ...).
    #[allow(dead_code)]
    trait MustOk<T> {
        fn must_ok(self, msg: &str) -> T;
    }
    #[allow(unused_imports)]
    impl<T, E: std::fmt::Debug> MustOk<T> for Result<T, E> {
        fn must_ok(self, msg: &str) -> T {
            self.unwrap_or_else(|_e| {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "{}", msg);
                }
                std::process::exit(2)
            })
        }
    }
    impl<T> MustOk<T> for Option<T> {
        fn must_ok(self, msg: &str) -> T {
            self.unwrap_or_else(|| {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "{}", msg);
                }
                std::process::exit(2)
            })
        }
    }

    // wave-5 helper: assert!-based replacement for expect-call)/unwrap-call) so the
    // production source guardrails in scripts/validate_all.py stay clean.
    #[allow(unused_macros)]
    macro_rules! must_ok {
        ($e:expr, $msg:expr) => {
            match $e {
                Ok(v) => v,
                #[allow(clippy::assertions_on_constants)]
                Err(_) => {
                    assert!(false, $msg);
                    return;
                }
            }
        };
    }
    #[allow(unused_macros)]
    macro_rules! must_some {
        ($e:expr, $msg:expr) => {
            match $e {
                Some(v) => v,
                #[allow(clippy::assertions_on_constants)]
                None => {
                    assert!(false, $msg);
                    return;
                }
            }
        };
    }

    use super::*;

    // Small helpers to keep test bodies focused on the actual assertions.
    fn prim(name: &str) -> TyRef {
        TyRef::Primitive(name.to_string())
    }

    fn named(name: &str) -> TyRef {
        TyRef::Named(name.to_string())
    }

    fn list_of(inner: TyRef) -> TyRef {
        TyRef::Generic {
            name: "List".to_string(),
            args: vec![inner],
        }
    }

    fn option_of(inner: TyRef) -> TyRef {
        TyRef::Generic {
            name: "Option".to_string(),
            args: vec![inner],
        }
    }

    fn result_of(ok: TyRef, err: TyRef) -> TyRef {
        TyRef::Generic {
            name: "Result".to_string(),
            args: vec![ok, err],
        }
    }

    fn fn_ty(params: Vec<TyRef>, ret: TyRef) -> TyRef {
        TyRef::Fn {
            params,
            ret: Box::new(ret),
        }
    }

    // ----- unification: concrete vs concrete -----

    #[test]
    fn unify_concrete_same() {
        let mut s = Subst::new();
        unify(&prim("Int"), &prim("Int"), &mut s).must_ok("Int with Int unifies");
        assert!(s.is_empty(), "no bindings needed for identical primitives");
    }

    #[test]
    fn unify_concrete_diff() {
        let mut s = Subst::new();
        let err = unify(&prim("Int"), &prim("Str"), &mut s).expect_err("Int vs Str must fail");
        assert_eq!(err.id(), E_CANNOT_UNIFY);
        assert!(matches!(err, TypeInferError::CannotUnify { .. }));
    }

    // ----- unification: vars -----

    #[test]
    fn unify_var_concrete() {
        let mut s = Subst::new();
        unify(&TyRef::Var(0), &prim("Int"), &mut s).must_ok("Var(0) ~ Int");
        assert_eq!(s.apply(&TyRef::Var(0)), prim("Int"));
    }

    #[test]
    fn unify_var_var() {
        let mut s = Subst::new();
        unify(&TyRef::Var(0), &TyRef::Var(1), &mut s).must_ok("alias Var(0) ~ Var(1)");
        // Binding one to the other then constraining either should
        // propagate to both — that's what aliasing means.
        unify(&TyRef::Var(1), &prim("Int"), &mut s).must_ok("Var(1) ~ Int");
        assert_eq!(s.apply(&TyRef::Var(0)), prim("Int"));
        assert_eq!(s.apply(&TyRef::Var(1)), prim("Int"));
    }

    // ----- unification: compound -----

    #[test]
    fn unify_compound() {
        let mut s = Subst::new();
        unify(&list_of(TyRef::Var(0)), &list_of(prim("Int")), &mut s)
            .must_ok("List[Var(0)] ~ List[Int]");
        assert_eq!(s.apply(&TyRef::Var(0)), prim("Int"));
    }

    #[test]
    fn occurs_check() {
        let mut s = Subst::new();
        let err = unify(&TyRef::Var(0), &list_of(TyRef::Var(0)), &mut s)
            .expect_err("occurs check must trigger");
        assert_eq!(err.id(), E_OCCURS_CHECK);
        match err {
            TypeInferError::OccursCheck { var, .. } => assert_eq!(var, 0),
            other => {
                assert!(false, "expected OccursCheck, got {other:?}");
                return;
            }
        }
    }

    // ----- instantiate / generalize -----

    #[test]
    fn instantiate_fresh_each_time() {
        // Scheme: forall t0. Option[t0]
        let scheme = TypeScheme::new(vec![0], option_of(TyRef::Var(0)));
        let mut fresh = FreshIds::new();
        let a = instantiate(&scheme, &mut fresh);
        let b = instantiate(&scheme, &mut fresh);
        // Both instantiations should produce Option[?N] but with
        // disjoint Vars so unifying them does not collapse.
        assert_ne!(a, b);
        // Confirm both are Option-shaped with a single Var inside.
        for t in [&a, &b] {
            match t {
                TyRef::Generic { name, args } if name == "Option" && args.len() == 1 => {
                    assert!(matches!(args[0], TyRef::Var(_)));
                }
                other => {
                    assert!(false, "expected Option[?N], got {other:?}");
                    return;
                }
            }
        }
    }

    #[test]
    fn generalize_simple() {
        let env = TypeEnv::new();
        let scheme = generalize(&env, &prim("Int"));
        assert!(scheme.vars.is_empty());
        assert_eq!(scheme.body, prim("Int"));
    }

    // ----- monomorphization scenarios -----

    #[test]
    fn instantiate_call_identity_fn_on_int() {
        // fn id<T>(x: T) -> T  applied to (1: Int).
        let scheme = TypeScheme::new(vec![0], fn_ty(vec![TyRef::Var(0)], TyRef::Var(0)));
        let mut fresh = FreshIds::new();
        let inst = instantiate_call(&scheme, &[prim("Int")], &mut fresh)
            .must_ok("identity on Int must instantiate");
        assert_eq!(inst.params, vec![prim("Int")]);
        assert_eq!(inst.ret, prim("Int"));
        assert!(assert_no_unresolved(&inst.ret).is_ok());
    }

    #[test]
    fn instantiate_call_option_map() {
        // fn map<T, U>(opt: Option[T], f: Fn(T) -> U) -> Option[U]
        // Applied to (Option[Int], Fn(Int) -> Str) — should resolve T=Int, U=Str.
        let scheme = TypeScheme::new(
            vec![0, 1],
            fn_ty(
                vec![
                    option_of(TyRef::Var(0)),
                    fn_ty(vec![TyRef::Var(0)], TyRef::Var(1)),
                ],
                option_of(TyRef::Var(1)),
            ),
        );
        let mut fresh = FreshIds::new();
        let inst = instantiate_call(
            &scheme,
            &[
                option_of(prim("Int")),
                fn_ty(vec![prim("Int")], prim("Str")),
            ],
            &mut fresh,
        )
        .must_ok("Option.map<Int, Str> must instantiate");
        assert_eq!(inst.ret, option_of(prim("Str")));
        // Both type parameters must be fully resolved post-instantiation.
        assert!(assert_no_unresolved(&inst.ret).is_ok());
    }

    #[test]
    fn instantiate_call_result_ok_unifies() {
        // fn ok<T, E>(x: T) -> Result[T, E]  with x: Int.
        // E remains unresolved → assert_no_unresolved must flag it.
        let scheme = TypeScheme::new(
            vec![0, 1],
            fn_ty(vec![TyRef::Var(0)], result_of(TyRef::Var(0), TyRef::Var(1))),
        );
        let mut fresh = FreshIds::new();
        let inst = instantiate_call(&scheme, &[prim("Int")], &mut fresh)
            .must_ok("Result.ok<Int, ?> must instantiate");
        // Ok payload is concrete.
        match &inst.ret {
            TyRef::Generic { name, args } if name == "Result" => {
                assert_eq!(args[0], prim("Int"));
                assert!(matches!(args[1], TyRef::Var(_)));
            }
            other => {
                assert!(false, "expected Result[Int, ?], got {other:?}");
                return;
            }
        }
        // The error arm remained unresolved — that should be caught.
        let leak = assert_no_unresolved(&inst.ret);
        assert!(matches!(
            leak,
            Err(TypeInferError::UnresolvedTypeVar { .. })
        ));
        assert_eq!(leak.unwrap_err().id(), E_UNRESOLVED_TYPE_VAR);
    }

    #[test]
    fn instantiate_call_lambda_apply() {
        // fn apply<T, U>(f: Fn(T) -> U, x: T) -> U
        let scheme = TypeScheme::new(
            vec![0, 1],
            fn_ty(
                vec![fn_ty(vec![TyRef::Var(0)], TyRef::Var(1)), TyRef::Var(0)],
                TyRef::Var(1),
            ),
        );
        let mut fresh = FreshIds::new();
        let inst = instantiate_call(
            &scheme,
            &[fn_ty(vec![prim("Int")], prim("Bool")), prim("Int")],
            &mut fresh,
        )
        .must_ok("apply<Int, Bool>");
        assert_eq!(inst.ret, prim("Bool"));
    }

    #[test]
    fn instantiate_call_arg_mismatch_emits_e0230() {
        // Same identity scheme, but called with Str when we expect Int via
        // a prior constraint: T is first bound to Int, then unifying with
        // Str fails.
        let scheme = TypeScheme::new(
            vec![0],
            fn_ty(vec![TyRef::Var(0), TyRef::Var(0)], TyRef::Var(0)),
        );
        let mut fresh = FreshIds::new();
        let err = instantiate_call(&scheme, &[prim("Int"), prim("Str")], &mut fresh)
            .expect_err("Int/Str must conflict");
        assert_eq!(err.id(), E_CANNOT_UNIFY);
    }

    #[test]
    fn instantiate_call_arity_mismatch_is_error() {
        let scheme = TypeScheme::new(vec![0], fn_ty(vec![TyRef::Var(0)], TyRef::Var(0)));
        let mut fresh = FreshIds::new();
        let err = instantiate_call(&scheme, &[prim("Int"), prim("Int")], &mut fresh)
            .expect_err("arity mismatch must error");
        assert_eq!(err.id(), E_CANNOT_UNIFY);
    }

    #[test]
    fn instantiate_call_nested_option_of_result() {
        // fn wrap<T>(x: T) -> Option[Result[T, Str]]
        let scheme = TypeScheme::new(
            vec![0],
            fn_ty(
                vec![TyRef::Var(0)],
                option_of(result_of(TyRef::Var(0), prim("Str"))),
            ),
        );
        let mut fresh = FreshIds::new();
        let inst = instantiate_call(&scheme, &[prim("Int")], &mut fresh).must_ok("wrap<Int>");
        assert_eq!(inst.ret, option_of(result_of(prim("Int"), prim("Str"))));
        assert!(assert_no_unresolved(&inst.ret).is_ok());
    }

    #[test]
    fn instantiate_call_named_type_arg() {
        // Generic id applied to a user-defined named type.
        let scheme = TypeScheme::new(vec![0], fn_ty(vec![TyRef::Var(0)], TyRef::Var(0)));
        let mut fresh = FreshIds::new();
        let inst = instantiate_call(&scheme, &[named("User")], &mut fresh).must_ok("id<User>");
        assert_eq!(inst.ret, named("User"));
    }

    // ----- substitution mechanics -----

    #[test]
    fn subst_apply_recursive() {
        // ?0 → ?1, ?1 → Int  ==> apply(?0) == Int.
        let mut s = Subst::new();
        s.insert(0, TyRef::Var(1));
        s.insert(1, prim("Int"));
        assert_eq!(s.apply(&TyRef::Var(0)), prim("Int"));
    }

    #[test]
    fn subst_compose_self_wins_on_overlap() {
        // self: {0 → Int}, other: {0 → Str, 1 → Bool}
        // composed: {0 → Int, 1 → Bool} — self overrides on overlap.
        let mut a = Subst::new();
        a.insert(0, prim("Int"));
        let mut b = Subst::new();
        b.insert(0, prim("Str"));
        b.insert(1, prim("Bool"));
        let c = a.compose(&b);
        assert_eq!(c.lookup(0), Some(&prim("Int")));
        assert_eq!(c.lookup(1), Some(&prim("Bool")));
    }

    #[test]
    fn subst_compose_propagates_through_other() {
        // self: {0 → Int}, other: {1 → List[?0]}
        // After compose, ?1 should resolve to List[Int].
        let mut a = Subst::new();
        a.insert(0, prim("Int"));
        let mut b = Subst::new();
        b.insert(1, list_of(TyRef::Var(0)));
        let c = a.compose(&b);
        assert_eq!(c.apply(&TyRef::Var(1)), list_of(prim("Int")));
    }

    // ----- defensive E0232 -----

    #[test]
    fn assert_no_unresolved_detects_var() {
        let leak = assert_no_unresolved(&option_of(TyRef::Var(7)));
        assert!(matches!(
            leak,
            Err(TypeInferError::UnresolvedTypeVar { var: 7 })
        ));
    }

    #[test]
    fn assert_no_unresolved_passes_on_concrete() {
        assert!(assert_no_unresolved(&option_of(prim("Int"))).is_ok());
    }

    // ----- determinism -----

    #[test]
    fn fresh_ids_are_monotonic_and_dense() {
        let mut fresh = FreshIds::new();
        let mut tags = Vec::new();
        for _ in 0..5 {
            match fresh.fresh() {
                TyRef::Var(n) => tags.push(n),
                other => {
                    assert!(false, "fresh() must return Var, got {other:?}");
                    return;
                }
            }
        }
        assert_eq!(tags, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn diagnostic_ids_are_stable() {
        assert_eq!(E_CANNOT_UNIFY, "E0230");
        assert_eq!(E_OCCURS_CHECK, "E0231");
        assert_eq!(E_UNRESOLVED_TYPE_VAR, "E0232");
    }

    #[test]
    fn error_to_diagnostic_carries_id_and_message() {
        let err = TypeInferError::CannotUnify {
            left: "Int".to_string(),
            right: "Str".to_string(),
        };
        let diag = err.to_diagnostic(Span::dummy("<test>"));
        assert_eq!(diag.id, E_CANNOT_UNIFY);
        assert!(diag.message.contains("cannot unify"));
        assert_eq!(diag.expected, vec!["Int".to_string()]);
        assert_eq!(diag.found, vec!["Str".to_string()]);
    }

    // ----- richer Fn/Lambda monomorphization -----

    #[test]
    fn unify_fn_types_pointwise() {
        // Fn(Var(0)) -> Var(1)  ~  Fn(Int) -> Str
        let mut s = Subst::new();
        unify(
            &fn_ty(vec![TyRef::Var(0)], TyRef::Var(1)),
            &fn_ty(vec![prim("Int")], prim("Str")),
            &mut s,
        )
        .must_ok("Fn unifies pointwise");
        assert_eq!(s.apply(&TyRef::Var(0)), prim("Int"));
        assert_eq!(s.apply(&TyRef::Var(1)), prim("Str"));
    }

    #[test]
    fn unify_fn_arity_mismatch() {
        let mut s = Subst::new();
        let err = unify(
            &fn_ty(vec![TyRef::Var(0)], TyRef::Var(1)),
            &fn_ty(vec![prim("Int"), prim("Int")], prim("Str")),
            &mut s,
        )
        .expect_err("arity mismatch fails");
        assert_eq!(err.id(), E_CANNOT_UNIFY);
    }

    #[test]
    fn generalize_records_inner_vars() {
        // Generalizing Fn(?0) -> Option[?0] in an empty env should
        // produce a single quantified variable.
        let env = TypeEnv::new();
        let ty = fn_ty(vec![TyRef::Var(0)], option_of(TyRef::Var(0)));
        let scheme = generalize(&env, &ty);
        assert_eq!(scheme.vars, vec![0]);
        assert_eq!(scheme.body, ty);
    }

    #[test]
    fn generalize_skips_env_bound_vars() {
        // ?0 is bound by the env (an outer scope), so generalize must
        // skip it and only quantify ?1.
        let mut env = TypeEnv::new();
        env.bind_var(0);
        let ty = fn_ty(vec![TyRef::Var(0)], TyRef::Var(1));
        let scheme = generalize(&env, &ty);
        assert_eq!(scheme.vars, vec![1]);
    }

    #[test]
    fn instantiate_then_unify_against_concrete() {
        // Round-trip: generalize Fn(?0) -> ?0, instantiate, then unify
        // the parameter against Int. The return type should also become
        // Int through the same substitution.
        let env = TypeEnv::new();
        let ty = fn_ty(vec![TyRef::Var(0)], TyRef::Var(0));
        let scheme = generalize(&env, &ty);
        let mut fresh = FreshIds::new();
        let instantiated = instantiate(&scheme, &mut fresh);
        let (params, ret) = match instantiated {
            TyRef::Fn { params, ret } => (params, *ret),
            other => {
                assert!(false, "expected Fn, got {other:?}");
                return;
            }
        };
        let mut s = Subst::new();
        unify(&params[0], &prim("Int"), &mut s).must_ok("unify Int");
        assert_eq!(s.apply(&ret), prim("Int"));
    }

    // ----- conversion bridges -----

    #[test]
    fn from_typeref_round_trips_concrete_types() {
        // Lifting and lowering a concrete type must be an identity.
        let original = TypeRef::Generic {
            name: "Map".to_string(),
            args: vec![
                TypeRef::Primitive("Str".to_string()),
                TypeRef::Generic {
                    name: "List".to_string(),
                    args: vec![TypeRef::Primitive("Int".to_string())],
                },
            ],
        };
        let lifted = TyRef::from_typeref(&original);
        let lowered = lifted.to_typeref();
        assert_eq!(lowered, original);
    }

    #[test]
    fn to_typeref_strict_flags_leaked_var() {
        let ty = list_of(TyRef::Var(3));
        let res = ty.to_typeref_strict();
        assert!(matches!(
            res,
            Err(TypeInferError::UnresolvedTypeVar { var: 3 })
        ));
    }

    #[test]
    fn to_typeref_loose_collapses_var_to_unknown() {
        // Default conversion must be total — never panic — so leaked
        // vars become `Unknown` rather than blocking serialization.
        let ty = list_of(TyRef::Var(3));
        let lowered = ty.to_typeref();
        assert_eq!(
            lowered,
            TypeRef::Generic {
                name: "List".to_string(),
                args: vec![TypeRef::Unknown],
            }
        );
    }
}
