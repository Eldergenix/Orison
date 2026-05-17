//! Region/lifetime tracking scaffold for the body-level borrow checker.
//!
//! The bootstrap borrow checker treats every `Expr::Block` as a fresh
//! region. Inside that region:
//!
//!   * each `let name = init` binding introduces a region named after the
//!     binding (`name`),
//!   * each borrow expression records a dependency on the region of the
//!     borrowed identifier,
//!   * mutable and shared borrows are tracked separately so the checker
//!     can flag a `&mut x` taken while a `&x` is still live,
//!   * regions are popped automatically when the block exits, so a borrow
//!     of a block-local binding cannot escape via the block's tail
//!     expression or via a `return`.
//!
//! The data model is deliberately minimal: borrows are flat records keyed
//! by their stable region index (a `usize` assigned in source order). The
//! checker in [`crate::borrow`] is the single consumer of this module; we
//! keep the helper here so that the borrow checker file stays focused on
//! diagnostic construction.
//!
//! All collections are [`BTreeMap`] / [`BTreeSet`] so iteration order is
//! deterministic across runs — the same input always yields the same
//! sequence of diagnostics.

use crate::source::Span;
use std::collections::{BTreeMap, BTreeSet};

/// Kind of borrow recorded against a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowMode {
    /// `&x`: shared, immutable, may coexist with other shared borrows.
    Shared,
    /// `&mut x`: exclusive, may not coexist with any other borrow.
    Mut,
}

/// One live borrow of an identifier inside the current region stack.
#[derive(Debug, Clone)]
pub struct LiveBorrow {
    /// Name of the identifier being borrowed.
    pub target: String,
    /// Borrow mode (shared or mut).
    pub mode: BorrowMode,
    /// Span where the borrow was taken.
    pub span: Span,
    /// Region index that owns this borrow (the block depth + insertion
    /// order). Borrows are dropped when their owning region is popped.
    pub region: usize,
}

/// Region/lifetime tracker. Each pushed region records the identifiers
/// declared inside it and the borrows that depend on it; popping the
/// region drops all of them in source order.
#[derive(Debug, Default)]
pub struct RegionMap {
    /// Next region index to hand out. Monotonic for determinism.
    next_id: usize,
    /// Stack of region indices currently in scope, deepest last.
    stack: Vec<usize>,
    /// region_id -> identifiers declared *by `let`* inside that region.
    locals: BTreeMap<usize, BTreeSet<String>>,
    /// Live borrows, addressable by region. Sorted because the underlying
    /// map keys are integers; we use a `Vec<LiveBorrow>` so that
    /// insertion order is preserved within a single region.
    borrows: BTreeMap<usize, Vec<LiveBorrow>>,
}

impl RegionMap {
    /// Construct an empty tracker. Equivalent to `RegionMap::default()`
    /// but more explicit at call sites.
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a fresh region onto the stack and return its index. The
    /// index is stable for the lifetime of the [`RegionMap`].
    pub fn enter(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.stack.push(id);
        self.locals.entry(id).or_default();
        self.borrows.entry(id).or_default();
        id
    }

    /// Pop the deepest region. Borrows and locals registered against it
    /// are forgotten. Calling `exit` on an empty stack is a no-op so the
    /// tracker is safe to drive from a recursive walker without
    /// branching on stack depth at every site.
    pub fn exit(&mut self) {
        if let Some(id) = self.stack.pop() {
            self.locals.remove(&id);
            self.borrows.remove(&id);
        }
    }

    /// Record a `let` binding in the current (deepest) region. Has no
    /// effect when the stack is empty — the borrow checker treats such
    /// inputs as malformed and emits no body diagnostics for them.
    pub fn declare_local(&mut self, name: &str) {
        if let Some(&region) = self.stack.last() {
            if let Some(set) = self.locals.get_mut(&region) {
                set.insert(name.to_string());
            }
        }
    }

    /// `true` if `name` was introduced by a `let` in *any* currently live
    /// region (i.e. it would die when the function body returns). Used by
    /// the escape check to detect references that outlive their source.
    pub fn is_local(&self, name: &str) -> bool {
        self.stack
            .iter()
            .any(|id| self.locals.get(id).is_some_and(|s| s.contains(name)))
    }

    /// Region index that introduced `name`, if any. Returns the deepest
    /// (most recently entered) match so shadowing behaves as expected.
    pub fn region_of_local(&self, name: &str) -> Option<usize> {
        for id in self.stack.iter().rev() {
            if let Some(set) = self.locals.get(id) {
                if set.contains(name) {
                    return Some(*id);
                }
            }
        }
        None
    }

    /// Add a live borrow to the current region. Returns the region index
    /// the borrow was attached to, or `None` if the stack is empty.
    pub fn add_borrow(&mut self, target: &str, mode: BorrowMode, span: Span) -> Option<usize> {
        let region = *self.stack.last()?;
        let entry = LiveBorrow {
            target: target.to_string(),
            mode,
            span,
            region,
        };
        self.borrows.entry(region).or_default().push(entry);
        Some(region)
    }

    /// Iterate every borrow currently live across the full region stack,
    /// from outermost region to innermost, preserving insertion order
    /// within each region. The order is fully deterministic.
    pub fn live_borrows(&self) -> Vec<&LiveBorrow> {
        let mut out: Vec<&LiveBorrow> = Vec::new();
        for id in &self.stack {
            if let Some(list) = self.borrows.get(id) {
                for b in list {
                    out.push(b);
                }
            }
        }
        out
    }

    /// Live borrows that target identifier `name`, in deterministic
    /// (outer-to-inner, insertion) order.
    pub fn borrows_of(&self, name: &str) -> Vec<&LiveBorrow> {
        self.live_borrows()
            .into_iter()
            .filter(|b| b.target == name)
            .collect()
    }

    /// Current stack depth. Useful for tests and debug logging; the
    /// checker itself does not branch on this value.
    pub fn depth(&self) -> usize {
        self.stack.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> Span {
        Span::dummy("/t.ori")
    }

    #[test]
    fn enter_and_exit_balance() {
        let mut m = RegionMap::new();
        assert_eq!(m.depth(), 0);
        let a = m.enter();
        let b = m.enter();
        assert!(b > a);
        assert_eq!(m.depth(), 2);
        m.exit();
        assert_eq!(m.depth(), 1);
        m.exit();
        assert_eq!(m.depth(), 0);
        // exit on empty is a no-op
        m.exit();
        assert_eq!(m.depth(), 0);
    }

    #[test]
    fn declare_local_and_lookup() {
        let mut m = RegionMap::new();
        m.enter();
        m.declare_local("x");
        assert!(m.is_local("x"));
        assert!(!m.is_local("y"));
        m.exit();
        // After exit the local is gone.
        assert!(!m.is_local("x"));
    }

    #[test]
    fn shadowing_returns_innermost_region() {
        let mut m = RegionMap::new();
        let outer = m.enter();
        m.declare_local("x");
        let inner = m.enter();
        m.declare_local("x");
        assert_eq!(m.region_of_local("x"), Some(inner));
        m.exit();
        assert_eq!(m.region_of_local("x"), Some(outer));
        m.exit();
    }

    #[test]
    fn borrows_are_dropped_on_exit() {
        let mut m = RegionMap::new();
        m.enter();
        m.add_borrow("x", BorrowMode::Shared, s());
        assert_eq!(m.borrows_of("x").len(), 1);
        m.exit();
        assert!(m.borrows_of("x").is_empty());
    }

    #[test]
    fn live_borrows_are_outer_to_inner_then_insertion_order() {
        let mut m = RegionMap::new();
        m.enter();
        m.add_borrow("a", BorrowMode::Shared, s());
        m.enter();
        m.add_borrow("b", BorrowMode::Shared, s());
        m.add_borrow("c", BorrowMode::Mut, s());
        let live = m.live_borrows();
        assert_eq!(live.len(), 3);
        assert_eq!(live[0].target, "a");
        assert_eq!(live[1].target, "b");
        assert_eq!(live[2].target, "c");
        m.exit();
        m.exit();
    }

    #[test]
    fn region_indices_are_monotonic_for_determinism() {
        let mut m = RegionMap::new();
        let a = m.enter();
        m.exit();
        let b = m.enter();
        m.exit();
        assert!(b > a, "region indices must be monotonic");
    }
}
