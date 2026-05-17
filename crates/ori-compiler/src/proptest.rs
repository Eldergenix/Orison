//! In-repo property-based testing micro-framework for Orison.
//!
//! This module is intentionally tiny: it brings deterministic random
//! generation, shrinking, and a `quickcheck` driver to the bootstrap
//! crates without pulling in any third-party dependency. The framework
//! is shaped after Haskell's QuickCheck / Rust's `quickcheck` crate but
//! covers only the features that the in-tree property tests actually
//! need (see `tests/proptest_smoke.rs`).
//!
//! ## Determinism
//!
//! Every run is fully replayable from the `u64` seed passed to
//! [`quickcheck`]. The generator uses a 64-bit splitmix64 stream, which
//! is small, self-contained, and good enough for property-test inputs
//! (it is *not* a cryptographic RNG and must not be used for security
//! decisions). On failure, [`PropOutcome::Counterexample`] reports the
//! original seed so the failing case can be re-derived bit-for-bit.
//!
//! ## Shrinking
//!
//! The shrinker is a greedy single-step search: given a failing input
//! it asks the value for its `shrink()` candidates (smaller variants),
//! tries each in order, and accepts the first one that still fails. It
//! then loops on that smaller value until no shrink keeps the property
//! false. This converges on a locally-minimal counterexample without
//! the search-tree machinery of `quickcheck`/`proptest`; it is
//! deliberately simple so the implementation stays auditable.
//!
//! ## Safety
//!
//! No `unsafe`, no `unwrap`, no `expect`, no `panic!`. The driver
//! catches counterexamples by inspecting `bool` return values and
//! reports them via a `Result`-shaped enum.

use std::fmt::Debug;

// ---------------------------------------------------------------------------
// RNG
// ---------------------------------------------------------------------------

/// A deterministic splitmix64 RNG.
///
/// The algorithm is the public-domain splitmix64 by Sebastiano Vigna
/// (<https://prng.di.unimi.it/splitmix64.c>). It has a 64-bit state, a
/// 64-bit period, and decent statistical properties for property
/// testing. Every method is deterministic in `(state, sequence of
/// calls)`, so reseeding with the same value reproduces the same
/// stream.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Construct a new RNG seeded with `seed`. Two `Rng`s seeded with
    /// the same value produce identical streams.
    pub fn new(seed: u64) -> Self {
        // splitmix64 is undefined for state 0 in some references; we
        // mix the seed once so even a `0` seed yields a usable stream.
        Self {
            state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15),
        }
    }

    /// Advance the stream and return the next 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        // splitmix64.
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Return a value in `[lo, hi)`. When `hi <= lo` the function
    /// returns `lo` rather than dividing by zero — this matches the
    /// "empty range collapses" convention.
    pub fn gen_range(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            return lo;
        }
        let span = hi - lo;
        lo + (self.next_u64() % span)
    }

    /// Return `true` with probability `p`. `p <= 0` always returns
    /// `false`; `p >= 1` always returns `true`. Other values are
    /// rendered by comparing a uniform `u64` against `p * 2^64`.
    pub fn gen_bool(&mut self, p: f64) -> bool {
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

// ---------------------------------------------------------------------------
// Arbitrary
// ---------------------------------------------------------------------------

/// Types that can be randomly generated and shrunk.
///
/// Implementors must keep `arbitrary` deterministic in the RNG state
/// (no environment access, no time, no thread-local randomness) and
/// `shrink` total: a value with no smaller variants returns an empty
/// vector rather than allocating sentinels.
pub trait Arbitrary: Sized + Clone + Debug {
    /// Generate a fresh arbitrary value from the RNG.
    fn arbitrary(rng: &mut Rng) -> Self;

    /// Yield strictly-smaller candidates the shrinker should try, in
    /// the order the shrinker should walk them. An empty vector means
    /// "this value cannot be made smaller".
    fn shrink(&self) -> Vec<Self>;
}

impl Arbitrary for bool {
    fn arbitrary(rng: &mut Rng) -> Self {
        rng.gen_bool(0.5)
    }

    fn shrink(&self) -> Vec<Self> {
        // `true` shrinks toward `false`; `false` is already minimal.
        if *self {
            vec![false]
        } else {
            Vec::new()
        }
    }
}

impl Arbitrary for u64 {
    fn arbitrary(rng: &mut Rng) -> Self {
        // Biased toward small values so generated inputs are usually
        // easy to print and shrink. We sample one of three buckets at
        // uniform weight then sample inside the bucket.
        match rng.gen_range(0, 3) {
            0 => rng.gen_range(0, 16),
            1 => rng.gen_range(0, 1024),
            _ => rng.next_u64(),
        }
    }

    fn shrink(&self) -> Vec<Self> {
        if *self == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        out.push(0);
        if *self > 1 {
            out.push(*self / 2);
        }
        if *self > 0 {
            out.push(*self - 1);
        }
        out
    }
}

impl Arbitrary for i64 {
    fn arbitrary(rng: &mut Rng) -> Self {
        let magnitude = u64::arbitrary(rng) as i64;
        if rng.gen_bool(0.5) {
            magnitude.wrapping_neg()
        } else {
            magnitude
        }
    }

    fn shrink(&self) -> Vec<Self> {
        if *self == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        out.push(0);
        if *self > 0 {
            if *self > 1 {
                out.push(*self / 2);
            }
            out.push(*self - 1);
        } else {
            // `i64::MIN.wrapping_neg() == i64::MIN`, so guard the
            // overflow case before negating.
            if *self != i64::MIN {
                out.push(-*self);
            }
            if *self < -1 {
                out.push(*self / 2);
            }
            if *self < 0 {
                out.push(*self + 1);
            }
        }
        out
    }
}

/// Printable-ASCII character set used for `String` generation. Avoids
/// control characters, quotes, backslashes and braces so generated
/// strings can be embedded in source-like contexts without escaping.
const SAFE_ASCII: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_";

impl Arbitrary for String {
    fn arbitrary(rng: &mut Rng) -> Self {
        let len = rng.gen_range(0, 9) as usize; // short strings (0..8)
        let mut out = String::with_capacity(len);
        for _ in 0..len {
            let idx = rng.gen_range(0, SAFE_ASCII.len() as u64) as usize;
            out.push(SAFE_ASCII[idx] as char);
        }
        out
    }

    fn shrink(&self) -> Vec<Self> {
        let mut out = Vec::new();
        if self.is_empty() {
            return out;
        }
        // Try truncating to half length and to length-1 first; both are
        // strictly smaller than the input.
        let half = self.len() / 2;
        out.push(self.chars().take(half).collect());
        out.push(self.chars().take(self.len() - 1).collect());
        // Try dropping each individual character.
        let chars: Vec<char> = self.chars().collect();
        for i in 0..chars.len() {
            let mut s = String::with_capacity(self.len());
            for (j, c) in chars.iter().enumerate() {
                if i != j {
                    s.push(*c);
                }
            }
            out.push(s);
        }
        out
    }
}

impl<T> Arbitrary for Vec<T>
where
    T: Arbitrary,
{
    fn arbitrary(rng: &mut Rng) -> Self {
        let len = rng.gen_range(0, 9) as usize;
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            out.push(T::arbitrary(rng));
        }
        out
    }

    fn shrink(&self) -> Vec<Self> {
        let mut out: Vec<Vec<T>> = Vec::new();
        if self.is_empty() {
            return out;
        }
        // Halve and drop-one strategies first: the shrinker tries
        // big jumps before fine-grained edits.
        out.push(Vec::new());
        if self.len() > 1 {
            let half = self.len() / 2;
            out.push(self.iter().take(half).cloned().collect());
        }
        // Drop each index in turn.
        for i in 0..self.len() {
            let mut v: Vec<T> = Vec::with_capacity(self.len() - 1);
            for (j, item) in self.iter().enumerate() {
                if i != j {
                    v.push(item.clone());
                }
            }
            out.push(v);
        }
        // Shrink one element at a time, leaving the rest intact.
        for i in 0..self.len() {
            for candidate in self[i].shrink() {
                let mut v: Vec<T> = self.clone();
                v[i] = candidate;
                out.push(v);
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// quickcheck driver
// ---------------------------------------------------------------------------

/// Outcome of [`quickcheck`].
///
/// `Ok` reports how many randomly generated cases held the property
/// (always equal to `runs` on success). `Counterexample` reports the
/// original seed (so the failing trace can be replayed verbatim), the
/// run index that first surfaced the failure, and a debug rendering
/// of the smallest counterexample the shrinker could find.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropOutcome {
    /// Every generated case satisfied the property.
    Ok {
        /// Number of cases that ran (equal to `runs`).
        runs: u32,
    },
    /// At least one case violated the property.
    Counterexample {
        /// Original seed passed to [`quickcheck`].
        seed: u64,
        /// Number of cases evaluated, including the failing one.
        runs: u32,
        /// `Debug` rendering of the locally-minimal counterexample
        /// the shrinker converged on.
        minimal: String,
    },
}

/// Default seed used by every smoke property test in this crate.
///
/// Holding the seed constant means CI runs and local runs explore the
/// same trace by default. Tests that want to widen coverage may pass a
/// different seed (e.g. derived from a CI build number) explicitly.
pub const DEFAULT_SEED: u64 = 0xDEAD_BEEF_CAFE_F00D;

/// Default number of randomly generated cases per property.
pub const DEFAULT_RUNS: u32 = 64;

/// Run the property `prop` up to `runs` times with random inputs of
/// type `R` derived from `seed`.
///
/// Returns [`PropOutcome::Ok`] when every case held. On the first
/// case that returned `false` the driver invokes the shrinker until
/// no candidate keeps the property false, then returns
/// [`PropOutcome::Counterexample`]. The driver never panics: a `prop`
/// closure that would panic on bad input should instead return
/// `false`, which the caller can render as a regular counterexample.
pub fn quickcheck<P, R>(seed: u64, runs: u32, prop: P) -> PropOutcome
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

/// Greedy single-step shrinker: tries every `shrink()` candidate in
/// order, accepts the first one that still falsifies `prop`, and loops
/// on that smaller value until a fixpoint is reached.
///
/// We cap the total number of shrink iterations so that pathological
/// `shrink()` implementations cannot live-lock the test runner. The
/// cap is generous enough that every in-tree property test reaches a
/// true local minimum well before it triggers.
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Two RNGs seeded with the same value produce the same prefix of
    /// outputs. This guards the determinism contract.
    #[test]
    fn rng_is_deterministic_per_seed() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..64 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    /// `gen_range(lo, hi)` always yields a value strictly less than
    /// `hi`. A separate check guarantees `gen_range(x, x)` returns `x`.
    #[test]
    fn gen_range_respects_bounds() {
        let mut rng = Rng::new(1);
        for _ in 0..512 {
            let v = rng.gen_range(10, 20);
            assert!(v >= 10 && v < 20, "out of bounds: {v}");
        }
        assert_eq!(Rng::new(7).gen_range(5, 5), 5);
        assert_eq!(Rng::new(7).gen_range(5, 4), 5);
    }

    /// `gen_bool(0.0)` is always false; `gen_bool(1.0)` is always true.
    #[test]
    fn gen_bool_handles_extremes() {
        let mut rng = Rng::new(99);
        for _ in 0..64 {
            assert!(!rng.gen_bool(0.0));
            assert!(rng.gen_bool(1.0));
        }
    }

    /// Numeric shrinks always move strictly toward zero.
    #[test]
    fn u64_shrinks_toward_zero() {
        for v in [1u64, 2, 7, 1024, u64::MAX] {
            for s in v.shrink() {
                assert!(s < v, "shrink {s} of {v} is not strictly smaller");
            }
        }
        assert!(0u64.shrink().is_empty());
    }

    /// String shrinks never produce a string longer than the original.
    #[test]
    fn string_shrinks_are_not_longer() {
        let original = String::from("hello");
        for s in original.shrink() {
            assert!(s.len() < original.len() || s.is_empty());
        }
        assert!(String::new().shrink().is_empty());
    }

    /// `Vec` shrinks never produce a longer vector than the original.
    #[test]
    fn vec_shrinks_are_not_longer() {
        let original: Vec<u64> = vec![3, 1, 4, 1, 5];
        for v in original.shrink() {
            assert!(v.len() <= original.len());
        }
        assert!(Vec::<u64>::new().shrink().is_empty());
    }

    /// `quickcheck` reports `Ok` for a property that always holds.
    #[test]
    fn quickcheck_reports_ok_for_true_property() {
        let outcome = quickcheck::<_, u64>(DEFAULT_SEED, 32, |_x| true);
        match outcome {
            PropOutcome::Ok { runs } => assert_eq!(runs, 32),
            other => assert_eq!(other, PropOutcome::Ok { runs: 32 }),
        }
    }

    /// `quickcheck` reports a counterexample and converges on a small
    /// witness for a property that fails on any non-zero `u64`.
    #[test]
    fn quickcheck_shrinks_to_minimum_counterexample() {
        let outcome = quickcheck::<_, u64>(DEFAULT_SEED, 64, |x| x == 0);
        match outcome {
            PropOutcome::Counterexample {
                seed,
                runs: _,
                minimal,
            } => {
                assert_eq!(seed, DEFAULT_SEED);
                // The smallest non-zero `u64` is `1`. The shrinker
                // walks toward zero one step at a time and must
                // terminate at exactly `1`.
                assert_eq!(minimal, "1");
            }
            other => assert_eq!(
                other,
                PropOutcome::Counterexample {
                    seed: DEFAULT_SEED,
                    runs: 1,
                    minimal: "1".to_string(),
                }
            ),
        }
    }

    /// The default seed and runs constants match the documented
    /// defaults so external tests can refer to them by name.
    #[test]
    fn default_constants_are_stable() {
        assert_eq!(DEFAULT_SEED, 0xDEAD_BEEF_CAFE_F00D);
        assert_eq!(DEFAULT_RUNS, 64);
    }
}
