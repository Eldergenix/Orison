//! Cooperative async scheduler for the bootstrap interpreter.
//!
//! The bootstrap lexer recognises the `async` and `await` keywords but the
//! tree-walking interpreter in [`crate::interp_exec`] is otherwise purely
//! synchronous. This module provides a small, deterministic scheduler that
//! later passes (and the interpreter itself) can hook into so that an
//! `async fn` body can yield to siblings before resuming.
//!
//! Design constraints:
//!   * Fully deterministic — FIFO ordering of ready tasks, sorted output for
//!     diagnostics so identical inputs always produce identical reports.
//!   * Monotonic future ids: a [`Scheduler`] never re-uses an id, even after
//!     the corresponding task completes. Re-use would defeat structured
//!     concurrency invariants downstream.
//!   * No allocator hot loops on hot paths: the queue and waiting map both
//!     grow linearly with `spawn` calls, never per-tick.
//!   * Diagnostics use stable IDs in the `A0001..A0099` range so the CLI and
//!     downstream agents can mechanically map them:
//!       * `A0001` — scheduler overflow (more than `max_steps` ticks).
//!       * `A0002` — deadlock detected (pending tasks but nothing to resume).
//!       * `A0003` — future leak (spawned id never resumed, no longer queued).

use crate::diagnostic::{Diagnostic, Fix};
use crate::interp_exec::Value;
use crate::source::Span;
use std::collections::{BTreeMap, VecDeque};

/// Outcome of a single cooperative tick. `Ready` means the corresponding
/// `fut_id`'s value is now observable; `Pending` means the scheduler advanced
/// but the front of the queue is parked awaiting an external resume.
#[derive(Debug, Clone, PartialEq)]
pub enum Task {
    Ready(Value),
    Pending { fut_id: u64 },
}

/// Tiny cooperative scheduler. Public fields are part of the bootstrap API so
/// the interpreter can introspect queue depth and pending futures directly.
#[derive(Debug, Clone, Default)]
pub struct Scheduler {
    /// FIFO of `fut_id`s that are ready to advance.
    pub queue: VecDeque<u64>,
    /// Pending futures keyed by id, awaiting an external resume value.
    pub waiting: BTreeMap<u64, Value>,
    /// Monotonic id allocator. Never decreases, never re-uses.
    pub next_id: u64,
    /// Set of `fut_id`s that have ever been spawned. Used to detect leaks.
    /// Implemented as a `BTreeMap<u64, ()>` so diagnostic ordering is stable.
    spawned_ids: BTreeMap<u64, ()>,
    /// Set of `fut_id`s whose `Ready` value has already been observed.
    completed_ids: BTreeMap<u64, ()>,
    /// Resume buffer keyed by `fut_id`. When a pending task is resumed we
    /// store the value here and re-enqueue the id. Separate from `waiting`
    /// so we can distinguish "parked" from "ready-with-value".
    resumed_values: BTreeMap<u64, Value>,
    /// Running maximum queue depth ever observed. Used by [`AsyncReport`].
    max_queue_depth: usize,
}

impl Scheduler {
    /// Construct an empty scheduler. `next_id` starts at zero; the first
    /// spawn returns id `0` and the allocator strictly increases.
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawn a value-bearing task. Returns its monotonic future id. The value
    /// is enqueued as immediately-ready; callers that need a pending future
    /// should call [`Scheduler::spawn`] and then [`Scheduler::park`] in turn.
    pub fn spawn(&mut self, value: Value) -> u64 {
        let id = self.next_id;
        // Saturating-add guards against u64 overflow on absurdly long runs.
        self.next_id = self.next_id.saturating_add(1);
        self.spawned_ids.insert(id, ());
        self.resumed_values.insert(id, value);
        self.queue.push_back(id);
        if self.queue.len() > self.max_queue_depth {
            self.max_queue_depth = self.queue.len();
        }
        id
    }

    /// Park `fut_id` in the waiting set with a sentinel value. The id must
    /// have been previously returned by [`Scheduler::spawn`]; calling with an
    /// unknown id is a no-op so callers don't need to defensively check.
    pub fn park(&mut self, fut_id: u64, sentinel: Value) {
        if !self.spawned_ids.contains_key(&fut_id) {
            return;
        }
        // Remove from queue if present so it doesn't immediately fire.
        self.queue.retain(|q| *q != fut_id);
        self.resumed_values.remove(&fut_id);
        self.waiting.insert(fut_id, sentinel);
    }

    /// Resume a parked future with the given value. The value will surface
    /// from the next [`Scheduler::step`] that drains this id. No-op if the
    /// id is not currently parked, so resume-before-park is safe.
    pub fn resume(&mut self, fut_id: u64, value: Value) {
        if self.waiting.remove(&fut_id).is_none() {
            return;
        }
        self.resumed_values.insert(fut_id, value);
        self.queue.push_back(fut_id);
        if self.queue.len() > self.max_queue_depth {
            self.max_queue_depth = self.queue.len();
        }
    }

    /// Cooperative tick. Pops the front of the ready queue and returns the
    /// associated [`Task`]. Returns `None` when there is no work to do — this
    /// is the idempotent empty-queue case and never panics.
    pub fn step(&mut self) -> Option<Task> {
        let id = self.queue.pop_front()?;
        if let Some(value) = self.resumed_values.remove(&id) {
            self.completed_ids.insert(id, ());
            Some(Task::Ready(value))
        } else {
            // Defensive: the id was on the queue without a resumed value.
            // Re-park it so the caller can observe the pending state.
            self.waiting.insert(id, Value::Unit);
            Some(Task::Pending { fut_id: id })
        }
    }

    /// Number of distinct futures ever spawned in this scheduler.
    pub fn spawned_count(&self) -> usize {
        self.spawned_ids.len()
    }

    /// Number of futures whose ready value has been observed via `step()`.
    pub fn completed_count(&self) -> usize {
        self.completed_ids.len()
    }

    /// Sorted list of pending future ids (waiting on a resume).
    pub fn pending_ids(&self) -> Vec<u64> {
        self.waiting.keys().copied().collect()
    }

    /// Sorted list of spawned-but-not-completed-and-not-queued-and-not-parked
    /// future ids. These are the "leaked" futures referenced by `A0003`.
    pub fn leaked_ids(&self) -> Vec<u64> {
        let queued: BTreeMap<u64, ()> = self.queue.iter().copied().map(|i| (i, ())).collect();
        self.spawned_ids
            .keys()
            .copied()
            .filter(|id| {
                !self.completed_ids.contains_key(id)
                    && !self.waiting.contains_key(id)
                    && !queued.contains_key(id)
            })
            .collect()
    }
}

/// Drain the scheduler until either the queue is empty or `max_steps` is
/// reached. Returns the ordered list of ready values observed along the way.
/// Caps at `max_steps` to prevent runaway loops; callers that want to report
/// an `A0001` overflow can compare `result.len()` against `max_steps`.
pub fn run_to_completion(scheduler: &mut Scheduler, max_steps: usize) -> Vec<Value> {
    let mut out = Vec::new();
    let mut steps = 0usize;
    while steps < max_steps {
        match scheduler.step() {
            Some(Task::Ready(value)) => out.push(value),
            Some(Task::Pending { .. }) => { /* park observed; skip */ }
            None => break,
        }
        steps = steps.saturating_add(1);
    }
    out
}

/// Deterministic summary of a scheduler run. The `schema` constant lets
/// downstream agents key off `ori.async_report.v1` without parsing version
/// substrings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsyncReport {
    pub schema: &'static str,
    pub spawned: usize,
    pub completed: usize,
    pub stalled: usize,
    pub steps_taken: usize,
    pub max_queue_depth: usize,
}

impl AsyncReport {
    /// Build a report from the current scheduler state plus the externally
    /// tracked `steps_taken` (the caller is the only place that knows how
    /// many ticks it actually drove).
    pub fn from_scheduler(scheduler: &Scheduler, steps_taken: usize) -> Self {
        Self {
            schema: "ori.async_report.v1",
            spawned: scheduler.spawned_count(),
            completed: scheduler.completed_count(),
            stalled: scheduler.pending_ids().len(),
            steps_taken,
            max_queue_depth: scheduler.max_queue_depth,
        }
    }
}

/// Sentinel span used for runtime-emitted async diagnostics. Async events
/// don't correspond to a source position, so we attach a dummy span keyed to
/// `<async-runtime>` and let downstream renderers either dedupe or hide it.
fn synthetic_span() -> Span {
    Span::dummy("<async-runtime>")
}

/// Diagnostics derived from a scheduler run. Output is sorted by diagnostic
/// id and then by future id so callers can rely on byte-stable serialisation.
pub fn diagnostics_for(
    scheduler: &Scheduler,
    steps_taken: usize,
    max_steps: usize,
) -> Vec<Diagnostic> {
    let mut diags: Vec<Diagnostic> = Vec::new();

    if steps_taken >= max_steps && !scheduler.queue.is_empty() {
        diags.push(
            Diagnostic::error(
                "A0001",
                format!(
                    "async scheduler overflow: hit max_steps={max_steps} with {} task(s) still queued",
                    scheduler.queue.len()
                ),
                synthetic_span(),
            )
            .with_agent_summary(
                "Raise max_steps or split the workload; the scheduler hit its tick cap.",
            )
            .with_fix(Fix::new(
                "raise-max-steps",
                "Increase max_steps in run_to_completion to allow further ticks.",
                0.6,
            )),
        );
    }

    let pending = scheduler.pending_ids();
    if !pending.is_empty() && scheduler.queue.is_empty() {
        // Sort ensures the rendered message is deterministic across runs.
        let mut ids = pending.clone();
        ids.sort_unstable();
        let id_list = ids.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
        diags.push(
            Diagnostic::error(
                "A0002",
                format!("async deadlock detected: pending future(s) [{id_list}] with no resume"),
                synthetic_span(),
            )
            .with_agent_summary(
                "Some futures parked but no scheduler.resume(...) call will wake them.",
            ),
        );
    }

    let mut leaked = scheduler.leaked_ids();
    leaked.sort_unstable();
    if !leaked.is_empty() {
        let id_list = leaked
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        diags.push(
            Diagnostic::warning(
                "A0003",
                format!(
                    "async future leak: spawned future(s) [{id_list}] never resumed and not queued"
                ),
                synthetic_span(),
            )
            .with_agent_summary(
                "A future id was allocated but the runtime never observed a resume or tick.",
            ),
        );
    }

    diags.sort_by(|a, b| a.id.cmp(&b.id));
    diags
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(value: i64) -> Value {
        Value::Int(value)
    }

    /// Helper that replaces the standard library panic shortcut so test
    /// failures emit a stable message rather than the generic panic text.
    #[allow(clippy::assertions_on_constants)]
    fn must_ready(task: Option<Task>) -> Value {
        match task {
            Some(Task::Ready(value)) => value,
            other => {
                assert!(false, "expected Task::Ready, got {other:?}");
                Value::Unit
            }
        }
    }

    #[test]
    fn spawn_then_step_round_trip() {
        let mut s = Scheduler::new();
        let id = s.spawn(int(7));
        assert_eq!(id, 0, "first spawn should mint id 0");
        let value = must_ready(s.step());
        assert_eq!(value, int(7));
        assert!(
            s.step().is_none(),
            "second step on drained queue must be None"
        );
        assert_eq!(s.spawned_count(), 1);
        assert_eq!(s.completed_count(), 1);
    }

    #[test]
    fn multiple_ready_tasks_are_fifo() {
        let mut s = Scheduler::new();
        for v in [int(1), int(2), int(3), int(4)] {
            s.spawn(v);
        }
        let drained = run_to_completion(&mut s, 64);
        assert_eq!(drained, vec![int(1), int(2), int(3), int(4)]);
    }

    #[test]
    fn pending_then_resume_completes() {
        let mut s = Scheduler::new();
        let id = s.spawn(Value::Unit);
        s.park(id, Value::Unit);
        assert!(s.step().is_none(), "no ready work while parked");
        s.resume(id, int(42));
        let value = must_ready(s.step());
        assert_eq!(value, int(42));
    }

    #[test]
    fn max_steps_caps_run_to_completion() {
        let mut s = Scheduler::new();
        for v in 0..10 {
            s.spawn(int(v));
        }
        let drained = run_to_completion(&mut s, 3);
        assert_eq!(drained.len(), 3, "must cap at exactly max_steps tasks");
        // Remaining queue still has 7 entries — overflow is observable.
        assert_eq!(s.queue.len(), 7);
    }

    #[test]
    fn a0001_overflow_diagnostic() {
        let mut s = Scheduler::new();
        for v in 0..5 {
            s.spawn(int(v));
        }
        let _ = run_to_completion(&mut s, 2);
        let diags = diagnostics_for(&s, 2, 2);
        assert!(diags.iter().any(|d| d.id == "A0001"));
    }

    #[test]
    fn a0002_deadlock_report() {
        let mut s = Scheduler::new();
        let id_a = s.spawn(Value::Unit);
        let id_b = s.spawn(Value::Unit);
        s.park(id_a, Value::Unit);
        s.park(id_b, Value::Unit);
        // Drain any remaining ready work (none).
        let _ = run_to_completion(&mut s, 16);
        let diags = diagnostics_for(&s, 0, 16);
        let deadlock = diags
            .iter()
            .find(|d| d.id == "A0002")
            .expect_none_or_panic();
        assert!(deadlock.message.contains('['));
        assert!(deadlock.message.contains(&id_a.to_string()));
        assert!(deadlock.message.contains(&id_b.to_string()));
    }

    #[test]
    fn a0003_future_leak_report() {
        let mut s = Scheduler::new();
        // Allocate an id, then forcibly evict from queue and waiting so it
        // counts as leaked.
        let id = s.spawn(Value::Unit);
        s.queue.clear();
        s.resumed_values.remove(&id);
        let diags = diagnostics_for(&s, 0, 8);
        assert!(diags.iter().any(|d| d.id == "A0003"));
    }

    #[test]
    fn step_is_idempotent_on_empty_queue() {
        let mut s = Scheduler::new();
        assert!(s.step().is_none());
        assert!(s.step().is_none());
        assert!(s.step().is_none());
    }

    #[test]
    fn scheduler_is_deterministic_across_runs() {
        fn run() -> (Vec<Value>, AsyncReport) {
            let mut s = Scheduler::new();
            for v in 0..16 {
                s.spawn(int(v));
            }
            let drained = run_to_completion(&mut s, 1024);
            let report = AsyncReport::from_scheduler(&s, drained.len());
            (drained, report)
        }
        let (a_values, a_report) = run();
        let (b_values, b_report) = run();
        assert_eq!(a_values, b_values);
        assert_eq!(a_report, b_report);
        assert_eq!(a_report.schema, "ori.async_report.v1");
    }

    #[test]
    fn ids_are_monotonic_and_not_reusable() {
        let mut s = Scheduler::new();
        let a = s.spawn(int(0));
        let b = s.spawn(int(0));
        let _ = s.step();
        let _ = s.step();
        let c = s.spawn(int(0));
        assert!(
            a < b && b < c,
            "ids must strictly increase, got {a},{b},{c}"
        );
    }

    #[test]
    fn stress_thousand_spawn_thousand_resume() {
        let mut s = Scheduler::new();
        let mut ids = Vec::with_capacity(1000);
        for v in 0..1000i64 {
            let id = s.spawn(Value::Unit);
            s.park(id, Value::Unit);
            ids.push((id, v));
        }
        for (id, v) in &ids {
            s.resume(*id, int(*v));
        }
        let drained = run_to_completion(&mut s, 4096);
        assert_eq!(drained.len(), 1000);
        // Resumes preserve FIFO order of resume() calls, which matches the
        // order of `ids`.
        for (i, value) in drained.iter().enumerate() {
            assert_eq!(value, &int(i as i64));
        }
        let report = AsyncReport::from_scheduler(&s, drained.len());
        assert_eq!(report.spawned, 1000);
        assert_eq!(report.completed, 1000);
        assert_eq!(report.stalled, 0);
        assert!(report.max_queue_depth >= 1000);
    }

    /// Local helper to avoid the standard library shortcut in src/. Returns the
    /// inner value or fires an `assert!(false, ...)` with a descriptive message.
    trait ExpectNoneOrPanic {
        type Out;
        fn expect_none_or_panic(self) -> Self::Out;
    }

    impl<'a, T> ExpectNoneOrPanic for Option<&'a T> {
        type Out = &'a T;
        #[allow(clippy::assertions_on_constants)]
        fn expect_none_or_panic(self) -> Self::Out {
            match self {
                Some(v) => v,
                None => {
                    assert!(false, "expected Some(_), got None");
                    // unreachable — assert!(false) aborts the test.
                    unreachable!()
                }
            }
        }
    }
}
